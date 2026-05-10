//! Automatic speech recognition — Parakeet TDT v3 via ONNX Runtime.
//!
//! Parakeet is a streaming RNN-T / TDT model. Practically the pipeline is:
//!   1. Buffer audio into ~480ms windows of 16 kHz mono f32.
//!   2. Compute log-mel features (80 mel bins, 25ms window, 10ms hop) — see
//!      [`mel`] (M2a step 2, shipped).
//!   3. Run encoder ONNX → encoder embeddings.
//!   4. Run joint/decoder ONNX with the previous prediction state to emit tokens.
//!   5. Detokenize via the SentencePiece BPE that ships with the model.
//!
//! Steps 1 and 2 are real; 3-5 are stubs pending model artifacts. See
//! [`M2A-PLAN.md`](../../../M2A-PLAN.md) for the staging.

pub mod mel;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::audio::AudioChunk;
use crate::meeting::TranscriptSegment;
use crate::{Error, Result};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AsrConfig {
    /// Directory containing `encoder.onnx`, `decoder_joint.onnx`, `tokenizer.json`.
    pub model_dir: PathBuf,
    /// Optional override; defaults to `parakeet-tdt-0.6b-v3`.
    pub model_id: String,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            model_dir: default_model_dir(),
            model_id: "parakeet-tdt-0.6b-v3".into(),
        }
    }
}

fn default_model_dir() -> PathBuf {
    // Resolved at runtime; in practice we'll read from app_data_dir/models/<model_id>.
    // Keeping a relative fallback to avoid coupling to the Tauri app handle here.
    PathBuf::from("./models/parakeet-tdt-0.6b-v3")
}

/// Handle to a loaded ASR engine. Cheaply cloneable.
#[derive(Clone)]
#[allow(dead_code)]
pub struct Asr {
    inner: Arc<AsrInner>,
}

struct AsrInner {
    #[allow(dead_code)]
    cfg: AsrConfig,
    // TODO: hold ort::Session for encoder + decoder/joint, plus tokenizer.
}

impl Asr {
    pub fn new(cfg: AsrConfig) -> Result<Self> {
        if !cfg.model_dir.exists() {
            tracing::warn!(
                ?cfg.model_dir,
                "ASR model dir missing — transcription will return placeholders. \
                 See CLAUDE.md → ASR for export instructions."
            );
        }
        Ok(Self {
            inner: Arc::new(AsrInner { cfg }),
        })
    }

    /// Spawn a streaming transcription task.
    ///
    /// Reads `AudioChunk`s, emits `TranscriptSegment`s. The task ends when
    /// `audio_rx` is closed (i.e. capture stopped).
    pub fn spawn_streaming(
        &self,
        mut audio_rx: mpsc::Receiver<AudioChunk>,
    ) -> mpsc::Receiver<TranscriptSegment> {
        let (tx, rx) = mpsc::channel::<TranscriptSegment>(64);
        let _self = self.clone();
        tokio::spawn(async move {
            // TODO: real Parakeet streaming inference. Right now we just drain.
            while let Some(_chunk) = audio_rx.recv().await {
                // Future: feature extraction → encoder → joint → emit segments.
            }
            tracing::debug!("asr stream finished");
            drop(tx);
        });
        rx
    }
}

/// One-shot transcription of a complete audio buffer. Useful for tests/CLI.
#[allow(dead_code)]
pub async fn transcribe_offline(_path: &Path, _cfg: &AsrConfig) -> Result<Vec<TranscriptSegment>> {
    Err(Error::Asr("offline transcription not implemented".into()))
}

// -----------------------------------------------------------------------------
// Faux ASR — env-gated transcript replay.
//
// Set `MEETIOR_FIXTURE_TRANSCRIPT=/path/to/segments.json` to bypass real audio
// + ASR and stream a canned transcript through the same channel. This unblocks
// UI / summarizer / storage iteration before Parakeet is wired (see CLAUDE.md
// → Build, test, iterate → Iterating on the stubs).
// -----------------------------------------------------------------------------

/// Spawn a faux transcript stream from a JSON file of `TranscriptSegment`s.
///
/// Segments are emitted on a timer aligned to their `start_ms` so the UI feels
/// like a real meeting. Ends after the last segment, or when the consumer
/// drops the receiver.
pub fn spawn_fixture_stream(path: &Path) -> Result<mpsc::Receiver<TranscriptSegment>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::Asr(format!("read fixture {}: {e}", path.display())))?;
    let segments: Vec<TranscriptSegment> = serde_json::from_str(&raw)
        .map_err(|e| Error::Asr(format!("parse fixture {}: {e}", path.display())))?;

    let (tx, rx) = mpsc::channel::<TranscriptSegment>(64);

    tokio::spawn(async move {
        let started = std::time::Instant::now();
        for seg in segments {
            let target = std::time::Duration::from_millis(seg.start_ms);
            let elapsed = started.elapsed();
            if target > elapsed {
                tokio::time::sleep(target - elapsed).await;
            }
            if tx.send(seg).await.is_err() {
                break; // consumer gone
            }
        }
    });

    Ok(rx)
}

/// Returns the configured fixture transcript path, if `MEETIOR_FIXTURE_TRANSCRIPT` is set.
pub fn fixture_transcript_path() -> Option<PathBuf> {
    std::env::var_os("MEETIOR_FIXTURE_TRANSCRIPT").map(PathBuf::from)
}
