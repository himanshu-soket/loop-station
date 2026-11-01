//! The kit's file catalog: every resource in the evaluation kit, classified
//! by what it is and where it belongs in the workspaces.
//!
//! Discovery is a bounded recursive walk (`walkdir` is already a workspace
//! dependency) that records path, size, kind and category. Nothing is read
//! except small text files whose *preview* the UI shows; dataset contents
//! are served separately through [`super::files`].

use std::path::Path;

use serde::Serialize;

use super::EvalPaths;

/// Hard bound on walked entries, so a pathological directory cannot stall
/// discovery.
const MAX_ENTRIES: usize = 5_000;

/// What a file is, as far as the workspaces are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// JSONL rows (banks, examples, cached answers).
    Jsonl,
    /// JSON document (configs, reports).
    Json,
    /// Tab-separated table.
    Tsv,
    /// Comma-separated table.
    Csv,
    /// Parquet columnar dataset.
    Parquet,
    /// Python source.
    Python,
    /// Shell script.
    Shell,
    /// Slurm batch script.
    Sbatch,
    /// Markdown documentation.
    Markdown,
    /// HTML report or template.
    Html,
    /// Plain text.
    Text,
    /// Anything else.
    Other,
}

impl ResourceKind {
    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Jsonl => "JSONL",
            Self::Json => "JSON",
            Self::Tsv => "TSV",
            Self::Csv => "CSV",
            Self::Parquet => "Parquet",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::Sbatch => "Slurm",
            Self::Markdown => "Markdown",
            Self::Html => "HTML",
            Self::Text => "Text",
            Self::Other => "File",
        }
    }

    fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        match lower.rsplit_once('.').map(|(_, ext)| ext) {
            Some("jsonl") => Self::Jsonl,
            Some("json") => Self::Json,
            Some("tsv") | Some("tab") => Self::Tsv,
            Some("csv") => Self::Csv,
            Some("parquet") | Some("pq") => Self::Parquet,
            Some("py") => Self::Python,
            Some("sh") | Some("bash") | Some("zsh") => Self::Shell,
            Some("sbatch") | Some("slurm") => Self::Sbatch,
            Some("md") | Some("markdown") => Self::Markdown,
            Some("html") | Some("htm") => Self::Html,
            Some("txt") | Some("cfg") | Some("conf") | Some("ini") | Some("yaml") | Some("yml")
            | Some("toml") => Self::Text,
            _ => Self::Other,
        }
    }
}

/// Which workspace a resource belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Workspace {
    /// Question banks and custom-question files.
    Banks,
    /// Datasets (row files: JSONL/CSV/TSV/Parquet).
    Datasets,
    /// Benchmarks: run configurations and bank presets.
    Benchmarks,
    /// Model evaluation outputs (runs, reports, summaries).
    Evaluations,
    /// Reports and analytics artifacts.
    Reports,
    /// Scripts, configs, tests and docs.
    Tools,
}

impl Workspace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Banks => "banks",
            Self::Datasets => "datasets",
            Self::Benchmarks => "benchmarks",
            Self::Evaluations => "evaluations",
            Self::Reports => "reports",
            Self::Tools => "tools",
        }
    }
}

/// One discovered resource.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// Stable id: the path relative to the kit root, URL-safe.
    pub id: String,
    /// File name.
    pub name: String,
    /// Path relative to the kit root (`internal_eval/…`).
    pub rel_path: String,
    /// Absolute path on this machine.
    pub abs_path: String,
    /// Bytes on disk.
    pub bytes: u64,
    /// Kind classification.
    pub kind: ResourceKind,
    /// Owning workspace.
    pub workspace: Workspace,
    /// Directory the file sits in, relative to the kit.
    pub dir: String,
    /// Last-modified time, unix ms (0 when unknown).
    pub modified_ms: u64,
    /// Row count for line-oriented files, when cheaply known.
    pub rows: Option<u64>,
}

impl Resource {
    /// True when the browser can preview this file's contents.
    pub fn previewable(&self) -> bool {
        matches!(
            self.kind,
            ResourceKind::Jsonl
                | ResourceKind::Json
                | ResourceKind::Tsv
                | ResourceKind::Csv
                | ResourceKind::Markdown
                | ResourceKind::Python
                | ResourceKind::Shell
                | ResourceKind::Sbatch
                | ResourceKind::Html
                | ResourceKind::Text
        )
    }
}

