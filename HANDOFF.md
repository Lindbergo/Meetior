# HANDOFF

Things I couldn't do from the agent's Linux dev environment, or
deliberately deferred — running list, prune as you check items off.

## Picking this up?

If you're starting a fresh Claude session (or coming back from mobile),
read these in order — together they take ~3 minutes:

1. **[`CLAUDE.md`](./CLAUDE.md)** — durable brief: stack, repo layout,
   build/test loop, conventions. The one section worth re-reading
   every time is "Build, test, iterate" → "Dev loops, ranked by cost".
2. **[`PRODUCT.md`](./PRODUCT.md)** — the *what* (data model, user
   journeys, behavior boundaries).
3. **This file** — what's pending and where the gaps are.
4. **[`M2A-PLAN.md`](./M2A-PLAN.md)** — the next big chunk of work
   (real ASR via Parakeet). Step-by-step, each step ships on its own.

Current branch state (last touched): `claude/meeting-transcription-app-fFtqw`.

**Shipped:**
- M2b (clients, notes, edits) — merged
- M2c (notes search) — merged
- Vite-only `invoke` stub (`src/lib/api-stub.ts`) — `pnpm dev` lets you
  click through everything without macOS or Ollama
- M2a step 1 (mic capture via cpal + fixture-audio fallback) — DSP
  unit-tested; cpal device handshake pending macOS
- M2a step 2 (streaming log-mel feature extraction) — fully unit-tested,
  including the streaming-correctness invariant
- M2a step 3 prep (BPE tokenizer trait + streaming detokenizer) —
  scaffolding for the real Parakeet wiring
- Crash recovery on launch (orphaned recordings → done)

**Counts:** 71 unit tests passing, `cargo clippy` clean, `pnpm check` /
`pnpm build` clean.

**Genuinely blocked on:**
- macOS access (cpal mic verification, ScreenCaptureKit, real perf)
- Parakeet ONNX artifacts (one-time Python export on a Linux GPU box —
  pre-req in M2A-PLAN.md)

To resume: tell the agent **"continue M2a step N"** (replace N) and
the plan doc is detailed enough that a cold session can execute it.
Once the model files exist, step 3 can be wired up; step 4 (system
audio) and step 5 (speaker hint inference) need macOS.

---

## Manual verification on macOS

I never opened the Tauri shell. Please `pnpm tauri dev` and walk through:

**Sidebar (step 3)**
- [ ] Sidebar renders with the curated 9-color palette
- [ ] "+" reveals input. **Enter** creates, **Esc** cancels, blur-on-empty
      cancels. New client becomes the active filter.
- [ ] All / Unassigned / per-client filters narrow the list and counts match

**Start dialog (step 3)**
- [ ] Opens on **Start meeting**, **Esc** closes, backdrop click closes
- [ ] **"+ New client…"** flow creates client *and* starts in one go
- [ ] Active sidebar filter pre-fills the picker

**Meeting list (step 3)**
- [ ] Rows show the color dot + client name when assigned

**Active meeting view (step 4)**
- [ ] 60/40 split: transcript left, notes right (each scrolls independently)
- [ ] Notes composer: **Enter** saves, **Shift+Enter** inserts a newline
- [ ] Saved notes auto-scroll to the new note on save
- [ ] Timestamps render as "m:ss" from meeting start
- [ ] Reopening a recording window restores the existing notes (mount-time
      `getMeeting` fetch)

**Detail view edits (step 5)**
- [ ] Title: click **Edit** → input → Enter saves, Esc cancels
- [ ] Client dropdown: change reassigns; **Unassigned** option clears it.
      Color dot updates next to the dropdown
- [ ] Summary: **Edit summary** flips to textarea; **Write your own**
      surfaces when no summary exists yet (so you don't have to run Ollama
      first)
- [ ] **Regenerate** button replaces summary + todos via Ollama
- [ ] Todos: click text to edit; **×** to delete; **Add a todo…** input
      with Enter to add

**Notes search (M2c)**
- [ ] Type a query in the header search box → list view replaces with
      hits (debounced 200 ms). Clearing the box restores the meeting list.
