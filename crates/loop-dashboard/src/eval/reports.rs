//! Evaluation report cards: discovery of report artifacts across every
//! evaluation run and setting, plus read-only serving of their contents.
//!
//! Two families of artifacts feed the same card model:
//!
//! 1. **Live runs** — folders under the kit's `runs/` roots that contain
//!    run signatures (`config.json`, `run.env`, `questions.jsonl`,
//!    `summary.json`). Each per-setting directory can hold `report.json`
//!    (machine-readable results), `report.html` (the rendered gap report)
//!    and `graded_flat.csv` (one row per graded question).
//! 2. **Imported HTML reports** — self-contained report pages exported from
//!    elsewhere (e.g. `eka_all_eval_reports.zip`) that embed their results
//!    as a `<script id="report-data" type="application/json">` block, and
//!    settings-sweep summary pages that render an Overall table. The
//!    original files are preserved verbatim; this module only *reads* them.
//!
//! Nothing is invented: every metric on a card comes from a real file on
//! disk, and a missing artifact yields `None` rather than a guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::EvalPaths;

/// Upper bound on discovered report files (defensive; the shipped archive
/// holds ~70).
const MAX_REPORT_FILES: usize = 4_000;

/// Largest HTML file we will scan for embedded JSON (the biggest real
/// report is ~1.9 MB).
const MAX_HTML_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// One per-model score row, mirroring `runs::ModelScore` (the same keys
/// `report.json` and the embedded `report-data` blocks use).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportScore {
    /// Model key: `eka` for the model under test, else a comparison key.
    pub model: String,
    /// Graded answers this row covers.
    pub n: Option<u64>,
    /// Accuracy = (correct + ½·partial) / graded, percent.
    pub acc: Option<f64>,
    /// Fully-correct share, percent.
    pub strict: Option<f64>,
    /// Lenient share, percent.
    pub lenient: Option<f64>,
    /// Mean judge score.
    pub score: Option<f64>,
}

/// One setting row parsed from a settings-sweep summary page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepRow {
    /// Setting label exactly as printed (`High @0.4`, `Greedy`, …).
    pub setting: String,
    /// Whether the row is one of the run's own settings (true) or a
    /// reference row for another checkpoint (false).
    pub own: bool,
    pub acc: Option<f64>,
    pub strict: Option<f64>,
    pub school: Option<f64>,
    pub higher: Option<f64>,
    pub general: Option<f64>,
    pub empty: Option<String>,
    pub median_tokens: Option<u64>,
}

/// What a report card points at on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSource {
    /// `settings/<name>/report.json` inside a live run.
    RunReport,
    /// `summary.json` / `summary.html` at a live run's root.
    RunSummary,
    /// An imported standalone HTML report.
    ImportedHtml,
    /// An imported aggregate page (`_overview/index.html` and friends).
    ImportedOverview,
}

/// One report card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportCard {
    /// Stable id (URL-safe; used by `/api/eval/reports/{id}`).
    pub id: String,
    /// Where this card came from.
    pub source: ReportSource,
    /// Checkpoint/run folder name (`sft6-w6000`) or overview group.
    pub run: String,
    /// Human title: `EKA sft6-w6000 full high04`.
    pub title: String,
    /// Model under test, when recorded (`eka-sft6-w6000`).
    pub model: Option<String>,
    /// Decoding setting name (`high04`, `t0`, `sample`) or `sweep`.
    pub setting: String,
    /// Input bank, when recorded (`qbank-v2-all`).
    pub input: Option<String>,
    /// Question count.
    pub samples: Option<u64>,
    /// Graded count.
    pub graded: Option<u64>,
    /// Model-under-test accuracy, percent.
    pub acc: Option<f64>,
    /// Fully-correct share, percent.
    pub strict: Option<f64>,
    /// Comparison-model scores, when present.
    pub comparisons: Vec<ReportScore>,
    /// Judge description, when recorded.
    pub judge: Option<String>,
    /// `completed` when metrics exist, else `incomplete`.
    pub status: &'static str,
    /// Artifacts available on disk, with catalog ids for serving.
    pub artifacts: ReportArtifacts,
    /// mtime of the primary file, unix ms.
    pub modified_ms: u64,
    /// Primary file path relative to the kit root.
    pub rel_path: String,
}

