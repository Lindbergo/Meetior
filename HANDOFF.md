# HANDOFF

Things I couldn't do from the agent's Linux dev environment, or
deliberately deferred — running list, prune as you check items off.

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
- [ ] **Filter chips for date range** (PRODUCT.md → "Filter UI: client +
      date range chips, SQLite indexes only"). Client filter ships in
      step 3; date range is still pending.
- [ ] **Cross-client digest view + `digest()` on summarizer** (PRODUCT.md
      M2b roadmap).
- [ ] **Audio import flow** (`importer.rs` with symphonia + `import_meeting`
      command + UI button; reject .mp4/.mov for v1).
- [ ] **Crash-recovery on launch:** any meeting in `recording` status →
      mark `done`, keep saved segments and notes (PRODUCT.md M2b roadmap;
      currently a recording-status meeting after a crash will still be
      shown as "active").
- [ ] **Vite-only `invoke` stub** for UI iteration without rebuilding Rust
      (CLAUDE.md describes the pattern; left it for whoever wants the
      faster loop).

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
