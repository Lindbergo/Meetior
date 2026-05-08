//! Tauri commands — the bridge between the Svelte UI and the Rust backend.
//!
//! Keep these thin: validate inputs, dispatch to a domain module, return
//! serializable values. No business logic should live here.

use chrono::Utc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use crate::asr::{Asr, AsrConfig};
use crate::audio;
use crate::meeting::{Meeting, MeetingDetail, MeetingStatus, Session, Todo, TranscriptSegment};
use crate::{AppState, Error, Result};

const EVT_TRANSCRIPT_SEGMENT: &str = "meetior://transcript-segment";
const EVT_MEETING_STATUS: &str = "meetior://meeting-status";

#[tauri::command]
pub async fn start_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<Meeting> {
    let mut session_slot = state.session.lock().await;
    if session_slot.is_some() {
        return Err(Error::InvalidState("a meeting is already recording".into()));
    }

    let title = title.unwrap_or_else(|| {
        format!("Meeting {}", Utc::now().format("%Y-%m-%d %H:%M"))
    });
    let meeting = Meeting::new(title);
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

#[derive(serde::Serialize)]
struct TranscriptEventPayload {
    meeting_id: String,
    #[serde(flatten)]
    segment: TranscriptSegment,
}
