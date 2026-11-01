//! SFT data-quality pipeline adapter (`sftqc`).
//!
//! `sftqc` (in `~/soket/sftqc`) is a 4-layer quality gate over OpenAI-format
//! JSONL: rules → LM audit → embeddings near-dup → optional LLM judge. Each
//! run writes, into its output directory:
//!
//! ```text
//! <out>/report.json    machine-readable summary (rows, layers, languages…)
//! <out>/report.md      human summary
//! <out>/scores.jsonl   one scoring record per input row
//! <out>/high/<name>.jsonl   training-ready rows that passed
//! <out>/low/<name>.jsonl    rejected rows
//! ```
//!
//! The dashboard surfaces each *discovered run* as a stage chain derived from
//! the layers the report says actually ran (`settings.model` null ⇒ the LM
//! layer was skipped with `--no-lm`, etc.). Counts come from `report.json`
//! only — the JSONL outputs are never read, so a run over millions of rows
//! costs one small JSON parse.
//!
//! Execution state is inferred from filesystem facts only: a run is
//! "completed" when `report.json` exists, and "running" is never claimed
//! because nothing observes the process. That distinction is surfaced in the
//! payload (`execution: "completed"` vs unknown) rather than guessed.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{read_json, IntegrationState, PipelinePaths};

/// One discovered SFT QC run (an output directory containing report.json).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftRun {
    /// Run id: output directory name.
    pub id: String,
    /// Absolute path of the run's output directory.
    pub path: String,
    /// Input dataset label from the report (`input` field).
    pub input: Option<String>,
    /// Rows processed.
    pub rows: Option<u64>,
    /// Rows routed to `high/` (training-ready).
    pub high: Option<u64>,
    /// Rows routed to `low/`.
    pub low: Option<u64>,
    /// Low rows that also carried hard errors.
    pub low_with_errors: Option<u64>,
    /// Rows dropped by the language filter.
    pub filtered_by_lang: Option<u64>,
    /// Wall-clock seconds reported by the run.
    pub seconds: Option<f64>,
    /// Scoring model used by the LM layer, when it ran.
    pub model: Option<String>,
    /// Language restriction (`--lang`), when set.
    pub lang_filter: Option<String>,
    /// Language → row count.
    pub languages: Vec<LanguageCount>,
    /// Layers that actually executed (derived from report settings).
    pub layers: Vec<String>,
    /// Output dataset names found under `high/`.
    pub outputs: Vec<String>,
    /// mtime of report.json, unix ms — when this run was produced.
    pub completed_at_ms: u64,
    /// Error-flag code → row count (`errors` in report.json).
    pub errors: Vec<FlagCount>,
    /// Warning-flag code → row count (`warnings` in report.json).
    pub warnings: Vec<FlagCount>,
    /// Rows per layer of origin (`flags_by_layer` in report.json).
    pub flags_by_layer: Vec<FlagCount>,
    /// Rows with errors that were routed low (`low_with_errors`).
    pub settings: RunSettings,
}

/// A flag-code bucket within a run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagCount {
    pub code: String,
    pub rows: u64,
}

/// The thresholds and switches one run recorded in its report.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSettings {
    /// Repairable problems fixed in place before checking (`--fix`).
    pub fix: Option<bool>,
    /// Where warned (not failed) rows go: "low" or "high".
    pub warn_to: Option<String>,
    /// Minimum composite score to stay in high/.
    pub min_score: Option<f64>,
    /// Junk-token rule: minimum symbol-token loss.
    pub junk_nll: Option<f64>,
    /// Junk-token rule: max probability on any symbol there.
    pub junk_psym: Option<f64>,
    /// Junk-token rule: min loss above the row's median token loss.
    pub junk_margin: Option<f64>,
    /// Robust z-score limit for unnatural-text flags.
    pub z_limit: Option<f64>,
}

/// A language bucket within a run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCount {
    pub language: String,
    pub rows: u64,
}

/// Summary payload for `/api/pipelines/sft`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftSummary {
    /// Integration connectivity (project present? runs found?).
    pub integration: IntegrationState,
    /// Human label for the connectivity badge.
    pub integration_label: String,
    /// Path of the sftqc project on this machine, when present.
    pub project_path: Option<String>,
    /// Discovered runs, newest first (bounded).
    pub runs: Vec<SftRun>,
    /// Newest run across all roots, for overview cards.
    pub latest: Option<Box<SftRun>>,
    /// Totals across runs (definitions documented in the UI).
    pub totals: SftTotals,
    /// Measured selftest runs (catch rates), newest first.
    pub selftests: Vec<SelftestRun>,
    /// Why discovery failed, when it did.
    pub unavailable_reason: Option<String>,
}

