//! Question banks: bounded, cached indexing and page-at-a-time serving of
//! the kit's JSONL question files.
//!
//! The real banks are large (up to 96,500 questions / 62 MB), so they are
//! **never** loaded whole into a request. Instead each file gets an
//! [`Index`] built once by a single streaming pass that records only what
//! the browser needs:
//!
//! * per-question metadata (id, group, subject, topic, difficulty,
//!   language, type, offsets) — a few hundred bytes each;
//! * aggregate facet counts (groups, subjects, difficulties, …).
//!
//! A page request then re-reads just the lines it needs using the stored
//! byte offsets, so serving page 400 of `qbank_v2_all` costs one bounded
//! `read_at` per row, not a 62 MB scan. Indexes are cached in-process for
//! [`INDEX_TTL`] and rebuilt on demand when the file changes on disk.
//!
//! The same machinery serves the small example files (`examples/*.jsonl`),
//! which share the pipeline's schema.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::EvalPaths;

/// How long a built index stays valid before a rebuild is forced.
pub const INDEX_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Upper bound on indexed questions per file. The largest real bank holds
/// 96,500 rows; the cap leaves headroom while bounding memory on a
/// maliciously huge file.
const MAX_ROWS: usize = 500_000;

/// Upper bound on a single line. Real questions are under 2 KB; the cap
/// bounds work on malformed input.
const MAX_LINE_BYTES: u64 = 256 * 1024;

/// Longest text excerpt kept per question for search. Search runs over the
/// stored excerpt plus metadata, never the full question body, so a query
/// cannot force a full-file re-read.
const SEARCH_EXCERPT_CHARS: usize = 220;

/// One question's indexed metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionMeta {
    /// Stable id, e.g. `Class 1|C1-MATH-L1-001`.
    pub id: String,
    /// Suite: `school` / `higher` / `general`.
    pub suite: String,
    /// Group: class, year or bank section.
    pub group: String,
    pub subject: String,
    pub topic: String,
    /// Difficulty band, e.g. `L1`–`L5`.
    pub difficulty: String,
    /// Language when the question is multilingual, else empty.
    pub language: String,
    /// Question format, e.g. `mcq`, `short_answer`.
    pub question_type: String,
    /// Whether the bank carries options for this question.
    pub has_options: bool,
    /// Whether an answer key / reference is attached.
    pub has_reference: bool,
    /// Byte offset of the row inside the file (used to fetch the body).
    #[serde(skip)]
    pub offset: u64,
    /// Row length in bytes.
    #[serde(skip)]
    pub len: u32,
    /// Lowercased search haystack (question excerpt + id + topic).
    #[serde(skip)]
    pub hay: String,
}

/// Aggregate facet counts for one bank.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankFacets {
    pub suites: Vec<super::CountBucket>,
    pub groups: Vec<super::CountBucket>,
    pub subjects: Vec<super::CountBucket>,
    pub difficulties: Vec<super::CountBucket>,
    pub languages: Vec<super::CountBucket>,
    pub question_types: Vec<super::CountBucket>,
    /// Questions carrying an answer key.
    pub with_reference: u64,
    /// Questions carrying options.
    pub with_options: u64,
}

/// A built index for one JSONL question file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankIndex {
    /// File name, e.g. `qbank_v2.jsonl`.
    pub name: String,
    /// Absolute path on this machine.
    pub path: String,
    /// Bytes on disk.
    pub bytes: u64,
    /// Indexed question count.
    pub rows: u64,
    /// Rows skipped because they were not valid JSON objects.
    pub malformed: u64,
    /// Facet counts.
    pub facets: BankFacets,
    /// Distinct subjects (before capping in `facets`).
    pub distinct_subjects: u64,
    /// Distinct topics.
    pub distinct_topics: u64,
    /// Per-question metadata, file order.
    #[serde(skip)]
    pub questions: Vec<QuestionMeta>,
}

impl BankIndex {
    /// Total questions indexed.
    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }
}

