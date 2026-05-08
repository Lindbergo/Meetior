//! Audio capture pipeline.
//!
//! Two sources are mixed into a single 16 kHz mono float32 stream feeding the
//! ASR engine:
//!   * **Microphone** — captured cross-platform via `cpal`.
//!   * **System audio** — captured on macOS via ScreenCaptureKit (other Meet
//!     participants). Other OSes are not supported in v1.
//!
//! This module currently exposes the *shape* of the pipeline. The full
//! ScreenCaptureKit wiring is left for the next implementation pass —
//! see CLAUDE.md → "Roadmap → Audio".

use tokio::sync::mpsc;

use crate::{Error, Result};

/// Sample format used by the rest of the pipeline (Parakeet expects 16 kHz mono f32).
#[allow(dead_code)]
pub const SAMPLE_RATE: u32 = 16_000;

/// A chunk of audio samples in our canonical format (16 kHz mono f32, [-1.0, 1.0]).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AudioChunk {
    /// Monotonic offset in ms from session start.
    pub offset_ms: u64,
    pub samples: Vec<f32>,
}

/// Public entrypoint — returns a receiver that yields normalized chunks.
///
/// `stop_rx` cancels the capture loop. The returned receiver closes when
/// capture has fully drained.
pub fn start_capture(
    mut stop_rx: mpsc::Receiver<()>,
) -> Result<mpsc::Receiver<AudioChunk>> {
    let (tx, rx) = mpsc::channel::<AudioChunk>(64);

    // TODO: wire real capture. For now we spawn a stub that exits on stop_rx.
    // The shape below is what the real pipeline should produce: ~200ms chunks,
    // 16 kHz mono f32, mixed mic + system audio.
    tokio::spawn(async move {
        tracing::warn!(
            "audio::start_capture is a stub — no audio is being captured yet. \
             Implement the macOS ScreenCaptureKit + cpal mic pipeline (see CLAUDE.md)."
        );
        let _ = stop_rx.recv().await;
        drop(tx);
    });

    Ok(rx)
}

/// Resample arbitrary input to 16 kHz mono f32. Stub for now; will use `rubato`.
#[allow(dead_code)]
pub fn resample_to_target(
    _input: &[f32],
    _input_rate: u32,
    _input_channels: u16,
) -> Result<Vec<f32>> {
    Err(Error::Audio("resample_to_target not implemented".into()))
}
