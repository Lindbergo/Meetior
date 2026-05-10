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

impl OllamaClient {
    pub fn new(host: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            host: host.into(),
            model: model.into(),
        }
    }
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new(
            env::var("MEETIOR_OLLAMA_HOST").unwrap_or_else(|_| DEFAULT_HOST.into()),
            env::var("MEETIOR_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        )
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

#[derive(Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn segment(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: 0,
            end_ms: 1000,
            speaker: Some("Alice".into()),
            speaker_source: crate::meeting::SpeakerSource::Unknown,
            text: text.into(),
        }
    }

    fn ollama_chat_response(content: &str) -> serde_json::Value {
        // Real Ollama responses have many more fields; we only need `message.content`.
        json!({ "message": { "role": "assistant", "content": content } })
    }

    #[tokio::test]
    async fn happy_path_parses_summary_and_todos() {
        let server = MockServer::start().await;
        let model_json = json!({
            "summary": "Quick standup. Alice will own copy. Bob is blocked on Stripe keys.",
            "todos": ["Send Stripe keys to Bob", "Draft webhook spec by EOD tomorrow"],
        })
        .to_string();

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ollama_chat_response(&model_json)))
            .expect(1)
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let result = client.summarize(&[segment("hello there")]).await.unwrap();

        assert!(result.summary.starts_with("Quick standup"));
        assert_eq!(result.todos.len(), 2);
        assert_eq!(result.todos[0].text, "Send Stripe keys to Bob");
        assert!(!result.todos[0].done);
        // IDs are generated server-side and unique per todo.
        assert_ne!(result.todos[0].id, result.todos[1].id);
    }

    #[tokio::test]
    async fn missing_todos_field_yields_empty_list() {
        let server = MockServer::start().await;
        let model_json = json!({ "summary": "no actionable items" }).to_string();

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ollama_chat_response(&model_json)))
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let result = client.summarize(&[segment("...")]).await.unwrap();

        assert_eq!(result.summary, "no actionable items");
        assert!(result.todos.is_empty());
    }

    #[tokio::test]
    async fn malformed_inner_json_is_a_summarizer_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(ollama_chat_response("not json at all { ")),
            )
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let err = client.summarize(&[segment("...")]).await.unwrap_err();
        assert!(matches!(err, Error::Summarizer(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn http_5xx_is_an_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let err = client.summarize(&[segment("...")]).await.unwrap_err();
        assert!(matches!(err, Error::Http(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn request_uses_configured_model_and_json_format() {
        use wiremock::matchers::body_partial_json;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({
                "model": "my-custom-model",
                "format": "json",
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(ollama_chat_response(
                &json!({ "summary": "ok", "todos": [] }).to_string(),
            )))
            .expect(1)
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "my-custom-model");
        client.summarize(&[segment("hi")]).await.unwrap();
        // Mock's `expect(1)` is verified on drop — server panics if not hit.
    }
}