/// Which artifact files exist for a card, by catalog id.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportArtifacts {
    /// Rendered HTML report (catalog id).
    pub html: Option<String>,
    /// Machine-readable JSON (catalog id) — a real `report.json` or the
    /// JSON embedded in an HTML report.
    pub json: Option<String>,
    /// Per-question grades (catalog id).
    pub csv: Option<String>,
    /// Settings-sweep summary page (catalog id).
    pub summary_html: Option<String>,
    /// Bytes of each artifact, for honest size labels.
    pub html_bytes: Option<u64>,
    pub json_bytes: Option<u64>,
    pub csv_bytes: Option<u64>,
}

/// The full payload served for one report (card + parsed detail).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDetail {
    #[serde(flatten)]
    pub card: ReportCard,
    /// Settings rows for sweep summaries.
    pub sweep: Vec<SweepRow>,
    /// Lede paragraph, when the report records one.
    pub lede: Option<String>,
    /// Method notes, when the report records them.
    pub method: Vec<String>,
    /// Generation stats for the model under test.
    pub gen: BTreeMap<String, serde_json::Value>,
    /// Raw `overall` block (per-model scores) verbatim from the file.
    pub overall: Option<serde_json::Value>,
}

// ------------------------------------------------------------------
// Embedded-JSON extraction (for imported HTML reports)
// ------------------------------------------------------------------

/// Extract the contents of `<script id="report-data" type="application/json">
/// … </script>` from an HTML report. Returns the raw JSON text.
///
/// The block is located by literal markers (no DOM parser needed); the
/// search is bounded by reading at most [`MAX_HTML_SCAN_BYTES`].
fn extract_report_data(html: &str) -> Option<&str> {
    const OPEN: &str = r#"<script id="report-data" type="application/json">"#;
    let start = html.find(OPEN)? + OPEN.len();
    let rest = &html[start..];
    let end = rest.find("</script>")?;
    Some(&rest[..end])
}

/// Public wrapper for the API layer (serving embedded JSON verbatim).
pub fn extract_report_data_pub(html: &str) -> Option<&str> {
    extract_report_data(html)
}

/// Pull one top-level field out of a JSON object given as text, without
/// parsing the whole document. Objects and arrays are brace-matched with
/// string awareness; scalars read to the next `,`/`}`. A `None` return
/// means "not present" — never a guess.
fn json_field(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let mut from = 0;
    let i = loop {
        let at = src[from..].find(&needle)? + from;
        // Structural guard: the previous non-space byte must be `{` or `,`
        // so `"n_graded":` inside a nested string cannot match.
        let bytes = src.as_bytes();
        let mut j = at;
        while j > 0 && (bytes[j - 1] as char).is_ascii_whitespace() {
            j -= 1;
        }
        if j == 0 || matches!(bytes[j - 1], b'{' | b',') {
            break at + needle.len();
        }
        from = at + needle.len();
    };
    let bytes = src.as_bytes();
    let mut k = i;
    while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() {
        return None;
    }
    match bytes[k] {
        b'{' | b'[' => {
            let (open, close) = if bytes[k] == b'{' {
                (b'{', b'}')
            } else {
                (b'[', b']')
            };
            let mut depth = 0usize;
            let mut in_str = false;
            let mut esc = false;
            let start = k;
            while k < bytes.len() {
                let c = bytes[k];
                if in_str {
                    if esc {
                        esc = false;
                    } else if c == b'\\' {
                        esc = true;
                    } else if c == b'"' {
                        in_str = false;
                    }
                } else if c == b'"' {
                    in_str = true;
                } else if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(src[start..=k].to_string());
                    }
                }
                k += 1;
            }
            None
        }
        b'"' => {
            let start = k;
            k += 1;
            let mut esc = false;
            while k < bytes.len() {
                let c = bytes[k];
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    return Some(src[start..=k].to_string());
                }
                k += 1;
            }
            None
        }
        _ => {
            // number / bool / null
            let start = k;
            while k < bytes.len() && !matches!(bytes[k], b',' | b'}' | b']') {
                k += 1;
            }
            let v = src[start..k].trim();
            (!v.is_empty()).then(|| v.to_string())
        }
    }
}

/// Read a JSON field as `u64`.
fn field_u64(src: &str, key: &str) -> Option<u64> {
    json_field(src, key)?.parse().ok()
}

/// Read a JSON field as a plain string (unquoted).
fn field_str(src: &str, key: &str) -> Option<String> {
    let raw = json_field(src, key)?;
    serde_json::from_str::<String>(&raw).ok()
}

