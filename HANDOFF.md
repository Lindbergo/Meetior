# HANDOFF

Things I couldn't do from the agent's Linux dev environment, or
deliberately deferred — running list, prune as you check items off.

## Manual verification on macOS

I never opened the Tauri shell, so the UI from M2b step 3 is unverified
visually. Please `pnpm tauri dev` and confirm:

- [ ] Sidebar renders with the curated 9-color palette
- [ ] "+" in sidebar reveals input. **Enter** creates, **Esc** cancels,
      blur-on-empty cancels. Newly created client becomes active filter.
- [ ] "All meetings" / "Unassigned" / per-client filters narrow the list
      and counts match
- [ ] Start dialog: opens on **Start meeting**, **Esc** closes, backdrop
      click closes, **"+ New client…"** flow creates client *and* starts
      the meeting in one go
- [ ] Active sidebar filter pre-fills the start dialog's client picker
- [ ] Meeting list rows show the color dot + client name when assigned
- [ ] Pre-existing fixture flow still works:
      `export MEETIOR_FIXTURE_TRANSCRIPT="$PWD/examples/fixture-transcript.json"`
      → Start with a client → segments stream → Stop → row has the tag
      → Open → Generate summary & todos → renders

`pnpm tauri build` for a pre-release sanity check whenever you want;
I never ran it (it's "minutes" per CLAUDE.md and not part of the dev
loop).

## Things I deliberately didn't ship in M2b yet

Backend is ready for all of these — UI is the only missing piece:

- [ ] **Client edit / delete UI.** `update_client` and `delete_client`
      commands exist; `delete_client` already nullifies meeting.client_id
      so deleting a client just unassigns its meetings (history is
      preserved). Need a hover-revealed ✕ + a rename affordance in the
      sidebar.
- [ ] **Meeting edit affordances** in the detail view: title, client
      reassignment, summary text, add/edit/delete todos. Commands exist
      (`update_meeting_title`, `set_meeting_client`, `update_summary`,
      `add_todo`, `update_todo_text`, `delete_todo`). This is **step 5**
      in my plan.
- [ ] **Notes pane** in the active-meeting view. `append_note` command
      exists (computes `t_ms` server-side from `meeting.started_at`).
      This is **step 4**.
- [ ] **Vite-only `invoke` stub** for UI iteration without rebuilding
      Rust. CLAUDE.md describes the pattern but says don't commit it —
      I left it for whoever wants the faster loop.

## Things to keep an eye on, not blocking

- **Storage tests use real temp files**, not `:memory:` — see comment in
  `storage.rs::tests::store()`. r2d2 + SqliteConnectionManager makes
  shared `:memory:` awkward. Files leak until process exit; fine for
  the current count (~20), reconsider if it bloats.
- **No DB migration version table.** `ensure_column()` handles additive
  forward-compat. If we ever need destructive migrations (rename / drop
  column, type changes), we'll need a real migration runner.
- **Fixture transcripts have no `speaker_source` field.** Parses as
  `Unknown` via `#[serde(default)]`. Fine for the fake pipeline; real
  ASR (M2a) must set this from the capture source so the "you vs them"
  hint works.
- **`Speaker hint` on notes defaults to `Unknown`** in `append_note`
  unless the caller passes one. There's no source to infer it from
  until M2a lands real audio. The notes pane (step 4) will need either
  a manual you/them toggle or to wait for M2a.

## Performance / profiling

CLAUDE.md notes a perf budget (idle CPU < 15 % on M-series, transcription
latency < 2 s p95, cold start < 1 s). Nothing in M2b should regress this,
but you'll want to spot-check via Instruments or `cargo flamegraph` once
M2a lands and the audio + ASR loops actually run.
