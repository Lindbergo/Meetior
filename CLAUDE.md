# CLAUDE.md — Meetior

This file is the durable brief for any Claude session that opens this repo.
Read it first; update it when decisions change.

---

## What we're building

A **macOS-only**, **fully-local** meeting assistant. The flow is:

```
Google Meet runs in browser
      │
      ▼
Audio capture (mic + system audio)
      │
      ▼
Streaming ASR  ─►  Parakeet TDT 0.6B v3 (ONNX)
      │
      ▼
Transcript stored (SQLite) + streamed to UI
      │
      ▼
On meeting end → local LLM (Ollama) summarizes
      │
      ▼
Summary + extracted todos shown in UI
```

Privacy guarantee: **no audio, transcript, or text leaves the machine.**
The only network call is to a local Ollama server on `127.0.0.1:11434`.

---

## Stack — and why

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri 2** | Native, small binary, Rust backend. Performant, as requested. |
| Frontend | **Svelte 5 + TS + Vite** | Smallest runtime in the major frameworks; fast cold-start. |
| Async runtime | **tokio** | Standard. |
| Audio (mic) | **cpal** | Cross-platform input. |
| Audio (system) | **screencapturekit** crate | macOS 13+ system-audio capture without virtual drivers. |
| ASR | **Parakeet TDT 0.6B v3** via **ort** (ONNX Runtime) | Best open streaming ASR as of 2026; pure-Rust runtime call site. |
| Tokenizer | **tokenizers** crate | Loads Parakeet's SentencePiece BPE. |
| LLM | **Ollama** HTTP API | Easiest local-LLM swapping. Default model: `llama3.1:8b-instruct-q4_K_M`. |
| Persistence | **SQLite (rusqlite + r2d2)** | Bundled, file-based, fits offline-first. |
| Logging | **tracing** | Structured logs via `RUST_LOG`. |

### Decisions explicitly NOT made yet
- Speaker diarization — left null in segments. (Pyannote ONNX is a candidate.)
- Auto-detect meeting start — v1 is manual. Planned: VAD on system-audio +
  watch the focused window's title for `Meet — `.
- Bundling Ollama vs. embedded llama.cpp — kept Ollama for v1 since it's
  swappable; revisit when we ship.

---

## Repo layout

```
.
├── package.json               # Vite + Svelte + Tauri CLI
├── vite.config.ts
├── svelte.config.js
├── tsconfig.json
├── index.html
├── src/                       # Svelte frontend
│   ├── main.ts
│   ├── app.css
│   ├── App.svelte             # Top-level: list / active / detail
│   └── lib/
│       ├── api.ts             # Typed wrapper over invoke + events
│       ├── MeetingList.svelte
│       ├── ActiveMeeting.svelte
│       └── MeetingDetail.svelte
└── src-tauri/                 # Rust backend
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── entitlements.plist     # mic + screen-capture (system audio)
    ├── Info.plist             # NSMicrophoneUsageDescription, etc.
    ├── capabilities/default.json
    ├── icons/                 # placeholder; run `pnpm tauri icon`
    └── src/
        ├── main.rs            # Thin wrapper around lib::run()
        ├── lib.rs             # Tauri builder + AppState
        ├── error.rs           # `Error` enum, serde-serializable
        ├── meeting.rs         # Domain types + Session
        ├── audio.rs           # Capture pipeline (STUB)
        ├── asr.rs             # Parakeet ONNX inference (STUB)
        ├── summarizer.rs      # Ollama HTTP client (working)
        ├── storage.rs         # SQLite schema + queries (working)
        └── commands.rs        # Tauri commands surface
```

---

## Setup (macOS)

Prereqs once:

```sh
# Toolchain
xcode-select --install
brew install rustup-init && rustup-init -y
brew install pnpm onnxruntime ollama

# Pull a model for Ollama (background)
ollama pull llama3.1:8b-instruct-q4_K_M
```

Project bootstrap:

```sh
pnpm install
```

Run dev:

```sh
pnpm tauri dev
```

`ort` uses `load-dynamic` — point it at Homebrew's onnxruntime:

```sh
export ORT_DYLIB_PATH=$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib
```

Add this to your shell profile or a `.envrc` (direnv).

---

## Data locations

At runtime the app stores data under macOS app-data:

- DB: `~/Library/Application Support/app.meetior/meetior/meetior.sqlite`
- Models: `~/Library/Application Support/app.meetior/meetior/models/<id>/`
  (override with `MEETIOR_MODEL_DIR`)

Local dev fallback model dir: `./models/parakeet-tdt-0.6b-v3/`.

---

## Tauri command surface

All commands live in `src-tauri/src/commands.rs`. Keep them thin — validate
input, call into a domain module, return serializable types.

| Command | Args | Returns | Notes |
|---|---|---|---|
| `start_meeting` | `title?: string` | `Meeting` | Errors if a session is already live. |
| `stop_meeting` | `id: string` | `Meeting` | Drains pipeline, marks `done`. |
| `list_meetings` | — | `Meeting[]` | Newest first. |
| `get_meeting` | `id: string` | `MeetingDetail` | Includes segments, summary, todos. |
| `summarize_meeting` | `id: string` | `MeetingDetail` | Calls Ollama. Idempotent. |
| `toggle_todo` | `meetingId, todoId` | `Todo` | Flips `done`. |

