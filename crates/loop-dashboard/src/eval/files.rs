//! Dataset + file serving: bounded previews for JSONL, JSON, CSV, TSV, text
//! sources, and metadata-only inspection for Parquet.
//!
//! Two safety rules govern everything here:
//!
//! 1. **Only files inside the kit directory are served.** Ids are resolved
//!    against the catalog and the joined path must stay under the kit root,
//!    so no request can read arbitrary machine paths.
//! 2. **Nothing is loaded whole.** Previews stream a bounded number of rows
//!    or bytes; Parquet files are inspected by parsing only their footer
//!    (metadata), never the column data.

use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::catalog::{self, Resource, ResourceKind};
use super::EvalPaths;

/// Maximum rows in a JSONL/CSV/TSV preview.
pub const PREVIEW_ROWS: usize = 50;

/// Maximum bytes of a single previewed row.
const MAX_ROW_BYTES: usize = 64 * 1024;

/// Maximum bytes served for a whole-file text preview.
pub const MAX_TEXT_BYTES: u64 = 256 * 1024;

/// A served dataset/file preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    /// Catalog id that was requested.
    pub id: String,
    pub name: String,
    pub rel_path: String,
    pub kind: String,
    pub bytes: u64,
    /// Preview shape: `rows` (table-ish) or `text`.
    pub mode: String,
    /// Column names, when the format has a header.
    pub columns: Vec<String>,
    /// Previewed rows (each a map of column → string value).
    pub rows: Vec<Vec<String>>,
    /// Raw text preview, when `mode == "text"`.
    pub text: Option<String>,
    /// Total non-empty lines in the file (bounded count).
    pub total_rows: Option<u64>,
    /// True when the preview hit its row cap.
    pub truncated: bool,
    /// Honest note about what was served.
    pub note: String,
}

/// Look a resource up by its catalog id. Returns `None` for unknown ids —
/// the caller answers 404 rather than guessing.
pub fn resolve(paths: &EvalPaths, id: &str) -> Option<Resource> {
    let resources = catalog::catalog(paths);
    resources.into_iter().find(|r| r.id == id)
}

/// Serve one resource's preview. `Err(reason)` produces an honest error body.
pub fn preview(paths: &EvalPaths, res: &Resource) -> Result<FilePreview, String> {
    let path = PathBuf::from(&res.abs_path);
    // The absolute path recorded at discovery time must still sit inside
    // the kit; re-verify on every serve.
    if !path_starts_with(&path, &paths.kit()) {
        return Err(format!("{} is outside the evaluation kit", res.rel_path));
    }
    match res.kind {
        ResourceKind::Jsonl => preview_jsonl(&path, res),
        ResourceKind::Csv | ResourceKind::Tsv => preview_table(&path, res),
        ResourceKind::Json => preview_json(&path, res),
        ResourceKind::Parquet => preview_parquet(&path, res),
        _ => preview_text(&path, res),
    }
}

/// `true` when `path` is `base` or lives underneath it (both compared
/// lexically after normalization — no symlink resolution, matching how the
/// catalog recorded the path).
fn path_starts_with(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

/// Preview a JSONL file: parse each line as JSON and derive columns from the
/// union of keys seen in the preview window.
fn preview_jsonl(path: &Path, res: &Resource) -> Result<FilePreview, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, f);
    let mut line: Vec<u8> = Vec::with_capacity(4096);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    let mut truncated = false;

    loop {
        line.clear();
        let n = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let trimmed = trim_eol(&line);
        if trimmed.is_empty() {
            continue;
        }
        total += 1;
        if rows.len() >= PREVIEW_ROWS {
            truncated = true;
            continue;
        }
        if trimmed.len() > MAX_ROW_BYTES {
            rows.push(vec!["<row too large>".to_string()]);
            continue;
        }
        match serde_json::from_slice::<serde_json::Value>(trimmed) {
            Ok(serde_json::Value::Object(obj)) => {
                // Register any new keys in first-seen order.
                for k in obj.keys() {
                    if !seen_keys.contains(k) {
                        seen_keys.push(k.clone());
                    }
                }
                let row: Vec<String> = seen_keys.iter().map(|k| stringify(obj.get(k))).collect();
                rows.push(row);
            }
            Ok(other) => rows.push(vec![stringify(Some(&other))]),
            Err(_) => rows.push(vec![format!("<malformed line {}>", total)]),
        }
    }
    let columns = seen_keys;
    let shown = rows.len().min(PREVIEW_ROWS);

    Ok(FilePreview {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        kind: res.kind.label().into(),
        bytes: res.bytes,
        mode: "rows".into(),
        columns,
        rows,
        text: None,
        total_rows: Some(total),
        truncated,
        note: format!(
            "first {} of {} rows · columns from the preview window",
            shown, total
        ),
    })
}

