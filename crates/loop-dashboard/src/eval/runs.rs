//! Evaluation runs and their reports: discovery of run folders plus
//! read-only parsing of the artifacts each run writes (`config.json`,
//! `summary.json`, per-setting `report.json`).
//!
//! The kit's own documentation (`README.md` §5) defines the layout:
//!
//! ```text
//! RUN/run.env, config.json      all options (used for resume)
//! RUN/questions.jsonl           the questions (stable id "group|id", qhash)
//! RUN/compare/gen_<model>.jsonl comparison answers
//! RUN/settings/<setting>/…      answers, judge chunks, report.json
//! RUN/summary.json, summary.html  every setting + comparison models
//! ```
//!
//! This module never invents a metric: every number it returns is read from
//! one of those files, and a missing artifact yields `None` rather than a
//! guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::EvalPaths;

/// Upper bound on discovered run directories.
const MAX_RUNS: usize = 200;

/// One decoding setting inside a run, as recorded by `config.json`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingSpec {
    /// Setting name, e.g. `med06`.
    pub name: String,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Reasoning effort, when the setting sets one.
    pub effort: Option<String>,
}

/// Per-model accuracy figures from a `report.json` / `summary.json` block.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelScore {
    /// Model key: `eka` for the model under test, else a comparison key.
    pub model: String,
    /// Display name where the run recorded one.
    pub label: Option<String>,
    /// Graded answers this row covers.
    pub n: Option<u64>,
    /// Accuracy = (correct + ½·partial) / graded, percent.
    pub acc: Option<f64>,
    /// Fully-correct share, percent.
    pub strict: Option<f64>,
    /// Lenient share (partials with only working missing count), percent.
    pub lenient: Option<f64>,
    /// Mean judge score.
    pub score: Option<f64>,
}

/// One setting's report, when the run produced one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingReport {
    pub name: String,
    pub temperature: Option<f64>,
    pub effort: Option<String>,
    /// Questions in the report.
    pub n: Option<u64>,
    /// Per-model scores from `overall`.
    pub overall: Vec<ModelScore>,
    /// Accuracy by group (`by_group`).
    pub by_group: Vec<GroupScore>,
    /// Error-type counts per model.
    pub errors: BTreeMap<String, BTreeMap<String, u64>>,
    /// Gap statistics: questions the model under test missed that a
    /// comparison model answered correctly.
    pub gap_questions: Option<u64>,
    /// Questions only the model under test answered correctly.
    pub only_right: Option<u64>,
    /// Whether report.json exists on disk.
    pub has_report: bool,
}

/// A group row from `by_group`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupScore {
    pub group: String,
    pub n: Option<u64>,
    pub acc: Option<f64>,
    pub best_model: Option<String>,
    pub best_acc: Option<f64>,
}

/// One discovered evaluation run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRun {
    /// Run directory name.
    pub id: String,
    /// Absolute path.
    pub path: String,
    /// Label from `config.json` (`label`), when present.
    pub label: Option<String>,
    /// Model under test, from `config.json`.
    pub model: Option<String>,
    /// Server URLs the run used.
    pub urls: Vec<String>,
    /// Input spec (bank preset or file).
    pub input: Option<String>,
    /// Decoding settings the run swept.
    pub settings: Vec<SettingSpec>,
    /// Comparison models the run judged against.
    pub compare: Vec<String>,
    /// Answer token cap.
    pub max_tokens: Option<u64>,
    /// Parallel requests per setting.
    pub workers: Option<u64>,
    /// Question count from `questions.jsonl` (line count, bounded).
    pub questions: Option<u64>,
    /// Per-setting reports, in config order.
    pub reports: Vec<SettingReport>,
    /// Comparison-answer counts per model.
    pub compare_answers: BTreeMap<String, u64>,
    /// `summary.json` contents, when present (kept verbatim; the UI reads
    /// what it needs).
    pub summary: Option<serde_json::Value>,
    /// Artifacts present on disk (relative names).
    pub artifacts: Vec<String>,
    /// mtime of the run directory, unix ms.
    pub modified_ms: u64,
}

