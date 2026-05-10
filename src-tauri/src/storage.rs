//! SQLite-backed persistence.
//!
//! Schema is initialized on first open and grows via additive migrations
//! (new tables / new columns only). For the data model see PRODUCT.md →
//! "Data model".
//!
//! Tables:
//!   * `meetings`  — one row per meeting; FK to `clients` (nullable).
//!   * `clients`   — first-class client/account entity (just name + color).
//!   * `segments`  — transcript segments, indexed by meeting_id + idx.
//!   * `notes`     — live notes the user typed during the meeting.
//!   * `summaries` — one summary per meeting.
//!   * `todos`     — extracted todos.
//!
//! Cascade rules:
//!   * `meetings.client_id` → `clients.id` ON DELETE SET NULL — deleting a
//!     client moves its meetings to "Unassigned" rather than nuking history.
//!   * `segments`/`notes`/`summaries`/`todos`.meeting_id → `meetings.id`
//!     ON DELETE CASCADE — deleting a meeting drops its child rows.

use std::path::Path;

use chrono::{DateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

use crate::meeting::{
    Client, ClientColor, Meeting, MeetingDetail, MeetingSource, MeetingStatus, Note,
    SpeakerHint, SpeakerSource, Todo, TranscriptSegment,
};
use crate::{Error, Result};

pub struct Store {
    pool: Pool<SqliteConnectionManager>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA synchronous = NORMAL;",
            )
        });
        let pool = Pool::new(manager)?;
        let store = Self { pool };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clients (
                 id          TEXT PRIMARY KEY,
                 name        TEXT NOT NULL,
                 color       TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS meetings (
                 id          TEXT PRIMARY KEY,
                 title       TEXT NOT NULL,
                 started_at  TEXT NOT NULL,
                 ended_at    TEXT,
                 status      TEXT NOT NULL,
                 client_id   TEXT REFERENCES clients(id) ON DELETE SET NULL,
                 source      TEXT NOT NULL DEFAULT 'live'
             );
             CREATE INDEX IF NOT EXISTS idx_meetings_started_at ON meetings(started_at DESC);
             CREATE INDEX IF NOT EXISTS idx_meetings_client_id ON meetings(client_id);
             CREATE TABLE IF NOT EXISTS segments (
                 meeting_id      TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
                 idx             INTEGER NOT NULL,
                 start_ms        INTEGER NOT NULL,
                 end_ms          INTEGER NOT NULL,
                 speaker         TEXT,
                 speaker_source  TEXT NOT NULL DEFAULT 'unknown',
                 text            TEXT NOT NULL,
                 PRIMARY KEY (meeting_id, idx)
             );
             CREATE TABLE IF NOT EXISTS notes (
                 meeting_id    TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
                 idx           INTEGER NOT NULL,
                 t_ms          INTEGER NOT NULL,
                 speaker_hint  TEXT NOT NULL DEFAULT 'unknown',
                 text          TEXT NOT NULL,
                 PRIMARY KEY (meeting_id, idx)
             );
             CREATE TABLE IF NOT EXISTS summaries (
                 meeting_id  TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
                 summary     TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS todos (
                 id          TEXT PRIMARY KEY,
                 meeting_id  TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
                 text        TEXT NOT NULL,
                 done        INTEGER NOT NULL DEFAULT 0,
                 created_at  TEXT NOT NULL
             );",
        )?;

        // Forward-compatibility: if a database created by an earlier build
        // is opened, top up the schema with columns added since.
        ensure_column(&conn, "meetings", "client_id", "TEXT")?;
        ensure_column(
            &conn,
            "meetings",
            "source",
            "TEXT NOT NULL DEFAULT 'live'",
        )?;
        ensure_column(
            &conn,
            "segments",
            "speaker_source",
            "TEXT NOT NULL DEFAULT 'unknown'",
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Clients
    // ------------------------------------------------------------------

    pub fn create_client(&self, name: &str) -> Result<Client> {
        let used = self.list_clients()?.into_iter().map(|c| c.color).collect::<Vec<_>>();
        let color = ClientColor::next_unused(&used);
        let client = Client::new(name, color);
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO clients (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![client.id, client.name, client.color.as_str(), client.created_at.to_rfc3339()],
        )?;
        Ok(client)
    }

    pub fn list_clients(&self) -> Result<Vec<Client>> {
        let conn = self.pool.get()?;
        let rows = conn
            .prepare("SELECT id, name, color, created_at FROM clients ORDER BY name COLLATE NOCASE")?
            .query_map([], row_to_client)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_client(&self, id: &str) -> Result<Client> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT id, name, color, created_at FROM clients WHERE id = ?1")?;
        let client = stmt
            .query_row(params![id], row_to_client)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Error::NotFound(format!("client {id}")),
                other => Error::Storage(other),
            })?;
        Ok(client)
    }

    pub fn update_client(&self, id: &str, name: &str, color: ClientColor) -> Result<Client> {
        let conn = self.pool.get()?;
        let updated = conn.execute(
            "UPDATE clients SET name = ?2, color = ?3 WHERE id = ?1",
            params![id, name, color.as_str()],
        )?;
        if updated == 0 {
            return Err(Error::NotFound(format!("client {id}")));
        }
        self.get_client(id)
    }

    pub fn delete_client(&self, id: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let n = conn.execute("DELETE FROM clients WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(Error::NotFound(format!("client {id}")));
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Meetings
    // ------------------------------------------------------------------

    pub fn insert_meeting(&self, m: &Meeting) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO meetings (id, title, started_at, ended_at, status, client_id, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                m.id,
                m.title,
                m.started_at.to_rfc3339(),
                m.ended_at.map(|t| t.to_rfc3339()),
                status_str(m.status),
                m.client_id,
                meeting_source_str(m.source),
            ],
        )?;
        Ok(())
    }

    pub fn update_meeting_status(
        &self,
        id: &str,
        status: MeetingStatus,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<Meeting> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE meetings SET status = ?2, ended_at = ?3 WHERE id = ?1",
            params![id, status_str(status), ended_at.map(|t| t.to_rfc3339())],
        )?;
        self.get_meeting_row(id)
    }

    /// Update editable fields. `client_id = Some(None)` is the wire format
    /// for "unassign"; passing `None` here means "leave the field alone."
    pub fn update_meeting(
        &self,
        id: &str,
        title: Option<&str>,
        client_id: Option<Option<&str>>,
    ) -> Result<Meeting> {
        let conn = self.pool.get()?;
        if let Some(t) = title {
            conn.execute(
                "UPDATE meetings SET title = ?2 WHERE id = ?1",
                params![id, t],
            )?;
        }
        if let Some(cid) = client_id {
            conn.execute(
                "UPDATE meetings SET client_id = ?2 WHERE id = ?1",
                params![id, cid],
            )?;
        }
        self.get_meeting_row(id)
    }

    pub fn list_meetings(&self) -> Result<Vec<Meeting>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, started_at, ended_at, status, client_id, source
             FROM meetings ORDER BY started_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_meeting)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn get_meeting_row(&self, id: &str) -> Result<Meeting> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, started_at, ended_at, status, client_id, source
             FROM meetings WHERE id = ?1",
        )?;
        let m = stmt
            .query_row(params![id], row_to_meeting)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::NotFound(format!("meeting {id}"))
                }
                other => Error::Storage(other),
            })?;
        Ok(m)
    }

    pub fn get_meeting_detail(&self, id: &str) -> Result<MeetingDetail> {
        let meeting = self.get_meeting_row(id)?;
        let client = match &meeting.client_id {
            Some(cid) => Some(self.get_client(cid)?),
            None => None,
        };

        let conn = self.pool.get()?;

        let segments = conn
            .prepare(
                "SELECT start_ms, end_ms, speaker, speaker_source, text FROM segments
                 WHERE meeting_id = ?1 ORDER BY idx ASC",
            )?
            .query_map(params![id], |r| {
                Ok(TranscriptSegment {
                    start_ms: r.get::<_, i64>(0)? as u64,
                    end_ms: r.get::<_, i64>(1)? as u64,
                    speaker: r.get(2)?,
                    speaker_source: parse_speaker_source(&r.get::<_, String>(3)?),
                    text: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let notes = conn
            .prepare(
                "SELECT idx, t_ms, speaker_hint, text FROM notes
                 WHERE meeting_id = ?1 ORDER BY idx ASC",
            )?
            .query_map(params![id], |r| {
                Ok(Note {
                    idx: r.get::<_, i64>(0)? as u32,
                    t_ms: r.get::<_, i64>(1)? as u64,
                    speaker_hint: parse_speaker_hint(&r.get::<_, String>(2)?),
                    text: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let summary: Option<String> = conn
            .query_row(
                "SELECT summary FROM summaries WHERE meeting_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok();

        let todos = conn
            .prepare(
                "SELECT id, text, done FROM todos WHERE meeting_id = ?1 ORDER BY created_at ASC",
            )?
            .query_map(params![id], |r| {
                Ok(Todo {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    done: r.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(MeetingDetail {
            meeting,
            client,
            segments,
            notes,
            summary,
            todos,
        })
    }

    // ------------------------------------------------------------------
    // Segments
    // ------------------------------------------------------------------

    pub fn append_segment(&self, meeting_id: &str, seg: &TranscriptSegment) -> Result<()> {
        let conn = self.pool.get()?;
        let next_idx: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(idx) + 1, 0) FROM segments WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO segments
                 (meeting_id, idx, start_ms, end_ms, speaker, speaker_source, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                meeting_id,
                next_idx,
                seg.start_ms as i64,
                seg.end_ms as i64,
                seg.speaker,
                speaker_source_str(seg.speaker_source),
                seg.text,
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Notes
    // ------------------------------------------------------------------

    /// Append a note line to a meeting. Returns the persisted Note (with
    /// the assigned `idx`) so the caller can show it immediately.
    pub fn append_note(
        &self,
        meeting_id: &str,
        t_ms: u64,
        speaker_hint: SpeakerHint,
        text: &str,
    ) -> Result<Note> {
        let conn = self.pool.get()?;
        let next_idx: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(idx) + 1, 0) FROM notes WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO notes (meeting_id, idx, t_ms, speaker_hint, text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                meeting_id,
                next_idx,
                t_ms as i64,
                speaker_hint_str(speaker_hint),
                text,
            ],
        )?;
        Ok(Note {
            idx: next_idx as u32,
            t_ms,
            speaker_hint,
            text: text.to_string(),
        })
    }

    pub fn list_notes(&self, meeting_id: &str) -> Result<Vec<Note>> {
        let conn = self.pool.get()?;
        let rows = conn
            .prepare(
                "SELECT idx, t_ms, speaker_hint, text FROM notes
                 WHERE meeting_id = ?1 ORDER BY idx ASC",
            )?
            .query_map(params![meeting_id], |r| {
                Ok(Note {
                    idx: r.get::<_, i64>(0)? as u32,
                    t_ms: r.get::<_, i64>(1)? as u64,
                    speaker_hint: parse_speaker_hint(&r.get::<_, String>(2)?),
                    text: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // Summaries / todos
    // ------------------------------------------------------------------

    pub fn save_summary(&self, meeting_id: &str, summary: &str, todos: &[Todo]) -> Result<()> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO summaries (meeting_id, summary, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(meeting_id) DO UPDATE SET summary = excluded.summary,
                                                   created_at = excluded.created_at",
            params![meeting_id, summary, Utc::now().to_rfc3339()],
        )?;
        tx.execute("DELETE FROM todos WHERE meeting_id = ?1", params![meeting_id])?;
        for t in todos {
            tx.execute(
                "INSERT INTO todos (id, meeting_id, text, done, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    t.id,
                    meeting_id,
                    t.text,
                    t.done as i64,
                    Utc::now().to_rfc3339(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn toggle_todo(&self, meeting_id: &str, todo_id: &str) -> Result<Todo> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE todos SET done = 1 - done WHERE id = ?1 AND meeting_id = ?2",
            params![todo_id, meeting_id],
        )?;
        self.get_todo(meeting_id, todo_id)
    }

    /// Update only the summary text (preserves todos). Used by the
    /// edit-summary affordance in the detail view.
    pub fn update_summary_text(&self, meeting_id: &str, summary: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "UPDATE summaries SET summary = ?2 WHERE meeting_id = ?1",
            params![meeting_id, summary],
        )?;
        if n == 0 {
            // No row yet — first edit before AI summary exists. Insert one.
            conn.execute(
                "INSERT INTO summaries (meeting_id, summary, created_at)
                 VALUES (?1, ?2, ?3)",
                params![meeting_id, summary, Utc::now().to_rfc3339()],
            )?;
        }
        Ok(())
    }

    /// Append a manual todo. The id is generated server-side so the UI
    /// receives a fully-formed Todo to render immediately.
    pub fn add_todo(&self, meeting_id: &str, text: &str) -> Result<Todo> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.pool.get()?;
        // Verify meeting exists so we get a clean NotFound rather than a FK
        // violation surfaced as a generic SQL error.
        let _ = self.get_meeting_row(meeting_id)?;
        conn.execute(
            "INSERT INTO todos (id, meeting_id, text, done, created_at)
             VALUES (?1, ?2, ?3, 0, ?4)",
            params![id, meeting_id, text, Utc::now().to_rfc3339()],
        )?;
        Ok(Todo { id, text: text.to_string(), done: false })
    }

    pub fn update_todo_text(&self, meeting_id: &str, todo_id: &str, text: &str) -> Result<Todo> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "UPDATE todos SET text = ?3 WHERE id = ?1 AND meeting_id = ?2",
            params![todo_id, meeting_id, text],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("todo {todo_id}")));
        }
        self.get_todo(meeting_id, todo_id)
    }

    pub fn delete_todo(&self, meeting_id: &str, todo_id: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "DELETE FROM todos WHERE id = ?1 AND meeting_id = ?2",
            params![todo_id, meeting_id],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("todo {todo_id}")));
        }
        Ok(())
    }

    fn get_todo(&self, meeting_id: &str, todo_id: &str) -> Result<Todo> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, text, done FROM todos WHERE id = ?1 AND meeting_id = ?2",
        )?;
        let t = stmt
            .query_row(params![todo_id, meeting_id], |r| {
                Ok(Todo {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    done: r.get::<_, i64>(2)? != 0,
                })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::NotFound(format!("todo {todo_id}"))
                }
                other => Error::Storage(other),
            })?;
        Ok(t)
    }
}

// ----------------------------------------------------------------------
// Row mapping + enum (de)serialization helpers
// ----------------------------------------------------------------------

fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !cols.iter().any(|c| c == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

fn row_to_meeting(r: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    let started: String = r.get(2)?;
    let ended: Option<String> = r.get(3)?;
    let status: String = r.get(4)?;
    let client_id: Option<String> = r.get(5)?;
    let source: String = r.get(6)?;
    Ok(Meeting {
        id: r.get(0)?,
        title: r.get(1)?,
        started_at: parse_ts(&started, 2)?,
        ended_at: ended.map(|s| parse_ts(&s, 3)).transpose()?,
        status: parse_status(&status),
        client_id,
        source: parse_meeting_source(&source),
    })
}

fn row_to_client(r: &rusqlite::Row<'_>) -> rusqlite::Result<Client> {
    let created: String = r.get(3)?;
    Ok(Client {
        id: r.get(0)?,
        name: r.get(1)?,
        color: ClientColor::from_str(&r.get::<_, String>(2)?).unwrap_or(ClientColor::Gray),
        created_at: parse_ts(&created, 3)?,
    })
}

fn parse_ts(s: &str, col: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
        })
}

fn status_str(s: MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Idle => "idle",
        MeetingStatus::Recording => "recording",
        MeetingStatus::Transcribing => "transcribing",
        MeetingStatus::Summarizing => "summarizing",
        MeetingStatus::Done => "done",
        MeetingStatus::Error => "error",
    }
}

fn parse_status(s: &str) -> MeetingStatus {
    match s {
        "recording" => MeetingStatus::Recording,
        "transcribing" => MeetingStatus::Transcribing,
        "summarizing" => MeetingStatus::Summarizing,
        "done" => MeetingStatus::Done,
        "error" => MeetingStatus::Error,
        _ => MeetingStatus::Idle,
    }
}

fn meeting_source_str(s: MeetingSource) -> &'static str {
    match s {
        MeetingSource::Live => "live",
        MeetingSource::Imported => "imported",
    }
}

fn parse_meeting_source(s: &str) -> MeetingSource {
    match s {
        "imported" => MeetingSource::Imported,
        _ => MeetingSource::Live,
    }
}

fn speaker_source_str(s: SpeakerSource) -> &'static str {
    match s {
        SpeakerSource::Mic => "mic",
        SpeakerSource::System => "system",
        SpeakerSource::Unknown => "unknown",
    }
}

fn parse_speaker_source(s: &str) -> SpeakerSource {
    match s {
        "mic" => SpeakerSource::Mic,
        "system" => SpeakerSource::System,
        _ => SpeakerSource::Unknown,
    }
}

fn speaker_hint_str(h: SpeakerHint) -> &'static str {
    match h {
        SpeakerHint::You => "you",
        SpeakerHint::Them => "them",
        SpeakerHint::Unknown => "unknown",
    }
}