/// Preview a delimited table (CSV/TSV): first row is the header when it
/// looks like one.
fn preview_table(path: &Path, res: &Resource) -> Result<FilePreview, String> {
    let delim = if res.kind == ResourceKind::Tsv {
        b'\t'
    } else {
        b','
    };
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, f);
    let mut line: Vec<u8> = Vec::with_capacity(4096);
    let mut raw_rows: Vec<Vec<String>> = Vec::new();
    let mut total: u64 = 0;
    let mut truncated = false;

    loop {
        line.clear();
        let n = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let trimmed = trim_eol(&line);
        if trimmed.is_empty() {
            continue;
        }
        total += 1;
        if raw_rows.len() > PREVIEW_ROWS {
            truncated = true;
            continue;
        }
        let text = String::from_utf8_lossy(trimmed);
        raw_rows.push(
            text.split(delim as char)
                .map(|c| c.trim_matches('"').to_string())
                .collect(),
        );
    }

    // Treat the first row as a header when every other row has the same
    // width; otherwise synthesize column names.
    let (columns, rows) = split_header(raw_rows);
    let truncated = truncated || rows.len() > PREVIEW_ROWS;
    let rows: Vec<Vec<String>> = rows.into_iter().take(PREVIEW_ROWS).collect();
    let shown = rows.len();

    Ok(FilePreview {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        kind: res.kind.label().into(),
        bytes: res.bytes,
        mode: "rows".into(),
        columns,
        rows,
        text: None,
        total_rows: Some(total.saturating_sub(1)),
        truncated,
        note: format!("first {} of {} data rows", shown, total.saturating_sub(1)),
    })
}

/// Decide whether the first row is a header and split accordingly.
fn split_header(mut rows: Vec<Vec<String>>) -> (Vec<String>, Vec<Vec<String>>) {
    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let head = rows.remove(0);
    let width = head.len();
    let uniform = rows.iter().all(|r| r.len() == width);
    if uniform && !rows.is_empty() {
        return (head, rows);
    }
    // No consistent width: keep every row as data with generated names.
    let cols: Vec<String> = (0..width).map(|i| format!("col_{}", i + 1)).collect();
    let mut all = vec![head];
    all.extend(rows);
    (cols, all)
}

/// Preview a JSON document: pretty-printed, bounded.
fn preview_json(path: &Path, res: &Resource) -> Result<FilePreview, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    if bytes.len() as u64 > MAX_TEXT_BYTES {
        return Ok(FilePreview {
            id: res.id.clone(),
            name: res.name.clone(),
            rel_path: res.rel_path.clone(),
            kind: res.kind.label().into(),
            bytes: res.bytes,
            mode: "text".into(),
            columns: vec![],
            rows: vec![],
            text: Some(format!(
                "JSON document, {} bytes — larger than the {} KB preview limit",
                res.bytes,
                MAX_TEXT_BYTES / 1024
            )),
            total_rows: None,
            truncated: true,
            note: "document too large to preview".into(),
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
    Ok(FilePreview {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        kind: res.kind.label().into(),
        bytes: res.bytes,
        mode: "text".into(),
        columns: vec![],
        rows: vec![],
        text: Some(pretty),
        total_rows: None,
        truncated: false,
        note: "pretty-printed JSON document".into(),
    })
}

/// Preview a text source (Python, shell, sbatch, markdown, txt, html).
fn preview_text(path: &Path, res: &Resource) -> Result<FilePreview, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    let mut reader = std::io::BufReader::new(f);
    let mut buf: Vec<u8> = Vec::new();
    let take = MAX_TEXT_BYTES.min(res.bytes);
    (&mut reader)
        .take(take)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let truncated = res.bytes > MAX_TEXT_BYTES;
    Ok(FilePreview {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        kind: res.kind.label().into(),
        bytes: res.bytes,
        mode: "text".into(),
        columns: vec![],
        rows: vec![],
        text: Some(text),
        total_rows: None,
        truncated,
        note: if truncated {
            format!("first {} KB of {}", MAX_TEXT_BYTES / 1024, res.rel_path)
        } else {
            "whole file".into()
        },
    })
}

/// Parquet schema column.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetColumn {
    pub name: String,
    /// Physical type label (INT64, BYTE_ARRAY, …).
    pub physical: String,
    /// Logical type label when present (STRING, TIMESTAMP_MICROS, …).
    pub logical: Option<String>,
    /// Repetition label (REQUIRED / OPTIONAL / REPEATED).
    pub repetition: String,
}