/// Read a JSON field as a string array.
fn field_str_vec(src: &str, key: &str) -> Vec<String> {
    json_field(src, key)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// Map an `overall` JSON object into score rows, model-under-test first.
fn overall_scores(overall_src: Option<&str>) -> Vec<ReportScore> {
    let Some(src) = overall_src else {
        return Vec::new();
    };
    let Some(obj) = serde_json::from_str::<serde_json::Value>(src).ok() else {
        return Vec::new();
    };
    let Some(map) = obj.as_object() else {
        return Vec::new();
    };
    let mut rows: Vec<ReportScore> = map
        .iter()
        .map(|(model, v)| ReportScore {
            model: model.clone(),
            n: v.get("n").and_then(|x| x.as_u64()),
            acc: v.get("acc").and_then(|x| x.as_f64()),
            strict: v.get("strict").and_then(|x| x.as_f64()),
            lenient: v.get("lenient").and_then(|x| x.as_f64()),
            score: v.get("score").and_then(|x| x.as_f64()),
        })
        .collect();
    rows.sort_by(|a, b| {
        let ae = a.model == "eka";
        let be = b.model == "eka";
        be.cmp(&ae).then_with(|| a.model.cmp(&b.model))
    });
    rows
}

// ------------------------------------------------------------------
// HTML text helpers (for summary pages)
// ------------------------------------------------------------------

/// Strip tags and collapse whitespace.
fn text_of(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode the small set of entities these pages actually use.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// First `<h1>` text of an HTML page.
fn h1_of(html: &str) -> Option<String> {
    let start = html.find("<h1")?;
    let after = &html[start..];
    let open_end = after.find('>')? + 1;
    let body = &after[open_end..];
    let end = body.find("</h1>")?;
    Some(decode_entities(&text_of(&body[..end])))
}

/// The `<div class="meta">…</div>` spans of a summary page.
fn meta_spans(html: &str) -> Vec<String> {
    let start = match html.find("<div class=\"meta\">") {
        Some(i) => i + "<div class=\"meta\">".len(),
        None => return Vec::new(),
    };
    let rest = &html[start..];
    let end = match rest.find("</div>") {
        Some(e) => e,
        None => return Vec::new(),
    };
    rest[..end]
        .split("<span")
        .skip(1)
        .filter_map(|sp| {
            let gt = sp.find('>')?;
            let close = sp.find("</span>")?;
            Some(decode_entities(&text_of(&sp[gt + 1..close])))
        })
        .collect()
}

/// Parse "96,500 questions" / "1,950 questions" meta into a count.
fn questions_from_meta(meta: &[String]) -> Option<u64> {
    meta.iter().find_map(|m| {
        if !m.to_lowercase().contains("question") {
            return None;
        }
        // The count leads: "1,950 questions".
        m.split_whitespace().next()?.replace(',', "").parse().ok()
    })
}

/// Judge description from meta ("judge: gpt-oss-120b, blind, shuffled"),
/// with the leading label stripped.
fn judge_from_meta(meta: &[String]) -> Option<String> {
    meta.iter()
        .find(|m| m.to_lowercase().starts_with("judge:"))
        .map(|m| m.trim_start_matches("judge:").trim().to_string())
}

/// One `<tr>` split into cell texts.
fn row_cells(tr: &str) -> Vec<String> {
    tr.split("<t")
        .skip(1)
        .map(|cell| {
            let gt = cell.find('>').map(|i| i + 1).unwrap_or(cell.len());
            let end = cell.find("</t").unwrap_or(cell.len());
            decode_entities(&text_of(&cell[gt..end.min(cell.len())]))
        })
        .collect()
}

/// All `<table>` blocks of a page, each as rows of cell texts.
fn tables_of(html: &str) -> Vec<Vec<Vec<String>>> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(tstart) = rest.find("<table") {
        let after = &rest[tstart..];
        let Some(tend) = after.find("</table>") else {
            break;
        };
        let block = &after[..tend];
        let rows: Vec<Vec<String>> = block
            .split("<tr")
            .skip(1)
            .map(|tr| {
                let end = tr.find("</tr>").unwrap_or(tr.len());
                row_cells(&tr[..end])
            })
            .filter(|cells| !cells.is_empty())
            .collect();
        if !rows.is_empty() {
            out.push(rows);
        }
        rest = &after[tend + "</table>".len()..];
    }
    out
}

/// Column index for a header name (case-insensitive substring match).
fn col_index(header: &[String], needles: &[&str]) -> Option<usize> {
    header.iter().position(|h| {
        let hl = h.to_lowercase();
        needles.iter().any(|n| hl.contains(n))
    })
}

/// Rows of the score table of a summary/hub page: the table whose header
/// names an Accuracy column. Settings-sweep pages anchor on the Overall
/// section; hub pages simply expose their one score table.
fn overall_table_rows(html: &str) -> Vec<Vec<String>> {
    // Prefer the table after the Overall heading when present…
    if let Some(anchor) = html.find("Overall</h2>").or_else(|| html.find(">Overall<")) {
        let rest = &html[anchor..];
        if let Some(tstart) = rest.find("<table") {
            let after = &rest[tstart..];
            if let Some(tend) = after.find("</table>") {
                let rows: Vec<Vec<String>> = after[..tend]
                    .split("<tr")
                    .skip(1)
                    .map(|tr| {
                        let end = tr.find("</tr>").unwrap_or(tr.len());
                        row_cells(&tr[..end])
                    })
                    .filter(|cells| !cells.is_empty())
                    .collect();
                if !rows.is_empty() {
                    return rows;
                }
            }
        }
    }
    // …else the first table that actually has an Accuracy column.
    tables_of(html)
        .into_iter()
        .find(|rows| {
            rows.first()
                .map(|h| col_index(h, &["accuracy"]).is_some())
                .unwrap_or(false)
        })
        .unwrap_or_default()
}

/// Numeric prefix of a cell like "69.5" or "–".
fn cell_num(cell: &str) -> Option<f64> {
    let tok = cell.split_whitespace().next()?;
    tok.parse().ok()
}

/// Median tokens from a cell like "1,424".
fn cell_tokens(cell: &str) -> Option<u64> {
    cell.replace(',', "")
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Parse the score table of a summary/hub page into rows, mapping columns
/// by their header names (the shapes differ between generations: `Model
/// and setting`, `Checkpoint · setting`, split `Model`/`Setting`, …).
fn parse_sweep_rows(html: &str) -> Vec<SweepRow> {
    let rows = overall_table_rows(html);
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let Some(i_acc) = col_index(header, &["accuracy"]) else {
        return Vec::new();
    };
    let i_set = col_index(header, &["model and setting", "checkpoint", "setting"]).unwrap_or(0);
    let i_model = col_index(header, &["model"]).filter(|i| *i != i_set);
    let i_strict = col_index(header, &["fully correct", "strict"]);
    let i_school = col_index(header, &["class 1", "school"]);
    let i_higher = col_index(header, &["ug / pg", "higher"]);
    let i_general = col_index(header, &["general"]);
    let i_empty = col_index(header, &["empty"]);
    let i_tokens = col_index(header, &["median tokens"]);
    let num =
        |cells: &[String], i: Option<usize>| i.and_then(|i| cells.get(i)).and_then(|c| cell_num(c));

    let mut out = Vec::new();
    let mut seen_ref = false;
    for cells in rows.iter().skip(1) {
        if cells.len() <= i_acc {
            continue;
        }
        let mut setting = cells.get(i_set).cloned().unwrap_or_default();
        if setting.is_empty() {
            continue;
        }
        // Split Model|Setting layouts join the two columns.
        if let Some(im) = i_model {
            if let Some(m) = cells.get(im) {
                if !m.is_empty() && m != "EKA" {
                    setting = format!("{m} · {setting}");
                }
            }
        }
        // Reference rows print a checkpoint name before the setting
        // ("sft6-w8400 High @0.4 pp1.0") or a bare model name
        // ("gpt-oss-120b"); own settings carry an `@`/temperature or a
        // greedy/penalty marker.
        let looks_ref = !setting.contains('@')
            && !setting.to_lowercase().contains("greedy")
            && !setting.to_lowercase().contains("penalty")
            && !setting.contains("temp");
        let own = !looks_ref && !seen_ref;
        if !own {
            seen_ref = true;
        }
        out.push(SweepRow {
            setting,
            own,
            acc: num(cells, Some(i_acc)),
            strict: num(cells, i_strict),
            school: num(cells, i_school),
            higher: num(cells, i_higher),
            general: num(cells, i_general),
            empty: i_empty
                .and_then(|i| cells.get(i))
                .cloned()
                .filter(|c| c != "–"),
            median_tokens: i_tokens
                .and_then(|i| cells.get(i))
                .and_then(|c| cell_tokens(c)),
        });
    }
    out
}

// ------------------------------------------------------------------
// Card assembly
// ------------------------------------------------------------------

/// File mtime in ms (0 when unknown).
fn mtime_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Catalog id for a path relative to the kit root.
fn catalog_id(rel: &str) -> String {
    rel.replace('/', "::")
}

/// Discover every report card under the kit's run roots. Live-run cards
/// come from run directories; imported HTML reports come from any other
/// subdirectory. Sorted newest-first.
pub fn discover(paths: &EvalPaths) -> Vec<ReportCard> {
    let mut cards = Vec::new();
    for root in paths.run_roots() {
        if !root.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for group in rd.filter_map(|e| e.ok()) {
            let gdir = group.path();
            if !gdir.is_dir() {
                continue;
            }
            let gname = group.file_name().to_string_lossy().to_string();
            collect_group(&gdir, &gname, paths, &mut cards);
        }
    }
    cards.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then(a.id.cmp(&b.id)));
    cards.dedup_by(|a, b| a.id == b.id);
    cards.truncate(MAX_REPORT_FILES);
    cards
}

/// Collect cards from one top-level group directory under a run root.
fn collect_group(gdir: &Path, gname: &str, paths: &EvalPaths, cards: &mut Vec<ReportCard>) {
    // Is this a live run (has run signatures)?
    let signatures = ["config.json", "run.env", "questions.jsonl", "summary.json"];
    let is_run = signatures.iter().any(|s| gdir.join(s).is_file());
    if is_run {
        collect_live_run(gdir, gname, paths, cards);
        return;
    }
    // Otherwise treat it as an imported report bundle: every *.html file
    // becomes a candidate card, grouped by its checkpoint subfolder (the
    // original directory structure of the archive).
    let Ok(rd) = std::fs::read_dir(gdir) else {
        return;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            // One level of nesting: the subfolder name is the checkpoint
            // (`sft6-w6000`, `_overview`, …).
            let ckpt = entry.file_name().to_string_lossy().to_string();
            if let Ok(srd) = std::fs::read_dir(&p) {
                for se in srd.filter_map(|e| e.ok()) {
                    let sp = se.path();
                    if sp.is_file() && is_html(&sp) {
                        imported_html_card(&sp, &ckpt, paths, cards);
                    }
                }
            }
            continue;
        }
        if is_html(&p) {
            imported_html_card(&p, gname, paths, cards);
        }
    }
}

