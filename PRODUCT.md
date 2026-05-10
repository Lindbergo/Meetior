# PRODUCT.md — Meetior

The product brief. Read this for **what** we're building and **what we're
not**. Read `CLAUDE.md` for **how** we're building it.

Update this file whenever a product decision changes. Decisions captured
here override speculative work elsewhere in the repo.

---

## Mission

A macOS-only meeting assistant for Customer Success workflows. You're in a
Google Meet with a client, you click record, and when the meeting ends the
app gives you a transcript, a summary, and the action items — tagged to
the client, ready to follow up on. Everything happens on your machine.
No cloud, no telemetry, no account.

---

## In scope for v1

- **Live recording** of a Google Meet (or anything making sound on your Mac).
- **Audio file import** — drop a `.wav`, `.mp3`, `.m4a`, or `.flac` and
  treat it the same as a live meeting. Doubles as our integration-test
  fixture mechanism.
- **Streaming transcription** with Parakeet TDT v3 (local, ONNX).
- **Local summary + todos** via Ollama on `127.0.0.1:11434`.
- **Clients** as first-class entities. Name + color, picked from a list
  when starting/importing a meeting. Meetings without a client allowed
  ("Unassigned").
- **Per-meeting and global todos.** Global view filterable by client.
- **Live notes pane** during recording — plain text with a timestamp
  recorded on each newline, plus a speaker hint ("You" vs "Them") derived
  from whether the audio came from the mic or system source.
- **Edit affordances** after the fact: meeting title, client, AI summary,
  todos. **Transcript is read-only.**
- **Find meetings** by filtering: client + date range. No search yet.
- **Cross-client digest views**: e.g. "last 7 days, all clients" rolling
  summary. Generated on demand via Ollama.
- **Forever retention.** Manual delete only.

## Out of scope for v1

These are good ideas, deliberately deferred to keep v1 shippable.

- Auto-detection of meeting start. v1 is manual-start only.
- Speaker diarization beyond mic-vs-system source. No "Alice / Bob" labels.
- Full-text or semantic search across meetings.
- Editing the transcript (typo fixes, redactions).
- Settings UI: retention policy, model picker, audio source toggles, hotkey.
- Markdown formatting in the notes pane.
- Menu-bar presence; the app is a dock app you open.
- Calendar integration, browser extension, mobile companion.
- Export to other task managers (Things, Reminders, Notion, etc.).
- Custom CRM-style fields on clients. Just name + color.
- Update checks, telemetry, crash reports — see Privacy below.

---

## Privacy line (durable)

The app may make network calls to:

1. **`127.0.0.1:11434`** — local Ollama for summaries and digests.
2. **A single Parakeet model URL** on first run, to download the ONNX
   artifacts to `~/Library/Application Support/app.meetior/meetior/models/`.
   The URL is committed in code; the download is checksum-verified.

It MUST NOT make any other network call. Specifically:
no usage analytics, no error reporting, no version/update checks,
no third-party LLM APIs, no font/CSS CDNs, no autocomplete services.

If a future feature needs network, it gets explicit user opt-in **and** a
line item here.

---

## Data model

```
Client
  id                uuid
  name              text
  color             text (hex)
  created_at        timestamptz

Meeting
  id                uuid
  client_id         uuid? → Client.id   (nullable: "Unassigned")
  title             text
  started_at        timestamptz
  ended_at          timestamptz?
  status            enum(recording, transcribing, summarizing, done, error)
  source            enum(live, imported)

TranscriptSegment   (continuously autosaved)
  meeting_id        uuid → Meeting.id
  idx               int
  start_ms          int
  end_ms            int
  speaker_source    enum(mic, system, unknown)   ← cheap "you vs them"
  text              text

Note                (continuously autosaved during recording)
  meeting_id        uuid → Meeting.id
  idx               int
  t_ms              int                  -- meeting clock when line was finalized
  speaker_hint      enum(you, them, unknown)
  text              text

Summary
  meeting_id        uuid PRIMARY KEY → Meeting.id
  summary           text                 -- editable
  created_at        timestamptz
  edited_at         timestamptz?

Todo
  id                uuid
  meeting_id        uuid → Meeting.id
  text              text                 -- editable
  done              bool
  source            enum(ai, manual)     -- so the UI can show provenance
  created_at        timestamptz
```