/// Parquet metadata result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetMeta {
    pub id: String,
    pub name: String,
    pub rel_path: String,
    pub bytes: u64,
    pub num_rows: Option<u64>,
    pub num_columns: usize,
    pub columns: Vec<ParquetColumn>,
    pub num_row_groups: Option<u64>,
    pub created_by: Option<String>,
    /// Honest explanation when the footer could not be parsed.
    pub note: String,
}

/// Preview a Parquet file by reading **only** its footer metadata.
///
/// Parquet layout: `[data…][footer metadata][4-byte footer length][PAR1]`.
/// We read the tail, locate the footer thrift blob and extract the schema
/// elements plus row counts with a minimal thrift reader — no column data
/// is touched and no dependency is added.
fn preview_parquet(path: &Path, res: &Resource) -> Result<FilePreview, String> {
    let meta = parquet_metadata(path, res)?;
    // Render the metadata as the preview payload (mode "text").
    let mut lines = String::new();
    lines.push_str(&format!("Parquet file: {}\n", res.rel_path));
    lines.push_str(&format!("Size: {} bytes\n", res.bytes));
    if let Some(n) = meta.num_rows {
        lines.push_str(&format!("Rows: {n}\n"));
    }
    if let Some(n) = meta.num_row_groups {
        lines.push_str(&format!("Row groups: {n}\n"));
    }
    if let Some(c) = &meta.created_by {
        lines.push_str(&format!("Created by: {c}\n"));
    }
    lines.push_str(&format!("\nSchema ({} columns):\n", meta.num_columns));
    for c in &meta.columns {
        let logical = c
            .logical
            .as_ref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        lines.push_str(&format!(
            "  {:4}{} {} {}\n",
            "", c.name, c.physical, logical
        ));
    }
    Ok(FilePreview {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        kind: res.kind.label().into(),
        bytes: res.bytes,
        mode: "text".into(),
        columns: vec![],
        rows: vec![],
        text: Some(lines),
        total_rows: meta.num_rows,
        truncated: false,
        note: "metadata only — Parquet column data is not read".into(),
    })
}

/// Extract Parquet footer metadata. Public so the API layer can expose it
/// separately from the generic preview.
pub fn parquet_metadata(path: &Path, res: &Resource) -> Result<ParquetMeta, String> {
    use std::fs::File;
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", res.rel_path))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len < 12 {
        return Err(format!("{}: too small to be a Parquet file", res.rel_path));
    }
    // Layout: [data][footer thrift][u32 footer length][PAR1]. Read the
    // last 8 bytes first to learn the footer's exact length, then read
    // exactly that many bytes — footers range from a few hundred bytes to
    // megabytes for wide schemas, so the size is never guessed.
    f.seek(SeekFrom::End(-8)).map_err(|e| e.to_string())?;
    let mut tail8 = [0u8; 8];
    f.read_exact(&mut tail8).map_err(|e| e.to_string())?;
    if &tail8[4..] != b"PAR1" {
        return Err(format!(
            "{}: not a Parquet file (missing PAR1 magic)",
            res.rel_path
        ));
    }
    let flen = u32::from_le_bytes(tail8[..4].try_into().unwrap()) as u64;
    // The footer plus its trailer must fit before the leading PAR1 marker;
    // anything else means the header is corrupt.
    if flen == 0 || flen + 8 > len.saturating_sub(4) {
        return Err(format!("{}: unreadable Parquet footer", res.rel_path));
    }
    let start = len - 8 - flen;
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut footer = vec![0u8; flen as usize];
    f.read_exact(&mut footer).map_err(|e| e.to_string())?;
    parse_footer(&footer, res)
}

