//! Log-mel spectrogram feature extraction (M2a step 2).
//!
//! Streaming-friendly: feed `AudioChunk` samples into [`MelSpectrogram::process`]
//! across calls, get back complete frames each time. The bit of audio that
//! doesn't fit a full frame stays in the internal overlap buffer for the next
//! call, so segmentation across chunk boundaries is invisible to the caller.
//!
//! ## Configuration
//!
//! Defaults match NeMo's `AudioToMelSpectrogramPreprocessor` for Parakeet-style
//! models:
//!
//! | Parameter   | Value           |
//! |-------------|-----------------|
//! | sample_rate | 16 000 Hz       |
//! | n_fft       | 512             |
//! | win_length  | 400 (= 25 ms)   |
//! | hop_length  | 160 (= 10 ms)   |
//! | n_mels      | 80              |
//! | f_min       | 0 Hz            |
//! | f_max       | 8000 Hz         |
//! | window      | Hann            |
//! | mag_power   | 2.0 (power)     |
//! | log         | log(x + 2⁻²⁴)   |
//! | mel scale   | Slaney (librosa default, htk=False) |
//!
//! Pre-emphasis (NeMo default α=0.97) is **not applied here** — the export
//! script's preprocessor is expected to bake it in or skip it. If WER is poor
//! when M2a step 3 lands, suspect this first; toggle via [`MelConfig::pre_emph`].
//!
//! Per-utterance normalization (NeMo's `normalize="per_feature"`) is also not
//! applied here. It can't be done in a streaming way without buffering, and
//! Parakeet's ONNX export usually includes its own normalization layer.
//!
//! ## Streaming guarantee
//!
//! `process` is a deterministic function of the concatenated input across
//! calls — `mel.process(a) ++ mel.process(b)` produces the same frames as
//! `mel2.process(a ++ b)` (with float tolerance). Verified by
//! `streaming_matches_batch` in tests.

// Allowed until M2a step 3 wires this up — types are public-ish but no
// consumer exists yet beyond tests.
#![allow(dead_code)]

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

/// Tunables. Defaults track NeMo's AudioToMelSpectrogramPreprocessor.
#[derive(Debug, Clone, Copy)]
pub struct MelConfig {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub win_length: usize,
    pub hop_length: usize,
    pub n_mels: usize,
    pub f_min: f32,
    pub f_max: f32,
    /// Power applied to the magnitude spectrum before mel projection.
    /// 2.0 = power spectrum (NeMo default), 1.0 = magnitude.
    pub mag_power: f32,
    /// Added inside the log to avoid -inf on silent regions.
    pub log_epsilon: f32,
    /// Pre-emphasis coefficient `α` in `y[t] = x[t] − α·x[t−1]`.
    /// Set to 0.0 to disable (current default — see module docs).
    pub pre_emph: f32,
}

impl Default for MelConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            n_fft: 512,
            win_length: 400,
            hop_length: 160,
            n_mels: 80,
            f_min: 0.0,
            f_max: 8_000.0,
            mag_power: 2.0,
            log_epsilon: 2f32.powi(-24),
            pre_emph: 0.0,
        }
    }
}

/// Streaming log-mel feature extractor.
///
/// Cheaply built once (filterbank + window + FFT planner all constructed at
/// `new`). Then call [`process`](Self::process) with each incoming chunk —
/// returns whatever full frames are available; partial frames stay buffered.
pub struct MelSpectrogram {
    cfg: MelConfig,
    /// (n_mels, n_fft/2 + 1), row-major: `weights[mel][bin]`.
    filterbank: Vec<Vec<f32>>,
    /// Hann window of length `win_length`.
    window: Vec<f32>,
    /// FFT plan; reusable across frames.
    fft: Arc<dyn RealToComplex<f32>>,
    /// Length-`n_fft` scratch the FFT plan reads from; we zero-pad
    /// `win_length`-long windowed frames into here before each call.
    fft_input: Vec<f32>,
    /// Length-`n_fft/2 + 1` scratch the FFT plan writes into.
    fft_output: Vec<Complex32>,
    /// Samples carried over between `process` calls because they didn't
    /// fit a full frame yet.
    overlap: Vec<f32>,
    /// Last sample seen, for streaming pre-emphasis. Unused while pre_emph=0.
    pre_emph_prev: f32,
}

