//! Summarization & todo extraction via a local Ollama server.
//!
//! Default endpoint: `http://127.0.0.1:11434`. We talk to the `/api/chat`
//! endpoint with `format: "json"` and ask the model for a strict JSON shape:
//!
//! ```json
//! { "summary": "...", "todos": ["...", "..."] }
//! ```
//!
//! The model defaults to `llama3.1:8b-instruct-q4_K_M`. It's overridable
//! via `MEETIOR_OLLAMA_MODEL` and the host via `MEETIOR_OLLAMA_HOST`.

use std::env;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::meeting::{Todo, TranscriptSegment};
use crate::{Error, Result};

const DEFAULT_HOST: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL: &str = "llama3.1:8b-instruct-q4_K_M";

pub struct OllamaClient {
    http: reqwest::Client,
    host: String,
    model: String,
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            host: env::var("MEETIOR_OLLAMA_HOST").unwrap_or_else(|_| DEFAULT_HOST.into()),
            model: env::var("MEETIOR_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    format: &'a str,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Serialize)]
struct ChatOptions {
    temperature: f32,
    num_ctx: u32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct SummaryPayload {
    summary: String,
    #[serde(default)]
    todos: Vec<String>,
}

pub struct Summary {
    pub summary: String,
    pub todos: Vec<Todo>,
}

impl OllamaClient {
    pub async fn summarize(&self, segments: &[TranscriptSegment]) -> Result<Summary> {
        let transcript = segments
            .iter()
            .map(|s| {
                let speaker = s.speaker.as_deref().unwrap_or("Speaker");
                format!("[{:>6}ms] {}: {}", s.start_ms, speaker, s.text)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You summarize meeting transcripts. \
                      Reply with strict JSON: {\"summary\": string, \"todos\": string[]}. \
                      The summary should be 3-6 sentences. Each todo should be a single, \
                      actionable item. Do not include todos that aren't clearly committed to.";

        let user = format!(
            "Transcript:\n\n{transcript}\n\nReturn the JSON object only."
        );

        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage { role: "system", content: system.into() },
                ChatMessage { role: "user", content: user },
            ],
            stream: false,
            format: "json",
            options: ChatOptions { temperature: 0.2, num_ctx: 8192 },
        };

        let url = format!("{}/api/chat", self.host.trim_end_matches('/'));
        let resp: ChatResponse = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let payload: SummaryPayload = serde_json::from_str(&resp.message.content)
            .map_err(|e| Error::Summarizer(format!("model returned invalid JSON: {e}")))?;

        let todos = payload
            .todos
            .into_iter()
            .map(|t| Todo { id: Uuid::new_v4().to_string(), text: t, done: false })
            .collect();

        Ok(Summary { summary: payload.summary, todos })
    }
}