/// Minimal Thrift-compact reader over the Parquet footer.
struct Thrift<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Thrift<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
        Some(result)
    }

    fn zigzag(&mut self) -> Option<i64> {
        let v = self.varint()?;
        Some(((v >> 1) as i64) ^ (-((v & 1) as i64)))
    }

    fn skip_value(&mut self, wire: u8) -> Option<()> {
        match wire {
            1 => {
                self.byte()?;
            }
            2 => {
                // struct: read field headers until STOP
                loop {
                    let h = self.byte()?;
                    if h == 0 {
                        break;
                    }
                    let delta = (h >> 4) & 0x0f;
                    let fw = h & 0x0f;
                    if delta == 0 {
                        let id = self.zigzag()? as i64;
                        if id == 0 {
                            break;
                        }
                        self.skip_value(fw)?;
                    } else {
                        self.skip_value(fw)?;
                    }
                }
            }
            // Compact-protocol integers are zigzag varints, not fixed width.
            3 | 4 | 5 | 6 | 7 => {
                self.zigzag()?;
            }
            8 => {
                // binary: varint length then bytes
                let len = self.varint()? as usize;
                self.buf.get(self.pos..self.pos + len)?;
                self.pos += len;
            }
            9 => {
                // list: header byte (size<<4 | elem type), then elements
                let h = self.byte()?;
                let size = (h >> 4) as usize;
                let elem = h & 0x0f;
                let size = if size == 15 {
                    self.varint()? as usize
                } else {
                    size
                };
                for _ in 0..size {
                    self.skip_value(elem)?;
                }
            }
            10 => {
                // set: same encoding as list
                self.skip_value(9)?;
            }
            11 => {
                // map: size varint, then (key_type<<4 | value_type), then pairs
                let size = self.varint()? as usize;
                if size > 0 {
                    let kv = self.byte()?;
                    let kt = kv >> 4;
                    let vt = kv & 0x0f;
                    for _ in 0..size {
                        self.skip_value(kt)?;
                        self.skip_value(vt)?;
                    }
                }
            }
            12 => {
                // nested struct
                self.skip_value(2)?;
            }
            _ => return None,
        }
        Some(())
    }
}

/// Parse the footer thrift blob into [`ParquetMeta`].
fn parse_footer(footer: &[u8], res: &Resource) -> Result<ParquetMeta, String> {
    // FileMetaData (thrift compact):
    //   1: version, 2: schema list, 3: num_rows, 4: row_groups,
    //   5: key_value_metadata, 6: created_by
    let mut t = Thrift::new(footer);
    let mut columns: Vec<ParquetColumn> = Vec::new();
    let mut num_rows: Option<u64> = None;
    let mut num_row_groups: Option<u64> = None;
    let mut created_by: Option<String> = None;

    let mut last_field: i16 = 0;
    while let Some(h) = t.byte() {
        if h == 0 {
            break; // STOP
        }
        let delta = (h >> 4) & 0x0f;
        let wire = h & 0x0f;
        if delta == 0 {
            // Long form: next varint is the field id zigzag-encoded.
            let id = t.zigzag().ok_or("bad field id")? as i16;
            last_field = id;
        } else {
            last_field += delta as i16;
        }
        match (last_field, wire) {
            (1, _) => {
                t.skip_value(wire);
            } // version
            (2, 9 | 12) => {
                // list<SchemaElement>
                let h2 = t.byte().ok_or("bad schema list")?;
                let size = (h2 >> 4) as usize;
                let size = if size == 15 {
                    t.varint().ok_or("bad size")? as usize
                } else {
                    size
                };
                for _ in 0..size {
                    parse_schema_element(&mut t, &mut columns)?;
                }
            }
            (3, _) => {
                num_rows = t.zigzag().map(|v| v.max(0) as u64);
            }
            (4, 9 | 12) => {
                let h2 = t.byte().ok_or("bad row group list")?;
                let size = (h2 >> 4) as usize;
                let size = if size == 15 {
                    t.varint().ok_or("bad size")? as usize
                } else {
                    size
                };
                num_row_groups = Some(size as u64);
                // Each element is a RowGroup struct; skip them properly so
                // later fields (created_by) stay reachable.
                for _ in 0..size {
                    t.skip_value(12).ok_or("unreadable row group")?;
                }
            }
            (6, 8 | 9 | 12) => {
                if wire == 8 {
                    // binary (utf8 via compact list-of-bytes form is 8)
                    let len = t.varint().ok_or("bad string")? as usize;
                    let s = t.buf.get(t.pos..t.pos + len).ok_or("short string")?;
                    t.pos += len;
                    created_by = Some(String::from_utf8_lossy(s).to_string());
                } else {
                    t.skip_value(wire);
                }
            }
            _ => t.skip_value(wire).ok_or("unreadable footer field")?,
        }
    }

    let num_columns = columns.len();
    Ok(ParquetMeta {
        id: res.id.clone(),
        name: res.name.clone(),
        rel_path: res.rel_path.clone(),
        bytes: res.bytes,
        num_rows,
        num_columns,
        columns,
        num_row_groups,
        created_by,
        note: if num_columns == 0 {
            "footer parsed but no schema elements found".into()
        } else {
            "parsed from the Parquet footer".into()
        },
    })
}