/// Aggregates across discovered runs. Counts are row-level and additive only
/// because every run processes its own input file; they are labelled as
/// "across N runs" in the UI rather than presented as one dataset.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftTotals {
    pub runs: u64,
    pub rows: u64,
    pub high: u64,
    pub low: u64,
}

/// Candidate run directories, in scan order. `<root>` itself plus any
/// `qc_out*` / `selftest_*` directories — the conventions visible in the
/// workspace — bounded to keep discovery cheap.
fn candidate_run_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if root.is_dir() {
        dirs.push(root.to_path_buf());
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut children: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                n.starts_with("qc_out") || n.starts_with("selftest_")
            })
            .collect();
        children.sort();
        dirs.extend(children);
    }
    // Hard bound on discovery work.
    dirs.truncate(24);
    dirs
}

/// Read a `{code: rows}` object into sorted pairs.
fn flag_counts(v: Option<&serde_json::Value>) -> Vec<FlagCount> {
    let mut out: Vec<FlagCount> = v
        .and_then(|x| x.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, val)| FlagCount {
                    code: k.clone(),
                    rows: val.as_u64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.rows.cmp(&a.rows).then(a.code.cmp(&b.code)));
    out
}

fn read_settings(s: &serde_json::Value) -> RunSettings {
    let g = |k: &str| s.get(k);
    RunSettings {
        fix: g("fix").and_then(|v| v.as_bool()),
        warn_to: super::json_str(s, "warn_to"),
        min_score: g("min_score").and_then(|v| v.as_f64()),
        junk_nll: g("junk_nll").and_then(|v| v.as_f64()),
        junk_psym: g("junk_psym").and_then(|v| v.as_f64()),
        junk_margin: g("junk_margin").and_then(|v| v.as_f64()),
        z_limit: g("z_limit").and_then(|v| v.as_f64()),
    }
}

/// Read one run directory into an [`SftRun`]. Returns `None` when the
/// directory has no `report.json` (not a run output).
fn load_run(dir: &Path) -> Option<SftRun> {
    let report_path = dir.join("report.json");
    if !report_path.is_file() {
        return None;
    }
    let completed_at_ms = report_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // A malformed report degrades to a stub entry naming the run, never a
    // 500 for the whole endpoint.
    let parsed = read_json(&report_path).ok()?;
    let settings = parsed.get("settings").cloned().unwrap_or_default();

    let languages = parsed
        .get("languages")
        .and_then(|l| l.as_object())
        .map(|obj| {
            let mut v: Vec<LanguageCount> = obj
                .iter()
                .map(|(k, val)| LanguageCount {
                    language: k.clone(),
                    rows: val.as_u64().unwrap_or(0),
                })
                .collect();
            v.sort_by(|a, b| b.rows.cmp(&a.rows).then(a.language.cmp(&b.language)));
            v
        })
        .unwrap_or_default();

    // Which layers ran: the report records the model actually used; a null
    // model means --no-lm. Embeddings/judge have no per-run marker in the
    // report, so only layers we can prove are listed.
    let model = settings
        .get("model")
        .and_then(|m| m.as_str())
        .map(String::from);
    let mut layers = vec!["rules".to_string(), "language".to_string()];
    if model.is_some() {
        layers.push("lm-audit".to_string());
    }
    if let Some(lang) = settings.get("lang").and_then(|l| l.as_str()) {
        if !lang.is_empty() {
            layers.push(format!("lang-filter:{lang}"));
        }
    }

    // Output datasets under high/: names only, contents never read.
    let outputs = super::list_file_names(&dir.join("high"));

    Some(SftRun {
        id: dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("run")
            .to_string(),
        path: dir.display().to_string(),
        input: super::json_str(&parsed, "input"),
        rows: super::json_u64(&parsed, "rows"),
        high: super::json_u64(&parsed, "high"),
        low: super::json_u64(&parsed, "low"),
        low_with_errors: super::json_u64(&parsed, "low_with_errors"),
        filtered_by_lang: super::json_u64(&parsed, "filtered_by_lang"),
        seconds: super::json_f64(&parsed, "seconds"),
        model,
        lang_filter: settings
            .get("lang")
            .and_then(|l| l.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        languages,
        layers,
        outputs,
        completed_at_ms,
        errors: flag_counts(parsed.get("errors")),
        warnings: flag_counts(parsed.get("warnings")),
        flags_by_layer: flag_counts(parsed.get("flags_by_layer")),
        settings: read_settings(&settings),
    })
}

/// One measured injection type from a selftest run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelftestKind {
    /// Injection kind (`clean`, `stray_qmark`, `mojibake`, …).
    pub kind: String,
    /// Injected rows caught by the expected check.
    pub caught: Option<u64>,
    /// Injected rows that ended in low/ (any reason).
    pub low: Option<u64>,
    /// Total rows of this kind.
    pub total: u64,
}