Events emitted to the UI:

- `meetior://transcript-segment` — `{ meeting_id, start_ms, end_ms, speaker, text }`
- `meetior://meeting-status` — `Meeting`

---

## Module ownership & invariants

### `audio.rs` — STUB
Owns capture. Output is **always 16 kHz mono f32** in `[-1.0, 1.0]`,
delivered as `AudioChunk { offset_ms, samples }` over a tokio channel.
Roughly 200 ms per chunk.

Implementation plan:
1. **Mic** via `cpal::default_input_device()`. Convert to f32 mono, resample
   with `rubato::FftFixedIn` to 16 kHz.
2. **System audio** on macOS via `screencapturekit` —
   `SCStream` with audio-only configuration. Same resampling.
3. Mix mic + system per-sample (sum then clamp). Or keep them as two channels
   if we later add diarization.

### `asr.rs` — STUB
Loads encoder + decoder/joint ONNX sessions from `model_dir`. Streaming
inference must:
1. Window 10 ms hops, 25 ms frames, 80 mel bins (compute log-mel features).
   Use a Rust mel-spectrogram impl; consider porting NeMo's defaults to keep
   parity with Parakeet's training.
2. Run `encoder.onnx` on rolling chunks of features.
3. Run `decoder_joint.onnx` autoregressively, threading the decoder hidden
   state across calls.
4. Detokenize with the SentencePiece tokenizer included in the model dir.

Model export — until NVIDIA publishes ONNX directly, do this once on a Linux
GPU box and check the artifacts into a private storage bucket:

```python
import nemo.collections.asr as nemo_asr
m = nemo_asr.models.ASRModel.from_pretrained("nvidia/parakeet-tdt-0.6b-v3")
m.export("parakeet.onnx")  # produces encoder.onnx + decoder_joint.onnx
```

Drop the artifacts + `tokenizer.json` into `models/parakeet-tdt-0.6b-v3/`.

### `summarizer.rs` — working
Calls `POST /api/chat` on Ollama with `format: "json"` and a system prompt
that constrains output to `{ summary, todos }`. Override host/model via
`MEETIOR_OLLAMA_HOST` / `MEETIOR_OLLAMA_MODEL`.

### `storage.rs` — working
SQLite schema in `migrate()`. `meetings`, `segments`, `summaries`, `todos`.
WAL mode, FK on. All write paths are synchronous against the `r2d2` pool.

### `meeting.rs` — working
Domain types + `Session` (the in-flight pipeline handle). `Session::shutdown`
sends a stop signal and awaits all spawned tasks.

### `commands.rs` — working (against stubs)
The full pipeline wiring already exists; once `audio.rs` and `asr.rs` are
real, transcript segments will start flowing into the UI without changes here.

---

## Roadmap

### M1 — Foundation (this commit)
- [x] Tauri 2 + Svelte scaffold compiles end-to-end.
- [x] Manual start/stop UI.
- [x] SQLite persistence.
- [x] Ollama summarization wired in.
- [x] Audio + ASR module stubs with clear contracts.

### M2 — Real transcription (manual start)
- [ ] mic capture via `cpal` → 16 kHz f32 chunks.
- [ ] system-audio capture via `screencapturekit` (macOS 13+).
- [ ] mel-spectrogram pre-processing.
- [ ] Parakeet ONNX inference (encoder + decoder/joint, streaming).
- [ ] Ship a `scripts/export-parakeet.py` for the model export step.

### M3 — Auto-detect & polish
- [ ] VAD-based "meeting started?" prompt.
- [ ] Optionally: a tiny browser extension that pings the app when a Meet tab
      is active. (Decided in initial requirements: defer to post-M2.)
- [ ] Speaker diarization (Pyannote ONNX or lightweight clustering on
      mic-vs-system tracks).
- [ ] Settings UI: model picker, audio source toggles, hotkey.

### M4 — Distribution
- [ ] Generate icons (`pnpm tauri icon`).
- [ ] Notarize + DMG via `pnpm tauri build`.
- [ ] First-run flow: download Parakeet ONNX from a release asset, verify
      checksum, place under app-data.

---

## Conventions

- **Errors**: never `.unwrap()` on user-reachable paths. Prefer `crate::Error`.
- **Logging**: `tracing` with structured fields. Avoid `println!`.
- **Async**: pipeline plumbing uses tokio mpsc channels; CPU-heavy work
  (mel-spec, ONNX) on `tokio::task::spawn_blocking` or a dedicated thread.
- **State**: `AppState` holds `Arc`s; lock at the call site, hold briefly.
- **UI**: keep it deliberately simple while we iterate on the backend.
  Three views only: list, active, detail. Don't add navigation, settings,
  or theming beyond the existing CSS variables yet.

---

## What NOT to do

- Don't add cross-platform audio code yet — macOS only until M3 ships.
- Don't reach for cloud APIs (OpenAI, Anthropic) for any feature. Local-only.
- Don't add Tauri plugins beyond `tauri-plugin-shell` without updating
  `capabilities/default.json` and listing the rationale here.
- Don't bypass `Store` for direct SQL — keep all schema knowledge in
  `storage.rs` so migrations stay in one place.
- Don't pin Parakeet's tokenizer or feature config in code; load them from
  files in `model_dir` so we can swap models without recompiling.