/// Discover run directories under every configured root.
pub fn discover_runs(paths: &EvalPaths) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in paths.run_roots() {
        if let Ok(rd) = std::fs::read_dir(&root) {
            for entry in rd.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_dir() {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out.truncate(MAX_RUNS);
    out
}

/// Read a JSON file, returning `None` on any failure.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Count non-empty lines in a JSONL file without loading it.
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

/// Map a `summarize()`-style JSON block into [`ModelScore`] rows.
fn model_scores(overall: Option<&serde_json::Value>) -> Vec<ModelScore> {
    let Some(obj) = overall.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    obj.iter()
        .map(|(model, v)| ModelScore {
            model: model.clone(),
            label: None,
            n: v.get("n").and_then(|x| x.as_u64()),
            acc: v.get("acc").and_then(|x| x.as_f64()),
            strict: v.get("strict").and_then(|x| x.as_f64()),
            lenient: v.get("lenient").and_then(|x| x.as_f64()),
            score: v.get("score").and_then(|x| x.as_f64()),
        })
        .collect()
}

/// Read one setting's `report.json` into a [`SettingReport`].
fn load_setting_report(run_dir: &Path, spec: &SettingSpec) -> SettingReport {
    let report_path = run_dir
        .join("settings")
        .join(&spec.name)
        .join("report.json");
    let has_report = report_path.is_file();
    let mut rep = SettingReport {
        name: spec.name.clone(),
        temperature: spec.temperature,
        effort: spec.effort.clone(),
        n: None,
        overall: Vec::new(),
        by_group: Vec::new(),
        errors: BTreeMap::new(),
        gap_questions: None,
        only_right: None,
        has_report,
    };
    let Some(v) = read_json(&report_path) else {
        return rep;
    };
    rep.n = v.get("n").and_then(|x| x.as_u64());
    rep.overall = model_scores(v.get("overall"));
    // `by_group` maps group → { model: {acc,…} }; surface the model under
    // test's accuracy and the best comparison model.
    if let Some(groups) = v.get("by_group").and_then(|g| g.as_object()) {
        for (group, gv) in groups {
            let mut row = GroupScore {
                group: group.clone(),
                n: None,
                acc: None,
                best_model: None,
                best_acc: None,
            };
            if let Some(models) = gv.as_object() {
                let mut best: Option<(&String, f64)> = None;
                for (m, mv) in models {
                    let acc = mv.get("acc").and_then(|x| x.as_f64());
                    let n = mv.get("n").and_then(|x| x.as_u64());
                    if m == "eka" {
                        row.acc = acc;
                        row.n = n;
                    } else if let Some(a) = acc {
                        if best.map(|(_, b)| a > b).unwrap_or(true) {
                            best = Some((m, a));
                        }
                    }
                }
                if let Some((m, a)) = best {
                    row.best_model = Some(m.clone());
                    row.best_acc = Some(a);
                }
            }
            rep.by_group.push(row);
        }
        rep.by_group.sort_by(|a, b| a.group.cmp(&b.group));
    }
    if let Some(errs) = v.get("errors").and_then(|e| e.as_object()) {
        for (model, types) in errs {
            if let Some(types) = types.as_object() {
                let mut m: BTreeMap<String, u64> = BTreeMap::new();
                for (t, c) in types {
                    if let Some(c) = c.as_u64() {
                        m.insert(t.clone(), c);
                    }
                }
                rep.errors.insert(model.clone(), m);
            }
        }
    }
    rep.gap_questions = v.get("eka_gap_questions").and_then(|x| x.as_u64());
    rep.only_right = v.get("eka_only_right").and_then(|x| x.as_u64());
    rep
}

/// Load one run directory into an [`EvalRun`]. Returns `None` when the
/// directory holds no recognizable run artifacts at all.
pub fn load_run(dir: &Path) -> Option<EvalRun> {
    let id = dir.file_name()?.to_str()?.to_string();
    // A run is recognizable by any of its signature files.
    let signatures = ["config.json", "run.env", "questions.jsonl", "summary.json"];
    if !signatures.iter().any(|s| dir.join(s).is_file()) {
        return None;
    }
    let cfg = read_json(&dir.join("config.json"));
    let get_str = |k: &str| -> Option<String> {
        cfg.as_ref()
            .and_then(|c| c.get(k))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let settings: Vec<SettingSpec> = cfg
        .as_ref()
        .and_then(|c| c.get("settings"))
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(SettingSpec {
                        name: s.get("name")?.as_str()?.to_string(),
                        temperature: s.get("temperature").and_then(|t| t.as_f64()),
                        effort: s
                            .get("effort")
                            .and_then(|e| e.as_str())
                            .map(|e| e.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let mut reports = Vec::new();
    for spec in &settings {
        reports.push(load_setting_report(dir, spec));
    }

    let compare: Vec<String> = cfg
        .as_ref()
        .and_then(|c| c.get("compare"))
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Comparison-answer counts: gen_<model>.jsonl line counts.
    let mut compare_answers = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(dir.join("compare")) {
        for entry in rd.filter_map(|e| e.ok()) {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if let Some(model) = name
                .strip_prefix("gen_")
                .and_then(|r| r.strip_suffix(".jsonl"))
            {
                if let Some(n) = count_lines(&p) {
                    compare_answers.insert(model.to_string(), n);
                }
            }
        }
    }

    let artifacts: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();

    let modified_ms = std::fs::metadata(dir)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Some(EvalRun {
        id,
        path: dir.display().to_string(),
        label: get_str("label"),
        model: get_str("model"),
        urls: cfg
            .as_ref()
            .and_then(|c| c.get("urls"))
            .and_then(|u| u.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        input: get_str("input"),
        settings,
        compare,
        max_tokens: cfg
            .as_ref()
            .and_then(|c| c.get("max_tokens"))
            .and_then(|v| v.as_u64()),
        workers: cfg
            .as_ref()
            .and_then(|c| c.get("workers"))
            .and_then(|v| v.as_u64()),
        questions: count_lines(&dir.join("questions.jsonl")),
        reports,
        compare_answers,
        summary: read_json(&dir.join("summary.json")),
        artifacts,
        modified_ms,
    })
}

/// Load every discovered run, newest first.
pub fn load_runs(paths: &EvalPaths) -> Vec<EvalRun> {
    let mut runs: Vec<EvalRun> = discover_runs(paths)
        .into_iter()
        .filter_map(|d| load_run(&d))
        .collect();
    runs.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then(a.id.cmp(&b.id)));
    runs
}

/// Cross-run comparison payload: one row per (run, setting) with the model
/// under test's headline accuracy, plus comparison-model rows where the
/// run recorded them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonRow {
    pub run_id: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub setting: String,
    pub temperature: Option<f64>,
    pub effort: Option<String>,
    pub n: Option<u64>,
    pub acc: Option<f64>,
    pub strict: Option<f64>,
    pub lenient: Option<f64>,
    pub score: Option<f64>,
    /// True when this row is the model under test (not a comparison model).
    pub is_under_test: bool,
}

/// Build the comparison table across runs. Only real numbers appear; runs
/// without reports contribute nothing rather than zeros.
pub fn comparisons(runs: &[EvalRun]) -> Vec<ComparisonRow> {
    let mut out = Vec::new();
    for run in runs {
        for rep in &run.reports {
            for score in &rep.overall {
                out.push(ComparisonRow {
                    run_id: run.id.clone(),
                    label: run.label.clone(),
                    model: Some(score.model.clone()),
                    setting: rep.name.clone(),
                    temperature: rep.temperature,
                    effort: rep.effort.clone(),
                    n: score.n,
                    acc: score.acc,
                    strict: score.strict,
                    lenient: score.lenient,
                    score: score.score,
                    is_under_test: score.model == "eka",
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("internal_eval/runs/EKA_p6-700_20261030");
        std::fs::create_dir_all(run.join("settings/med06")).unwrap();
        std::fs::create_dir_all(run.join("compare")).unwrap();
        std::fs::write(
            run.join("config.json"),
            r#"{"urls":["http://node:8004"],"model":"eka-v1-p6-700","label":"EKA p6-700",
               "input":"qbank-sample","max_tokens":16384,"workers":32,
               "settings":[{"name":"med06","temperature":0.6,"effort":"medium"},
                           {"name":"high04","temperature":0.4,"effort":"high"}],
               "compare":["gpt-oss-120b","qwen3-32b"]}"#,
        )
        .unwrap();
        std::fs::write(run.join("questions.jsonl"), "{}\n{}\n{}\n").unwrap();
        std::fs::write(
            run.join("settings/med06/report.json"),
            r#"{"n":4229,
               "overall":{"eka":{"n":4229,"acc":59.4,"strict":51.2,"lenient":60.1,"score":0.61},
                          "gpt-oss-120b":{"n":4229,"acc":90.9,"strict":88.0,"lenient":91.0,"score":0.9}},
               "by_group":{"Class 1":{"eka":{"n":50,"acc":72.0},"gpt-oss-120b":{"n":50,"acc":96.0}},
                           "Coding":{"eka":{"n":300,"acc":41.3},"gpt-oss-120b":{"n":300,"acc":92.0}}},
               "errors":{"eka":{"conceptual":120,"factual_error":80}},
               "eka_gap_questions":640,"eka_only_right":37}"#,
        )
        .unwrap();
        std::fs::write(run.join("compare/gen_gpt-oss-120b.jsonl"), "{}\n{}\n").unwrap();
        std::fs::write(run.join("summary.json"), r#"{"n_questions":4229}"#).unwrap();
        dir
    }

    #[test]
    fn loads_run_config_reports_and_counts() {
        let dir = run_fixture();
        let paths = EvalPaths::at(dir.path());
        let runs = load_runs(&paths);
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.id, "EKA_p6-700_20261030");
        assert_eq!(r.label.as_deref(), Some("EKA p6-700"));
        assert_eq!(r.settings.len(), 2);
        assert_eq!(r.questions, Some(3));
        assert_eq!(r.compare_answers.get("gpt-oss-120b"), Some(&2));

        let med = r.reports.iter().find(|x| x.name == "med06").unwrap();
        assert!(med.has_report);
        assert_eq!(med.n, Some(4229));
        let eka = med.overall.iter().find(|s| s.model == "eka").unwrap();
        assert_eq!(eka.acc, Some(59.4));
        let groups: Vec<&GroupScore> = med
            .by_group
            .iter()
            .filter(|g| g.group == "Coding")
            .collect();
        assert_eq!(groups[0].acc, Some(41.3));
        assert_eq!(groups[0].best_model.as_deref(), Some("gpt-oss-120b"));
        assert_eq!(med.gap_questions, Some(640));

        // The setting without a report still appears, honestly empty.
        let high = r.reports.iter().find(|x| x.name == "high04").unwrap();
        assert!(!high.has_report);
        assert_eq!(high.n, None);
    }

    #[test]
    fn comparisons_include_both_models() {
        let dir = run_fixture();
        let paths = EvalPaths::at(dir.path());
        let runs = load_runs(&paths);
        let rows = comparisons(&runs);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.is_under_test && r.acc == Some(59.4)));
        assert!(rows.iter().any(|r| !r.is_under_test && r.acc == Some(90.9)));
    }

    #[test]
    fn non_run_directories_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("internal_eval/runs/not_a_run")).unwrap();
        let paths = EvalPaths::at(dir.path());
        assert!(load_runs(&paths).is_empty());
    }

    #[test]
    fn missing_root_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_runs(&EvalPaths::at(dir.path())).is_empty());
    }
}