/// Classify a file by its place in the kit plus its extension.
fn classify(rel: &str, kind: ResourceKind) -> Workspace {
    let lower = rel.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    // Documentation first: *.md under any directory is a doc.
    if matches!(kind, ResourceKind::Markdown) {
        return Workspace::Tools;
    }
    // Run outputs: measurement artifacts are reports, everything else
    // (configuration, answers, grades) belongs to the evaluation itself.
    if lower.contains("/runs/") || lower.starts_with("runs/") {
        let is_report = matches!(kind, ResourceKind::Html)
            || matches!(name, "report.json" | "summary.json" | "graded_flat.csv");
        return if is_report {
            Workspace::Reports
        } else {
            Workspace::Evaluations
        };
    }
    // Question banks and custom questions.
    if lower.starts_with("banks/") {
        return Workspace::Banks;
    }
    if lower.starts_with("examples/") {
        // Custom-question files are question banks; reference tables and
        // anything else in examples/ stay tools.
        return match kind {
            ResourceKind::Jsonl => Workspace::Banks,
            _ => Workspace::Tools,
        };
    }
    // Cached comparison answers are datasets.
    if lower.starts_with("compare_cache/") {
        return Workspace::Datasets;
    }
    // Everything else with rows is a dataset.
    match kind {
        ResourceKind::Jsonl | ResourceKind::Csv | ResourceKind::Tsv | ResourceKind::Parquet => {
            Workspace::Datasets
        }
        // Configurations feed the benchmark view.
        ResourceKind::Json if lower.contains("config") => Workspace::Benchmarks,
        _ => Workspace::Tools,
    }
}

/// Count lines in a text file without loading it: a bounded streaming read.
/// Only called for files under [`LINE_COUNT_LIMIT_BYTES`].
fn count_lines(path: &Path) -> Option<u64> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, f);
    let mut n: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf).ok()?;
        if read == 0 {
            break;
        }
        if !buf.iter().all(|b| b.is_ascii_whitespace()) {
            n += 1;
        }
    }
    Some(n)
}

/// Files large enough that counting their lines would be wasteful on every
/// request; their row counts come from the bank index instead.
const LINE_COUNT_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