impl MelSpectrogram {
    pub fn new(cfg: MelConfig) -> Self {
        assert!(cfg.win_length <= cfg.n_fft, "win_length must be ≤ n_fft");
        assert!(cfg.hop_length > 0, "hop_length must be > 0");
        assert!(cfg.n_mels > 0, "n_mels must be > 0");

        let filterbank = build_mel_filterbank(&cfg);
        let window = hann_window(cfg.win_length);

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(cfg.n_fft);
        let fft_input = fft.make_input_vec();
        let fft_output = fft.make_output_vec();

        Self {
            cfg,
            filterbank,
            window,
            fft,
            fft_input,
            fft_output,
            overlap: Vec::new(),
            pre_emph_prev: 0.0,
        }
    }

    /// Reset streaming state. Call between unrelated audio sessions.
    pub fn reset(&mut self) {
        self.overlap.clear();
        self.pre_emph_prev = 0.0;
    }

    /// Process incoming samples, return any newly-completed frames as
    /// rows of `n_mels` log-mel coefficients each.
    pub fn process(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        // Concatenate carried-over tail with new samples. Doing this once
        // upfront keeps the inner loop indexing simple.
        let mut buf = std::mem::take(&mut self.overlap);
        buf.reserve(samples.len());

        // Streaming pre-emphasis: y[t] = x[t] - α·x[t-1]. With α=0 this is
        // a no-op; we still apply the filter so the codepath is deterministic.
        let alpha = self.cfg.pre_emph;
        if alpha == 0.0 {
            buf.extend_from_slice(samples);
        } else {
            let mut prev = self.pre_emph_prev;
            for &s in samples {
                buf.push(s - alpha * prev);
                prev = s;
            }
            self.pre_emph_prev = prev;
        }

        let n_fft = self.cfg.n_fft;
        let win = self.cfg.win_length;
        let hop = self.cfg.hop_length;

        let n_frames = if buf.len() >= win {
            (buf.len() - win) / hop + 1
        } else {
            0
        };

        let mut frames = Vec::with_capacity(n_frames);
        for i in 0..n_frames {
            let start = i * hop;
            let frame = &buf[start..start + win];
            frames.push(self.compute_frame(frame, n_fft));
        }

        // Carry over the tail that didn't fit a full frame.
        let consumed = n_frames * hop;
        self.overlap = buf.split_off(consumed);
        frames
    }

    /// Window → FFT → power → mel projection → log. Allocates one Vec
    /// of length `n_mels` per call; FFT input/output buffers are reused.
    fn compute_frame(&mut self, frame: &[f32], n_fft: usize) -> Vec<f32> {
        // Window into fft_input, zero-pad to n_fft.
        for (dst, (src, w)) in self
            .fft_input
            .iter_mut()
            .zip(frame.iter().zip(self.window.iter()))
        {
            *dst = src * w;
        }
        for dst in &mut self.fft_input[frame.len()..n_fft] {
            *dst = 0.0;
        }

        self.fft
            .process(&mut self.fft_input, &mut self.fft_output)
            .expect("FFT input/output sizes are fixed at construction");

        // |X|² (power) for each FFT bin, then mel projection.
        let n_bins = n_fft / 2 + 1;
        let mut power = vec![0.0f32; n_bins];
        for (i, c) in self.fft_output.iter().enumerate() {
            let mag2 = c.re * c.re + c.im * c.im;
            power[i] = if self.cfg.mag_power == 2.0 {
                mag2
            } else if self.cfg.mag_power == 1.0 {
                mag2.sqrt()
            } else {
                mag2.powf(self.cfg.mag_power / 2.0)
            };
        }

        let mut mel = vec![0.0f32; self.cfg.n_mels];
        for (m, weights) in self.filterbank.iter().enumerate() {
            let mut sum = 0.0f32;
            for (b, &w) in weights.iter().enumerate() {
                sum += w * power[b];
            }
            mel[m] = (sum + self.cfg.log_epsilon).ln();
        }
        mel
    }
}

