//! Code pretraining pipeline adapter (`codepipe`).
//!
//! `codepipe` (in `~/soket/codepipe`) prepares a large-scale code corpus for
//! pretraining. Its stages, each writing into its own output directory:
//!
//! ```text
//! 00_inventory/  manifest/ + sample/ + stats.json + report.md
//! 01_filter/     kept/<lang>/ + removed/<lang>/ + _done/tNNNNN.json + report.md
//! 02_exact/      kept/<lang>/ + hashes/ + drops/ + _done/ + report.md
//! 03_minhash/    kept/<lang>/ + work/<lang>/ + _done/ + report.md
//! ```
//!
//! Every stage's `_done/t*.json` files carry per-language counters
//! (`in_docs`, `in_bytes`, `kept_docs`, `kept_bytes`, `removed{reason:[docs,bytes]}`)
//! — the same records `python -m codepipe.report` aggregates into
//! `report.md`. This adapter aggregates the identical JSON, so the dashboard
//! shows the pipeline's own accounting without scanning any Parquet shard.
//!
//! Nothing here launches jobs, reads shard contents or walks data trees: a
//! stage summary is bounded by the number of `_done` task markers, and only
//! the newest `stats.json`/manifest metadata is touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{read_json, IntegrationState, PipelinePaths};

/// The real stage set of the codepipe project, in execution order.
/// Only stages that exist on disk are ever reported.
const STAGE_ORDER: &[(&str, &str)] = &[
    ("00_inventory", "Inventory"),
    ("01_filter", "Rule filter"),
    ("02_exact", "Exact dedup"),
    ("03_minhash", "MinHash near-dedup"),
];

/// One stage of the pretraining pipeline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PretrainStage {
    /// Stage id (directory name, e.g. `01_filter`).
    pub id: String,
    /// Human stage name.
    pub name: String,
    /// Absolute path of the stage output directory.
    pub path: String,
    /// Documents read by the stage (summed over `_done` markers).
    pub docs_in: u64,
    /// Documents kept.
    pub docs_kept: u64,
    /// Content bytes read.
    pub bytes_in: u64,
    /// Content bytes kept.
    pub bytes_kept: u64,
    /// Estimated tokens kept (bytes / 3.5 — the same estimate codepipe's
    /// own report uses; labelled as an estimate in the UI).
    pub est_tokens_kept: f64,
    /// Removal reason → document count, largest first.
    pub removal_reasons: Vec<ReasonCount>,
    /// Number of completed task markers found.
    pub tasks_done: u64,
    /// Languages with recorded counts.
    pub languages: u64,
    /// Whether the stage wrote a human-readable report.md.
    pub has_report: bool,
}

/// A removal-reason bucket.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonCount {
    pub reason: String,
    pub docs: u64,
    pub bytes: u64,
}

/// Summary payload for `/api/pipelines/pretrain`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PretrainSummary {
    /// Integration connectivity.
    pub integration: IntegrationState,
    /// Human label for the badge.
    pub integration_label: String,
    /// Path of the codepipe project, when present.
    pub project_path: Option<String>,
    /// Root the stages were discovered under (where outputs live).
    pub output_root: Option<String>,
    /// Stages present on disk, in pipeline order.
    pub stages: Vec<PretrainStage>,
    /// The last stage with completed tasks.
    pub latest_stage: Option<String>,
    /// Aggregate across stages (definitions documented in the UI).
    pub totals: PretrainTotals,
    /// Why discovery failed, when it did.
    pub unavailable_reason: Option<String>,
}

/// Cross-stage totals. Docs/bytes are per-stage throughput figures; they are
/// never summed across stages because one document flows through all of
/// them. The UI states this explicitly.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PretrainTotals {
    /// Stages discovered.
    pub stages: u64,
    /// Total completed task markers across stages.
    pub tasks_done: u64,
    /// Docs kept by the *final* discovered stage (the current corpus size).
    pub final_kept_docs: u64,
    /// Bytes kept by the final discovered stage.
    pub final_kept_bytes: u64,
    /// Estimated tokens kept by the final stage.
    pub final_est_tokens_kept: f64,
}

