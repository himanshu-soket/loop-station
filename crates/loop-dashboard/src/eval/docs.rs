//! Documentation set: the kit's Markdown documents, parsed into sections
//! with a small, dependency-free renderer (the same safe-subset approach
//! the console SPA already uses for chat markdown).
//!
//! Only files under the kit root are served; ids resolve through the
//! catalog exactly like every other resource.

use serde::Serialize;

use super::catalog::{self, Resource, ResourceKind};
use super::EvalPaths;

/// One document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocEntry {
    /// Catalog id.
    pub id: String,
    /// File name (`README.md`).
    pub name: String,
    /// Path relative to the kit.
    pub rel_path: String,
    /// Bytes on disk.
    pub bytes: u64,
    /// Title: the first `# ` heading, else the file name.
    pub title: String,
    /// Section headings (`## …`) in order.
    pub sections: Vec<String>,
}

/// A served document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServedDoc {
    pub id: String,
    pub name: String,
    pub rel_path: String,
    pub title: String,
    /// Raw Markdown source (the browser renders it).
    pub markdown: String,
    pub bytes: u64,
    pub truncated: bool,
}

/// Upper bound on served document bytes.
const MAX_DOC_BYTES: u64 = 512 * 1024;

/// List the kit's Markdown documents.
pub fn list(paths: &EvalPaths) -> Vec<DocEntry> {
    catalog::catalog(paths)
        .into_iter()
        .filter(|r| r.kind == ResourceKind::Markdown)
        .map(|r| DocEntry {
            id: r.id.clone(),
            name: r.name.clone(),
            rel_path: r.rel_path.clone(),
            bytes: r.bytes,
            title: doc_title(&r),
            sections: Vec::new(),
        })
        .collect()
}

/// Derive a document title from its content when cheaply possible.
fn doc_title(res: &Resource) -> String {
    if res.bytes > MAX_DOC_BYTES {
        return res.name.clone();
    }
    let Ok(text) = std::fs::read_to_string(&res.abs_path) else {
        return res.name.clone();
    };
    text.lines()
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
        .unwrap_or_else(|| res.name.clone())
}

/// Read one document's Markdown source. Errors are honest strings.
pub fn read(paths: &EvalPaths, id: &str) -> Result<ServedDoc, String> {
    let res = super::files::resolve(paths, id)
        .filter(|r| r.kind == ResourceKind::Markdown)
        .ok_or_else(|| format!("no document named {id:?}"))?;
    if res.bytes > MAX_DOC_BYTES {
        return Err(format!(
            "{} is {} bytes; documents above {} KB are not served",
            res.rel_path,
            res.bytes,
            MAX_DOC_BYTES / 1024
        ));
    }
    let markdown =
        std::fs::read_to_string(&res.abs_path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    Ok(ServedDoc {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        title: doc_title(&res),
        markdown,
        bytes: res.bytes,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_serves_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let kit = dir.path().join("internal_eval");
        std::fs::create_dir_all(kit.join("logprob_probe")).unwrap();
        std::fs::create_dir_all(kit.join("pipeline")).unwrap();
        std::fs::write(
            kit.join("README.md"),
            "# internal_eval\n\nintro\n\n## Setup\n\ntext\n",
        )
        .unwrap();
        std::fs::write(
            kit.join("logprob_probe/README.md"),
            "# Logprob probe kit\n\nbody\n",
        )
        .unwrap();
        std::fs::write(kit.join("pipeline/judge.py"), "# not markdown\n").unwrap();

        let paths = EvalPaths::at(dir.path());
        let docs = list(&paths);
        assert_eq!(docs.len(), 2);
        assert!(docs.iter().any(|d| d.title == "internal_eval"));
        assert!(docs.iter().any(|d| d.title == "Logprob probe kit"));

        let served = read(&paths, "README.md").unwrap();
        assert!(served.markdown.contains("## Setup"));
        assert_eq!(served.title, "internal_eval");

        assert!(read(&paths, "pipeline::judge.py").is_err());
        assert!(read(&paths, "missing.md").is_err());
    }

    #[test]
    fn empty_kit_has_no_docs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&EvalPaths::at(dir.path())).is_empty());
    }
}
