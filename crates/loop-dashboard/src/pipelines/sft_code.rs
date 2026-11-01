//! Source-code adapter for the `sftqc` project: a read-only code explorer
//! over the Python files the pipeline actually runs.
//!
//! The dashboard never executes the pipeline; this module only lists and
//! reads the `.py` files under `<root>/sftqc/`, extracting top-level
//! symbols (`def` / `class`) with their line numbers so the UI can jump
//! straight to a function. Files are bounded in size (see [`MAX_FILE_BYTES`])
//! and only `.py` files directly inside the project directory are listed —
//! no recursion, no dataset contents.

use std::path::PathBuf;

use serde::Serialize;

use super::{IntegrationState, PipelinePaths};

/// Hard cap on served file bytes (the project's largest file is ~17 KB;
/// 512 KB leaves enormous headroom while bounding memory).
pub(crate) const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Files surfaced by the explorer, in documented reading order (pipeline
/// first). Anything else found on disk is still listable but sorts after.
const KNOWN_ORDER: &[&str] = &[
    "pipeline.py",
    "rules.py",
    "lang.py",
    "lm_score.py",
    "dedup.py",
    "judge.py",
    "selftest.py",
    "gym_check.py",
    "__init__.py",
];

/// One top-level symbol (function or class) in a Python file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSymbol {
    /// `def` or `class`.
    pub kind: String,
    /// Symbol name.
    pub name: String,
    /// 1-based line where it starts.
    pub line: u32,
}

/// Metadata for one source file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeFileMeta {
    /// File name, e.g. `rules.py`.
    pub name: String,
    /// Absolute path on this machine.
    pub path: String,
    /// Line count (from the served content).
    pub lines: u32,
    /// Bytes on disk.
    pub bytes: u64,
    /// Top-level symbols, in file order.
    pub symbols: Vec<CodeSymbol>,
    /// Module docstring (first statement), when present.
    pub doc: Option<String>,
}

/// Payload for `GET /api/pipelines/sft/code`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftCodeIndex {
    /// Connectivity state of the workspace.
    pub integration: IntegrationState,
    /// Absolute project path.
    pub project_path: String,
    /// Known files, reading order first, then anything else discovered.
    pub files: Vec<CodeFileMeta>,
}

/// Extract the module docstring from Python source, if the first statement
/// is a string literal. Handles both `"""..."""` and `'...'` forms.
fn module_doc(src: &str) -> Option<String> {
    let mut rest = src.trim_start_matches('\u{feff}');
    // Skip leading comments and blank lines before the docstring.
    loop {
        let trimmed = rest.trim_start();
        if trimmed.starts_with('#') {
            rest = trimmed.split_once('\n').map(|(_, r)| r).unwrap_or("");
        } else {
            rest = trimmed;
            break;
        }
    }
    let triple = |q: &str| {
        rest.starts_with(q)
            .then(|| {
                rest[q.len()..]
                    .split(q)
                    .next()
                    .map(|d| d.trim().to_string())
            })
            .flatten()
    };
    triple("\"\"\"").or_else(|| triple("'''"))
}

/// Extract top-level `def` / `class` declarations with their line numbers.
/// Only column-0 declarations count: methods inside classes are reached
/// through their owning class.
fn top_level_symbols(src: &str) -> Vec<CodeSymbol> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim_end();
        let is_top = line.starts_with("def ") || line.starts_with("class ");
        if !is_top {
            continue;
        }
        // Decorated defs keep their own line; skip async? No — async def at
        // column 0 also declares a symbol.
        let name = line
            .trim_start_matches("async ")
            .trim_start_matches("def ")
            .trim_start_matches("class ")
            .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
            .find(|s| !s.is_empty())
            .map(String::from);
        if let Some(name) = name {
            out.push(CodeSymbol {
                kind: if line.starts_with("class ") {
                    "class"
                } else {
                    "def"
                }
                .into(),
                name,
                line: i as u32 + 1,
            });
        }
    }
    out
}

