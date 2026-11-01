//! Internal AI-evaluation workspaces (`internal_eval`).
//!
//! `internal_eval` is the EKA checkpoint evaluation pipeline: a question
//! bank at several decoding settings, blind grading by `gpt-oss-120b`
//! against comparison models, and per-run HTML/JSON reports. The kit lives
//! on disk as a directory of Python stages, Slurm scripts, banks and docs.
//!
//! This module is a **read-only adapter** over that directory — the same
//! pattern as [`crate::pipelines`]: it discovers roots, reads small files,
//! indexes the large JSONL banks with a bounded scan, and never executes
//! anything. Every number surfaced in the UI comes from a real file on
//! disk; nothing is invented.
//!
//! Discovery honours `LOOP_EVAL_ROOT` so tests (and other machines) can
//! point at an isolated fixture instead of `~/.loop/eval`.
//!
//! Layout expectations (all discovered, none assumed):
//!
//! ```text
//! <root>/banks/*.jsonl          question banks (one JSON object per line)
//! <root>/examples/*.jsonl|tsv   custom-question files and --refs tables
//! <root>/runs/<run>/…           run outputs (config.json, summary.json, …)
//! <root>/pipeline/*.py          evaluation stages
//! <root>/slurm/*.sbatch         cluster job definitions
//! <root>/*.md                   documentation
//! ```
//!
//! Discovery honours `LOOP_EVAL_ROOT` so tests (and other machines) can
//! point at an isolated fixture instead of `~/.loop/eval`.

pub mod api;
pub mod banks;
pub mod catalog;
pub mod docs;
pub mod files;
pub mod reports;
pub mod runs;

use std::path::PathBuf;

/// Default subdirectory of `$HOME` holding the evaluation kit.
pub const DEFAULT_ROOT_SUBDIR: &str = ".loop/eval";

/// Where the evaluation workspaces live and how to find them.
#[derive(Debug, Clone)]
pub struct EvalPaths {
    /// Root containing the `internal_eval` kit.
    pub root: PathBuf,
}

impl EvalPaths {
    /// Resolve from `LOOP_EVAL_ROOT`, else `~/.loop/eval`.
    ///
    /// Both `…/eval` and `…/eval/internal_eval` are accepted as the value:
    /// operators unpack the ZIP either way, and the kit's own `README.md`
    /// describes paths relative to `internal_eval/`.
    pub fn resolve() -> Self {
        let raw = std::env::var_os("LOOP_EVAL_ROOT")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let base = raw.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(DEFAULT_ROOT_SUBDIR)
        });
        Self {
            root: normalize_root(base),
        }
    }

    /// Paths rooted at an explicit directory (tests).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: normalize_root(root.into()),
        }
    }

    /// The kit directory itself (`<root>/internal_eval`, or `<root>` when
    /// the caller already pointed inside it).
    pub fn kit(&self) -> PathBuf {
        self.root.join("internal_eval")
    }

    /// Question-bank directory.
    pub fn banks(&self) -> PathBuf {
        self.kit().join("banks")
    }

    /// Examples directory (custom questions, reference tables).
    pub fn examples(&self) -> PathBuf {
        self.kit().join("examples")
    }

    /// Run-output roots, in scan order: `runs/` inside the kit first, then
    /// `runs/` beside it (the kit's default `EVAL_RUNS` writes there).
    pub fn run_roots(&self) -> Vec<PathBuf> {
        let mut v = vec![self.kit().join("runs"), self.root.join("runs")];
        v.dedup();
        v
    }

    /// True when the kit exists on this machine.
    pub fn kit_present(&self) -> bool {
        self.kit().is_dir()
    }
}

impl Default for EvalPaths {
    fn default() -> Self {
        Self::resolve()
    }
}

/// Accept `…/eval` and `…/eval/internal_eval` as the same root.
fn normalize_root(p: PathBuf) -> PathBuf {
    if p.file_name().and_then(|n| n.to_str()) == Some("internal_eval") {
        if let Some(parent) = p.parent() {
            return parent.to_path_buf();
        }
    }
    p
}

/// Connectivity of the evaluation integration, mirroring
/// [`crate::pipelines::IntegrationState`] without coupling the two modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    /// Kit found and at least one resource discovered.
    Connected,
    /// Kit found but empty (no banks, no runs, no scripts).
    Idle,
    /// Kit directory missing on this machine.
    NotConfigured,
}

impl IntegrationState {
    /// Wire label used by the UI badges.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Idle => "idle",
            Self::NotConfigured => "not_configured",
        }
    }

    /// Human label for the badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Idle => "connected · no resources",
            Self::NotConfigured => "not configured",
        }
    }
}

/// A `key: value` count bucket shared by several payloads.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountBucket {
    pub key: String,
    pub count: u64,
}

/// Sort counts by descending size, then key, and cap the list.
pub(crate) fn top_counts(
    map: &std::collections::BTreeMap<String, u64>,
    cap: usize,
) -> Vec<CountBucket> {
    let mut v: Vec<CountBucket> = map
        .iter()
        .map(|(k, c)| CountBucket {
            key: k.clone(),
            count: *c,
        })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
    v.truncate(cap);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn paths_resolve_under_root() {
        let p = EvalPaths::at("/tmp/fixture");
        assert_eq!(p.banks(), Path::new("/tmp/fixture/internal_eval/banks"));
        assert_eq!(
            p.examples(),
            Path::new("/tmp/fixture/internal_eval/examples")
        );
        assert_eq!(p.run_roots().len(), 2);
    }

    #[test]
    fn internal_eval_suffix_is_normalized() {
        let p = EvalPaths::at("/tmp/fixture/internal_eval");
        assert_eq!(p.kit(), Path::new("/tmp/fixture/internal_eval"));
        // The suffixed form and the plain form resolve identically.
        assert_eq!(p.kit(), EvalPaths::at("/tmp/fixture").kit());
    }

    #[test]
    fn env_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        // Scoped: no other test in this crate depends on this variable.
        std::env::set_var("LOOP_EVAL_ROOT", dir.path());
        let p = EvalPaths::resolve();
        std::env::remove_var("LOOP_EVAL_ROOT");
        assert_eq!(p.root, dir.path());
    }

    #[test]
    fn counts_sort_and_cap() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("b".to_string(), 2);
        m.insert("a".to_string(), 9);
        m.insert("c".to_string(), 5);
        let v = top_counts(&m, 2);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].key, "a");
        assert_eq!(v[1].key, "c");
    }
}
