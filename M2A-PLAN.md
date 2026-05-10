# M2A — Real ASR plan

A step-by-step plan for replacing the `audio.rs` / `asr.rs` stubs with a
real Parakeet streaming pipeline. **Each step ships on its own** — later
steps can compile + unit-test before earlier ones are fully wired, as
long as the env-var fixture path stays open.

For *why* each piece, see [`PRODUCT.md`](./PRODUCT.md) → "Real ASR" and
[`CLAUDE.md`](./CLAUDE.md) → "Module ownership" → `audio.rs` / `asr.rs`.

If you (Claude or human) are picking this up cold: read CLAUDE.md first
for the build/test loop, then come back here. Don't skip CLAUDE.md —
half the gotchas in this plan are documented there.

---

## Pre-req — Parakeet ONNX export (one-time, off-laptop)

Until NVIDIA publishes ONNX directly, export from the NeMo PyTorch
checkpoint on a Linux GPU box:

```python
# scripts/export-parakeet.py
import nemo.collections.asr as nemo_asr
m = nemo_asr.models.ASRModel.from_pretrained("nvidia/parakeet-tdt-0.6b-v3")
m.export("parakeet.onnx")  # produces encoder.onnx + decoder_joint.onnx
```

Drop `encoder.onnx`, `decoder_joint.onnx`, and `tokenizer.json` into
`models/parakeet-tdt-0.6b-v3/`. Keep in private object storage if you
don't want to commit the artifacts; fetch via the M4 first-run flow.

**This pre-req only blocks step 3's end-to-end test.** Steps 1 and 2
can land without it because they don't touch ONNX runtime.

---

## Step 1 — Mic capture (cpal → 16 kHz f32 chunks) — **shipped**

**Done when**: `audio::start_capture` returns a real `mpsc::Receiver<AudioChunk>`
that emits ~200 ms of 16 kHz mono f32 samples per chunk, tagged
`source: SpeakerSource::Mic`. ✓

Status: shipped in commit `8447afb` (or thereabouts). 11 unit tests
cover the DSP path (mixdown, resample, chunking, WAV round-trip). The
cpal mic-device wiring itself is **not** unit-tested — it needs a real
input device + macOS permission grant. Walk it on macOS via
`pnpm tauri dev` to verify; the surrounding pipeline (resampling,
chunking, source tagging) is verified.

**Files**: `src-tauri/src/audio.rs` (replace stub), `src-tauri/Cargo.toml`
(add `cpal = "0.15"`, `rubato = "0.15"`).

**Steps**:
1. Open the default input device with `cpal::default_input_device()`.
2. Convert to f32 mono (mix-down stereo if needed).
3. Resample to 16 kHz via `rubato::FftFixedIn`. Reuse the resampler buffer
   across chunks — don't allocate per chunk (perf budget).
4. Chunk into ~200 ms windows; tag each with `offset_ms` accumulated from
   the start of capture.
5. Honor the existing `MEETIOR_FIXTURE_TRANSCRIPT` env path — fall through
   to mic only if it isn't set.

**Add a fixture path** (mirrors `MEETIOR_FIXTURE_TRANSCRIPT`):
`MEETIOR_FIXTURE_AUDIO=path.wav` reads a WAV via `hound` instead of the
mic. Lets steps 2 and 3 iterate without touching real audio.

**Test**: `audio::tests::resamples_to_16khz_mono` — feed a 48 kHz stereo
fixture WAV through the resample pipeline, assert output rate / channel
count / sample count are right within rounding.

**Risks**:
- macOS first-run prompt for mic permission. `Info.plist` already has
  `NSMicrophoneUsageDescription`.
- cpal callbacks run on a real-time audio thread — don't block; hand off
  via a bounded `mpsc` channel.

---

## Step 2 — Mel-spectrogram preprocessing (pure CPU)

**Done when**: 16 kHz f32 chunks → 80-bin log-mel feature frames
(25 ms window, 10 ms hop), matching Parakeet's training config.

**Files**: new `src-tauri/src/asr/mel.rs` (or whatever submodule layout
you pick — see *Module layout* below). `Cargo.toml`: add `realfft = "3.5"`.

**Steps**:
1. Build the mel filterbank once at startup; cache it on the `Asr`
   handle.
2. Maintain a small `Vec<f32>` overlap buffer between chunks so frames
   that span chunk boundaries don't get dropped.
3. Apply window (Hann), FFT, magnitude, mel projection, log.

**Test**: `asr::mel::tests::log_mel_matches_reference` — feed a fixture
WAV, compare against a known-good NumPy reference. Commit the reference
as `tests/fixtures/mel-reference.npy` (or as a JSON of f32s for
simplicity).

**Risks**:
- Off-by-one in window/hop sizes silently degrades ASR. Pin to NeMo's
  exact constants; reference NeMo's `AudioToMelSpectrogramPreprocessor`
  defaults.
- Streaming context: the overlap buffer is the invariant. Document it.

---

## Step 3 — Parakeet streaming inference

**Done when**: feeding a fixture WAV through the full pipeline produces
recognizable text within a small WER of a reference.

**Files**: `src-tauri/src/asr.rs` (replace stub `Asr::spawn_streaming`),
`Cargo.toml`: confirm `ort = "2.0"` and add `tokenizers = "0.21"`.

**Steps**:
1. `Asr::new(AsrConfig)` loads `encoder.onnx` + `decoder_joint.onnx`
   from `model_dir`. Lazy — don't `ort::init()` if the dir is empty
   (so unit tests outside macOS still compile).