fn is_html(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_lowercase().as_str(), "html" | "htm"))
        .unwrap_or(false)
}

/// Cards from one live run directory: per-setting `report.json`/`report.html`
/// plus the run-level summary.
fn collect_live_run(run_dir: &Path, run_id: &str, paths: &EvalPaths, cards: &mut Vec<ReportCard>) {
    let cfg = std::fs::read_to_string(run_dir.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let get_cfg = |k: &str| {
        cfg.as_ref()
            .and_then(|c| c.get(k))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    // Per-setting cards.
    let settings_dir = run_dir.join("settings");
    if let Ok(rd) = std::fs::read_dir(&settings_dir) {
        let mut names: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        names.sort();
        for sdir in names {
            if !sdir.is_dir() {
                continue;
            }
            let setting = sdir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let json_path = sdir.join("report.json");
            let html_path = sdir.join("report.html");
            let csv_path = sdir.join("graded_flat.csv");
            if !json_path.is_file() && !html_path.is_file() && !csv_path.is_file() {
                continue; // nothing to show for this setting
            }
            let rep = std::fs::read_to_string(&json_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
            let eka = rep
                .as_ref()
                .and_then(|r| r.get("overall"))
                .and_then(|o| o.get("eka"));
            let f = |k: &str| eka.and_then(|e| e.get(k)).and_then(|v| v.as_f64());
            let n = rep
                .as_ref()
                .and_then(|r| r.get("n"))
                .and_then(|v| v.as_u64());
            let label = get_cfg("label").unwrap_or_else(|| run_id.to_string());
            cards.push(ReportCard {
                id: format!("{run_id}~{setting}"),
                source: ReportSource::RunReport,
                run: run_id.to_string(),
                title: format!("{label} · {setting}"),
                model: get_cfg("model"),
                setting,
                input: get_cfg("input"),
                samples: n,
                graded: n,
                acc: f("acc"),
                strict: f("strict"),
                comparisons: Vec::new(),
                judge: None,
                status: if f("acc").is_some() {
                    "completed"
                } else {
                    "incomplete"
                },
                artifacts: ReportArtifacts {
                    html: html_path
                        .is_file()
                        .then(|| catalog_id(&rel_of(paths, &html_path))),
                    json: json_path
                        .is_file()
                        .then(|| catalog_id(&rel_of(paths, &json_path))),
                    csv: csv_path
                        .is_file()
                        .then(|| catalog_id(&rel_of(paths, &csv_path))),
                    summary_html: None,
                    html_bytes: html_path.is_file().then(|| file_len(&html_path)),
                    json_bytes: json_path.is_file().then(|| file_len(&json_path)),
                    csv_bytes: csv_path.is_file().then(|| file_len(&csv_path)),
                },
                modified_ms: [Some(&json_path), Some(&html_path), Some(&csv_path)]
                    .into_iter()
                    .flatten()
                    .map(|p| mtime_of(p))
                    .max()
                    .unwrap_or(0),
                rel_path: rel_of(paths, &json_path),
            });
        }
    }

    // Run-level summary card.
    let sum_json = run_dir.join("summary.json");
    let sum_html = run_dir.join("summary.html");
    if sum_json.is_file() || sum_html.is_file() {
        let rep = std::fs::read_to_string(&sum_json)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let eka_best = rep.as_ref().and_then(|r| {
            r.get("settings")
                .and_then(|s| s.as_array())
                .and_then(|arr| {
                    arr.iter().find_map(|st| {
                        st.get("overall").and_then(|o| o.get("eka")).and_then(|e| {
                            e.get("acc").and_then(|a| a.as_f64()).map(|acc| {
                                (acc, st.get("name").and_then(|n| n.as_str()).unwrap_or(""))
                            })
                        })
                    })
                })
        });
        let label = get_cfg("label").unwrap_or_else(|| run_id.to_string());
        cards.push(ReportCard {
            id: format!("{run_id}~summary"),
            source: ReportSource::RunSummary,
            run: run_id.to_string(),
            title: format!("{label} — settings sweep"),
            model: get_cfg("model"),
            setting: "sweep".into(),
            input: get_cfg("input"),
            samples: rep
                .as_ref()
                .and_then(|r| r.get("n_questions"))
                .and_then(|v| v.as_u64()),
            graded: None,
            acc: eka_best.map(|(a, _)| a),
            strict: None,
            comparisons: Vec::new(),
            judge: None,
            status: if eka_best.is_some() {
                "completed"
            } else {
                "incomplete"
            },
            artifacts: ReportArtifacts {
                html: sum_html
                    .is_file()
                    .then(|| catalog_id(&rel_of(paths, &sum_html))),
                json: sum_json
                    .is_file()
                    .then(|| catalog_id(&rel_of(paths, &sum_json))),
                csv: None,
                summary_html: None,
                html_bytes: sum_html.is_file().then(|| file_len(&sum_html)),
                json_bytes: sum_json.is_file().then(|| file_len(&sum_json)),
                csv_bytes: None,
            },
            modified_ms: mtime_of(if sum_json.is_file() {
                &sum_json
            } else {
                &sum_html
            }),
            rel_path: rel_of(paths, &sum_json),
        });
    }
}

/// Path relative to the kit root (best effort; used for catalog ids).
fn rel_of(paths: &EvalPaths, p: &Path) -> String {
    let kit = paths.kit();
    p.strip_prefix(&kit)
        .map(|r| r.display().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// One card from an imported HTML report (detail page with embedded JSON
/// or settings-sweep summary page).
fn imported_html_card(path: &Path, group: &str, paths: &EvalPaths, cards: &mut Vec<ReportCard>) {
    let Ok(html) = std::fs::read_to_string(path) else {
        return;
    };
    let meta = std::fs::metadata(path).ok();
    let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    if bytes > MAX_HTML_SCAN_BYTES {
        return;
    }
    let modified_ms = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let rel = rel_of(paths, path);
    let id = catalog_id(&rel);

    // Detail report: embedded report-data JSON.
    if let Some(data) = extract_report_data(&html) {
        let overall_src = json_field(data, "overall");
        let scores = overall_scores(overall_src.as_deref());
        let (eka_acc, eka_strict) = {
            let eka = scores.iter().find(|s| s.model == "eka");
            (eka.and_then(|e| e.acc), eka.and_then(|e| e.strict))
        };
        let title = field_str(data, "eka_name")
            .or_else(|| h1_of(&html))
            .unwrap_or_else(|| name.clone());
        // Setting name from the file stem: `full__high04` → `high04`,
        // `sample__gap_report` → `gap_report`.
        let stem = name.trim_end_matches(".html").to_string();
        let setting = stem.rsplit("__").next().unwrap_or(&stem).to_string();
        // Model key, in order of reliability: the vLLM method note
        // ("EKA hero-1026-3000 full: served by vLLM at …"), else the lede's
        // parenthesised `(eka-sft6-w6000 …`.
        let method = field_str_vec(data, "method");
        let model = method
            .iter()
            .find(|m| m.contains("vLLM"))
            .and_then(|m| m.split(':').next().map(|n| n.trim().to_string()))
            .or_else(|| {
                // "EKA checkpoint iter3700 (65,536 context, …)" style ledes.
                field_str(data, "lede").and_then(|lede| {
                    lede.find("checkpoint ").and_then(|at| {
                        let rest = &lede[at + "checkpoint ".len()..];
                        let tok = rest
                            .split(|c: char| c == ' ' || c == '(')
                            .next()?
                            .trim()
                            .to_string();
                        (!tok.is_empty()).then_some(tok)
                    })
                })
            })
            .or_else(|| {
                // Fall back to the checkpoint folder name (`iter3000_gguf`).
                if group.starts_with('_') {
                    None
                } else {
                    Some(group.to_string())
                }
            });
        // Judge: the dedicated "Judge: <model> (…)" method note.
        let judge = method.iter().find_map(|m| {
            let at = m.find("Judge:")? + "Judge:".len();
            let rest = m[at..].trim_start();
            let name = rest.split_whitespace().next()?;
            let name = name.trim_end_matches(',').to_string();
            (!name.is_empty()).then_some(name)
        });
        // Reports without method notes still name their judge in the lede
        // ("… graded by gpt-oss-120b …"); try that before giving up.
        let judge = judge.or_else(|| {
            let lede = field_str(data, "lede")?;
            // "… graded by gpt-oss-120b …" (judge after the verb) …
            if let Some(at) = lede.find("graded by ") {
                let rest = &lede[at + "graded by ".len()..];
                let name = rest
                    .split(|c: char| c == ' ' || c == ',' || c == '(')
                    .next()?
                    .trim()
                    .to_string();
                return (!name.is_empty()).then_some(name);
            }
            // … or "gpt-oss-120b (reasoning high) graded all four answers"
            // (judge before the verb, possibly with a parenthesised mode).
            if let Some(at) = lede.find(" graded ") {
                let before = lede[..at].trim_end();
                let base = before.rsplit(|c: char| c == '.' || c == ';').next()?.trim();
                let name = base.split('(').next()?.trim().to_string();
                let name = name.split_whitespace().last()?.to_string();
                return (!name.is_empty()).then_some(name);
            }
            None
        });
        let input = method_first_input(&method);
        let artifacts = ReportArtifacts {
            html: Some(id.clone()),
            json: Some(format!("{}::data", id)),
            csv: None,
            summary_html: None,
            html_bytes: Some(bytes),
            json_bytes: Some(data.len() as u64),
            csv_bytes: None,
        };
        cards.push(ReportCard {
            id,
            source: ReportSource::ImportedHtml,
            run: group.to_string(),
            title,
            model,
            setting,
            input,
            samples: field_u64(data, "n_sampled"),
            graded: field_u64(data, "n_graded"),
            acc: eka_acc,
            strict: eka_strict,
            comparisons: scores.into_iter().filter(|s| s.model != "eka").collect(),
            judge,
            status: if eka_acc.is_some() {
                "completed"
            } else {
                "incomplete"
            },
            artifacts,
            modified_ms,
            rel_path: rel.clone(),
        });
        return;
    }

    // Settings-sweep summary or hub page: anything in the bundle that is
    // not a detail report. Pages without a parsable score table carry no
    // metrics (status: incomplete) but can still be opened from the card.
    let meta = meta_spans(&html);
    let title = h1_of(&html).unwrap_or_else(|| name.clone());
    let stem = name.trim_end_matches(".html").to_string();
    let setting = stem.rsplit("__").next().unwrap_or(&stem).to_string();
    // The lede paragraph carries `<code>eka-…</code>` (model) and
    // `<code>qbank-…</code>` (input bank).
    let codes = first_lede_codes(&html);
    let (model, input) = (codes.first().cloned(), codes.get(1).cloned());
    let rows = parse_sweep_rows(&html);
    let best_own = rows
        .iter()
        .filter(|r| r.own)
        .filter_map(|r| r.acc)
        .fold(None::<f64>, |acc, a| Some(acc.map_or(a, |b: f64| b.max(a))));
    let artifacts = ReportArtifacts {
        html: Some(id.clone()),
        json: None,
        csv: None,
        summary_html: Some(id.clone()),
        html_bytes: Some(bytes),
        json_bytes: None,
        csv_bytes: None,
    };
    cards.push(ReportCard {
        id,
        source: ReportSource::ImportedOverview,
        run: group.to_string(),
        title,
        model,
        setting: if setting == "index" {
            "overview".into()
        } else {
            "sweep".into()
        },
        input,
        samples: questions_from_meta(&meta),
        graded: None,
        acc: best_own,
        strict: None,
        comparisons: rows
            .iter()
            .filter(|r| !r.own)
            .filter_map(|r| {
                r.acc.map(|acc| ReportScore {
                    model: r.setting.clone(),
                    n: None,
                    acc: Some(acc),
                    strict: r.strict,
                    lenient: None,
                    score: None,
                })
            })
            .collect(),
        judge: judge_from_meta(&meta),
        status: if rows.iter().any(|r| r.own) {
            "completed"
        } else {
            "incomplete"
        },
        artifacts,
        modified_ms,
        rel_path: rel,
    });
}

/// Text of the first `<p class="muted">` lede paragraph, tags stripped.
fn muted_paragraph(html: &str) -> Option<String> {
    const OPEN: &str = r#"<p class="muted">"#;
    let start = html.find(OPEN)? + OPEN.len();
    let rest = &html[start..];
    let end = rest.find("</p>")?;
    let txt = decode_entities(&text_of(&rest[..end]));
    (!txt.is_empty()).then_some(txt)
}

/// `<code>…</code>` contents of the first lede paragraph, in order. These
/// pages use them for the model key and input bank.
fn first_lede_codes(html: &str) -> Vec<String> {
    let start = match html.find("<p class=\"muted\">") {
        Some(i) => i + "<p class=\"muted\">".len(),
        None => return Vec::new(),
    };
    let rest = &html[start..];
    let end = match rest.find("</p>") {
        Some(e) => e,
        None => return Vec::new(),
    };
    rest[..end]
        .split("<code>")
        .skip(1)
        .filter_map(|c| {
            let close = c.find("</code>")?;
            Some(decode_entities(&text_of(&c[..close])))
        })
        .collect()
}

/// Input bank from the first method note ("Questions: 96,500 from
/// `qbank-v2-all`; …").
fn method_first_input(method: &[String]) -> Option<String> {
    method.iter().find_map(|m| {
        if !m.starts_with("Questions:") {
            return None;
        }
        // Method notes embed the bank in <code>…</code>; strip tags first.
        let clean = m.replace("<code>", "").replace("</code>", "");
        let after = clean.split(" from ").nth(1)?;
        let tok = after
            .split(|c: char| c == ';' || c == ',')
            .next()?
            .trim()
            .trim_matches('`');
        (!tok.is_empty()).then(|| tok.to_string())
    })
}

/// Full detail for one card id.
pub fn detail(paths: &EvalPaths, cards: &[ReportCard], id: &str) -> Option<ReportDetail> {
    let card = cards.iter().find(|c| c.id == id)?;
    let mut detail = ReportDetail {
        card: card.clone(),
        sweep: Vec::new(),
        lede: None,
        method: Vec::new(),
        gen: BTreeMap::new(),
        overall: None,
    };
    let path = paths.kit().join(card.rel_path.replace("::", "/"));
    let Ok(html) = std::fs::read_to_string(&path) else {
        return Some(detail);
    };
    if let Some(data) = extract_report_data(&html) {
        detail.lede = field_str(data, "lede");
        detail.method = field_str_vec(data, "method");
        detail.overall = json_field(data, "overall")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        if let Some(gen) = json_field(data, "gen_stats")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            if let Some(map) = gen.as_object() {
                for (k, v) in map {
                    detail.gen.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(eka) = json_field(data, "eka_stats")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            detail.gen.insert("eka".into(), eka);
        }
    } else {
        // Settings-sweep summary page: rows + the muted lede paragraph.
        detail.sweep = parse_sweep_rows(&html);
        detail.lede = muted_paragraph(&html);
    }
    Some(detail)
}