- [ ] Active sidebar filter constrains the search ("Search notes in
      Acme Corp…")
- [ ] Hit cards show meeting title + client tag + date + "m:ss" + speaker
      hint, with the matched substring highlighted (`<mark>`)
- [ ] Click a hit → opens the meeting detail
- [ ] Try a query containing `%` and `_` — should match those literally
      (escaping is tested in storage; verify visually)

**M2a step 1 (mic capture)**
- [ ] On macOS, `pnpm tauri dev` → Start a meeting (no
      `MEETIOR_FIXTURE_TRANSCRIPT` set, no `MEETIOR_FIXTURE_AUDIO` set).
      First run prompts for microphone permission. Speak. Confirm
      backend logs show `mic capture starting` with the device name +
      sample rate. ASR is still a stub so no segments stream — but
      `cargo test --lib audio` passes here, so the chunking + resample
      math is verified
- [ ] With `MEETIOR_FIXTURE_AUDIO=path/to/test.wav` set, Start →
      backend logs show `MEETIOR_FIXTURE_AUDIO set — using fixture audio`
      and chunks are emitted at real-time pace. Any 16-bit / float WAV
      at any rate works (gets resampled + mixed to mono in software)
- [ ] `Info.plist` `NSMicrophoneUsageDescription` text reads correctly
      in the macOS prompt

**End-to-end fixture flow**
- [ ] `export MEETIOR_FIXTURE_TRANSCRIPT="$PWD/examples/fixture-transcript.json"`
      → start with a client → segments stream → take a few notes →
      stop → row shows the tag → open → edit title, summary, todos →
      Generate / Regenerate summary → todos render → notes show in
      detail view read-only

`pnpm tauri build` is the pre-release sanity check; I never ran it.

## Things still not shipped

Backend is ready for these — UI is the missing piece:

- [ ] **Client edit / rename** in the sidebar (`update_client` exists). The
      detail view lets you reassign a meeting's client but you can't yet
      rename a client without touching the DB.
- [ ] **Client delete** in the sidebar (`delete_client` exists; cascade
      already nullifies meeting.client_id so history is preserved).
- [ ] **Date-range filter chips** alongside the client filter in the
      sidebar — backend `list_meetings` doesn't yet accept a date filter
      either; both sides need landing together.
- [ ] **Audio import** (`importer.rs` with symphonia + `import_meeting`
      Tauri command). Practical value depends on ASR being real, so this
      is naturally sequenced after M2a step 3.
- [ ] **Filter chips for date range** (PRODUCT.md → "Filter UI: client +
      date range chips, SQLite indexes only"). Client filter ships in
      step 3; date range is still pending.
- [ ] **Cross-client digest view + `digest()` on summarizer** (PRODUCT.md
      M2b roadmap).
- [ ] **Audio import flow** (`importer.rs` with symphonia + `import_meeting`
      command + UI button; reject .mp4/.mov for v1).
- [x] **Crash-recovery on launch:** shipped at
      `Store::recover_orphaned_recordings`. Any meeting in
      `recording`/`transcribing`/`summarizing` is flipped to `done` with
      `ended_at = now` (or its existing value via COALESCE). Segments
      and notes preserved. 4 unit tests cover the cases.
- [x] **Vite-only `invoke` stub** for UI iteration without rebuilding Rust
      — shipped at `src/lib/api-stub.ts`. `pnpm dev` auto-routes to it
      when there's no Tauri shell. State persists in `localStorage`;
      `window.meetiorResetStub()` wipes it.

## Things to watch, not blocking

- **Storage tests use real temp files**, not `:memory:` — see
  `storage.rs::tests::store()`. r2d2 + SqliteConnectionManager makes
  shared `:memory:` awkward. Files leak until process exit; fine for now,
  reconsider if the count grows.
- **No DB migration version table.** `ensure_column()` handles additive
  forward-compat. Destructive changes (rename/drop, type changes) will
  need a real migration runner.
- **Fixture transcripts have no `speaker_source` field.** Parses as
  `Unknown` via `#[serde(default)]`. Real ASR (M2a) must set this from
  the capture source so the "you vs them" hint is meaningful.
- **`speaker_hint` on notes is always `Unknown`** today — the frontend
  has no audio source data to infer from. Tag rendering already handles
  `you` / `them`, so once M2a wires up a "recent dominant source" hint,
  the existing UI just works.
- **Concurrent note saves** are sequential by design (the composer awaits
  before unlocking). If users find that laggy, swap to optimistic-with-
  reconcile, but submillisecond local SQLite shouldn't need it.

## Performance / profiling

CLAUDE.md notes a perf budget (idle CPU < 15 % on M-series, transcription
latency < 2 s p95, cold start < 1 s). Nothing in M2b should regress this,
but spot-check via Instruments or `cargo flamegraph` once M2a lands and
the audio + ASR loops actually run.
