//! Audio capture pipeline.
//!
//! Two sources are mixed into a single 16 kHz mono float32 stream feeding the
//! ASR engine:
//!   * **Microphone** — captured cross-platform via `cpal`.
//!   * **System audio** — captured on macOS via ScreenCaptureKit (other Meet
//!     participants). Other OSes are not supported in v1.
//!
//! M2a step 1 implements the mic path + a fixture-audio fallback for
//! development without real audio. System audio is M2a step 4. See
//! [`M2A-PLAN.md`](../../M2A-PLAN.md) for the staging.
//!
//! ## Fixture path
//!
//! Set `MEETIOR_FIXTURE_AUDIO=path.wav` to bypass the mic and replay a WAV
//! instead. Mirrors the `MEETIOR_FIXTURE_TRANSCRIPT` pattern in `asr.rs` so
//! UI / mel-spec / ASR work can iterate without a microphone.
//!
//! ## Threading
//!
//! `cpal::Stream` is `!Send` on macOS, so it lives in a dedicated `std::thread`
//! whose only job is to outlive the stream. The audio callback (real-time
//! priority) hands raw frames to a tokio task via an unbounded channel; that
//! task batches, resamples to 16 kHz mono, and emits `AudioChunk`s on the
//! bounded channel that ASR consumes. Bounded so that if ASR falls behind, we
//! drop chunks rather than ballooning RAM.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;

use tokio::sync::mpsc;

use crate::meeting::SpeakerSource;
use crate::{Error, Result};

/// Sample format used by the rest of the pipeline (Parakeet expects 16 kHz mono f32).
pub const SAMPLE_RATE: u32 = 16_000;

/// Target chunk length in milliseconds. ~200 ms balances ASR latency against
/// per-chunk overhead.
pub const CHUNK_MS: u64 = 200;

/// Samples per chunk at the canonical rate.
pub const CHUNK_SAMPLES: usize = (SAMPLE_RATE as u64 * CHUNK_MS / 1000) as usize;

/// A chunk of audio samples in our canonical format (16 kHz mono f32, [-1.0, 1.0]).
//
// `dead_code` is allowed until M2a step 3 — the ASR stub drains chunks as
// `_chunk` without reading fields. Drop the allow when real inference lands.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Monotonic offset in ms from session start.
    pub offset_ms: u64,
    pub samples: Vec<f32>,
    /// Which capture stream produced this chunk. Propagates through ASR onto
    /// `TranscriptSegment.speaker_source`.
    pub source: SpeakerSource,
}

// ---------------------------------------------------------------------------
// Public entrypoint
// ---------------------------------------------------------------------------

/// Returns a receiver yielding 16 kHz mono f32 chunks until `stop_rx` fires.
///
/// Dispatches to the fixture-audio path if `MEETIOR_FIXTURE_AUDIO` is set,
/// otherwise opens the default input device via `cpal`.
pub fn start_capture(stop_rx: mpsc::Receiver<()>) -> Result<mpsc::Receiver<AudioChunk>> {
    if let Some(path) = fixture_audio_path() {
        tracing::info!(?path, "MEETIOR_FIXTURE_AUDIO set — using fixture audio");
        return spawn_fixture_audio_stream(path, stop_rx);
    }
    spawn_mic_capture(stop_rx)
}