2. Feed log-mel chunks into the encoder. Maintain a rolling cache of
   encoder outputs.
3. Run the decoder/joint autoregressively; **thread the decoder hidden
   state across calls** — this is the hardest part to get right.
4. Detokenize with the SentencePiece tokenizer in `model_dir`. Handle
   the leading `▁` (U+2581) word-boundary marker correctly.
5. Emit `TranscriptSegment { start_ms, end_ms, speaker: None,
   speaker_source, text }` over the existing channel.

**Test**: `asr::tests::transcribes_known_wav` (gated behind a feature
flag if the model files aren't in CI). Loose match on text — exact
output is brittle, target a WER threshold instead.

**Manual smoke**: `pnpm tauri dev`, Start, speak, segments appear < 2 s.

**Risks**:
- Hidden state shape: silent quality degradation if wrong.
- Latency: first measurement of the perf budget. If > 2 s p95, suspect
  per-chunk allocations and unbatched encoder runs first. `cargo
  flamegraph` is in CLAUDE.md.

---

## Step 4 — System audio capture (screencapturekit, macOS 13+)

**Done when**: a second `mpsc::Receiver<AudioChunk>` emits chunks tagged
`source: SpeakerSource::System`, capturing system output (e.g. the other
end of a Google Meet call).

**Files**: `audio.rs` grows a `start_system_capture` companion to
`start_capture`. Mix or interleave the two streams before handing them
to ASR — see *Mixing strategy* below.

**Steps**:
1. `SCStream` with audio-only `SCStreamConfiguration`.
2. Convert delivered `CMSampleBuffer`s to f32 PCM. The fiddly part.
3. Same 16 kHz mono resample as mic.
4. Tag chunks `SpeakerSource::System`.

**Mixing strategy**: for v1, sum mic + system per-sample then clamp to
[-1.0, 1.0]. Each AudioChunk still carries a single `source` tag — for
mixed chunks, pick whichever was louder in the window. Reconsider when
diarization (M3) arrives; at that point we'll likely keep two streams.

**Test**: hard to unit-test (real OS API). Add a mock-buffer path that
asserts tagging. Manual: play a YouTube video, confirm the transcript
includes its audio.

**Risks**:
- Screen-recording permission prompt on first run.
- `screencapturekit` crate's API surface — verify against the version
  you're depending on; some forks lag the upstream Swift API.

**Defer-able**: everything else works with mic-only. Don't block ASR
landing on this step.

---

## Step 5 — Tag propagation + speaker hint

**Done when**:
- Each `TranscriptSegment` carries the right `speaker_source`.
- The notes pane defaults `speaker_hint` to `you` / `them` based on
  recent dominant audio source instead of always `unknown`.

**Files**: `commands.rs::append_note` either auto-infers the hint or the
frontend reads a new `recent_speaker_hint(meeting_id)` query.
Recommendation: query, so the inference state lives in the audio module
where the data is.

**Steps**:
1. `audio.rs` keeps a 500 ms rolling RMS for each source.
2. Expose `recent_dominant_source() -> SpeakerSource` on the `Session`.
3. Frontend (`ActiveMeeting.svelte`) calls this before flushing a note,
   passes the resulting `speaker_hint` into `appendNote`.

**Test**: feed two interleaved fake source streams, assert the
dominant-source query flips correctly across the boundary.

---

## Module layout

`asr.rs` will get bigger. Consider splitting:

```
src-tauri/src/asr/
  mod.rs       # public API (Asr, AsrConfig, spawn_streaming, fixture_*)
  mel.rs       # mel-spectrogram (step 2)
  decoder.rs   # decoder/joint state machine (step 3)
  tokenizer.rs # SentencePiece detokenization helpers (step 3)
```

`audio.rs` similarly:

```
src-tauri/src/audio/
  mod.rs       # AudioChunk type, start_capture / start_system_capture
  mic.rs       # cpal path (step 1)
  system.rs    # screencapturekit path (step 4)
  resample.rs  # shared rubato helpers
```

Don't create the submodule layout until step 2 — premature for step 1
alone.

---

## Cross-cutting

**Buffer reuse**: every per-chunk allocation is a perf budget violation.
Pool buffers or use `Vec::clear` + reuse.

**Threading**: cpal callbacks → bounded `mpsc` → `tokio::task::spawn_blocking`
for ASR (CPU-heavy). Don't run ONNX on the tokio thread pool.

**Logging**: `tracing::debug!` for chunk timings; gate behind
`MEETIOR_DUMP_AUDIO=1` if you also want to dump captured PCM to
`/tmp/meetior-debug.wav` for ffplay inspection.

**Don't break the fixture path**: `MEETIOR_FIXTURE_TRANSCRIPT` must keep
working through every step. Many of the M2b/M2c features rely on it for
agent-environment iteration.

---

## Roughly sequenced

1. Step 1 (mic capture) — backend, ~½ day, no blockers
2. Step 2 (mel-spec) — backend, ~½ day, no blockers
3. **Pre-req: model export** — happens off-laptop; can run in parallel
4. Step 3 (Parakeet inference) — backend, ~1–2 days, blocks on (3)
5. Step 4 (system audio) — backend, ~½ day, defer-able
6. Step 5 (tag propagation + speaker hint) — backend + frontend, ~few hours

After M2a lands: revisit perf budget with Instruments, and walk the
HANDOFF.md verification list (most checkboxes there can finally be
ticked because real transcripts are flowing).
