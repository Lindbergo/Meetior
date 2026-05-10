//! Meetior — local meeting transcription & summarization.
//!
//! See `CLAUDE.md` at repo root for architecture, conventions and roadmap.

mod asr;
mod audio;
mod commands;
mod error;
mod meeting;
mod storage;
mod summarizer;

use std::sync::Arc;

use tauri::{Manager, RunEvent};
use tokio::sync::Mutex;

pub use error::{Error, Result};

/// Shared application state, available to every Tauri command via `State<AppState>`.
pub struct AppState {
    pub store: Arc<storage::Store>,
    pub session: Arc<Mutex<Option<meeting::Session>>>,
    pub summarizer: Arc<summarizer::OllamaClient>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,meetior_lib=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data dir resolvable")
                .join("meetior");
            std::fs::create_dir_all(&data_dir).ok();

            let store = Arc::new(storage::Store::open(data_dir.join("meetior.sqlite"))?);
            let summarizer = Arc::new(summarizer::OllamaClient::default());

            app.manage(AppState {
                store,
                session: Arc::new(Mutex::new(None)),
                summarizer,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_meeting,
            commands::stop_meeting,
            commands::list_meetings,
            commands::get_meeting,
            commands::summarize_meeting,
            commands::toggle_todo,
            commands::create_client,
            commands::list_clients,
            commands::update_client,
            commands::delete_client,
            commands::update_meeting_title,
            commands::set_meeting_client,
            commands::append_note,
            commands::search_notes,
            commands::update_summary,
            commands::add_todo,
            commands::update_todo_text,
            commands::delete_todo,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                tracing::info!("exit requested");
            }
        });
}