/// A single served question (metadata + full body).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServedQuestion {
    pub id: String,
    pub suite: String,
    pub group: String,
    pub subject: String,
    pub topic: String,
    pub difficulty: String,
    pub language: String,
    pub question_type: String,
    pub has_options: bool,
    pub has_reference: bool,
    /// Full question text.
    pub question: String,
    /// Answer key / reference text, when the bank carries one.
    pub reference: Option<String>,
    /// Question hash used by the pipeline's comparison cache.
    pub qhash: Option<String>,
    /// Any extra fields present in the row (kept verbatim, bounded).
    pub extra: BTreeMap<String, String>,
}

/// Pull a trimmed string field out of a JSON object.
fn jstr(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Build the lowercase search haystack: question excerpt plus the fields a
/// user realistically types into a filter box.
fn haystack(q: &serde_json::Value, id: &str, group: &str, topic: &str, subject: &str) -> String {
    let mut parts: Vec<String> = vec![
        id.to_lowercase(),
        group.to_lowercase(),
        topic.to_lowercase(),
        subject.to_lowercase(),
    ];
    let body = jstr(q, "question");
    let excerpt: String = body.chars().take(SEARCH_EXCERPT_CHARS).collect();
    parts.push(excerpt.to_lowercase());
    parts.join("\n")
}

/// Index one JSONL file. Never panics: unreadable files yield an index with
/// `rows == 0` and `malformed` counting the damage, so the UI can say so.
pub fn index_file(path: &Path) -> BankIndex {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let mut idx = BankIndex {
        name,
        path: path.display().to_string(),
        bytes: 0,
        rows: 0,
        malformed: 0,
        facets: BankFacets::default(),
        distinct_subjects: 0,
        distinct_topics: 0,
        questions: Vec::new(),
    };
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return idx,
    };
    idx.bytes = meta.len();
    if !meta.is_file() || meta.len() == 0 {
        return idx;
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return idx,
    };
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut line: Vec<u8> = Vec::with_capacity(2048);
    let mut offset: u64 = 0;
    // Facet counters accumulated while streaming; frozen into capped
    // vectors after the pass so the serialized struct stays small.
    let mut suites: BTreeMap<String, u64> = BTreeMap::new();
    let mut groups: BTreeMap<String, u64> = BTreeMap::new();
    let mut difficulties: BTreeMap<String, u64> = BTreeMap::new();
    let mut languages: BTreeMap<String, u64> = BTreeMap::new();
    let mut qtypes: BTreeMap<String, u64> = BTreeMap::new();
    let mut subjects: BTreeMap<String, u64> = BTreeMap::new();
    let mut topics: BTreeMap<String, u64> = BTreeMap::new();

    loop {
        line.clear();
        let read = match reader.read_until(b'\n', &mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        let row_len = read as u64;
        let trimmed: &[u8] = trim_line(line.as_slice());
        if trimmed.is_empty() {
            offset += row_len;
            continue;
        }
        if idx.questions.len() >= MAX_ROWS {
            break;
        }
        match serde_json::from_slice::<serde_json::Value>(trimmed) {
            Ok(v) if v.is_object() => {
                let id = jstr(&v, "id");
                let subject = jstr(&v, "subject");
                let topic = jstr(&v, "topic");
                let group = jstr(&v, "group");
                let language = jstr(&v, "language");
                let reference_raw = jstr(&v, "reference");
                let has_reference = !reference_raw.is_empty()
                    || v.get("correct_answer").is_some_and(|x| !x.is_null())
                    || v.get("expected_answer").is_some_and(|x| !x.is_null());
                let has_options = v
                    .get("has_options")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false)
                    || !jstr(&v, "options").is_empty();
                let hay = haystack(&v, &id, &group, &topic, &subject);
                idx.questions.push(QuestionMeta {
                    id,
                    suite: jstr(&v, "suite"),
                    group: group.clone(),
                    subject: subject.clone(),
                    topic: topic.clone(),
                    difficulty: jstr(&v, "difficulty"),
                    language: language.clone(),
                    question_type: jstr(&v, "question_type"),
                    has_options,
                    has_reference,
                    offset,
                    len: trimmed.len() as u32,
                    hay,
                });
                bump(&mut suites, jstr(&v, "suite"));
                bump(&mut groups, group.clone());
                bump(&mut difficulties, jstr(&v, "difficulty"));
                bump(&mut languages, language);
                bump(&mut qtypes, jstr(&v, "question_type"));
                *subjects.entry(subject).or_insert(0) += 1;
                *topics.entry(topic).or_insert(0) += 1;
                if has_reference {
                    idx.facets.with_reference += 1;
                }
                if has_options {
                    idx.facets.with_options += 1;
                }
                idx.rows += 1;
            }
            _ => idx.malformed += 1,
        }
        offset += row_len;
    }

    idx.distinct_subjects = subjects.len() as u64;
    idx.distinct_topics = topics.len() as u64;
    // Facet lists are capped for the wire; the distinct_* counters above
    // keep the honest totals.
    idx.facets.suites = super::top_counts(&suites, 12);
    idx.facets.groups = super::top_counts(&groups, 40);
    idx.facets.subjects = super::top_counts(&subjects, 40);
    idx.facets.difficulties = super::top_counts(&difficulties, 12);
    idx.facets.languages = super::top_counts(&languages, 48);
    idx.facets.question_types = super::top_counts(&qtypes, 32);
    idx
}

