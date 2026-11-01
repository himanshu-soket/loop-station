//! Read-only adapters for Soket's data pipelines (SFT QC + code pretraining).
//!
//! Both pipelines are Python projects that already write machine-readable
//! metadata to disk:
//!
//! * `sftqc` (`~/soket/sftqc`) writes `<out>/report.json` per run — layer
//!   scores, language mix, calibration and pass/fail counts.
//! * `codepipe` (`~/soket/codepipe`) writes `<stage>/_done/tNNNNN.json`
//!   per task plus `stats.json` / `report.md` summaries per stage.
//!
//! This module never executes a pipeline: it only discovers roots, reads the
//! small JSON artifacts and summarizes them. No dataset contents are read,
//! so summarizing a multi-terabyte corpus costs a few stat() calls and JSON
//! parses of files the pipeline itself wrote.
//!
//! Discovery is configuration-driven through [`PipelinePaths`], which honors
//! `SOKET_PIPELINE_ROOT` so tests (and other machines) can point at an
//! isolated fixture instead of `~/soket`.

pub mod api;
pub mod pretrain;
pub mod sft;
pub mod sft_code;

use std::path::{Path, PathBuf};

/// Where the pipeline workspaces live and how to find them.
#[derive(Debug, Clone)]
pub struct PipelinePaths {
    /// Root containing both pipeline projects (`~/soket` by default).
    pub root: PathBuf,
}

/// Default workspace root: `~/soket`.
pub const DEFAULT_ROOT_SUBDIR: &str = "soket";

impl Default for PipelinePaths {
    fn default() -> Self {
        Self::resolve()
    }
}

impl PipelinePaths {
    /// Resolve the root from `SOKET_PIPELINE_ROOT`, else `~/soket`.
    pub fn resolve() -> Self {
        let root = match std::env::var_os("SOKET_PIPELINE_ROOT") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(DEFAULT_ROOT_SUBDIR),
        };
        Self { root }
    }

    /// Paths rooted at an explicit directory (tests).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The SFT QC project directory (`<root>/sftqc`).
    pub fn sft_project(&self) -> PathBuf {
        self.root.join("sftqc")
    }

    /// The code pretraining project directory (`<root>/codepipe`).
    pub fn pretrain_project(&self) -> PathBuf {
        self.root.join("codepipe")
    }

    /// True when the SFT project exists on this machine.
    pub fn sft_present(&self) -> bool {
        self.sft_project().is_dir()
    }

    /// True when the pretraining project exists on this machine.
    pub fn pretrain_present(&self) -> bool {
        self.pretrain_project().is_dir()
    }
}

/// Connectivity of a pipeline integration, kept separate from job execution
/// state: a workspace can be connected while idle, and a running job can
/// coexist with a degraded telemetry connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    /// Project found and at least one run/stage discovered.
    Connected,
    /// Project found but no runs/stages yet.
    Idle,
    /// Project directory is missing on this machine.
    NotConfigured,
}

impl IntegrationState {
    /// Lower-case label for badges.
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Idle => "connected · no runs",
            Self::NotConfigured => "not configured",
        }
    }
}

/// Read a UTF-8 JSON file, returning `Err(reason)` on failure. Never panics.
pub(crate) fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Bounded listing of file names in a directory (no recursion, sorted).
pub(crate) fn list_file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Extract a u64 from a JSON object without coercing types.
pub(crate) fn json_u64(v: &serde_json::Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| x.as_u64())
}

/// Snake-case wire label for an integration state (matches serde's
/// `rename_all = "snake_case"` on the enum itself).
pub(crate) fn state_wire(s: IntegrationState) -> &'static str {
    match s {
        IntegrationState::Connected => "connected",
        IntegrationState::Idle => "idle",
        IntegrationState::NotConfigured => "not_configured",
    }
}

/// Extract an f64 from a JSON object without coercing types.
pub(crate) fn json_f64(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

/// Extract a trimmed string from a JSON object.
pub(crate) fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_under_root() {
        let p = PipelinePaths::at("/tmp/fixture");
        assert_eq!(p.sft_project(), Path::new("/tmp/fixture/sftqc"));
        assert_eq!(p.pretrain_project(), Path::new("/tmp/fixture/codepipe"));
    }

    #[test]
    fn env_override_wins() {
        // Scoped env change; other tests in this binary do not depend on it.
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SOKET_PIPELINE_ROOT", dir.path());
        let p = PipelinePaths::resolve();
        std::env::remove_var("SOKET_PIPELINE_ROOT");
        assert_eq!(p.root, dir.path());
    }
}
