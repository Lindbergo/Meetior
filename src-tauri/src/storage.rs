//! SQLite-backed persistence.
//!
//! Schema is initialized on first open. We keep it deliberately small:
//!   * `meetings` — one row per meeting
//!   * `segments` — transcript segments, indexed by meeting_id + start_ms
//!   * `summaries` — one summary per meeting
//!   * `todos` — extracted todos
//!
//! All write paths run on a blocking thread via `tokio::task::spawn_blocking`
//! at the call-site (or rusqlite's connection is used directly from sync code).

use std::path::Path;

use chrono::{DateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::meeting::{Meeting, MeetingDetail, MeetingStatus, Todo, TranscriptSegment};
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
            "CREATE TABLE IF NOT EXISTS meetings (
                 id          TEXT PRIMARY KEY,
                 title       TEXT NOT NULL,
                 started_at  TEXT NOT NULL,
                 ended_at    TEXT,
                 status      TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS segments (
                 meeting_id  TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
                 idx         INTEGER NOT NULL,
                 start_ms    INTEGER NOT NULL,
                 end_ms      INTEGER NOT NULL,
                 speaker     TEXT,
                 text        TEXT NOT NULL,
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
        Ok(())
    }

    pub fn insert_meeting(&self, m: &Meeting) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO meetings (id, title, started_at, ended_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                m.id,
                m.title,
                m.started_at.to_rfc3339(),
                m.ended_at.map(|t| t.to_rfc3339()),
                status_str(m.status),
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

    pub fn list_meetings(&self) -> Result<Vec<Meeting>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, started_at, ended_at, status
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
            "SELECT id, title, started_at, ended_at, status FROM meetings WHERE id = ?1",
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
        let conn = self.pool.get()?;

        let segments = conn
            .prepare(
                "SELECT start_ms, end_ms, speaker, text FROM segments
                 WHERE meeting_id = ?1 ORDER BY idx ASC",
            )?
            .query_map(params![id], |r| {
                Ok(TranscriptSegment {
                    start_ms: r.get::<_, i64>(0)? as u64,
                    end_ms: r.get::<_, i64>(1)? as u64,
                    speaker: r.get(2)?,
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

        Ok(MeetingDetail { meeting, segments, summary, todos })
    }

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
            "INSERT INTO segments (meeting_id, idx, start_ms, end_ms, speaker, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                meeting_id,
                next_idx,
                seg.start_ms as i64,
                seg.end_ms as i64,
                seg.speaker,
                seg.text,
            ],
        )?;
        Ok(())
    }

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
        let t = conn
            .query_row(
                "SELECT id, text, done FROM todos WHERE id = ?1 AND meeting_id = ?2",
                params![todo_id, meeting_id],
                |r| {
                    Ok(Todo {
                        id: r.get(0)?,
                        text: r.get(1)?,
                        done: r.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::NotFound(format!("todo {todo_id}"))
                }
                other => Error::Storage(other),
            })?;
        Ok(t)
    }
}

fn row_to_meeting(r: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    let started: String = r.get(2)?;
    let ended: Option<String> = r.get(3)?;
    let status: String = r.get(4)?;
    Ok(Meeting {
        id: r.get(0)?,
        title: r.get(1)?,
        started_at: DateTime::parse_from_rfc3339(&started)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?
            .with_timezone(&Utc),
        ended_at: ended
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|d| d.with_timezone(&Utc))
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })
            })
            .transpose()?,
        status: parse_status(&status),
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
