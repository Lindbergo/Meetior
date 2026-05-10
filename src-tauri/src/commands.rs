//! Tauri commands — the bridge between the Svelte UI and the Rust backend.
//!
//! Keep these thin: validate inputs, dispatch to a domain module, return
//! serializable values. No business logic should live here.

use chrono::Utc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use crate::asr::{Asr, AsrConfig};
use crate::audio;
use crate::meeting::{
    Client, ClientColor, Meeting, MeetingDetail, MeetingStatus, Note, NoteHit, Session,
    SpeakerHint, Todo, TranscriptSegment,
};
use crate::{AppState, Error, Result};

const EVT_TRANSCRIPT_SEGMENT: &str = "meetior://transcript-segment";
const EVT_MEETING_STATUS: &str = "meetior://meeting-status";

#[tauri::command]
pub async fn start_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    client_id: Option<String>,
) -> Result<Meeting> {
    let mut session_slot = state.session.lock().await;
    if session_slot.is_some() {
        return Err(Error::InvalidState("a meeting is already recording".into()));
    }

    let title = title.unwrap_or_else(|| {
        format!("Meeting {}", Utc::now().format("%Y-%m-%d %H:%M"))
    });
    let meeting = Meeting::new(title).with_client(client_id);
    state.store.insert_meeting(&meeting)?;

    // Capture → ASR → store + emit.
    //
    // If `MEETIOR_FIXTURE_TRANSCRIPT` is set we bypass audio + ASR and replay a
    // canned transcript instead. This is the fast path for UI / summarizer
    // iteration on machines without ScreenCaptureKit or a Parakeet model.
    let (stop_tx, stop_rx) = mpsc::channel::<()>(1);
    let mut transcript_rx = if let Some(path) = crate::asr::fixture_transcript_path() {
        tracing::info!(?path, "using fixture transcript stream");
        // Drop stop_rx into a no-op task so the channel stays alive until stop.
        let mut stop_rx = stop_rx;
        tokio::spawn(async move { let _ = stop_rx.recv().await; });
        crate::asr::spawn_fixture_stream(&path).map_err(|e| Error::Asr(e.to_string()))?
    } else {
        let audio_rx =
            audio::start_capture(stop_rx).map_err(|e| Error::Audio(e.to_string()))?;
        let asr = Asr::new(AsrConfig::default()).map_err(|e| Error::Asr(e.to_string()))?;
        asr.spawn_streaming(audio_rx)
    };

    let store = state.store.clone();
    let meeting_id = meeting.id.clone();
    let app_for_task = app.clone();
    let join = tokio::spawn(async move {
        while let Some(seg) = transcript_rx.recv().await {
            if let Err(e) = store.append_segment(&meeting_id, &seg) {
                tracing::error!(?e, "failed to append segment");
                continue;
            }
            let payload = TranscriptEventPayload {
                meeting_id: meeting_id.clone(),
                segment: seg,
            };
            let _ = app_for_task.emit(EVT_TRANSCRIPT_SEGMENT, &payload);
        }
    });

    *session_slot = Some(Session {
        meeting_id: meeting.id.clone(),
        stop: stop_tx,
        joins: vec![join],
    });

    let _ = app.emit(EVT_MEETING_STATUS, &meeting);
    Ok(meeting)
}

#[tauri::command]
pub async fn stop_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Meeting> {
    let session = {
        let mut slot = state.session.lock().await;
        slot.take()
    };

    let session = session.ok_or_else(|| Error::InvalidState("no active meeting".into()))?;
    if session.meeting_id != id {
        return Err(Error::InvalidState("meeting id does not match active session".into()));
    }
    session.shutdown().await;

    let meeting = state
        .store
        .update_meeting_status(&id, MeetingStatus::Done, Some(Utc::now()))?;
    let _ = app.emit(EVT_MEETING_STATUS, &meeting);
    Ok(meeting)
}

#[tauri::command]
pub async fn list_meetings(state: State<'_, AppState>) -> Result<Vec<Meeting>> {
    state.store.list_meetings()
}

#[tauri::command]
pub async fn get_meeting(state: State<'_, AppState>, id: String) -> Result<MeetingDetail> {
    state.store.get_meeting_detail(&id)
}

#[tauri::command]
pub async fn summarize_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<MeetingDetail> {
    let detail = state.store.get_meeting_detail(&id)?;
    if detail.segments.is_empty() {
        return Err(Error::InvalidState("no transcript to summarize".into()));
    }

    let m = state.store.update_meeting_status(&id, MeetingStatus::Summarizing, detail.meeting.ended_at)?;
    let _ = app.emit(EVT_MEETING_STATUS, &m);

    let summary = state
        .summarizer
        .summarize(&detail.segments)
        .await
        .map_err(|e| Error::Summarizer(e.to_string()))?;

    state
        .store
        .save_summary(&id, &summary.summary, &summary.todos)?;

    let m = state.store.update_meeting_status(&id, MeetingStatus::Done, detail.meeting.ended_at)?;
    let _ = app.emit(EVT_MEETING_STATUS, &m);

    state.store.get_meeting_detail(&id)
}