/// Aggregate one stage's `_done/*.json` markers.
///
/// Mirrors `codepipe.report.load()`: per-language dicts with
/// `in_docs/in_bytes/kept_docs/kept_bytes/removed{reason: [docs, bytes]}`.
fn aggregate_stage(dir: &Path) -> Option<PretrainStage> {
    let done = dir.join("_done");
    if !done.is_dir() {
        return None;
    }
    let mut docs_in = 0u64;
    let mut docs_kept = 0u64;
    let mut bytes_in = 0u64;
    let mut bytes_kept = 0u64;
    let mut reasons: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut langs = std::collections::HashSet::new();
    let mut tasks = 0u64;

    let entries = std::fs::read_dir(&done).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(v) = read_json(&p) else { continue };
        let Some(obj) = v.as_object() else { continue };
        tasks += 1;
        for (_lang, s) in obj {
            let get = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            docs_in += get("in_docs");
            docs_kept += get("kept_docs");
            bytes_in += get("in_bytes");
            bytes_kept += get("kept_bytes");
            if get("in_docs") > 0 || get("kept_docs") > 0 {
                langs.insert(_lang.clone());
            }
            if let Some(removed) = s.get("removed").and_then(|r| r.as_object()) {
                for (reason, pair) in removed {
                    let docs = pair.get(0).and_then(|x| x.as_u64()).unwrap_or(0);
                    let bytes = pair.get(1).and_then(|x| x.as_u64()).unwrap_or(0);
                    let e = reasons.entry(reason.clone()).or_insert((0, 0));
                    e.0 += docs;
                    e.1 += bytes;
                }
            }
        }
    }

    // Inventory has no _done markers (it writes stats.json instead); a stage
    // dir with zero markers is not a run.
    if tasks == 0 {
        return None;
    }

    let mut removal_reasons: Vec<ReasonCount> = reasons
        .into_iter()
        .map(|(reason, (docs, bytes))| ReasonCount {
            reason,
            docs,
            bytes,
        })
        .collect();
    removal_reasons.sort_by(|a, b| b.docs.cmp(&a.docs));

    let stage_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("stage")
        .to_string();
    let name = STAGE_ORDER
        .iter()
        .find(|(id, _)| *id == stage_id)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| stage_id.clone());

    Some(PretrainStage {
        id: stage_id,
        name,
        path: dir.display().to_string(),
        docs_in,
        docs_kept,
        bytes_in,
        bytes_kept,
        // Same estimate formula as codepipe's own report (bytes / 3.5).
        est_tokens_kept: bytes_kept as f64 / 3.5,
        removal_reasons,
        tasks_done: tasks,
        languages: langs.len() as u64,
        has_report: dir.join("report.md").is_file(),
    })
}

