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
M2b (clients, notes, edits) and M2c (notes search) are merged. Manual
macOS verification is the biggest open item before M2a starts — see
the checklist below. Everything ships green on `pnpm check`,
`cargo clippy`, and 36 unit tests.

To resume: tell the agent **"continue M2a step N"** (replace N) and
the plan doc is detailed enough that a cold session can execute it.

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
