# CLAUDE.md — Meetior

This file is the durable brief for any Claude session that opens this repo.
Read it first; update it when decisions change.

For **what** we're building (product scope, data model, user journeys,
behavior boundaries), see [`PRODUCT.md`](./PRODUCT.md). This file covers
the **how**: stack, layout, conventions, and the build/test/iterate loop.

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

`ort` uses `load-dynamic` — point it at Homebrew's onnxruntime, and skip
ort's auto-download at build time:

```sh
export ORT_DYLIB_PATH=$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib
export ORT_SKIP_DOWNLOAD=1
```

Add both to your shell profile or a `.envrc` (direnv).

### First end-to-end demo (no Parakeet, no audio yet)

```sh
ollama serve &  # if not already running
export MEETIOR_FIXTURE_TRANSCRIPT="$PWD/examples/fixture-transcript.json"
pnpm tauri dev
```

Click **Start meeting** → fixture transcript streams in → **Stop** → open
the meeting → **Generate summary & todos** → Ollama produces a real
summary + todos from the canned text. Validates the entire pipeline shape
without any of the M2 work.

---

## Data locations

At runtime the app stores data under macOS app-data:

- DB: `~/Library/Application Support/app.meetior/meetior/meetior.sqlite`
- Models: `~/Library/Application Support/app.meetior/meetior/models/<id>/`
  (override with `MEETIOR_MODEL_DIR`)

Local dev fallback model dir: `./models/parakeet-tdt-0.6b-v3/`.

---

## Build, test, iterate

The shortest path to "is my change correct?" — use the cheapest tool that
gives a real answer. Don't run the full app to verify backend logic; don't
restart Tauri to verify a CSS tweak.

### Dev loops, ranked by cost

| Loop | Cost | Use when |
|---|---|---|
| `pnpm check` | <2 s | After Svelte/TS edits — types only. |
| `cargo check -p meetior` | a few s | After Rust edits — borrow checker only, no codegen. |
| `cargo test -p meetior <pat>` | seconds | After backend logic edits — focused unit tests. |
| `pnpm dev` (Vite only) | hot reload | UI-only iteration with the Tauri shell **closed**; mock `invoke` (see below). |
| `pnpm tauri dev` | 30–60 s cold, hot after | End-to-end: clicking through the app on real macOS. |
| `pnpm tauri build` | minutes | Pre-release sanity. Don't run in normal dev. |

Run all three quick checks in parallel before pushing:

```sh
pnpm check & (cd src-tauri && cargo check) & (cd src-tauri && cargo test) & wait
```

### UI-only iteration (no Tauri shell)

When iterating on Svelte, run Vite alone and stub the backend in
`src/lib/api.ts` behind a `import.meta.env.DEV && !window.__TAURI_INTERNALS__`
guard so the UI renders with fake data. Faster than rebuilding Rust. Don't
commit the stubs — guard them or keep them in a local dev branch.

### Unit testing the backend

Each domain module should grow tests next to it. Pattern:

```rust
// in src-tauri/src/storage.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store { Store::open(":memory:").unwrap() }

    #[test]
    fn round_trips_a_meeting() {
        let s = store();
        let m = crate::meeting::Meeting::new("test");
        s.insert_meeting(&m).unwrap();
        assert_eq!(s.list_meetings().unwrap().len(), 1);
    }
}
```

Conventions:
- `storage.rs` tests use `:memory:` SQLite — no temp files, no cleanup.
- `summarizer.rs` tests hit a fake Ollama via `wiremock` (add to dev-deps
  when needed) or skip when `MEETIOR_OLLAMA_HOST` is unset.
- `audio.rs` / `asr.rs` tests use **fixture WAVs** under
  `src-tauri/tests/fixtures/`. Keep them short (≤ 5 s) and check expected
  text loosely — exact ASR output is brittle. Word-error-rate against a
  reference is fine.
- Integration tests covering Tauri commands go in `src-tauri/tests/`. Use
  `tauri::test::mock_app()` rather than spinning a real window.

### Iterating on the stubs without macOS audio

You don't need ScreenCaptureKit working to make progress on the rest of
the pipeline.

**Faux ASR — already implemented.** Set `MEETIOR_FIXTURE_TRANSCRIPT` to a
JSON file of `TranscriptSegment`s and `commands::start_meeting` will replay
it on a timer instead of starting real audio capture. A sample is shipped
at `examples/fixture-transcript.json`:

```sh
export MEETIOR_FIXTURE_TRANSCRIPT="$PWD/examples/fixture-transcript.json"
pnpm tauri dev
```