Notes:
- `speaker_source` on segments is derived from which capture stream the
  audio came from. We mix into one mono stream for ASR but keep the source
  tag per-chunk and propagate it to the segment that overlaps that chunk
  most. Real diarization (Pyannote) is M3+.
- `speaker_hint` on a Note is filled by looking at which source has been
  louder in the last ~2 s when the user pressed Enter.
- `Summary.edited_at` lets us show "edited" in the UI without a full audit
  log.
- We deliberately do not have a `tags` table or freeform tagging — clients
  are the only categorization for v1.

---

## Core user journeys

### Live meeting (the main flow)

1. You open the app. Dock window shows a sidebar of clients on the left,
   recent meetings on the right.
2. Click **Start meeting**. A dialog asks for client (typeahead over
   existing or "+ New", or "No client") and an optional title.
3. The app opens the **active meeting view**: live transcript on the left,
   notes pane on the right, big Stop button. Recording starts immediately.
4. Words stream in. You type notes. Each newline anchors a timestamp +
   "You" / "Them" hint.
5. Click **Stop**. App marks status `summarizing` and calls Ollama.
6. If Ollama is up: summary + todos appear under the transcript. You can
   edit any of them, add manual todos, change the client, rename.
7. If Ollama is down: clear error, **Retry** button. Transcript and notes
   are already saved; nothing is lost.

### Import a recording

Same as above, except step 1–3 are: click **Import**, pick file, pick
client + title, the file is decoded and streamed through the same ASR
pipeline. Status goes straight to `transcribing`. No notes pane (no live
session).

### Find a past meeting

Sidebar → click a client (or "All" / "Unassigned"). Main area shows
meetings for that filter. Date-range chips at the top: today / this week
/ this month / custom. Click a row to open detail.

### Cross-client digest

Sidebar → "All clients" → "Generate digest" button with a date range.
Calls Ollama with the summaries (not full transcripts) of meetings in
that range and produces a rolling status snapshot. On-demand only;
nothing is auto-generated.

---

## UX layout (v1)

```
┌─ Meetior ─────────────────────────────────────────────────┐
│  ┌──────────────┐  ┌────────────────────────────────────┐ │
│  │ All clients  │  │ Filter: [client] [date range]      │ │
│  │ Unassigned   │  ├────────────────────────────────────┤ │
│  │ ─────        │  │ Acme Corp · Mon Mar 4 · 28 min  ›  │ │
│  │ • Acme       │  │ Foo Inc · Mon Mar 4 · 12 min    ›  │ │
│  │ • Foo Inc    │  │ ...                                │ │
│  │ • Bar LLC    │  │                                    │ │
│  │ + New client │  │                                    │ │
│  ├──────────────┤  │                                    │ │
│  │ All todos    │  │                                    │ │
│  │ Digest       │  │                                    │ │
│  └──────────────┘  └────────────────────────────────────┘ │
│              [ Start meeting ]  [ Import ]                │
└───────────────────────────────────────────────────────────┘
```

Three views, no other navigation:
1. **List** — sidebar + meeting list (above).
2. **Active** — live transcript + notes pane + Stop button.
3. **Detail** — meeting metadata, summary (editable), todos (editable),
   transcript (read-only).

The "All todos" and "Digest" sidebar items are both list-views with the
same chrome as the meeting list — no new layouts.

---

## Behavior boundaries

- **One active meeting at a time.** Trying to start a second while one is
  recording is an error.
- **Continuous autosave.** Each transcript segment is written immediately
  on receipt. Notes flush on each newline and at most every 2 s. If the
  app is killed mid-meeting, you keep everything except the last in-flight
  ~200 ms of audio.