/// Walk the kit and classify every file. Bounded and non-panicking.
pub fn catalog(paths: &EvalPaths) -> Vec<Resource> {
    let kit = paths.kit();
    let mut out: Vec<Resource> = Vec::new();
    if !kit.is_dir() {
        return out;
    }
    for entry in walkdir::WalkDir::new(&kit)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .take(MAX_ENTRIES)
    {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Hidden files and editor droppings are not resources.
        if name.starts_with('.') {
            continue;
        }
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let rel = path.strip_prefix(&kit).unwrap_or(path);
        let rel_path = rel.display().to_string();
        let kind = ResourceKind::from_name(&name);
        let workspace = classify(&rel_path, kind);
        let rows = (matches!(
            kind,
            ResourceKind::Jsonl | ResourceKind::Csv | ResourceKind::Tsv
        ) && meta.len() <= LINE_COUNT_LIMIT_BYTES)
            .then(|| count_lines(path))
            .flatten();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        out.push(Resource {
            id: rel_path.replace('/', "::"),
            name,
            rel_path,
            abs_path: path.display().to_string(),
            bytes: meta.len(),
            kind,
            workspace,
            dir: rel
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            modified_ms,
            rows,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

/// Aggregate counts per workspace, for the portal cards.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSummary {
    pub files: u64,
    pub bytes: u64,
    pub banks: u64,
    pub bank_questions: u64,
    pub datasets: u64,
    pub benchmarks: u64,
    pub evaluations: u64,
    pub reports: u64,
    pub tools: u64,
    pub runs: u64,
    pub scripts: u64,
    pub docs: u64,
}

/// Summarize a catalog into card-level numbers.
pub fn summarize(resources: &[Resource]) -> CatalogSummary {
    let mut s = CatalogSummary {
        files: resources.len() as u64,
        ..CatalogSummary::default()
    };
    for r in resources {
        s.bytes += r.bytes;
        match r.workspace {
            Workspace::Banks => s.banks += 1,
            Workspace::Datasets => s.datasets += 1,
            Workspace::Benchmarks => s.benchmarks += 1,
            Workspace::Evaluations => s.evaluations += 1,
            Workspace::Reports => s.reports += 1,
            Workspace::Tools => s.tools += 1,
        }
        match r.kind {
            ResourceKind::Python | ResourceKind::Shell | ResourceKind::Sbatch => s.scripts += 1,
            ResourceKind::Markdown => s.docs += 1,
            _ => {}
        }
    }
    // Distinct run directories (second path component under runs/).
    let mut runs: Vec<&str> = Vec::new();
    for r in resources {
        if let Some(rest) = r.rel_path.strip_prefix("runs/") {
            let top = rest.split('/').next().unwrap_or("");
            if !top.is_empty() && !runs.contains(&top) {
                runs.push(top);
            }
        }
    }
    s.runs = runs.len() as u64;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path().join("internal_eval");
        for sub in [
            "banks",
            "examples",
            "pipeline",
            "slurm",
            "config",
            "compare_cache",
        ] {
            std::fs::create_dir_all(k.join(sub)).unwrap();
        }
        std::fs::write(k.join("banks/qbank.jsonl"), "{}\n{}\n").unwrap();
        std::fs::write(k.join("examples/custom_questions.jsonl"), "{}\n").unwrap();
        std::fs::write(k.join("examples/refs.tsv"), "a\tb\n").unwrap();
        std::fs::write(k.join("pipeline/judge.py"), "# stage\n").unwrap();
        std::fs::write(k.join("slurm/gen.sbatch"), "#SBATCH\n").unwrap();
        std::fs::write(k.join("README.md"), "# doc\n").unwrap();
        std::fs::write(k.join("config/exclude_nodes.txt"), "node1\n").unwrap();
        std::fs::write(k.join(".hidden"), "x").unwrap();
        dir
    }

    #[test]
    fn classifies_resources_into_workspaces() {
        let dir = kit();
        let res = catalog(&EvalPaths::at(dir.path()));
        let find = |name: &str| res.iter().find(|r| r.name == name).unwrap();

        assert_eq!(find("qbank.jsonl").workspace, Workspace::Banks);
        assert_eq!(find("custom_questions.jsonl").workspace, Workspace::Banks);
        assert_eq!(find("refs.tsv").workspace, Workspace::Tools);
        assert_eq!(find("judge.py").workspace, Workspace::Tools);
        assert_eq!(find("gen.sbatch").workspace, Workspace::Tools);
        assert_eq!(find("README.md").workspace, Workspace::Tools);
        assert_eq!(find("exclude_nodes.txt").workspace, Workspace::Tools);

        assert_eq!(res.iter().filter(|r| r.name == ".hidden").count(), 0);
        assert_eq!(find("qbank.jsonl").rows, Some(2));
        assert_eq!(find("qbank.jsonl").id, "banks::qbank.jsonl");
    }

    #[test]
    fn run_outputs_split_into_evaluations_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path().join("internal_eval/runs/EKA_x/config.json");
        std::fs::create_dir_all(k.parent().unwrap()).unwrap();
        std::fs::write(&k, "{}").unwrap();
        let html = dir.path().join("internal_eval/runs/EKA_x/summary.html");
        std::fs::write(&html, "<html></html>").unwrap();
        let res = catalog(&EvalPaths::at(dir.path()));
        let cfg = res.iter().find(|r| r.name == "config.json").unwrap();
        let sum = res.iter().find(|r| r.name == "summary.html").unwrap();
        assert_eq!(cfg.workspace, Workspace::Evaluations);
        assert_eq!(sum.workspace, Workspace::Reports);
        let s = summarize(&res);
        assert_eq!(s.runs, 1);
        assert_eq!(s.evaluations, 1);
        assert_eq!(s.reports, 1);
    }

    #[test]
    fn kinds_map_from_extensions() {
        assert_eq!(ResourceKind::from_name("a.jsonl"), ResourceKind::Jsonl);
        assert_eq!(ResourceKind::from_name("a.PARQUET"), ResourceKind::Parquet);
        assert_eq!(ResourceKind::from_name("a.sbatch"), ResourceKind::Sbatch);
        assert_eq!(ResourceKind::from_name("noext"), ResourceKind::Other);
    }

    #[test]
    fn empty_kit_yields_empty_catalog() {
        let dir = tempfile::tempdir().unwrap();
        assert!(catalog(&EvalPaths::at(dir.path())).is_empty());
        assert_eq!(summarize(&[]).files, 0);
    }
}