// ---------------------------------------------------------------------------
// Filterbank + window construction
// ---------------------------------------------------------------------------

fn hann_window(len: usize) -> Vec<f32> {
    if len <= 1 {
        return vec![1.0; len];
    }
    let denom = (len - 1) as f32;
    (0..len)
        .map(|i| {
            0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / denom).cos()
        })
        .collect()
}

/// Slaney-style mel scale (librosa default, `htk=False`). Linear under
/// 1000 Hz, logarithmic above. Works for any non-negative frequency.
fn hz_to_mel(f: f32) -> f32 {
    const F_SP: f32 = 200.0 / 3.0; // linear-region step
    const MIN_LOG_HZ: f32 = 1000.0;
    const MIN_LOG_MEL: f32 = MIN_LOG_HZ / F_SP; // = 15.0
    let logstep = (6.4f32).ln() / 27.0;
    if f >= MIN_LOG_HZ {
        MIN_LOG_MEL + (f / MIN_LOG_HZ).ln() / logstep
    } else {
        f / F_SP
    }
}

fn mel_to_hz(m: f32) -> f32 {
    const F_SP: f32 = 200.0 / 3.0;
    const MIN_LOG_HZ: f32 = 1000.0;
    const MIN_LOG_MEL: f32 = MIN_LOG_HZ / F_SP;
    let logstep = (6.4f32).ln() / 27.0;
    if m >= MIN_LOG_MEL {
        MIN_LOG_HZ * (logstep * (m - MIN_LOG_MEL)).exp()
    } else {
        F_SP * m
    }
}