- **Ollama down = honest failure.** On Stop, we show an error + Retry; we
  do not queue or silently skip. Transcript and notes survive; the
  meeting status becomes `done` (transcribed) without a summary, and the
  detail view offers **Generate summary & todos** to retry.
- **Crash recovery on next launch.** If a meeting is found in `recording`
  status, mark it `done` (since the audio source is gone), keep all saved
  segments and notes. No half-states linger.
- **Mic permission denied** at start = surface the OS dialog; if user
  refuses, capture system audio only and proceed; meetings get
  `speaker_source = system` for everything and notes default `speaker_hint
  = unknown`.
- **No client** is a valid bucket, not a missing value. The picker has
  three modes: pick existing, create new, no client.

---

## Module boundaries (technical implications)

These adjust the technical layout in `CLAUDE.md`:

- **`storage.rs`** owns *all* schema. Adds `clients`, `notes` tables. No
  new module for clients yet — a `Client` struct + queries on `Store` is
  enough for "just a name + color." Promote to `clients.rs` only if logic
  outgrows SQL.
- **`audio.rs`** keeps a *separate* mic and system-audio stream up to the
  mixing point. Each chunk carries a `source: SpeakerSource` tag.
  Mixing into mono for ASR happens just before handoff to `asr.rs`; the
  source tag is forwarded so segments inherit a `speaker_source`.
- **`asr.rs`** is unchanged in shape; it just preserves the source tag.
- **`importer.rs`** (NEW) — decodes `.wav` / `.mp3` / `.m4a` / `.flac` via
  `symphonia` and emits the same `AudioChunk` stream as live capture, with
  `speaker_source = unknown` (we don't try to split). Wired into a second
  Tauri command `import_meeting(path, client_id?, title?)`.
- **`notes.rs`** is *not* a module. Note storage + the timestamp/speaker-hint
  derivation live in `storage.rs` + a tiny helper alongside the active-meeting
  command. A module is overkill.
- **`summarizer.rs`** gets a second method `digest(meetings: &[Summary],
  range: DateRange)` for cross-client rolling summaries. Same Ollama
  client, different prompt.
- **`commands.rs`** grows: `create_client`, `list_clients`, `import_meeting`,
  `update_meeting`, `update_summary`, `add_todo`, `delete_todo`,
  `update_todo_text`, `append_note`, `generate_digest`. Keep them all
  thin.

---

## Decisions

These were open questions resolved during planning. Record changes here
with a date if any decision is reopened.

### Parakeet model hosting
GitHub release attachment on this repo, tagged
`parakeet-tdt-0.6b-v3-onnx`. ~600 MB — comfortably under the 2 GB/file
limit, CDN-backed, anonymous public download, no extra infra or
third-party platform in the privacy story. SHA-256 of each artifact is
committed in code; first-run downloads to app-data and verifies before
loading. Bootstrap is a one-time export on a Linux/GPU box via
`scripts/export-parakeet.py`, then upload + tag.

### Client color palette
Curated 9 swatches: red, orange, amber, green, teal, blue, indigo,
purple, gray. New clients get the next unused color auto-assigned; the
user can change it from a swatch row in the client editor. No free hex
picker — non-designers produce clashing palettes and the picker adds
friction to a frequent action. Same pattern as Linear / Things.

### Video file import (v1)
Rejected up front. The Import file picker filters to audio extensions
(`.wav` / `.mp3` / `.m4a` / `.flac`); drag-and-drop of `.mp4` / `.mov` /
`.webm` shows a friendly error: "Video files aren't supported yet.
Convert to `.m4a` first, or wait for v1.1." Accepting then failing at
symphonia decode produces an opaque error and worse UX.

**Planned for v1.1:** ffmpeg-based audio extraction so video files just
work. Single dependency, no new architecture.

---

## Open questions (decide before they block work)

- **Digest prompt.** What's the actual system prompt for "rolling status
  per client"? Best iterated on with real meeting data — not worth
  designing in the abstract. Park until M2b gets to the digest view.
