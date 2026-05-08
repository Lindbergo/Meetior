//! Meeting domain types + the in-flight `Session` that ties audio → ASR → storage.
//!
//! The `Session` owns the running pipeline tasks for one active meeting and
//! a cancellation handle to stop them cleanly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    Idle,
    Recording,
    Transcribing,
    Summarizing,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: MeetingStatus,
}

impl Meeting {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            started_at: Utc::now(),
            ended_at: None,
            status: MeetingStatus::Recording,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingDetail {
    #[serde(flatten)]
    pub meeting: Meeting,
    pub segments: Vec<TranscriptSegment>,
    pub summary: Option<String>,
    pub todos: Vec<Todo>,
}

/// Active recording session. Held in `AppState` while a meeting is live.
pub struct Session {
    pub meeting_id: String,
    pub stop: mpsc::Sender<()>,
    pub joins: Vec<JoinHandle<()>>,
}

impl Session {
    pub async fn shutdown(self) {
        let _ = self.stop.send(()).await;
        for h in self.joins {
            let _ = h.await;
        }
    }
}