/// Read one file into metadata. Returns `None` when unreadable or oversized.
fn load_file(path: &PathBuf) -> Option<CodeFileMeta> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let src = std::fs::read_to_string(path).ok()?;
    let name = path.file_name()?.to_str()?.to_string();
    if !name.ends_with(".py") {
        return None;
    }
    Some(CodeFileMeta {
        name,
        path: path.display().to_string(),
        lines: src.lines().count() as u32,
        bytes: meta.len(),
        symbols: top_level_symbols(&src),
        doc: module_doc(&src),
    })
}

/// Build the code index for a workspace root.
pub fn code_index(paths: &PipelinePaths) -> SftCodeIndex {
    let project = paths.sft_project();
    let mut files: Vec<CodeFileMeta> = std::fs::read_dir(&project)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter_map(|p| load_file(&p))
                .collect()
        })
        .unwrap_or_default();

    // Documented reading order first; unknown files alphabetically after.
    files.sort_by(|a, b| {
        let ka = KNOWN_ORDER.iter().position(|n| *n == a.name);
        let kb = KNOWN_ORDER.iter().position(|n| *n == b.name);
        match (ka, kb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        }
    });

    let integration = if files.is_empty() {
        IntegrationState::NotConfigured
    } else {
        IntegrationState::Connected
    };

    SftCodeIndex {
        integration,
        project_path: project.display().to_string(),
        files,
    }
}

/// Serve one source file's full text. Errors are honest strings, never
/// panics; traversal outside the project directory is refused.
pub fn read_source(paths: &PipelinePaths, name: &str) -> Result<String, String> {
    if name.contains('/') || name.contains('\\') || !name.ends_with(".py") {
        return Err(format!("invalid file name {name:?}"));
    }
    let project = paths.sft_project();
    if !project.is_dir() {
        return Err(format!("sftqc project not found at {}", project.display()));
    }
    let path = project.join(name);
    // Belt-and-braces: the joined path must stay inside the project.
    if !path.starts_with(&project) {
        return Err("path escapes the project directory".into());
    }
    let meta = std::fs::metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: not a file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{name}: too large ({} bytes)", meta.len()));
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("sftqc");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("pipeline.py"),
            "\"\"\"Doc.\"\"\"\n\ndef run(path):\n    pass\n\nclass Row:\n    def method(self):\n        pass\n",
        )
        .unwrap();
        std::fs::write(
            proj.join("rules.py"),
            "# comment\nFLAG = 1\n\ndef check_row(obj):\n    return []\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn missing_project_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let idx = code_index(&PipelinePaths::at(dir.path()));
        assert_eq!(idx.integration, IntegrationState::NotConfigured);
        assert!(idx.files.is_empty());
    }

    #[test]
    fn files_are_ordered_and_symbols_extracted() {
        let dir = fixture();
        let idx = code_index(&PipelinePaths::at(dir.path()));
        let names: Vec<&str> = idx.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["pipeline.py", "rules.py"]);
        assert_eq!(idx.files[0].doc.as_deref(), Some("Doc."));
        assert_eq!(
            idx.files[0].symbols.len(),
            2,
            "methods inside classes excluded"
        );
        assert_eq!(idx.files[0].symbols[0].name, "run");
        assert_eq!(idx.files[0].symbols[0].line, 3);
        assert_eq!(idx.files[0].symbols[1].name, "Row");
        // Leading comment skipped before the docstring lookup.
        assert_eq!(idx.files[1].doc, None);
        assert_eq!(idx.files[1].symbols[0].name, "check_row");
    }

    #[test]
    fn read_source_serves_text_and_refuses_traversal() {
        let dir = fixture();
        let paths = PipelinePaths::at(dir.path());
        let src = read_source(&paths, "rules.py").unwrap();
        assert!(src.contains("check_row"));
        assert!(read_source(&paths, "../secrets.py").is_err());
        assert!(read_source(&paths, "sub/dir.py").is_err());
        assert!(read_source(&paths, "missing.py").is_err());
    }
}