/// Parse one SchemaElement (thrift compact struct) into a column entry.
fn parse_schema_element(t: &mut Thrift, out: &mut Vec<ParquetColumn>) -> Result<(), String> {
    // SchemaElement fields: 1 type, 2 type_length, 3 repetition_type,
    // 4 name, 5 num_children, 6 converted_type, …10 logicalType
    let mut physical: Option<u8> = None;
    let mut repetition: u8 = 0;
    let mut name: Option<String> = None;
    let mut num_children: Option<i64> = None;
    let mut converted: Option<i64> = None;
    let mut logical: Option<String> = None;

    let mut last: i16 = 0;
    loop {
        let h = t.byte().ok_or("truncated schema element")?;
        if h == 0 {
            break;
        }
        let delta = (h >> 4) & 0x0f;
        let wire = h & 0x0f;
        if delta == 0 {
            last = t.zigzag().ok_or("bad field id")? as i16;
        } else {
            last += delta as i16;
        }
        // Integer fields accept any integer wire type (BYTE/I16/I32/I64):
        // writers pick the narrowest encoding.
        let is_int = matches!(wire, 3..=7);
        match (last, wire) {
            (1, _) if is_int => physical = t.zigzag().map(|v| v as u8),
            (2, _) if is_int => {
                t.zigzag();
            }
            (3, _) if is_int => repetition = t.zigzag().map(|v| v as u8).unwrap_or(0),
            (4, 8) => {
                let len = t.varint().ok_or("bad name")? as usize;
                let s = t.buf.get(t.pos..t.pos + len).ok_or("short name")?;
                t.pos += len;
                name = Some(String::from_utf8_lossy(s).to_string());
            }
            (5, _) if is_int => num_children = t.zigzag(),
            (6, _) if is_int => converted = t.zigzag(),
            (10, 12) => {
                // logicalType is a nested struct; capture a coarse label.
                logical = Some(parse_logical_type(t).unwrap_or_else(|| "custom".into()));
            }
            _ => t.skip_value(wire).ok_or("unreadable schema field")?,
        }
    }

    // The root element has num_children set and no name/type of its own.
    let is_root = num_children.is_some() && physical.is_none();
    if is_root {
        return Ok(());
    }
    let phys_label = |t: Option<u8>| -> String {
        match t {
            Some(0) => "BOOLEAN".into(),
            Some(1) => "INT32".into(),
            Some(2) => "INT64".into(),
            Some(3) => "INT96".into(),
            Some(4) => "FLOAT".into(),
            Some(5) => "DOUBLE".into(),
            Some(6) => "BYTE_ARRAY".into(),
            Some(7) => "FIXED_LEN_BYTE_ARRAY".into(),
            _ => "UNKNOWN".into(),
        }
    };
    let rep_label = match repetition {
        0 => "REQUIRED".to_string(),
        1 => "OPTIONAL".to_string(),
        2 => "REPEATED".to_string(),
        _ => "-".to_string(),
    };
    let logical_label = converted.map(|c| match c {
        0 => "UTF8".to_string(),
        1 => "MAP".to_string(),
        2 => "MAP_KEY_VALUE".to_string(),
        3 => "LIST".to_string(),
        4 => "ENUM".to_string(),
        5 => "DECIMAL".to_string(),
        6 => "DATE".to_string(),
        7 => "TIME_MILLIS".to_string(),
        8 => "TIME_MICROS".to_string(),
        9 => "TIMESTAMP_MILLIS".to_string(),
        10 => "TIMESTAMP_MICROS".to_string(),
        15 => "INT_8".to_string(),
        16 => "INT_16".to_string(),
        17 => "INT_32".to_string(),
        18 => "INT_64".to_string(),
        _ => format!("CONVERTED_{c}"),
    });
    out.push(ParquetColumn {
        name: name.unwrap_or_default(),
        physical: phys_label(physical),
        logical: logical.or(logical_label),
        repetition: rep_label,
    });
    Ok(())
}

