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

/// Approximate characters per token for English text. Used only for the
/// single-pass-vs-chunked routing decision; off by 2× and the routing still
/// works (we'd just chunk slightly earlier or later than ideal).
const APPROX_CHARS_PER_TOKEN: usize = 4;

/// Above this estimated transcript-token count, switch from one Ollama
/// call to map-reduce summarization. 4000 leaves ~4k tokens for prompt
/// scaffolding + model response within an 8k context window — safe margin
/// on memory-constrained Macs.
pub const SINGLE_PASS_TOKEN_LIMIT: usize = 4_000;

/// Target tokens per chunk in the map-reduce path. Each chunk gets its
/// own Ollama call → bounded peak memory (one chunk's prompt + the model
/// in RAM) regardless of meeting length.
pub const CHUNK_TOKEN_TARGET: usize = 3_000;

const SUMMARIZE_SYSTEM: &str =
    "You summarize meeting transcripts. \
     Reply with strict JSON: {\"summary\": string, \"todos\": string[]}. \
     The summary should be 3-6 sentences. Each todo should be a single, \
     actionable item. Do not include todos that aren't clearly committed to.";

const SUMMARIZE_CHUNK_SYSTEM: &str =
    "You summarize one section of a longer meeting transcript. \
     Reply with strict JSON: {\"summary\": string, \"todos\": string[]}. \
     The summary should be 2-3 sentences focused on this section. Each todo \
     should be a single, actionable item committed to in this section.";

const COMBINE_SYSTEM: &str =
    "You combine partial summaries of consecutive sections of the same meeting \
     into a single coherent summary. \
     Reply with strict JSON: {\"summary\": string, \"todos\": string[]}. \
     The combined summary should be 5-8 sentences. Deduplicate todos that \
     appear across sections. Keep wording from the originals where possible.";

impl OllamaClient {
    /// Summarize a transcript. Auto-routes:
    ///   * Short transcripts (≤ [`SINGLE_PASS_TOKEN_LIMIT`] estimated tokens)
    ///     → one Ollama call, lower latency.
    ///   * Long transcripts → map-reduce. Each chunk gets its own Ollama
    ///     call (bounded peak memory regardless of meeting length), then a
    ///     final combine pass merges the partials and dedupes todos.
    ///
    /// The map-reduce path is what makes a 2 h meeting feasible on an 8 GB
    /// Mac: instead of jamming a 25 k-token transcript into one prompt
    /// (and either OOM'ing or truncating), we run ~6-8 short calls
    /// sequentially, releasing memory between calls.
    pub async fn summarize(&self, segments: &[TranscriptSegment]) -> Result<Summary> {
        if estimated_tokens(segments) <= SINGLE_PASS_TOKEN_LIMIT {
            self.summarize_single(segments, SUMMARIZE_SYSTEM).await
        } else {
            self.summarize_chunked(segments).await
        }
    }

    async fn summarize_single(
        &self,
        segments: &[TranscriptSegment],
        system: &str,
    ) -> Result<Summary> {
        let transcript = format_transcript(segments);
        let user = format!("Transcript:\n\n{transcript}\n\nReturn the JSON object only.");
        let payload = self.chat_for_summary(system, &user).await?;
        Ok(payload_to_summary(payload))
    }

    async fn summarize_chunked(&self, segments: &[TranscriptSegment]) -> Result<Summary> {
        let ranges = chunk_segment_ranges(segments, CHUNK_TOKEN_TARGET);
        tracing::info!(
            chunks = ranges.len(),
            total_segments = segments.len(),
            "summarize: long transcript → map-reduce"
        );

        let mut partials = Vec::with_capacity(ranges.len());
        for (i, (start, end)) in ranges.iter().enumerate() {
            tracing::debug!(chunk = i + 1, of = ranges.len(), "summarize chunk");
            let partial = self
                .summarize_single(&segments[*start..*end], SUMMARIZE_CHUNK_SYSTEM)
                .await?;
            partials.push(partial);
        }

        if partials.is_empty() {
            // Defensive: empty transcript shouldn't reach here (commands
            // already rejects it), but if it does, return an empty summary
            // rather than an Ollama call that could only confuse the model.
            return Ok(Summary { summary: String::new(), todos: Vec::new() });
        }
        if partials.len() == 1 {
            // Single chunk — no need for the combine pass.
            return Ok(partials.into_iter().next().unwrap());
        }
        self.combine_summaries(&partials).await
    }