/// Trim a trailing newline (and CR) from a raw line.
fn trim_line(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 && (raw[end - 1] == b'\n' || raw[end - 1] == b'\r') {
        end -= 1;
    }
    &raw[..end]
}

/// Increment a facet counter map.
fn bump(map: &mut BTreeMap<String, u64>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

/// Read one row's full JSON body using its stored offset.
fn read_row(path: &Path, offset: u64, len: u32) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).ok();
    Some(buf)
}

/// Convert a parsed JSON row into a [`ServedQuestion`], keeping unknown
/// fields verbatim so no bank information is lost.
fn to_served(v: serde_json::Value) -> ServedQuestion {
    let known = [
        "id",
        "suite",
        "group",
        "subject",
        "topic",
        "difficulty",
        "language",
        "question_type",
        "has_options",
        "question",
        "reference",
        "qhash",
    ];
    let mut extra = BTreeMap::new();
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            if known.contains(&k.as_str()) {
                continue;
            }
            let text = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if !text.is_empty() {
                extra.insert(k.clone(), text);
            }
        }
    }
    let reference_raw = jstr(&v, "reference");
    let has_reference = !reference_raw.is_empty()
        || v.get("correct_answer").is_some_and(|x| !x.is_null())
        || v.get("expected_answer").is_some_and(|x| !x.is_null());
    ServedQuestion {
        id: jstr(&v, "id"),
        suite: jstr(&v, "suite"),
        group: jstr(&v, "group"),
        subject: jstr(&v, "subject"),
        topic: jstr(&v, "topic"),
        difficulty: jstr(&v, "difficulty"),
        language: jstr(&v, "language"),
        question_type: jstr(&v, "question_type"),
        has_options: v
            .get("has_options")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
            || !jstr(&v, "options").is_empty(),
        has_reference,
        question: jstr(&v, "question"),
        reference: (!reference_raw.is_empty()).then_some(reference_raw),
        qhash: (!jstr(&v, "qhash").is_empty()).then(|| jstr(&v, "qhash")),
        extra,
    }
}

/// Filter parameters for a page request.
#[derive(Debug, Default, Clone)]
pub struct BankQuery {
    pub q: String,
    pub suite: Option<String>,
    pub group: Option<String>,
    pub subject: Option<String>,
    pub difficulty: Option<String>,
    pub language: Option<String>,
    pub question_type: Option<String>,
    pub page: u32,
    pub per_page: u32,
}

/// A served page of questions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankPage {
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub pages: u32,
    pub items: Vec<ServedQuestion>,
}

/// Maximum questions served per page.
pub const MAX_PER_PAGE: u32 = 100;