impl SelftestKind {
    /// Catch rate 0..=1 when both numerator and denominator exist.
    pub fn catch_rate(&self) -> f64 {
        match (self.caught, self.total) {
            (Some(c), t) if t > 0 => c as f64 / t as f64,
            _ => 0.0,
        }
    }
}

/// One selftest run (`selftest_*/selftest_results.json`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelftestRun {
    /// Directory name, e.g. `selftest_0.5b`.
    pub id: String,
    /// Absolute path of the results file.
    pub path: String,
    /// mtime of the results file, unix ms.
    pub completed_at_ms: u64,
    /// Clean rows wrongly carrying error flags (false positives).
    pub clean_rows_in_low: Option<u64>,
    /// Clean rows tested.
    pub clean_total: Option<u64>,
    /// Per-injection-kind measurements.
    pub kinds: Vec<SelftestKind>,
    /// Number of threshold-sweep settings tried.
    pub sweep_points: u64,
}

/// Discover `selftest_*` directories under the workspace root and read each
/// `selftest_results.json`. The shape is whatever `sftqc.selftest` wrote:
/// `{"<kind>": {"caught"?, "low"?, "total", ...}, "junk_sweep": [...]}`.
fn load_selftests(root: &Path) -> Vec<SelftestRun> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("selftest_") || !p.is_dir() {
            continue;
        }
        let res = p.join("selftest_results.json");
        if !res.is_file() {
            continue;
        }
        let completed_at_ms = res
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // A malformed results file still lists the run with no kinds —
        // honest absence rather than a dropped entry or a 500.
        let parsed = read_json(&res).unwrap_or(serde_json::Value::Null);
        let mut kinds = Vec::new();
        let mut clean_rows_in_low = None;
        let mut clean_total = None;
        let mut sweep_points = 0;
        if let Some(obj) = parsed.as_object() {
            for (k, v) in obj {
                if k == "junk_sweep" {
                    sweep_points = v.as_array().map(|a| a.len() as u64).unwrap_or(0);
                    continue;
                }
                let total = v.get("total").and_then(|t| t.as_u64());
                if k == "clean" {
                    clean_rows_in_low = v.get("rows_in_low").and_then(|t| t.as_u64());
                    clean_total = total;
                    continue;
                }
                kinds.push(SelftestKind {
                    kind: k.clone(),
                    caught: v.get("caught").and_then(|t| t.as_u64()),
                    low: v.get("low").and_then(|t| t.as_u64()),
                    total: total.unwrap_or(0),
                });
            }
        }
        kinds.sort_by(|a, b| a.kind.cmp(&b.kind));
        out.push(SelftestRun {
            id: name.to_string(),
            path: res.display().to_string(),
            completed_at_ms,
            clean_rows_in_low,
            clean_total,
            kinds,
            sweep_points,
        });
    }
    out.sort_by(|a, b| b.completed_at_ms.cmp(&a.completed_at_ms));
    out.reverse(); // newest first
    out
}