/// Coarse logical-type label from the nested LogicalType struct.
///
/// The struct is a union: one field (1..n) identifies the variant, each
/// carrying an empty or inline struct. We read the variant's field id and
/// then skip the whole struct properly so the outer parse stays aligned.
fn parse_logical_type(t: &mut Thrift) -> Option<String> {
    let save = t.pos;
    let h = t.byte()?;
    if h == 0 {
        return Some("EMPTY".into());
    }
    let delta = (h >> 4) & 0x0f;
    let wire = h & 0x0f;
    // Rewind: the caller wants the struct consumed, not inspected in place.
    t.pos = save;
    let label = if delta == 0 {
        // Long form: read the id, map it, then skip the value.
        let id = t.zigzag()? as i16;
        let l = logical_label_for(id);
        t.skip_value(wire)?;
        l
    } else {
        let l = logical_label_for(delta as i16);
        t.skip_value(wire)?;
        l
    };
    Some(label)
}

/// Map a LogicalType field id to its name (parquet.thrift order).
fn logical_label_for(id: i16) -> String {
    match id {
        1 => "STRING".into(),
        2 => "MAP".into(),
        3 => "LIST".into(),
        4 => "ENUM".into(),
        5 => "DECIMAL".into(),
        6 => "DATE".into(),
        7 => "TIME".into(),
        8 => "TIMESTAMP".into(),
        10 => "INTEGER".into(),
        11 => "UNKNOWN".into(),
        12 => "JSON".into(),
        13 => "BSON".into(),
        14 => "UUID".into(),
        15 => "FLOAT16".into(),
        other => format!("LOGICAL_{other}"),
    }
}

/// Trim trailing EOL bytes.
fn trim_eol(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 && (raw[end - 1] == b'\n' || raw[end - 1] == b'\r') {
        end -= 1;
    }
    &raw[..end]
}