fn parse_speaker_hint(s: &str) -> SpeakerHint {
    match s {
        "you" => SpeakerHint::You,
        "them" => SpeakerHint::Them,
        _ => SpeakerHint::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::Meeting;

    fn store() -> Store {
        let path = std::env::temp_dir()
            .join(format!("meetior_test_{}.sqlite", uuid::Uuid::new_v4()));
        Store::open(path).unwrap()
    }

    // --- meetings + segments + summary + todos -------------------------

    #[test]
    fn round_trips_a_meeting() {
        let s = store();
        let m = Meeting::new("standup");
        s.insert_meeting(&m).unwrap();
        let listed = s.list_meetings().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "standup");
        assert_eq!(listed[0].status, MeetingStatus::Recording);
        assert!(listed[0].client_id.is_none());
        assert_eq!(listed[0].source, MeetingSource::Live);
    }

    #[test]
    fn appends_segments_in_order() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        for (i, text) in ["hello", "world", "again"].iter().enumerate() {
            s.append_segment(
                &m.id,
                &TranscriptSegment {
                    start_ms: (i * 1000) as u64,
                    end_ms: (i * 1000 + 800) as u64,
                    speaker: Some("Alice".into()),
                    speaker_source: SpeakerSource::Unknown,
                    text: (*text).into(),
                },
            )
            .unwrap();
        }
        let detail = s.get_meeting_detail(&m.id).unwrap();
        let texts: Vec<_> = detail.segments.iter().map(|x| x.text.clone()).collect();
        assert_eq!(texts, vec!["hello", "world", "again"]);
    }

    #[test]
    fn save_summary_replaces_todos() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();

        let todo_a = Todo { id: "a".into(), text: "first".into(), done: false };
        s.save_summary(&m.id, "first run", &[todo_a]).unwrap();

        let todo_b = Todo { id: "b".into(), text: "second".into(), done: false };
        s.save_summary(&m.id, "second run", &[todo_b]).unwrap();

        let detail = s.get_meeting_detail(&m.id).unwrap();
        assert_eq!(detail.summary.as_deref(), Some("second run"));
        assert_eq!(detail.todos.len(), 1);
        assert_eq!(detail.todos[0].id, "b");
    }

    #[test]
    fn toggle_todo_flips_done() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let todo = Todo { id: "t1".into(), text: "do it".into(), done: false };
        s.save_summary(&m.id, "x", &[todo]).unwrap();

        let after = s.toggle_todo(&m.id, "t1").unwrap();
        assert!(after.done);
        let again = s.toggle_todo(&m.id, "t1").unwrap();
        assert!(!again.done);
    }

    #[test]
    fn update_status_sets_ended_at() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let ended = chrono::Utc::now();
        let updated = s
            .update_meeting_status(&m.id, MeetingStatus::Done, Some(ended))
            .unwrap();
        assert_eq!(updated.status, MeetingStatus::Done);
        assert!(updated.ended_at.is_some());
    }

    // --- clients --------------------------------------------------------

    #[test]
    fn create_client_auto_assigns_first_palette_color() {
        let s = store();
        let c = s.create_client("Acme Corp").unwrap();
        assert_eq!(c.name, "Acme Corp");
        assert_eq!(c.color, ClientColor::Red);
    }

    #[test]
    fn create_client_skips_used_colors() {
        let s = store();
        let _ = s.create_client("Acme").unwrap();
        let c = s.create_client("Foo Inc").unwrap();
        assert_eq!(c.color, ClientColor::Orange);
    }

    #[test]
    fn list_clients_sorted_by_name_case_insensitive() {
        let s = store();
        s.create_client("zebra").unwrap();
        s.create_client("Alpha").unwrap();
        s.create_client("middle").unwrap();
        let names: Vec<_> = s.list_clients().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["Alpha", "middle", "zebra"]);
    }

    #[test]
    fn update_client_changes_name_and_color() {
        let s = store();
        let c = s.create_client("Acme").unwrap();
        let updated = s.update_client(&c.id, "Acme Corp", ClientColor::Indigo).unwrap();
        assert_eq!(updated.name, "Acme Corp");
        assert_eq!(updated.color, ClientColor::Indigo);
    }

    #[test]
    fn delete_client_nullifies_meeting_client_id() {
        let s = store();
        let client = s.create_client("Acme").unwrap();
        let m = Meeting::new("kickoff").with_client(Some(client.id.clone()));
        s.insert_meeting(&m).unwrap();

        s.delete_client(&client.id).unwrap();

        let listed = s.list_meetings().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].client_id.is_none(),
                "deleting a client should set meeting.client_id to NULL, not delete the meeting");
    }

    #[test]
    fn get_meeting_detail_includes_client_when_assigned() {
        let s = store();
        let client = s.create_client("Acme").unwrap();
        let m = Meeting::new("kickoff").with_client(Some(client.id.clone()));
        s.insert_meeting(&m).unwrap();

        let detail = s.get_meeting_detail(&m.id).unwrap();
        assert_eq!(detail.client.as_ref().map(|c| c.name.as_str()), Some("Acme"));
    }

    #[test]
    fn update_meeting_changes_title_and_client() {
        let s = store();
        let c = s.create_client("Acme").unwrap();
        let m = Meeting::new("draft title");
        s.insert_meeting(&m).unwrap();

        let updated = s
            .update_meeting(&m.id, Some("real title"), Some(Some(c.id.as_str())))
            .unwrap();
        assert_eq!(updated.title, "real title");
        assert_eq!(updated.client_id.as_deref(), Some(c.id.as_str()));

        // Unassign by passing Some(None).
        let unassigned = s.update_meeting(&m.id, None, Some(None)).unwrap();
        assert!(unassigned.client_id.is_none());
        assert_eq!(unassigned.title, "real title", "title should be unchanged");
    }

    // --- notes ----------------------------------------------------------

    #[test]
    fn append_note_increments_idx() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();

        let n1 = s.append_note(&m.id, 1500, SpeakerHint::You, "first thought").unwrap();
        let n2 = s.append_note(&m.id, 4200, SpeakerHint::Them, "client said X").unwrap();

        assert_eq!(n1.idx, 0);
        assert_eq!(n2.idx, 1);
        assert_eq!(n1.speaker_hint, SpeakerHint::You);
        assert_eq!(n2.speaker_hint, SpeakerHint::Them);
    }

    #[test]
    fn list_notes_returns_in_idx_order_and_meeting_detail_includes_them() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        s.append_note(&m.id, 1000, SpeakerHint::You, "a").unwrap();
        s.append_note(&m.id, 2000, SpeakerHint::Them, "b").unwrap();
        s.append_note(&m.id, 3000, SpeakerHint::Unknown, "c").unwrap();

        let listed = s.list_notes(&m.id).unwrap();
        let texts: Vec<_> = listed.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);

        let detail = s.get_meeting_detail(&m.id).unwrap();
        assert_eq!(detail.notes.len(), 3);
        assert_eq!(detail.notes[1].text, "b");
    }

    #[test]
    fn deleting_meeting_cascades_to_notes() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        s.append_note(&m.id, 0, SpeakerHint::You, "note").unwrap();

        let conn = s.pool.get().unwrap();
        conn.execute("DELETE FROM meetings WHERE id = ?1", params![m.id]).unwrap();

        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE meeting_id = ?1",
                params![m.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0);
    }

    // --- granular summary + todo edits ---------------------------------

    #[test]
    fn update_summary_text_creates_then_updates() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();

        // First call inserts (no AI summary yet).
        s.update_summary_text(&m.id, "user-written summary").unwrap();
        let detail = s.get_meeting_detail(&m.id).unwrap();
        assert_eq!(detail.summary.as_deref(), Some("user-written summary"));

        // Second call updates in place; existing todos unaffected.
        let todo = Todo { id: "t1".into(), text: "x".into(), done: false };
        s.save_summary(&m.id, "ai summary", &[todo]).unwrap();
        s.update_summary_text(&m.id, "edited by user").unwrap();

        let detail = s.get_meeting_detail(&m.id).unwrap();
        assert_eq!(detail.summary.as_deref(), Some("edited by user"));
        assert_eq!(detail.todos.len(), 1, "todos must survive a summary edit");
    }

    #[test]
    fn add_todo_appends_manual_item() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let ai = Todo { id: "ai".into(), text: "ai item".into(), done: false };
        s.save_summary(&m.id, "x", &[ai]).unwrap();

        let manual = s.add_todo(&m.id, "manual item").unwrap();
        assert!(!manual.done);
        assert_eq!(manual.text, "manual item");

        let detail = s.get_meeting_detail(&m.id).unwrap();
        let texts: Vec<_> = detail.todos.iter().map(|t| t.text.clone()).collect();
        assert_eq!(texts, vec!["ai item", "manual item"]);
    }

    #[test]
    fn add_todo_on_unknown_meeting_returns_not_found() {
        let s = store();
        let err = s.add_todo("not-a-meeting", "x").unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn update_todo_text_changes_only_text() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let todo = Todo { id: "t1".into(), text: "original".into(), done: true };
        s.save_summary(&m.id, "x", &[todo]).unwrap();

        let updated = s.update_todo_text(&m.id, "t1", "edited").unwrap();
        assert_eq!(updated.text, "edited");
        assert!(updated.done, "done flag must be preserved across text edits");
    }

    #[test]
    fn delete_todo_removes_only_target() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let a = Todo { id: "a".into(), text: "keep".into(), done: false };
        let b = Todo { id: "b".into(), text: "drop".into(), done: false };
        s.save_summary(&m.id, "x", &[a, b]).unwrap();

        s.delete_todo(&m.id, "b").unwrap();
        let detail = s.get_meeting_detail(&m.id).unwrap();
        let ids: Vec<_> = detail.todos.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[test]
    fn delete_todo_unknown_returns_not_found() {
        let s = store();
        let m = Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        let err = s.delete_todo(&m.id, "nope").unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
    }
}