/// Build an `(n_mels, n_fft/2 + 1)` matrix of triangular filter weights.
/// Each row is one mel filter; the row sums to a peak of 1.0 at the filter's
/// center bin.
fn build_mel_filterbank(cfg: &MelConfig) -> Vec<Vec<f32>> {
    let n_bins = cfg.n_fft / 2 + 1;
    // Frequency of each FFT bin centre: bin k → k * sr / n_fft.
    let bin_freqs: Vec<f32> = (0..n_bins)
        .map(|k| k as f32 * cfg.sample_rate as f32 / cfg.n_fft as f32)
        .collect();

    // (n_mels + 2) mel-evenly-spaced anchor frequencies, including the lower
    // and upper edges of the first and last filter.
    let mel_min = hz_to_mel(cfg.f_min);
    let mel_max = hz_to_mel(cfg.f_max);
    let anchors_mel: Vec<f32> = (0..cfg.n_mels + 2)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (cfg.n_mels + 1) as f32)
        .collect();
    let anchors_hz: Vec<f32> = anchors_mel.iter().map(|&m| mel_to_hz(m)).collect();

    let mut fb = Vec::with_capacity(cfg.n_mels);
    for m in 0..cfg.n_mels {
        let lower = anchors_hz[m];
        let center = anchors_hz[m + 1];
        let upper = anchors_hz[m + 2];
        let mut row = vec![0.0f32; n_bins];
        for (b, &f) in bin_freqs.iter().enumerate() {
            let w = if f <= lower || f >= upper {
                0.0
            } else if f <= center {
                (f - lower) / (center - lower).max(f32::EPSILON)
            } else {
                (upper - f) / (upper - center).max(f32::EPSILON)
            };
            row[b] = w;
        }
        fb.push(row);
    }
    fb
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq_hz: f32, rate: u32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| (i as f32 / rate as f32 * freq_hz * std::f32::consts::TAU).sin())
            .collect()
    }

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps + eps * b.abs()
    }

    // ---- mel scale invariants -----------------------------------------

    #[test]
    fn hz_to_mel_is_monotonic_and_invertible() {
        let pts = [0.0, 100.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];
        let mut last = -f32::INFINITY;
        for f in pts {
            let m = hz_to_mel(f);
            assert!(m > last, "hz_to_mel non-monotonic at {f} Hz");
            last = m;
            assert!(
                approx_eq(mel_to_hz(m), f, 1e-3),
                "round-trip {f} Hz → {m} mel → {} Hz",
                mel_to_hz(m),
            );
        }
    }

    #[test]
    fn hz_to_mel_breakpoint_at_1000hz() {
        // Slaney scale crossover.
        assert!(approx_eq(hz_to_mel(1000.0), 15.0, 1e-3));
    }

    // ---- filterbank shape ---------------------------------------------

    #[test]
    fn filterbank_has_expected_shape() {
        let cfg = MelConfig::default();
        let fb = build_mel_filterbank(&cfg);
        assert_eq!(fb.len(), cfg.n_mels);
        let n_bins = cfg.n_fft / 2 + 1;
        for row in &fb {
            assert_eq!(row.len(), n_bins);
        }
    }

    #[test]
    fn filterbank_rows_have_a_peak() {
        // Every triangular filter should hit 1.0 (or very close) at its
        // center bin, and be ≤ 1.0 elsewhere.
        let fb = build_mel_filterbank(&MelConfig::default());
        for (i, row) in fb.iter().enumerate() {
            let max = row.iter().copied().fold(0.0f32, f32::max);
            assert!(
                max > 0.5 && max <= 1.0 + 1e-6,
                "filter {i}: max weight {max} out of expected range",
            );
        }
    }

    // ---- MelSpectrogram.process ---------------------------------------

    #[test]
    fn process_emits_expected_frame_count() {
        let cfg = MelConfig::default();
        let mut mel = MelSpectrogram::new(cfg);
        // 1 s of audio at 16 kHz, win=400, hop=160 → 99 frames.
        let samples = vec![0.0f32; 16_000];
        let frames = mel.process(&samples);
        let expected = (16_000 - cfg.win_length) / cfg.hop_length + 1;
        assert_eq!(frames.len(), expected);
        assert_eq!(expected, 98); // (16000 - 400) / 160 + 1 = 97 + 1 = 98
        for f in &frames {
            assert_eq!(f.len(), cfg.n_mels);
        }
    }

    #[test]
    fn process_returns_no_frames_for_short_input() {
        let mut mel = MelSpectrogram::new(MelConfig::default());
        let frames = mel.process(&vec![0.0f32; 100]);
        assert_eq!(frames.len(), 0);
    }

    #[test]
    fn silence_produces_log_floor_features() {
        let cfg = MelConfig::default();
        let mut mel = MelSpectrogram::new(cfg);
        let frames = mel.process(&vec![0.0f32; 2_000]);
        assert!(!frames.is_empty());
        let expected = cfg.log_epsilon.ln();
        for frame in &frames {
            for &v in frame {
                assert!(
                    approx_eq(v, expected, 1e-3),
                    "expected {expected}, got {v}",
                );
            }
        }
    }

    #[test]
    fn sine_wave_peaks_at_expected_mel_bin() {
        // A 1000 Hz sine should peak in the mel bin whose center is closest
        // to 1000 Hz. That's the mel bin closest to 15 mel out of [0, ~31].
        let cfg = MelConfig::default();
        let mut mel = MelSpectrogram::new(cfg);
        let samples = sine(1000.0, cfg.sample_rate, 8_000);
        let frames = mel.process(&samples);

        // Average across frames to smooth the spectrogram (sine isn't
        // perfectly aligned to FFT bins).
        let mut avg = vec![0.0f32; cfg.n_mels];
        for f in &frames {
            for (i, v) in f.iter().enumerate() {
                avg[i] += v;
            }
        }
        for v in &mut avg { *v /= frames.len() as f32; }

        let (peak, _) = avg.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        // Compute the expected bin: mel index closest to hz_to_mel(1000) = 15
        // mapped onto our [0, hz_to_mel(8000)] range across n_mels bins.
        let mel_max = hz_to_mel(cfg.f_max);
        let target_mel = hz_to_mel(1000.0);
        let expected_bin =
            (target_mel / mel_max * cfg.n_mels as f32).round() as usize;
        assert!(
            peak.abs_diff(expected_bin) <= 3,
            "1 kHz sine peak at bin {peak}, expected ~{expected_bin}",
        );
    }

    // ---- streaming guarantee ------------------------------------------

    #[test]
    fn streaming_matches_batch() {
        // The big correctness test: feeding [a, b] across two calls should
        // produce the same frames as feeding [a ++ b] in one call.
        let cfg = MelConfig::default();
        let samples = sine(440.0, cfg.sample_rate, 4_000);

        let mut split = MelSpectrogram::new(cfg);
        let mut out_split = split.process(&samples[..1234]);
        out_split.extend(split.process(&samples[1234..]));

        let mut batch = MelSpectrogram::new(cfg);
        let out_batch = batch.process(&samples);

        assert_eq!(out_split.len(), out_batch.len());
        for (i, (a, b)) in out_split.iter().zip(out_batch.iter()).enumerate() {
            for (j, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
                assert!(
                    approx_eq(*av, *bv, 1e-4),
                    "frame {i} mel {j}: split={av} vs batch={bv}",
                );
            }
        }
    }

    #[test]
    fn streaming_handles_chunk_boundary_at_every_offset() {
        // Stronger version: try splitting at every reasonable offset and
        // confirm the result is identical.
        let cfg = MelConfig::default();
        let samples = sine(2_000.0, cfg.sample_rate, 1_600);
        let mut batch = MelSpectrogram::new(cfg);
        let reference = batch.process(&samples);

        for split_at in [0, 1, 100, 399, 400, 401, 500, 800, 1599, 1600] {
            let mut mel = MelSpectrogram::new(cfg);
            let mut out = mel.process(&samples[..split_at]);
            out.extend(mel.process(&samples[split_at..]));
            assert_eq!(
                out.len(),
                reference.len(),
                "frame count diverged at split {split_at}",
            );
            for (i, (a, b)) in out.iter().zip(reference.iter()).enumerate() {
                for (av, bv) in a.iter().zip(b.iter()) {
                    assert!(
                        approx_eq(*av, *bv, 1e-4),
                        "split_at={split_at} frame {i}: {av} vs {bv}",
                    );
                }
            }
        }
    }

    #[test]
    fn reset_clears_streaming_state() {
        let cfg = MelConfig::default();
        let mut mel = MelSpectrogram::new(cfg);
        mel.process(&vec![0.5f32; 300]); // less than win_length, so all carries
        assert!(!mel.overlap.is_empty());
        mel.reset();
        assert!(mel.overlap.is_empty());
    }

    // ---- pre-emphasis (off by default) --------------------------------

    #[test]
    fn pre_emphasis_off_is_pass_through() {
        let cfg = MelConfig { pre_emph: 0.0, ..MelConfig::default() };
        let mut mel = MelSpectrogram::new(cfg);
        // Process some samples; with pre_emph=0 the streaming pre-emph state
        // shouldn't matter — splitting must still match batch.
        let samples = sine(500.0, cfg.sample_rate, 2_000);
        let mut split = MelSpectrogram::new(cfg);
        let mut out = split.process(&samples[..500]);
        out.extend(split.process(&samples[500..]));
        let batch = mel.process(&samples);
        assert_eq!(out.len(), batch.len());
    }

    #[test]
    fn pre_emphasis_streams_correctly() {
        // With α>0, the streaming filter still must equal the batch result.
        let cfg = MelConfig { pre_emph: 0.97, ..MelConfig::default() };
        let samples = sine(500.0, cfg.sample_rate, 2_000);

        let mut split = MelSpectrogram::new(cfg);
        let mut out = split.process(&samples[..617]);
        out.extend(split.process(&samples[617..]));

        let mut batch = MelSpectrogram::new(cfg);
        let reference = batch.process(&samples);

        assert_eq!(out.len(), reference.len());
        for (a, b) in out.iter().zip(reference.iter()) {
            for (av, bv) in a.iter().zip(b.iter()) {
                assert!(
                    approx_eq(*av, *bv, 1e-3),
                    "pre-emph streaming diverged: {av} vs {bv}",
                );
            }
        }
    }
}