Click **Start meeting** → segments stream into the active view at their
real timestamps → click **Stop** → open the meeting → **Generate summary &
todos** hits Ollama. Full flow without Parakeet or ScreenCaptureKit.

**Faux audio capture** is *not yet implemented*. When we need it (e.g. to
test the mel-spec pipeline against a known WAV), add `MEETIOR_FIXTURE_AUDIO`
in `audio.rs` symmetrically. Don't let either grow features — they exist so
M2 work doesn't block M3 work.

### Logging & inspection

```sh
# Verbose backend logs (filterable per-module)
RUST_LOG=meetior_lib=debug,ort=info pnpm tauri dev

# Inspect the live database
sqlite3 "$HOME/Library/Application Support/app.meetior/meetior/meetior.sqlite" \
  ".tables" "SELECT id, title, status FROM meetings;"

# Watch events in the UI
# Open the Tauri devtools (Cmd+Option+I) → Console → events appear via api.ts.
```

For audio-pipeline debugging, write captured chunks to `/tmp/meetior-debug.wav`
behind a `MEETIOR_DUMP_AUDIO=1` env. Inspect with QuickLook or `ffplay`.

### Manual smoke test (run before pushing UI/backend changes)

1. `pnpm tauri dev` — app opens, no console errors.
2. Click **Start meeting** — status flips, active view shows "Listening…".
   With a fixture WAV configured, segments appear within ~1 s.
3. Click **Stop meeting** — returns to list, meeting shows `done`.
4. Open the meeting → click **Generate summary & todos** with Ollama running
   → summary + todos render; checkbox toggles persist after reload.
5. Quit and reopen — meetings persist (SQLite survived).

If any step fails, fix it before adding new behavior. Don't paper over a
broken flow with a UI guard.

### Performance budget

We chose Rust + Tauri because we care about overhead. Rough targets:

- Idle CPU with a meeting recording: < 15 % on M-series, single core.
- Transcription latency (audio in → segment in UI): < 2 s p95.
- Cold app start: < 1 s to first paint.

Profile with Instruments (Time Profiler) and `cargo flamegraph`. Avoid
allocating per audio chunk — reuse buffers in the audio + ASR loops.

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

### M1 — Foundation (done)
- [x] Tauri 2 + Svelte scaffold compiles end-to-end.
- [x] Manual start/stop UI.
- [x] SQLite persistence (with unit tests).
- [x] Ollama summarization wired in.
- [x] Audio + ASR module stubs with clear contracts.
- [x] Faux ASR fixture pipeline so the full UI flow runs without Parakeet.

### M2 — Real transcription + product surface
Driven by [`PRODUCT.md`](./PRODUCT.md). Suggested order:

**M2a — Real ASR (foundation)**
- [ ] mic capture via `cpal` → 16 kHz f32 chunks. Tag chunks with
      `speaker_source = mic`.
- [ ] system-audio capture via `screencapturekit` (macOS 13+). Tag chunks
      `speaker_source = system`.
- [ ] mel-spectrogram pre-processing.
- [ ] Parakeet ONNX inference (encoder + decoder/joint, streaming).
      Propagate the source tag onto each emitted segment.
- [ ] `scripts/export-parakeet.py` for the model export step.

**M2b — Product surface around it**
- [ ] `clients` table + Tauri commands (`create_client`, `list_clients`).
      Sidebar UI with curated color palette.
- [ ] Client picker in the Start dialog (existing / new / no client).
- [ ] `notes` table + live notes pane in active-meeting view, with
      timestamp + speaker-hint derived from recent dominant source.
- [ ] `importer.rs` (symphonia) + `import_meeting` command + UI button.
      Reject `.mp4`/`.mov` up front for v1.
- [ ] Edit affordances: meeting title, client, summary text, todos
      (add / delete / edit text). Transcript stays read-only.
- [ ] Filter UI: client + date range chips. SQLite indexes only.
- [ ] Cross-client digest view + `digest()` method on `summarizer.rs`.
- [ ] Crash-recovery on launch: any meeting in `recording` status → mark
      `done`, keep saved segments and notes.

### M3 — Auto-detect & polish
- [ ] VAD-based "meeting started?" prompt.
- [ ] Optional browser extension that pings the app when a Meet tab is
      active. (Defer until M2 ships.)
- [ ] Real speaker diarization (Pyannote ONNX). Replaces the "you vs them"
      mic/system heuristic where it's confident.
- [ ] Settings UI: retention policy, model picker, audio source toggles,
      hotkey.
- [ ] Full-text search across transcripts (SQLite FTS5).

### M4 — Distribution
- [ ] Replace placeholder icons (`pnpm tauri icon source.png`).
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
