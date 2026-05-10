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

/// How a meeting got into the system. `Live` = recorded with mic + system
/// audio. `Imported` = decoded from an audio file the user dropped in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MeetingSource {
    #[default]
    Live,
    Imported,
}

/// Which capture stream a transcript segment came from.
///
/// Used as a cheap proxy for "you vs them" labelling in the UI without real
/// diarization (M3+). For fixture / imported audio we don't know, so the
/// default is `Unknown`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerSource {
    Mic,
    System,
    #[default]
    Unknown,
}

/// Speaker hint stamped on a Note when the user finalizes a line.
///
/// Derived from which audio source has been louder in the recent window when
/// the user pressed Enter. Kept separate from `SpeakerSource` because the
/// note is user-facing and the segment is internal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerHint {
    You,
    Them,
    #[default]
    Unknown,
}

/// Curated palette for clients (see PRODUCT.md → Decisions → Color palette).
/// The hex mapping lives in the frontend; the backend just stores the name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientColor {
    Red,
    Orange,
    Amber,
    Green,
    Teal,
    Blue,
    Indigo,
    Purple,
    Gray,
}

impl ClientColor {
    /// Palette order. New clients are auto-assigned the first color not in
    /// use; after all 9 are used the picker wraps.
    pub const PALETTE: [ClientColor; 9] = [
        Self::Red,
        Self::Orange,
        Self::Amber,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Indigo,
        Self::Purple,
        Self::Gray,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Amber => "amber",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Indigo => "indigo",
            Self::Purple => "purple",
            Self::Gray => "gray",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Self::PALETTE.iter().copied().find(|c| c.as_str() == s)
    }

    /// Pick the next palette entry not present in `used`. Wraps to `Red` if
    /// every color is taken.
    pub fn next_unused(used: &[ClientColor]) -> ClientColor {
        Self::PALETTE
            .iter()
            .copied()
            .find(|c| !used.contains(c))
            .unwrap_or(Self::Red)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub color: ClientColor,
    pub created_at: DateTime<Utc>,
}

impl Client {
    pub fn new(name: impl Into<String>, color: ClientColor) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            color,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: MeetingStatus,
    /// Nullable: meetings can be "Unassigned" (PRODUCT.md → Decisions).
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub source: MeetingSource,
}

impl Meeting {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            started_at: Utc::now(),
            ended_at: None,
            status: MeetingStatus::Recording,
            client_id: None,
            source: MeetingSource::Live,
        }
    }

    pub fn with_client(mut self, client_id: Option<String>) -> Self {
        self.client_id = client_id;
        self
    }

    pub fn with_source(mut self, source: MeetingSource) -> Self {
        self.source = source;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Free-form display label (e.g. fixture-supplied "Alice", or eventually
    /// a diarization output). The deterministic mic/system tag lives in
    /// `speaker_source`.
    pub speaker: Option<String>,
    #[serde(default)]
    pub speaker_source: SpeakerSource,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub idx: u32,
    pub t_ms: u64,
    pub speaker_hint: SpeakerHint,
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
    pub client: Option<Client>,
    pub segments: Vec<TranscriptSegment>,
    pub notes: Vec<Note>,
    pub summary: Option<String>,
    pub todos: Vec<Todo>,
}

/// A note returned by `search_notes`, denormalized with enough meeting
/// metadata to render the result list without a second round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteHit {
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_started_at: DateTime<Utc>,
    pub client_id: Option<String>,
    pub idx: u32,
    pub t_ms: u64,
    pub speaker_hint: SpeakerHint,
    pub text: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_unused_picks_first_palette_entry_when_none_used() {
        assert_eq!(ClientColor::next_unused(&[]), ClientColor::Red);
    }

    #[test]
    fn next_unused_skips_used_colors() {
        let used = [ClientColor::Red, ClientColor::Orange, ClientColor::Amber];
        assert_eq!(ClientColor::next_unused(&used), ClientColor::Green);
    }

    #[test]
    fn next_unused_wraps_when_all_taken() {
        assert_eq!(
            ClientColor::next_unused(&ClientColor::PALETTE),
            ClientColor::Red
        );
    }

    #[test]
    fn color_round_trips_through_string() {
        for c in ClientColor::PALETTE {
            assert_eq!(ClientColor::from_str(c.as_str()), Some(c));
        }
    }

    /// Snapshot of the JSON keys we ship across the Tauri IPC boundary.
    ///
    /// This catches the failure mode where someone adds / renames a Rust
    /// field but forgets to update `src/lib/api.ts`. A breakage here means
    /// the frontend `interface` is out of sync — the runtime symptom would
    /// be silent `undefined` reads in Svelte components, which type-check
    /// can't detect since TS believes the type definition.
    ///
    /// If a key changes intentionally, update both this test and api.ts in
    /// the same commit.
    #[test]
    fn ipc_payload_field_names_are_frozen() {
        use serde_json::Value;

        fn keys(v: Value) -> Vec<String> {
            let mut k: Vec<String> = v
                .as_object()
                .expect("expected object")
                .keys()
                .cloned()
                .collect();
            k.sort();
            k
        }

        let meeting = Meeting::new("t");
        assert_eq!(
            keys(serde_json::to_value(&meeting).unwrap()),
            vec![
                "client_id",
                "ended_at",
                "id",
                "source",
                "started_at",
                "status",
                "title",
            ],
        );

        let client = Client::new("Acme", ClientColor::Red);
        assert_eq!(
            keys(serde_json::to_value(&client).unwrap()),
            vec!["color", "created_at", "id", "name"],
        );

        let note = Note {
            idx: 0,
            t_ms: 1000,
            speaker_hint: SpeakerHint::You,
            text: "hi".into(),
        };
        assert_eq!(
            keys(serde_json::to_value(&note).unwrap()),
            vec!["idx", "speaker_hint", "t_ms", "text"],
        );

        let hit = NoteHit {
            meeting_id: "m".into(),
            meeting_title: "t".into(),
            meeting_started_at: chrono::Utc::now(),
            client_id: None,
            idx: 0,
            t_ms: 0,
            speaker_hint: SpeakerHint::Unknown,
            text: "".into(),
        };
        assert_eq!(
            keys(serde_json::to_value(&hit).unwrap()),
            vec![
                "client_id",
                "idx",
                "meeting_id",
                "meeting_started_at",
                "meeting_title",
                "speaker_hint",
                "t_ms",
                "text",
            ],
        );

        // Enum variants serialize lowercase (rename_all attribute on each).
        assert_eq!(serde_json::to_value(SpeakerHint::You).unwrap(), "you");
        assert_eq!(serde_json::to_value(SpeakerSource::Mic).unwrap(), "mic");
        assert_eq!(serde_json::to_value(MeetingSource::Imported).unwrap(), "imported");
        assert_eq!(serde_json::to_value(ClientColor::Indigo).unwrap(), "indigo");
        assert_eq!(serde_json::to_value(MeetingStatus::Recording).unwrap(), "recording");
    }
}