#[tauri::command]
pub async fn toggle_todo(
    state: State<'_, AppState>,
    meeting_id: String,
    todo_id: String,
) -> Result<Todo> {
    state.store.toggle_todo(&meeting_id, &todo_id)
}

// ----------------------------------------------------------------------
// Clients
// ----------------------------------------------------------------------

#[tauri::command]
pub async fn create_client(state: State<'_, AppState>, name: String) -> Result<Client> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("client name cannot be empty".into()));
    }
    state.store.create_client(trimmed)
}

#[tauri::command]
pub async fn list_clients(state: State<'_, AppState>) -> Result<Vec<Client>> {
    state.store.list_clients()
}

#[tauri::command]
pub async fn update_client(
    state: State<'_, AppState>,
    id: String,
    name: String,
    color: String,
) -> Result<Client> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("client name cannot be empty".into()));
    }
    let color = ClientColor::from_str(&color)
        .ok_or_else(|| Error::InvalidState(format!("unknown color: {color}")))?;
    state.store.update_client(&id, trimmed, color)
}

#[tauri::command]
pub async fn delete_client(state: State<'_, AppState>, id: String) -> Result<()> {
    state.store.delete_client(&id)
}

// ----------------------------------------------------------------------
// Meeting edits
// ----------------------------------------------------------------------

#[tauri::command]
pub async fn update_meeting_title(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<Meeting> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("title cannot be empty".into()));
    }
    let m = state.store.update_meeting(&id, Some(trimmed), None)?;
    let _ = app.emit(EVT_MEETING_STATUS, &m);
    Ok(m)
}

/// Assign or unassign a client. `client_id = None` moves the meeting to
/// "Unassigned"; `Some(id)` assigns to that client.
#[tauri::command]
pub async fn set_meeting_client(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    client_id: Option<String>,
) -> Result<Meeting> {
    // Validate the client exists if one was passed.
    if let Some(cid) = &client_id {
        let _ = state.store.get_client(cid)?;
    }
    let m = state
        .store
        .update_meeting(&meeting_id, None, Some(client_id.as_deref()))?;
    let _ = app.emit(EVT_MEETING_STATUS, &m);
    Ok(m)
}

// ----------------------------------------------------------------------
// Notes
// ----------------------------------------------------------------------

/// Append a line to the live notes pane. `t_ms` is computed server-side
/// from the meeting's `started_at` so timing stays consistent across clients
/// and survives a clock change.
#[tauri::command]
pub async fn append_note(
    state: State<'_, AppState>,
    meeting_id: String,
    text: String,
    speaker_hint: Option<SpeakerHint>,
) -> Result<Note> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("note text cannot be empty".into()));
    }
    let meeting = state.store.get_meeting_detail(&meeting_id)?.meeting;
    let elapsed_ms = (Utc::now() - meeting.started_at)
        .num_milliseconds()
        .max(0) as u64;
    state.store.append_note(
        &meeting_id,
        elapsed_ms,
        speaker_hint.unwrap_or_default(),
        trimmed,
    )
}

/// Substring search across notes. `client_id = None` searches all notes;
/// `Some(id)` constrains to that client's meetings. Empty / whitespace
/// query returns an empty list (so the UI can call this on every
/// keystroke without burning the result list).
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
    client_id: Option<String>,
) -> Result<Vec<NoteHit>> {
    state.store.search_notes(&query, client_id.as_deref())
}

// ----------------------------------------------------------------------
// Summary + todo edits
// ----------------------------------------------------------------------

#[tauri::command]
pub async fn update_summary(
    state: State<'_, AppState>,
    meeting_id: String,
    summary: String,
) -> Result<()> {
    state.store.update_summary_text(&meeting_id, &summary)
}

#[tauri::command]
pub async fn add_todo(
    state: State<'_, AppState>,
    meeting_id: String,
    text: String,
) -> Result<Todo> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("todo text cannot be empty".into()));
    }
    state.store.add_todo(&meeting_id, trimmed)
}

#[tauri::command]
pub async fn update_todo_text(
    state: State<'_, AppState>,
    meeting_id: String,
    todo_id: String,
    text: String,
) -> Result<Todo> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidState("todo text cannot be empty".into()));
    }
    state.store.update_todo_text(&meeting_id, &todo_id, trimmed)
}

#[tauri::command]
pub async fn delete_todo(
    state: State<'_, AppState>,
    meeting_id: String,
    todo_id: String,
) -> Result<()> {
    state.store.delete_todo(&meeting_id, &todo_id)
}

#[derive(serde::Serialize)]
struct TranscriptEventPayload {
    meeting_id: String,
    #[serde(flatten)]
    segment: TranscriptSegment,
}