/// Reads `MEETIOR_FIXTURE_AUDIO`. Symmetric with `asr::fixture_transcript_path`.
pub fn fixture_audio_path() -> Option<PathBuf> {
    std::env::var_os("MEETIOR_FIXTURE_AUDIO").map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Fixture audio path
// ---------------------------------------------------------------------------

/// Spawn a fixture-audio stream that reads `path` once, resamples to
/// 16 kHz mono, then emits `AudioChunk`s on a real-time pace until exhausted
/// or `stop_rx` fires.
fn spawn_fixture_audio_stream(
    path: PathBuf,
    mut stop_rx: mpsc::Receiver<()>,
) -> Result<mpsc::Receiver<AudioChunk>> {
    let samples = read_wav_to_mono_16k(&path)?;
    let chunks = chunk_samples(samples, SpeakerSource::Mic);
    let (tx, rx) = mpsc::channel::<AudioChunk>(64);

    tokio::spawn(async move {
        let started = std::time::Instant::now();
        for chunk in chunks {
            // Pace at real-time so the consumer experiences something
            // close to a live mic.
            let target =
                std::time::Duration::from_millis(chunk.offset_ms + CHUNK_MS);
            let elapsed = started.elapsed();
            if target > elapsed {
                tokio::select! {
                    _ = tokio::time::sleep(target - elapsed) => {}
                    _ = stop_rx.recv() => return,
                }
            }
            if tx.send(chunk).await.is_err() {
                return;
            }
        }
        // Drained the file; sit until stop fires so the meeting stays
        // "recording" until the user explicitly stops it (matches the
        // fixture-transcript behavior).
        let _ = stop_rx.recv().await;
    });

    Ok(rx)
}

/// Read a WAV from disk, mix to mono, resample to 16 kHz, return f32 samples
/// in `[-1.0, 1.0]`. Public for tests and the fixture path.
pub fn read_wav_to_mono_16k(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| Error::Audio(format!("open {}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels;
    let in_rate = spec.sample_rate;

    // Decode whatever sample format hound gives us into f32 in [-1, 1].
    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Audio(format!("decode f32: {e}")))?,
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Audio(format!("decode i16: {e}")))?,
        (hound::SampleFormat::Int, 24) | (hound::SampleFormat::Int, 32) => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / i32::MAX as f32))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Audio(format!("decode i32: {e}")))?,
        (fmt, bits) => {
            return Err(Error::Audio(format!(
                "unsupported WAV format: {fmt:?} {bits}-bit"
            )));
        }
    };

    let mono = mixdown_to_mono(&interleaved, channels);
    Ok(resample_linear_mono(&mono, in_rate as usize, SAMPLE_RATE as usize))
}

// ---------------------------------------------------------------------------
// DSP helpers (pure, easy to unit-test)
// ---------------------------------------------------------------------------

/// Average channels into a single mono stream. Input must be interleaved.
pub fn mixdown_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let ch = channels as usize;
    let mut out = Vec::with_capacity(interleaved.len() / ch);
    for frame in interleaved.chunks_exact(ch) {
        let sum: f32 = frame.iter().sum();
        out.push(sum / ch as f32);
    }
    out
}

/// Linear-interpolation resample of mono f32 between rates.
///
/// V1 simplification: simple and predictable, no external dependency, fully
/// testable. Trade-off is some aliasing on heavy downsampling — for ASR this
/// is acceptable because Parakeet was trained with similar light filtering.
/// If WER measurements show a quality regression we can swap to a sinc /
/// FFT-based resampler (rubato is already in `Cargo.toml`).
pub fn resample_linear_mono(input: &[f32], in_rate: usize, out_rate: usize) -> Vec<f32> {
    if input.is_empty() || in_rate == out_rate {
        return input.to_vec();
    }
    let ratio = out_rate as f64 / in_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let last = input.len() - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = (i as f64) / ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(last);
        let frac = (src - lo as f64) as f32;
        out.push(input[lo] * (1.0 - frac) + input[hi] * frac);
    }
    out
}

/// Split a 16 kHz mono buffer into [`CHUNK_MS`]-millisecond `AudioChunk`s,
/// tagged with the given source. The last chunk may be short.
pub fn chunk_samples(samples: Vec<f32>, source: SpeakerSource) -> Vec<AudioChunk> {
    let mut chunks = Vec::with_capacity(samples.len() / CHUNK_SAMPLES + 1);
    let mut offset_ms = 0u64;
    for block in samples.chunks(CHUNK_SAMPLES) {
        chunks.push(AudioChunk {
            offset_ms,
            samples: block.to_vec(),
            source,
        });
        offset_ms += (block.len() as u64 * 1000) / SAMPLE_RATE as u64;
    }
    chunks
}

// ---------------------------------------------------------------------------
// Mic capture (cpal)
// ---------------------------------------------------------------------------

