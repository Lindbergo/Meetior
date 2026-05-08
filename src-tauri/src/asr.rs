//! Automatic speech recognition — Parakeet TDT v3 via ONNX Runtime.
//!
//! Parakeet is a streaming RNN-T / TDT model. Practically the pipeline is:
//!   1. Buffer audio into ~480ms windows of 16 kHz mono f32.
//!   2. Compute log-mel features (80 mel bins, 25ms window, 10ms hop).
//!   3. Run encoder ONNX → encoder embeddings.
//!   4. Run joint/decoder ONNX with the previous prediction state to emit tokens.
//!   5. Detokenize via the SentencePiece BPE that ships with the model.
//!
//! v1 of this module is a **stub** that compiles, exposes the public surface
//! (`Asr::new`, `Asr::transcribe`), and documents what the full implementation
//! must do. See CLAUDE.md → "Roadmap → ASR" for the export script and model
//! files we expect on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::audio::AudioChunk;
use crate::meeting::TranscriptSegment;
use crate::{Error, Result};

#[derive(Debug, Clone)]
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