/// Apply the query filters to an index and return the requested page,
/// reading only the selected rows' bodies from disk.
pub fn page(index: &BankIndex, path: &Path, query: &BankQuery) -> BankPage {
    let needle = query.q.trim().to_lowercase();
    let matches = |m: &QuestionMeta| {
        if let Some(f) = &query.suite {
            if m.suite != *f {
                return false;
            }
        }
        if let Some(f) = &query.group {
            if m.group != *f {
                return false;
            }
        }
        if let Some(f) = &query.subject {
            if m.subject != *f {
                return false;
            }
        }
        if let Some(f) = &query.difficulty {
            if m.difficulty != *f {
                return false;
            }
        }
        if let Some(f) = &query.language {
            if m.language != *f {
                return false;
            }
        }
        if let Some(f) = &query.question_type {
            if m.question_type != *f {
                return false;
            }
        }
        if !needle.is_empty() && !m.hay.contains(&needle) {
            return false;
        }
        true
    };

    let filtered: Vec<&QuestionMeta> = index.questions.iter().filter(|m| matches(m)).collect();
    let total = filtered.len() as u64;
    let per_page = query.per_page.clamp(1, MAX_PER_PAGE);
    let pages = u32::try_from(total.div_ceil(per_page as u64))
        .unwrap_or(u32::MAX)
        .max(1);
    let page_no = query.page.clamp(1, pages);
    let skip = ((page_no - 1) * per_page) as usize;

    let mut items = Vec::new();
    for m in filtered.iter().skip(skip).take(per_page as usize) {
        if m.len as u64 > MAX_LINE_BYTES {
            continue;
        }
        if let Some(bytes) = read_row(path, m.offset, m.len) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                items.push(to_served(v));
            }
        }
    }

    BankPage {
        total,
        page: page_no,
        per_page,
        pages,
        items,
    }
}