/// Open the default input device via cpal and stream 16 kHz mono f32 chunks.
///
/// `cpal::Stream` is `!Send` on macOS, so it lives in a dedicated thread
/// whose lifetime is bounded by `stop_rx`. Audio frames cross threads via an
/// unbounded mpsc; resampling + chunking happens on a tokio task so the
/// real-time audio callback never blocks.
fn spawn_mic_capture(
    mut stop_rx: mpsc::Receiver<()>,
) -> Result<mpsc::Receiver<AudioChunk>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| Error::Audio("no default input device".into()))?;
    let supported = device
        .default_input_config()
        .map_err(|e| Error::Audio(format!("default input config: {e}")))?;

    let in_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    tracing::info!(
        device = device.name().as_deref().unwrap_or("?"),
        in_rate,
        channels,
        ?sample_format,
        "mic capture starting"
    );

    let stream_config: cpal::StreamConfig = supported.into();
    let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<Vec<f32>>();
    let (chunk_tx, chunk_rx) = mpsc::channel::<AudioChunk>(64);

    // Bridge tokio stop signal → sync channel the cpal thread can recv on.
    let (stop_thread_tx, stop_thread_rx) = std_mpsc::sync_channel::<()>(1);
    tokio::spawn(async move {
        let _ = stop_rx.recv().await;
        let _ = stop_thread_tx.send(());
    });

    // cpal thread: builds + plays the Stream, blocks until stop signal,
    // drops Stream on exit (which stops capture).
    std::thread::spawn(move || {
        let err_fn = |err| tracing::error!(?err, "cpal stream error");

        let stream_result = match sample_format {
            cpal::SampleFormat::F32 => {
                let tx = raw_tx.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &_| {
                        let _ = tx.send(mixdown_to_mono(data, channels));
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let tx = raw_tx.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &_| {
                        let f: Vec<f32> =
                            data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                        let _ = tx.send(mixdown_to_mono(&f, channels));
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let tx = raw_tx.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &_| {
                        let f: Vec<f32> = data
                            .iter()
                            .map(|s| (*s as f32 - 32768.0) / 32768.0)
                            .collect();
                        let _ = tx.send(mixdown_to_mono(&f, channels));
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                tracing::error!(?other, "unsupported cpal sample format");
                return;
            }
        };
        drop(raw_tx); // last sender lives on the stream; drop our copy

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(?e, "build_input_stream failed");
                return;
            }
        };
        if let Err(e) = stream.play() {
            tracing::error!(?e, "stream.play failed");
            return;
        }
        // Block until stop. Stream drops with this scope, ending capture.
        let _ = stop_thread_rx.recv();
    });

    // Resampler / chunker: aggregates raw frames into 16 kHz, [`CHUNK_MS`]
    // chunks. Re-creates a small batch resample per ~200 ms input window;
    // wasteful but simple. Optimize when perf budget says so.
    let target_input_per_chunk = ((in_rate as u64 * CHUNK_MS) / 1000) as usize;
    tokio::spawn(async move {
        let mut acc: Vec<f32> = Vec::with_capacity(target_input_per_chunk * 2);
        let mut offset_ms: u64 = 0;
        while let Some(raw) = raw_rx.recv().await {
            acc.extend_from_slice(&raw);
            while acc.len() >= target_input_per_chunk {
                let block: Vec<f32> = acc.drain(..target_input_per_chunk).collect();
                let resampled =
                    resample_linear_mono(&block, in_rate as usize, SAMPLE_RATE as usize);
                let chunk_ms =
                    (resampled.len() as u64 * 1000) / SAMPLE_RATE as u64;
                let chunk = AudioChunk {
                    offset_ms,
                    samples: resampled,
                    source: SpeakerSource::Mic,
                };
                if chunk_tx.send(chunk).await.is_err() {
                    return;
                }
                offset_ms += chunk_ms;
            }
        }
    });

    Ok(chunk_rx)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a sine wave at `freq_hz` over `samples` at `rate`.
    fn sine(freq_hz: f32, rate: u32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| (i as f32 / rate as f32 * freq_hz * std::f32::consts::TAU).sin())
            .collect()
    }

    /// Write `samples` (interleaved if multi-channel) as a 16-bit PCM WAV.
    fn write_wav(path: &Path, samples: &[f32], rate: u32, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn temp_wav_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("meetior_test_{}.wav", uuid::Uuid::new_v4()))
    }

    // ---- mixdown -------------------------------------------------------

    #[test]
    fn mixdown_mono_is_identity() {
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(mixdown_to_mono(&input, 1), input);
    }

    #[test]
    fn mixdown_stereo_averages_channels() {
        // L:0.4, R:0.6  →  0.5
        // L:-1.0, R:1.0 →  0.0
        let stereo = vec![0.4, 0.6, -1.0, 1.0];
        let mono = mixdown_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < 1e-6);
        assert!(mono[1].abs() < 1e-6);
    }

    // ---- resample ------------------------------------------------------

    #[test]
    fn resample_same_rate_is_identity() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_linear_mono(&input, 16_000, 16_000), input);
    }

    #[test]
    fn resample_48k_to_16k_yields_third_length() {
        // 1 s of 48 kHz sine → ~16k samples after 1/3 decimation.
        let input = sine(440.0, 48_000, 48_000);
        let out = resample_linear_mono(&input, 48_000, 16_000);
        let expected = 16_000;
        // Linear-interp introduces a small length rounding; allow ±1.
        assert!(
            (out.len() as i64 - expected as i64).abs() <= 1,
            "expected ~{expected} samples, got {}",
            out.len(),
        );
    }

    #[test]
    fn resample_preserves_amplitude_envelope() {
        // A flat DC signal at 0.5 should remain ~0.5 after resampling.
        let input = vec![0.5f32; 4_800];
        let out = resample_linear_mono(&input, 48_000, 16_000);
        assert!(out.len() > 1_500);
        // All output samples should be 0.5 ± tiny rounding (no aliasing on DC).
        for s in &out {
            assert!((s - 0.5).abs() < 1e-3, "got {s}");
        }
    }

    #[test]
    fn resample_handles_empty_input() {
        assert!(resample_linear_mono(&[], 48_000, 16_000).is_empty());
    }

    // ---- chunk_samples -------------------------------------------------

    #[test]
    fn chunk_samples_emits_200ms_chunks_with_monotonic_offsets() {
        // 1 s of audio at 16 kHz → 5 chunks of 3200 samples each.
        let samples = vec![0.0f32; 16_000];
        let chunks = chunk_samples(samples, SpeakerSource::Mic);
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0].offset_ms, 0);
        assert_eq!(chunks[1].offset_ms, 200);
        assert_eq!(chunks[4].offset_ms, 800);
        for c in &chunks {
            assert_eq!(c.samples.len(), CHUNK_SAMPLES);
            assert_eq!(c.source, SpeakerSource::Mic);
        }
    }

    #[test]
    fn chunk_samples_keeps_short_tail() {
        // 1.05 s → 5 full chunks + 1 short tail (800 samples = 50 ms).
        let samples = vec![0.0f32; 16_800];
        let chunks = chunk_samples(samples, SpeakerSource::Mic);
        assert_eq!(chunks.len(), 6);
        assert_eq!(chunks.last().unwrap().samples.len(), 800);
        assert_eq!(chunks.last().unwrap().offset_ms, 1000);
    }

    // ---- WAV reading + end-to-end fixture path -------------------------

    #[test]
    fn read_wav_round_trips_through_mono_16k() {
        // Write a 0.5 s 48 kHz stereo WAV; expect ~8000 samples after
        // mixdown + resample.
        let path = temp_wav_path();
        let stereo_l = sine(440.0, 48_000, 24_000);
        let stereo_r = sine(440.0, 48_000, 24_000);
        let mut interleaved = Vec::with_capacity(48_000);
        for i in 0..24_000 {
            interleaved.push(stereo_l[i]);
            interleaved.push(stereo_r[i]);
        }
        write_wav(&path, &interleaved, 48_000, 2);

        let out = read_wav_to_mono_16k(&path).unwrap();
        let expected = 8_000; // 0.5 s @ 16 kHz
        assert!(
            (out.len() as i64 - expected as i64).abs() <= 2,
            "expected ~{expected} samples, got {}",
            out.len(),
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_wav_handles_16k_mono_input_unchanged() {
        // 16 kHz mono → no mixdown, no resample.
        let path = temp_wav_path();
        let samples = sine(220.0, 16_000, 1_600);
        write_wav(&path, &samples, 16_000, 1);

        let out = read_wav_to_mono_16k(&path).unwrap();
        assert_eq!(out.len(), 1_600);
        let _ = std::fs::remove_file(&path);
    }

    // ---- AudioChunk shape ---------------------------------------------

    #[test]
    fn audio_chunk_carries_source_tag() {
        let mut samples = vec![0.0f32; CHUNK_SAMPLES];
        samples[0] = 1.0;
        let chunks = chunk_samples(samples, SpeakerSource::System);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, SpeakerSource::System);
    }
}
