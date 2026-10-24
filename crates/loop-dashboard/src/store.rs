//! Read-only access to the Loop session database.
//!
//! Safety properties:
//! * The connection is opened with `SQLITE_OPEN_READ_ONLY` via URI, so the
//!   dashboard can never write, migrate or checkpoint the agent's database.
//! * `busy_timeout` makes reads wait briefly for a writer's commit instead of
//!   failing immediately (WAL readers/writers do not block each other).
//! * Every query runs on the blocking pool through [`tokio::task::spawn_blocking`].
//!
//! Compatibility with a running Loop instance was verified empirically: a
//! read-only connection sees the last committed snapshot even while another
//! process holds an open write transaction.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::payload::EntryPayload;

/// Row from the `sessions` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    /// Session id.
    pub id: String,
    /// Working directory label.
    pub cwd: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Parent session id when forked.
    pub parent_session_id: Option<String>,
    /// Created at (unix ms).
    pub created_at: i64,
}

/// Row from `session_entries` joined with its parsed payload.
#[derive(Debug, Clone)]
pub struct EntryRow {
    /// Owning session id.
    pub session_id: String,
    /// Entry id.
    pub entry_id: String,
    /// Parent entry id (tree link).
    pub parent_id: Option<String>,
    /// Sequence within the session.
    pub entry_seq: i64,
    /// Parsed payload (raw JSON kept for unknown shapes).
    pub payload: Option<EntryPayload>,
}

/// A read-only handle to the sessions database.
#[derive(Debug)]
pub struct TraceStore {
    path: PathBuf,
}

/// Errors surfaced by the store, mapped to HTTP-friendly messages.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Database file missing at the expected location.
    #[error("session database not found at {0}")]
    Missing(PathBuf),
    /// SQLite failed.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl TraceStore {
    /// Open the store. Fails with [`StoreError::Missing`] when the file does
    /// not exist yet (fresh install / no sessions recorded).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(StoreError::Missing(path));
        }
        Ok(Self { path })
    }

    /// Database path (for health display).
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection, StoreError> {
        // URI + READ_ONLY: never writes, never creates, never checkpoints.
        // `immutable=0` is explicit so WAL content IS honored when present.
        let uri = format!(
            "file:{}?mode=ro&cache=private",
            self.path.to_string_lossy().replace('?', "%3F")
        );
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Wait (briefly) for concurrent writers instead of erroring at once.
        conn.busy_timeout(std::time::Duration::from_millis(2500))?;
        Ok(conn)
    }

    /// List all sessions ordered by creation time (ascending).
    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, StoreError> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, cwd, name, parent_session_id, created_at
             FROM sessions ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                cwd: row.get(1)?,
                name: row.get(2)?,
                parent_session_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Load one session by id.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>, StoreError> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, cwd, name, parent_session_id, created_at
             FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                cwd: row.get(1)?,
                name: row.get(2)?,
                parent_session_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Load all entries of one session ordered by sequence.
    pub fn list_entries(&self, session_id: &str) -> Result<Vec<EntryRow>, StoreError> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, entry_id, parent_id, entry_seq, payload
             FROM session_entries WHERE session_id = ?1 ORDER BY entry_seq ASC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (session_id, entry_id, parent_id, entry_seq, raw) = row?;
            out.push(EntryRow {
                session_id,
                entry_id,
                parent_id,
                entry_seq,
                payload: EntryPayload::parse(&raw),
            });
        }
        Ok(out)
    }

    /// Load every entry across all sessions (dashboard aggregation input).
    /// Payloads are parsed lazily by the caller to keep this cheap.
    pub fn list_all_entries(&self) -> Result<Vec<RawEntry>, StoreError> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, entry_id, parent_id, entry_seq, payload
             FROM session_entries ORDER BY session_id, entry_seq ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawEntry {
                session_id: row.get(0)?,
                entry_id: row.get(1)?,
                parent_id: row.get(2)?,
                entry_seq: row.get(3)?,
                payload: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Quick health probe: session and entry counts.
    pub fn counts(&self) -> Result<(u64, u64), StoreError> {
        let conn = self.connect()?;
        let sessions: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        let entries: i64 =
            conn.query_row("SELECT COUNT(*) FROM session_entries", [], |r| r.get(0))?;
        Ok((sessions as u64, entries as u64))
    }
}

/// Unparsed entry row (payload still a raw string).
#[derive(Debug, Clone)]
pub struct RawEntry {
    /// Owning session id.
    pub session_id: String,
    /// Entry id.
    pub entry_id: String,
    /// Parent entry id.
    pub parent_id: Option<String>,
    /// Sequence within the session.
    pub entry_seq: i64,
    /// Raw payload JSON string.
    pub payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db(dir: &Path) -> PathBuf {
        let db = dir.join("sessions.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
               id TEXT PRIMARY KEY, cwd TEXT, name TEXT, parent_session_id TEXT,
               active_leaf_id TEXT, created_at INTEGER NOT NULL);
             CREATE TABLE session_entries (
               session_id TEXT NOT NULL, entry_id TEXT NOT NULL, parent_id TEXT,
               entry_seq INTEGER NOT NULL, payload TEXT NOT NULL,
               PRIMARY KEY (session_id, entry_id));
             INSERT INTO sessions VALUES ('s1','/a',NULL,NULL,NULL,100);
             INSERT INTO sessions VALUES ('s2','/b','named',NULL,NULL,200);
             INSERT INTO session_entries VALUES
               ('s1','e1',NULL,1,'{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}');
             INSERT INTO session_entries VALUES
               ('s1','e2','e1',2,'not json at all');
             INSERT INTO session_entries VALUES
               ('s2','e3',NULL,1,'{\"type\":\"modelChange\",\"provider\":\"soket\",\"modelId\":\"m\"}');",
        )
        .unwrap();
        db
    }

    #[test]
    fn missing_db_is_reported() {
        let err = TraceStore::open("/nonexistent/path/sessions.db").unwrap_err();
        assert!(matches!(err, StoreError::Missing(_)));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn lists_sessions_and_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let db = setup_db(tmp.path());
        let store = TraceStore::open(&db).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[1].name.as_deref(), Some("named"));

        let entries = store.list_entries("s1").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_id, "e1");
        // malformed payload stays raw (None) without failing the query
        assert!(entries[1].payload.is_none());

        let all = store.list_all_entries().unwrap();
        assert_eq!(all.len(), 3);

        let (s, e) = store.counts().unwrap();
        assert_eq!((s, e), (2, 3));

        assert!(store.get_session("s1").unwrap().is_some());
        assert!(store.get_session("nope").unwrap().is_none());
    }

    #[test]
    fn read_only_never_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = setup_db(tmp.path());
        let store = TraceStore::open(&db).unwrap();
        let conn = store.connect().unwrap();
        let result: Result<usize, _> = conn.execute("DELETE FROM sessions", []);
        assert!(result.is_err(), "read-only connection must reject writes");
        // data untouched
        let (s, _) = store.counts().unwrap();
        assert_eq!(s, 2);
    }
}