/// Build the SFT summary for a workspace root.
pub fn summarize(paths: &PipelinePaths) -> SftSummary {
    let project = paths.sft_project();
    if !project.is_dir() {
        return SftSummary {
            integration: IntegrationState::NotConfigured,
            integration_label: IntegrationState::NotConfigured.label().into(),
            project_path: None,
            runs: Vec::new(),
            latest: None,
            totals: SftTotals::default(),
            selftests: Vec::new(),
            unavailable_reason: Some(format!(
                "sftqc project not found at {}. The SFT workspace reads run reports the \
                 pipeline writes; install or point SOKET_PIPELINE_ROOT at the workspace.",
                project.display()
            )),
        };
    }

    let mut runs: Vec<SftRun> = candidate_run_dirs(&paths.root)
        .iter()
        .filter_map(|d| load_run(d))
        .collect();
    // Newest first.
    runs.sort_by(|a, b| b.completed_at_ms.cmp(&a.completed_at_ms));

    let totals = SftTotals {
        runs: runs.len() as u64,
        rows: runs.iter().filter_map(|r| r.rows).sum(),
        high: runs.iter().filter_map(|r| r.high).sum(),
        low: runs.iter().filter_map(|r| r.low).sum(),
    };
    let integration = if runs.is_empty() {
        IntegrationState::Idle
    } else {
        IntegrationState::Connected
    };

    SftSummary {
        integration,
        integration_label: integration.label().into(),
        project_path: Some(project.display().to_string()),
        latest: runs.first().cloned().map(Box::new),
        totals,
        selftests: load_selftests(&paths.root),
        runs,
        unavailable_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("sftqc");
        std::fs::create_dir_all(proj.join("sftqc")).unwrap();
        // Project dir present.
        std::fs::write(proj.join("sftqc/pipeline.py"), "# pipeline").unwrap();
        dir
    }

    fn write_run(root: &Path, name: &str, rows: u64, high: u64) {
        let out = root.join(name);
        std::fs::create_dir_all(out.join("high")).unwrap();
        std::fs::create_dir_all(out.join("low")).unwrap();
        std::fs::write(
            out.join("report.json"),
            format!(
                r#"{{
                  "input": "data.jsonl",
                  "rows": {rows},
                  "settings": {{"model": "Qwen/Qwen2.5-1.5B-Instruct", "lang": null}},
                  "high": {high},
                  "low": {},
                  "low_with_errors": 0,
                  "filtered_by_lang": 0,
                  "languages": {{"en": {}, "hi": 2}},
                  "seconds": 12.5
                }}"#,
                rows - high,
                high - 2
            ),
        )
        .unwrap();
        std::fs::write(out.join("high/data.jsonl"), "{}\n").unwrap();
    }

    #[test]
    fn missing_project_reports_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::NotConfigured);
        assert!(s.unavailable_reason.is_some());
        assert!(s.runs.is_empty());
    }

    #[test]
    fn project_without_runs_is_idle_not_connected() {
        let dir = fixture();
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::Idle);
        assert_eq!(s.totals.runs, 0);
    }

    #[test]
    fn runs_are_discovered_newest_first_with_counts() {
        let dir = fixture();
        write_run(dir.path(), "qc_out", 20, 18);
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_run(dir.path(), "qc_out_en", 30, 25);

        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::Connected);
        assert_eq!(s.runs.len(), 2, "both qc_out* dirs are runs");
        // newest first
        assert_eq!(s.runs[0].id, "qc_out_en");
        assert_eq!(s.latest.as_ref().unwrap().id, "qc_out_en");
        assert_eq!(s.totals.rows, 50);
        assert_eq!(s.totals.high, 43);
        assert_eq!(s.totals.low, 7);
        // layers derived from settings: model present → lm-audit ran
        assert!(s.runs[0].layers.contains(&"lm-audit".to_string()));
        // languages sorted by count desc
        assert_eq!(s.runs[0].languages[0].language, "en");
        // output dataset names discovered without reading contents
        assert_eq!(s.runs[0].outputs, vec!["data.jsonl".to_string()]);
    }

    #[test]
    fn malformed_report_degrades_to_stub() {
        let dir = fixture();
        let out = dir.path().join("qc_out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("report.json"), "{not json").unwrap();
        let s = summarize(&PipelinePaths::at(dir.path()));
        // Malformed run is skipped, not fatal.
        assert_eq!(s.integration, IntegrationState::Idle);
    }

    #[test]
    fn no_lm_model_means_no_lm_layer() {
        let dir = fixture();
        let out = dir.path().join("qc_out");
        std::fs::create_dir_all(out.join("high")).unwrap();
        std::fs::write(
            out.join("report.json"),
            r#"{"rows": 5, "settings": {"model": null, "lang": "en"}, "high": 5, "low": 0}"#,
        )
        .unwrap();
        let s = summarize(&PipelinePaths::at(dir.path()));
        let run = &s.runs[0];
        assert!(!run.layers.iter().any(|l| l == "lm-audit"));
        assert!(run.layers.iter().any(|l| l.starts_with("lang-filter:en")));
        assert_eq!(run.lang_filter.as_deref(), Some("en"));
    }
}