/// Candidate stage directories: `<root>/NN_*` plus `<root>/stages/NN_*`
/// (both layouts appear in the project README's examples).
fn candidate_stage_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut scan = |base: &Path| {
        if let Ok(rd) = std::fs::read_dir(base) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let looks_like_stage = name.len() >= 3
                    && name.as_bytes()[0].is_ascii_digit()
                    && name.as_bytes()[1].is_ascii_digit()
                    && name.as_bytes()[2] == b'_';
                if looks_like_stage {
                    dirs.push(p);
                }
            }
        }
    };
    scan(root);
    scan(&root.join("stages"));
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Build the pretraining summary for a workspace root.
pub fn summarize(paths: &PipelinePaths) -> PretrainSummary {
    let project = paths.pretrain_project();
    if !project.is_dir() {
        return PretrainSummary {
            integration: IntegrationState::NotConfigured,
            integration_label: IntegrationState::NotConfigured.label().into(),
            project_path: None,
            output_root: None,
            stages: Vec::new(),
            latest_stage: None,
            totals: PretrainTotals::default(),
            unavailable_reason: Some(format!(
                "codepipe project not found at {}. The pretraining workspace reads the \
                 stage reports the pipeline writes; point SOKET_PIPELINE_ROOT at the \
                 workspace to enable it.",
                project.display()
            )),
        };
    }

    // Stage outputs live beside the project (README: OUT=/data/codepipe).
    // Look under the workspace root and, if none there, under the project
    // itself (self-contained runs).
    let mut roots = vec![paths.root.clone(), project.clone()];
    if let Some(parent) = project.parent() {
        roots.push(parent.join("codepipe_out"));
    }

    let mut stages: Vec<PretrainStage> = Vec::new();
    let mut used_root: Option<PathBuf> = None;
    for root in &roots {
        let found: Vec<PretrainStage> = candidate_stage_dirs(root)
            .iter()
            .filter_map(|d| aggregate_stage(d))
            .collect();
        if !found.is_empty() {
            stages = found;
            used_root = Some(root.clone());
            break;
        }
    }

    // Keep pipeline order regardless of directory sorting quirks.
    stages.sort_by(|a, b| {
        let rank = |id: &str| STAGE_ORDER.iter().position(|(s, _)| *s == id).unwrap_or(99);
        rank(&a.id).cmp(&rank(&b.id))
    });

    let totals = if let Some(last) = stages.last() {
        PretrainTotals {
            stages: stages.len() as u64,
            tasks_done: stages.iter().map(|s| s.tasks_done).sum(),
            final_kept_docs: last.docs_kept,
            final_kept_bytes: last.bytes_kept,
            final_est_tokens_kept: last.est_tokens_kept,
        }
    } else {
        PretrainTotals::default()
    };

    let integration = if stages.is_empty() {
        IntegrationState::Idle
    } else {
        IntegrationState::Connected
    };

    PretrainSummary {
        integration,
        integration_label: integration.label().into(),
        project_path: Some(project.display().to_string()),
        output_root: used_root.map(|p| p.display().to_string()),
        latest_stage: stages.last().map(|s| s.name.clone()),
        stages,
        totals,
        unavailable_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("codepipe/codepipe")).unwrap();
        std::fs::write(dir.path().join("codepipe/codepipe/filter.py"), "# f").unwrap();
        dir
    }

    /// Write one `_done` marker with the exact shape codepipe writes.
    fn mark(
        stage_dir: &Path,
        task: u32,
        lang: &str,
        in_docs: u64,
        kept: u64,
        reason: &str,
        rem: u64,
    ) {
        let done = stage_dir.join("_done");
        std::fs::create_dir_all(&done).unwrap();
        let body = format!(
            r#"{{"{lang}": {{"in_docs": {in_docs}, "in_bytes": {in_docs}00, "kept_docs": {kept},
               "kept_bytes": {kept}00, "removed": {{"{reason}": [{rem}, {rem}00]}}}}}}"#
        );
        std::fs::write(done.join(format!("t{task:05}.json")), body).unwrap();
    }

    #[test]
    fn missing_project_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::NotConfigured);
        assert!(s.unavailable_reason.is_some());
    }

    #[test]
    fn project_without_stage_outputs_is_idle() {
        let dir = fixture();
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::Idle);
        assert!(s.stages.is_empty());
    }

    #[test]
    fn stages_aggregate_done_markers_in_order() {
        let dir = fixture();
        let out = dir.path().join("codepipe");
        let s1 = out.join("01_filter");
        let s2 = out.join("02_exact");
        std::fs::create_dir_all(s1.join("kept")).unwrap();
        std::fs::create_dir_all(s2.join("kept")).unwrap();
        mark(&s1, 0, "Python", 100, 60, "vendored", 40);
        mark(&s1, 1, "Rust", 50, 45, "too_short", 5);
        mark(&s2, 0, "Python", 60, 40, "exact_dup", 20);

        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::Connected);
        assert_eq!(s.stages.len(), 2);
        assert_eq!(s.stages[0].id, "01_filter");
        assert_eq!(s.stages[0].name, "Rule filter");
        assert_eq!(s.stages[0].docs_in, 150);
        assert_eq!(s.stages[0].docs_kept, 105);
        assert_eq!(s.stages[0].tasks_done, 2);
        assert_eq!(s.stages[0].languages, 2);
        // removal reasons sorted by docs desc
        assert_eq!(s.stages[0].removal_reasons[0].reason, "vendored");
        // totals describe the FINAL stage, not a sum across stages
        assert_eq!(s.totals.final_kept_docs, 40);
        assert_eq!(s.totals.stages, 2);
        assert_eq!(s.totals.tasks_done, 3);
        assert_eq!(s.latest_stage.as_deref(), Some("Exact dedup"));
    }

    #[test]
    fn malformed_marker_is_skipped_not_fatal() {
        let dir = fixture();
        let s1 = dir.path().join("codepipe/01_filter");
        let done = s1.join("_done");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("t00000.json"), "{broken").unwrap();
        // Zero parseable markers → stage not counted as a run.
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert_eq!(s.integration, IntegrationState::Idle);
    }

    #[test]
    fn non_stage_directories_are_ignored() {
        let dir = fixture();
        let out = dir.path().join("codepipe");
        for name in ["kept", "sample", "notes"] {
            std::fs::create_dir_all(out.join(name)).unwrap();
        }
        let s = summarize(&PipelinePaths::at(dir.path()));
        assert!(s.stages.is_empty());
    }
}