    async fn combine_summaries(&self, partials: &[Summary]) -> Result<Summary> {
        let mut user = String::from("Partial summaries from consecutive sections:\n\n");
        for (i, p) in partials.iter().enumerate() {
            user.push_str(&format!("Section {}:\n", i + 1));
            user.push_str("  Summary: ");
            user.push_str(&p.summary);
            user.push('\n');
            if !p.todos.is_empty() {
                user.push_str("  Todos:\n");
                for t in &p.todos {
                    user.push_str(&format!("    - {}\n", t.text));
                }
            }
            user.push('\n');
        }
        user.push_str("Return the combined JSON object only.");
        let payload = self.chat_for_summary(COMBINE_SYSTEM, &user).await?;
        Ok(payload_to_summary(payload))
    }

    /// Single Ollama `/api/chat` round-trip with `format: "json"`. Returns
    /// the deserialized payload (summary + todos as strings); turning that
    /// into `Summary` (with stable Todo IDs) is the caller's job.
    async fn chat_for_summary(&self, system: &str, user: &str) -> Result<SummaryPayload> {
        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage { role: "system", content: system.into() },
                ChatMessage { role: "user", content: user.into() },
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

        serde_json::from_str(&resp.message.content)
            .map_err(|e| Error::Summarizer(format!("model returned invalid JSON: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Free-standing helpers (pure, easy to test)
// ---------------------------------------------------------------------------

fn format_transcript(segments: &[TranscriptSegment]) -> String {
    segments
        .iter()
        .map(|s| {
            let speaker = s.speaker.as_deref().unwrap_or("Speaker");
            format!("[{:>6}ms] {}: {}", s.start_ms, speaker, s.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn payload_to_summary(payload: SummaryPayload) -> Summary {
    let todos = payload
        .todos
        .into_iter()
        .map(|t| Todo { id: Uuid::new_v4().to_string(), text: t, done: false })
        .collect();
    Summary { summary: payload.summary, todos }
}

/// Estimated token count of the concatenated transcript text. English-tuned
/// heuristic; off by 2× still produces correct routing decisions.
pub fn estimated_tokens(segments: &[TranscriptSegment]) -> usize {
    let chars: usize = segments.iter().map(|s| s.text.len()).sum();
    chars / APPROX_CHARS_PER_TOKEN
}

/// Split a transcript into contiguous segment ranges where each range is
/// approximately `target_tokens` long. Splits only on segment boundaries —
/// never breaks a single segment's text. Returns `[(start, end), ...]`
/// indices into `segments`.
fn chunk_segment_ranges(
    segments: &[TranscriptSegment],
    target_tokens: usize,
) -> Vec<(usize, usize)> {
    if segments.is_empty() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut tokens_in_chunk = 0;
    for (i, seg) in segments.iter().enumerate() {
        let seg_tokens = seg.text.len() / APPROX_CHARS_PER_TOKEN;
        // If adding this segment would exceed the target AND we already
        // have at least one segment in the chunk, close the current chunk.
        // (We never emit an empty range — even an oversized solo segment
        //  goes in its own chunk so we don't lose data.)
        if tokens_in_chunk > 0 && tokens_in_chunk + seg_tokens > target_tokens {
            ranges.push((start, i));
            start = i;
            tokens_in_chunk = 0;
        }
        tokens_in_chunk += seg_tokens;
    }
    ranges.push((start, segments.len()));
    ranges
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

    // -------- chunking (pure) ------------------------------------------

    #[test]
    fn estimated_tokens_uses_char_heuristic() {
        // 12 chars / 4 = 3 tokens.
        let segs = [segment("hello world!")];
        assert_eq!(estimated_tokens(&segs), 3);
    }

    #[test]
    fn chunk_segment_ranges_returns_single_range_for_short_input() {
        let segs: Vec<_> = (0..3).map(|_| segment("short")).collect();
        let ranges = chunk_segment_ranges(&segs, 1000);
        assert_eq!(ranges, vec![(0, 3)]);
    }

    #[test]
    fn chunk_segment_ranges_splits_on_segment_boundaries() {
        // Each segment ≈ 25 tokens (100 chars / 4). Target 50 → 2 segments
        // per chunk, so 6 segments → 3 chunks.
        let body = "x".repeat(100);
        let segs: Vec<_> = (0..6).map(|_| segment(&body)).collect();
        let ranges = chunk_segment_ranges(&segs, 50);
        assert_eq!(ranges, vec![(0, 2), (2, 4), (4, 6)]);
    }

    #[test]
    fn chunk_segment_ranges_keeps_oversized_segment_in_its_own_chunk() {
        // First segment is huge — exceeds target by itself. We never want
        // to lose data, so it gets its own chunk.
        let huge = segment(&"x".repeat(10_000));
        let small = segment("short");
        let segs = vec![huge, small];
        let ranges = chunk_segment_ranges(&segs, 50);
        // Expected: huge alone in chunk 0, small in chunk 1.
        assert_eq!(ranges, vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn chunk_segment_ranges_handles_empty() {
        let ranges = chunk_segment_ranges(&[], 100);
        assert!(ranges.is_empty());
    }

    // -------- routing: short transcripts use one Ollama call -----------

    #[tokio::test]
    async fn short_transcript_uses_single_call() {
        let server = MockServer::start().await;
        let model_json = json!({
            "summary": "single-call summary",
            "todos": ["one"],
        }).to_string();

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ollama_chat_response(&model_json)))
            .expect(1) // exactly one call expected
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let segs = vec![segment("a quick discussion about the launch.")];
        let result = client.summarize(&segs).await.unwrap();
        assert_eq!(result.summary, "single-call summary");
    }

    // -------- map-reduce: long transcripts chunk + combine -------------

    /// Build a transcript long enough to force chunking. Each segment is
    /// ~600 chars (~150 tokens); 30 of them = ~4500 tokens, above
    /// SINGLE_PASS_TOKEN_LIMIT. Should chunk into ~2 sections at the
    /// CHUNK_TOKEN_TARGET=3000 setting.
    fn long_transcript() -> Vec<TranscriptSegment> {
        let body = "we discussed the project status and went over the blockers in detail. ".repeat(8);
        (0..30).map(|i| TranscriptSegment {
            start_ms: i as u64 * 1000,
            end_ms: i as u64 * 1000 + 800,
            speaker: Some("Speaker".into()),
            speaker_source: crate::meeting::SpeakerSource::Unknown,
            text: body.clone(),
        }).collect()
    }

    #[tokio::test]
    async fn long_transcript_runs_map_then_reduce() {
        // Verify the two phases happen by giving each phase its own mock
        // matched on the system prompt.
        use wiremock::matchers::body_string_contains;

        let segs = long_transcript();
        assert!(estimated_tokens(&segs) > SINGLE_PASS_TOKEN_LIMIT);

        let server = MockServer::start().await;

        // Map phase: chunk-summary system prompt → returns a chunk summary.
        let chunk_response = json!({
            "summary": "chunk summary",
            "todos": ["chunk todo"],
        }).to_string();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_string_contains("one section of a longer meeting"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(ollama_chat_response(&chunk_response)))
            .mount(&server)
            .await;

        // Reduce phase: combine system prompt → returns the merged summary.
        let combined_response = json!({
            "summary": "merged summary across all sections",
            "todos": ["chunk todo"],   // dedup baked into the model's response
        }).to_string();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_string_contains("combine partial summaries"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(ollama_chat_response(&combined_response)))
            .expect(1) // exactly one combine call
            .mount(&server)
            .await;

        let client = OllamaClient::new(server.uri(), "test-model");
        let result = client.summarize(&segs).await.unwrap();
        assert_eq!(result.summary, "merged summary across all sections");
        assert_eq!(result.todos.len(), 1);
        assert_eq!(result.todos[0].text, "chunk todo");
    }

    // Note: `summarize_chunked` short-circuits when only one chunk is
    // produced (skips the combine round-trip). With the current thresholds
    // (SINGLE_PASS_TOKEN_LIMIT=4000 > CHUNK_TOKEN_TARGET=3000) chunking
    // always produces ≥2 chunks, so that branch is defensive-only and
    // not testable via the public `summarize` path. Left intact as a
    // safety net if thresholds shift.
}