/// List the kit's JSONL question files: everything under `banks/` plus the
/// example custom-question files, sorted by name.
pub fn discover(paths: &EvalPaths) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in [paths.banks(), paths.examples()] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    out.push(p);
                }
            }
        }
    }
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rows: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let banks = dir.path().join("internal_eval/banks");
        std::fs::create_dir_all(&banks).unwrap();
        std::fs::write(banks.join("tiny.jsonl"), rows.join("\n") + "\n").unwrap();
        dir
    }

    #[test]
    fn indexes_rows_and_facets() {
        let dir = fixture(&[
            r#"{"id":"A|1","suite":"school","group":"A","subject":"Maths","topic":"Numbers","difficulty":"L1","language":"","question_type":"mcq","has_options":true,"reference":"correct_answer: 4","question":"Two plus two?","qhash":"abc"}"#,
            r#"{"id":"B|2","suite":"general","group":"B","subject":"","topic":"","difficulty":"L2","language":"Hindi","question_type":"","has_options":false,"reference":"","question":"राजधानी?","qhash":"def"}"#,
            "",
            "not json at all",
        ]);
        let path = dir.path().join("internal_eval/banks/tiny.jsonl");
        let idx = index_file(&path);
        assert_eq!(idx.rows, 2);
        assert_eq!(
            idx.malformed, 1,
            "the unparseable line is counted, blanks are not"
        );
        assert_eq!(idx.name, "tiny.jsonl");
        assert_eq!(idx.facets.with_reference, 1);
        assert_eq!(idx.facets.with_options, 1);
        assert_eq!(
            idx.distinct_subjects, 2,
            "empty subject still counts as a value"
        );
        assert_eq!(idx.questions[0].id, "A|1");
        assert!(idx.questions[0].hay.contains("two plus two"));
    }

    #[test]
    fn paging_reads_only_selected_rows() {
        let rows: Vec<String> = (0..30)
            .map(|i| {
                format!(
                    r#"{{"id":"G|{i}","suite":"school","group":"G","subject":"S","topic":"T","difficulty":"L1","language":"","question_type":"","has_options":false,"reference":"","question":"question number {i}"}}"#
                )
            })
            .collect();
        let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
        let dir = fixture(&refs);
        let path = dir.path().join("internal_eval/banks/tiny.jsonl");
        let idx = index_file(&path);
        assert_eq!(idx.rows, 30);

        // Page 3 of 10-per-page → rows 20..29.
        let pg = page(
            &idx,
            &path,
            &BankQuery {
                page: 3,
                per_page: 10,
                ..Default::default()
            },
        );
        assert_eq!(pg.total, 30);
        assert_eq!(pg.pages, 3);
        assert_eq!(pg.items.len(), 10);
        assert_eq!(pg.items[0].id, "G|20");
        assert_eq!(pg.items[9].id, "G|29");

        // Out-of-range page clamps to the last page.
        let pg = page(
            &idx,
            &path,
            &BankQuery {
                page: 99,
                per_page: 10,
                ..Default::default()
            },
        );
        assert_eq!(pg.page, 3);
    }

    #[test]
    fn filters_and_search_narrow_results() {
        let dir = fixture(&[
            r#"{"id":"A|1","suite":"school","group":"Class 1","subject":"Mathematics","topic":"Addition","difficulty":"L1","language":"","question_type":"mcq","has_options":false,"reference":"","question":"What is 3 + 2?"}"#,
            r#"{"id":"B|2","suite":"general","group":"Coding","subject":"CS","topic":"Loops","difficulty":"L3","language":"Hindi","question_type":"","has_options":false,"reference":"","question":"राजधानी क्या है?"}"#,
        ]);
        let path = dir.path().join("internal_eval/banks/tiny.jsonl");
        let idx = index_file(&path);

        let q = |f: fn(&mut BankQuery)| {
            let mut bq = BankQuery {
                per_page: 50,
                ..Default::default()
            };
            f(&mut bq);
            page(&idx, &path, &bq)
        };
        assert_eq!(q(|b| b.suite = Some("school".into())).total, 1);
        assert_eq!(q(|b| b.difficulty = Some("L3".into())).total, 1);
        assert_eq!(q(|b| b.language = Some("Hindi".into())).total, 1);
        assert_eq!(q(|b| b.question_type = Some("mcq".into())).total, 1);
        // Search is case-insensitive over question text and metadata.
        assert_eq!(q(|b| b.q = "राजधानी".into()).total, 1);
        assert_eq!(q(|b| b.q = "ADDITION".into()).total, 1);
        assert_eq!(
            q(|b| b.q = "class 1".into()).total,
            1,
            "group is searchable"
        );
        assert_eq!(q(|b| b.q = "nothing matches this".into()).total, 0);
    }

    #[test]
    fn served_rows_keep_unknown_fields() {
        let dir = fixture(&[
            r#"{"id":"A|1","suite":"general","group":"Physics","subject":"","topic":"","difficulty":"L2","language":"","question_type":"","has_options":false,"correct_answer":"2 s","question":"How long?"}"#,
        ]);
        let path = dir.path().join("internal_eval/banks/tiny.jsonl");
        let idx = index_file(&path);
        let pg = page(
            &idx,
            &path,
            &BankQuery {
                per_page: 10,
                ..Default::default()
            },
        );
        assert_eq!(pg.items.len(), 1);
        assert_eq!(
            pg.items[0].extra.get("correct_answer").map(String::as_str),
            Some("2 s")
        );
        assert!(
            pg.items[0].has_reference,
            "correct_answer counts as an answer key"
        );
    }

    #[test]
    fn discover_finds_banks_and_examples() {
        let dir = tempfile::tempdir().unwrap();
        let kit = dir.path().join("internal_eval");
        std::fs::create_dir_all(kit.join("banks")).unwrap();
        std::fs::create_dir_all(kit.join("examples")).unwrap();
        std::fs::write(kit.join("banks/qbank.jsonl"), "{}\n").unwrap();
        std::fs::write(kit.join("examples/custom_questions.jsonl"), "{}\n").unwrap();
        std::fs::write(kit.join("examples/refs.tsv"), "x\ty\n").unwrap();
        let found = discover(&EvalPaths::at(dir.path()));
        let names: Vec<&str> = found
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert_eq!(names, vec!["custom_questions.jsonl", "qbank.jsonl"]);
    }

    #[test]
    fn missing_file_yields_empty_index() {
        let idx = index_file(Path::new("/nonexistent/nope.jsonl"));
        assert_eq!(idx.rows, 0);
        assert_eq!(idx.bytes, 0);
    }
}