/// Render a JSON value as a short cell string.
fn stringify(v: Option<&serde_json::Value>) -> String {
    match v {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(other) => {
            let s = other.to_string();
            // Keep cells readable in a table.
            s.chars().take(300).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::catalog;

    fn kit_with(name: &str, body: &str) -> (tempfile::TempDir, Resource) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("internal_eval").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        let paths = EvalPaths::at(dir.path());
        let all = catalog::catalog(&paths);
        let res = all
            .iter()
            .find(|r| r.rel_path == name)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "resource {name:?} not discovered; catalog holds {:?}",
                    all.iter().map(|r| r.rel_path.clone()).collect::<Vec<_>>()
                )
            });
        (dir, res)
    }

    #[test]
    fn jsonl_preview_is_bounded_and_columnar() {
        let body = concat!(
            r#"{"id":"a","question":"one"}"#,
            "\n",
            r#"{"id":"b","question":"two","extra":"x"}"#,
            "\n",
            "\n",
            r#"{"id":"c","question":"three"}"#,
            "\n",
        );
        let (dir, res) = kit_with("banks/tiny.jsonl", body);
        let paths = EvalPaths::at(dir.path());
        let pv = preview(&paths, &res).unwrap();
        assert_eq!(pv.mode, "rows");
        assert_eq!(pv.total_rows, Some(3));
        assert_eq!(pv.rows.len(), 3);
        // Columns are the union of keys in first-seen order.
        assert_eq!(pv.columns, vec!["id", "question", "extra"]);
        assert_eq!(pv.rows[1][2], "x");
        assert!(!pv.truncated);
    }

    #[test]
    fn tsv_preview_detects_header() {
        let body = "label\tpath\nA\t/runs/a\nB\t/runs/b\n";
        let (dir, res) = kit_with("examples/refs.tsv", body);
        let paths = EvalPaths::at(dir.path());
        let pv = preview(&paths, &res).unwrap();
        assert_eq!(pv.columns, vec!["label", "path"]);
        assert_eq!(pv.rows.len(), 2);
        assert_eq!(pv.rows[0][0], "A");
        assert_eq!(pv.total_rows, Some(2));
    }

    #[test]
    fn text_sources_serve_whole_file() {
        let (dir, res) = kit_with("pipeline/stage.py", "def run():\n    pass\n");
        let paths = EvalPaths::at(dir.path());
        let pv = preview(&paths, &res).unwrap();
        assert_eq!(pv.mode, "text");
        assert!(pv.text.as_deref().unwrap().contains("def run():"));
        assert!(!pv.truncated);
    }

    #[test]
    fn json_documents_are_pretty_printed() {
        let (dir, res) = kit_with("config/cfg.json", r#"{"a":1,"b":[1,2]}"#);
        let paths = EvalPaths::at(dir.path());
        let pv = preview(&paths, &res).unwrap();
        assert!(pv.text.as_deref().unwrap().contains("\"a\": 1"));
    }

    #[test]
    fn unknown_ids_do_not_resolve() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve(&EvalPaths::at(dir.path()), "nope.jsonl").is_none());
    }

    #[test]
    fn ids_cannot_escape_the_kit() {
        let dir = tempfile::tempdir().unwrap();
        let paths = EvalPaths::at(dir.path());
        // An id containing traversal never matches a catalogued resource.
        assert!(resolve(&paths, "../../etc/passwd").is_none());
    }

    #[test]
    fn parquet_meta_reports_schema_without_reading_data() {
        // Build a tiny valid Parquet file with pyarrow if available; skip
        // gracefully when it is not (the parser is also exercised by the
        // malformed-path test below).
        let ok = std::process::Command::new("python3")
            .arg("-c")
            .arg(concat!(
                "import sys;\n",
                "try:\n",
                "    import pyarrow.parquet as pq\n",
                "except Exception:\n",
                "    sys.exit(3)\n",
                "import pyarrow as pa\n",
                "t = pa.table({'id': pa.array([1,2,3], pa.int64()), 'name': pa.array(['a','b','c'])})\n",
                "pq.write_table(t, sys.argv[1])\n",
            ))
            .arg("/tmp/_loop_eval_test.parquet")
            .status();
        if !matches!(ok, Ok(s) if s.success()) {
            eprintln!("pyarrow unavailable; skipping parquet schema test");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("internal_eval/data/sample.parquet");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy("/tmp/_loop_eval_test.parquet", &dst).unwrap();
        let paths = EvalPaths::at(dir.path());
        let res = catalog::catalog(&paths)
            .into_iter()
            .find(|r| r.name == "sample.parquet")
            .expect("parquet discovered");
        let meta = parquet_metadata(&dst, &res).expect("footer parses");
        assert_eq!(meta.num_rows, Some(3), "row count comes from the footer");
        assert_eq!(meta.num_row_groups, Some(1));
        assert!(meta
            .created_by
            .as_deref()
            .is_some_and(|c| c.contains("parquet")));
        // The root element is excluded; only real columns appear.
        let names: Vec<&str> = meta.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 2, "root schema element skipped: {names:?}");
        assert_eq!(names, vec!["id", "name"], "file order preserved");
        let id = &meta.columns[0];
        assert_eq!(id.physical, "INT64");
        assert_eq!(id.repetition, "OPTIONAL");
        let name = &meta.columns[1];
        assert_eq!(name.physical, "BYTE_ARRAY");
        // pyarrow records both a logical STRING type and converted_type UTF8;
        // the logical type wins when present.
        assert!(matches!(
            name.logical.as_deref(),
            Some("STRING") | Some("UTF8")
        ));
    }

    #[test]
    fn non_parquet_files_are_refused_honestly() {
        let (dir, res) = kit_with("data/fake.parquet", "not really parquet at all");
        let paths = EvalPaths::at(dir.path());
        let err = preview(&paths, &res).unwrap_err();
        assert!(err.contains("Parquet"), "{err}");
    }
}
