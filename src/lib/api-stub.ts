/**
 * Browser-only stub for the Tauri IPC layer.
 *
 * Activates when `pnpm dev` runs Vite without the Tauri shell wrapping it
 * (i.e. `window.__TAURI_INTERNALS__` is undefined and we're in dev mode).
 * Lets you click through the M2b/M2c UI in any browser without macOS or
 * an Ollama install. State persists to `localStorage` so reloads behave.
 *
 * The stub implements every `commands::*` Tauri command in `src-tauri/`
 * and the two events the UI listens for. Behavior should match the real
 * backend closely enough that UI bugs surface here instead of waiting
 * for macOS access:
 *   - `start_meeting` kicks off a fixture transcript stream that mimics
 *     the `MEETIOR_FIXTURE_TRANSCRIPT` codepath in `commands::start_meeting`.
 *   - Color auto-assign mirrors `ClientColor::next_unused`.
 *   - `delete_client` nullifies meeting `client_id` (FK ON DELETE SET NULL).
 *   - `search_notes` does the same case-insensitive substring + client
 *     filter as `Store::search_notes`, including LIKE-wildcard escaping.
 *
 * Not a substitute for macOS verification — Tauri IPC, ScreenCaptureKit,
 * and ONNX runtime aren't simulated. But for "does the UI hang together"
 * questions this is enough.
 */

import type {
  Client,
  ClientColor,
  Meeting,
  MeetingDetail,
  Note,
  NoteHit,
  SpeakerHint,
  Todo,
  TranscriptSegment,
} from "./api";

// ---------------------------------------------------------------------------
// State (persisted to localStorage so refresh works)
// ---------------------------------------------------------------------------

interface State {
  clients: Client[];
  meetings: Meeting[];
  segments: Record<string, TranscriptSegment[]>;
  notes: Record<string, Note[]>;
  summaries: Record<string, string>;
  todos: Record<string, Todo[]>;
}

const STORAGE_KEY = "meetior-stub-state-v1";

function emptyState(): State {
  return {
    clients: [],
    meetings: [],
    segments: {},
    notes: {},
    summaries: {},
    todos: {},
  };
}

function loadState(): State {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...emptyState(), ...JSON.parse(raw) };
  } catch {
    /* corrupted state – start fresh */
  }
  return emptyState();
}

function save() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* quota exceeded – ignore */
  }
}

const state: State = loadState();

// Recording state lives in memory only — a refresh ends any active recording.
let recording: { meetingId: string; intervalId: number; startedAt: number } | null = null;

// ---------------------------------------------------------------------------
// Event emitter
// ---------------------------------------------------------------------------

type EventListener = (e: { payload: unknown }) => void;
const listeners: Map<string, Set<EventListener>> = new Map();

function emit(event: string, payload: unknown) {
  const subs = listeners.get(event);
  if (!subs) return;
  for (const cb of subs) cb({ payload });
}

export async function stubListen(
  event: string,
  cb: EventListener,
): Promise<() => void> {
  let set = listeners.get(event);
  if (!set) {
    set = new Set();
    listeners.set(event, set);
  }
  set.add(cb);
  return () => {
    listeners.get(event)?.delete(cb);
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const PALETTE: ClientColor[] = [
  "red",
  "orange",
  "amber",
  "green",
  "teal",
  "blue",
  "indigo",
  "purple",
  "gray",
];

function nextColor(): ClientColor {
  const used = new Set(state.clients.map((c) => c.color));
  return PALETTE.find((c) => !used.has(c)) ?? "red";
}

function uuid(): string {
  return "stub-" + Math.random().toString(36).slice(2, 11);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function getMeeting(id: string): Meeting {
  const m = state.meetings.find((x) => x.id === id);
  if (!m) throw new Error(`stub: meeting ${id} not found`);
  return m;
}

function emitStatus(m: Meeting) {
  emit("meetior://meeting-status", m);
}

// Fixture transcript replayed when a meeting is started.
const FIXTURE_TRANSCRIPT: TranscriptSegment[] = [
  { start_ms: 500, end_ms: 4200, speaker: "Alice", speaker_source: "unknown",
    text: "Okay let's kick off. Quick standup, then we'll talk about the launch." },
  { start_ms: 4500, end_ms: 9800, speaker: "Bob", speaker_source: "unknown",
    text: "Yesterday I finished the auth migration. Today I'll start on the billing webhook. Blocked on getting the Stripe test keys from finance." },
  { start_ms: 10200, end_ms: 14600, speaker: "Alice", speaker_source: "unknown",
    text: "I'll ping Karen for the Stripe keys this afternoon. Bob, can you draft the webhook spec by EOD tomorrow?" },
  { start_ms: 15000, end_ms: 17000, speaker: "Bob", speaker_source: "unknown",
    text: "Yep, I'll have it ready." },
  { start_ms: 17500, end_ms: 23800, speaker: "Carol", speaker_source: "unknown",
    text: "On the launch — marketing wants the blog post by next Wednesday. I need final copy from product by Monday so we have time to edit." },
  { start_ms: 24200, end_ms: 27500, speaker: "Alice", speaker_source: "unknown",
    text: "I'll own the copy. Monday morning works. Anything else? Okay, thanks everyone." },
];

// Match `Store::search_notes` LIKE-escape so wildcards are literal.
function caseInsensitiveSubstring(haystack: string, needle: string): boolean {
  return haystack.toLowerCase().includes(needle.toLowerCase());
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

type Args = Record<string, unknown>;
type Handler = (args: Args) => Promise<unknown>;

const handlers: Record<string, Handler> = {
  // -------- clients ---------------------------------------------------

  list_clients: async () =>
    [...state.clients].sort((a, b) =>
      a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
    ),

  create_client: async ({ name }) => {
    const trimmed = String(name ?? "").trim();
    if (!trimmed) throw "client name cannot be empty";
    const c: Client = {
      id: uuid(),
      name: trimmed,
      color: nextColor(),
      created_at: new Date().toISOString(),
    };
    state.clients.push(c);
    save();
    return c;
  },

  update_client: async ({ id, name, color }) => {
    const c = state.clients.find((x) => x.id === id);
    if (!c) throw `client ${id} not found`;
    const trimmed = String(name ?? "").trim();
    if (!trimmed) throw "client name cannot be empty";
    if (!PALETTE.includes(color as ClientColor)) throw `unknown color: ${color}`;
    c.name = trimmed;
    c.color = color as ClientColor;
    save();
    return c;
  },

  delete_client: async ({ id }) => {
    const before = state.clients.length;
    state.clients = state.clients.filter((c) => c.id !== id);
    if (state.clients.length === before) throw `client ${id} not found`;
    // Cascade: meetings referencing this client become unassigned.
    for (const m of state.meetings) {
      if (m.client_id === id) m.client_id = null;
    }
    save();
    // Refresh list views.
    for (const m of state.meetings) emitStatus(m);
    return null;
  },

  // -------- meetings --------------------------------------------------

  list_meetings: async () =>
    [...state.meetings].sort(
      (a, b) =>
        new Date(b.started_at).getTime() - new Date(a.started_at).getTime(),
    ),

  get_meeting: async ({ id }): Promise<MeetingDetail> => {
    const m = getMeeting(String(id));
    const client = m.client_id
      ? state.clients.find((c) => c.id === m.client_id) ?? null
      : null;
    return {
      ...m,
      client,
      segments: state.segments[m.id] ?? [],
      notes: state.notes[m.id] ?? [],
      summary: state.summaries[m.id] ?? null,
      todos: state.todos[m.id] ?? [],
    };
  },

  start_meeting: async ({ title, clientId }) => {
    if (recording) throw "a meeting is already recording";
    const t = String(title ?? "").trim() ||
      `Meeting ${new Date().toLocaleString()}`;
    const now = new Date();
    const m: Meeting = {
      id: uuid(),
      title: t,
      started_at: now.toISOString(),
      ended_at: null,
      status: "recording",
      client_id: (clientId as string | null) ?? null,
      source: "live",
    };
    state.meetings.push(m);
    state.segments[m.id] = [];
    save();
    emitStatus(m);

    // Replay the fixture transcript at its real timestamps. Lightweight —
    // a single setInterval that watches elapsed time and emits whichever
    // segments have come due, mirroring `asr::spawn_fixture_stream`.
    const startedAt = Date.now();
    let nextIdx = 0;
    const intervalId = window.setInterval(() => {
      if (!recording) return;
      const elapsed = Date.now() - startedAt;
      while (
        nextIdx < FIXTURE_TRANSCRIPT.length &&
        FIXTURE_TRANSCRIPT[nextIdx].start_ms <= elapsed
      ) {
        const seg = FIXTURE_TRANSCRIPT[nextIdx++];
        state.segments[m.id]!.push(seg);
        emit("meetior://transcript-segment", { meeting_id: m.id, ...seg });
      }
      if (nextIdx >= FIXTURE_TRANSCRIPT.length) {
        // Fixture exhausted — leave the meeting recording until the user
        // explicitly stops it, matching the real fixture behavior.
      }
    }, 250);
    recording = { meetingId: m.id, intervalId, startedAt };
    return m;
  },

  stop_meeting: async ({ id }) => {
    if (!recording) throw "no active meeting";
    if (recording.meetingId !== id) throw "meeting id does not match";
    window.clearInterval(recording.intervalId);
    recording = null;
    const m = getMeeting(String(id));
    m.status = "done";
    m.ended_at = new Date().toISOString();
    save();
    emitStatus(m);
    return m;
  },

  update_meeting_title: async ({ id, title }) => {
    const m = getMeeting(String(id));
    const trimmed = String(title ?? "").trim();
    if (!trimmed) throw "title cannot be empty";
    m.title = trimmed;
    save();
    emitStatus(m);
    return m;
  },

  set_meeting_client: async ({ meetingId, clientId }) => {
    const m = getMeeting(String(meetingId));
    if (clientId && !state.clients.find((c) => c.id === clientId)) {
      throw `client ${clientId} not found`;
    }
    m.client_id = (clientId as string | null) ?? null;
    save();
    emitStatus(m);
    return m;
  },

  // -------- notes -----------------------------------------------------

  append_note: async ({ meetingId, text, speakerHint }) => {
    const m = getMeeting(String(meetingId));
    const trimmed = String(text ?? "").trim();
    if (!trimmed) throw "note text cannot be empty";
    const list = state.notes[m.id] ?? (state.notes[m.id] = []);
    const elapsed = Math.max(0, Date.now() - new Date(m.started_at).getTime());
    const note: Note = {
      idx: list.length,
      t_ms: elapsed,
      speaker_hint: (speakerHint as SpeakerHint | undefined) ?? "unknown",
      text: trimmed,
    };
    list.push(note);
    save();
    return note;
  },

  search_notes: async ({ query, clientId }) => {
    const q = String(query ?? "").trim();
    if (!q) return [] as NoteHit[];
    const cid = (clientId as string | null) ?? null;
    const hits: NoteHit[] = [];
    for (const m of state.meetings) {
      if (cid && m.client_id !== cid) continue;
      const ns = state.notes[m.id] ?? [];
      for (const n of ns) {
        if (caseInsensitiveSubstring(n.text, q)) {
          hits.push({
            meeting_id: m.id,
            meeting_title: m.title,
            meeting_started_at: m.started_at,
            client_id: m.client_id,
            idx: n.idx,
            t_ms: n.t_ms,
            speaker_hint: n.speaker_hint,
            text: n.text,
          });
          if (hits.length >= 200) break;
        }
      }
      if (hits.length >= 200) break;
    }
    hits.sort((a, b) => {
      const t = new Date(b.meeting_started_at).getTime() - new Date(a.meeting_started_at).getTime();
      return t !== 0 ? t : a.idx - b.idx;
    });
    return hits;
  },

  // -------- summary + todos -------------------------------------------

  summarize_meeting: async ({ id }) => {
    const m = getMeeting(String(id));
    if (!(state.segments[m.id]?.length)) throw "no transcript to summarize";
    m.status = "summarizing";
    emitStatus(m);
    await sleep(800); // pretend Ollama is thinking
    state.summaries[m.id] =
      "Discussed the launch plan and outstanding blockers. The team agreed on owners for the Stripe webhook spec, blog post copy, and Stripe test keys, with all deliverables due in the next week.";
    state.todos[m.id] = [
      { id: uuid(), text: "Draft webhook spec by EOD tomorrow", done: false },
      { id: uuid(), text: "Get Stripe test keys from Karen", done: false },
      { id: uuid(), text: "Send blog post copy by Monday morning", done: false },
    ];
    m.status = "done";
    save();
    emitStatus(m);
    // The real backend re-queries; we just return the detail.
    return await handlers.get_meeting!({ id });
  },

  update_summary: async ({ meetingId, summary }) => {
    const m = getMeeting(String(meetingId));
    state.summaries[m.id] = String(summary ?? "");
    save();
    return null;
  },

  add_todo: async ({ meetingId, text }) => {
    const m = getMeeting(String(meetingId));
    const trimmed = String(text ?? "").trim();
    if (!trimmed) throw "todo text cannot be empty";
    const t: Todo = { id: uuid(), text: trimmed, done: false };
    (state.todos[m.id] ??= []).push(t);
    save();
    return t;
  },

  update_todo_text: async ({ meetingId, todoId, text }) => {
    const list = state.todos[String(meetingId)] ?? [];
    const t = list.find((x) => x.id === todoId);
    if (!t) throw `todo ${todoId} not found`;
    const trimmed = String(text ?? "").trim();
    if (!trimmed) throw "todo text cannot be empty";
    t.text = trimmed;
    save();
    return t;
  },

  delete_todo: async ({ meetingId, todoId }) => {
    const key = String(meetingId);
    const list = state.todos[key] ?? [];
    const before = list.length;
    state.todos[key] = list.filter((t) => t.id !== todoId);
    if (state.todos[key].length === before) throw `todo ${todoId} not found`;
    save();
    return null;
  },

  toggle_todo: async ({ meetingId, todoId }) => {
    const list = state.todos[String(meetingId)] ?? [];
    const t = list.find((x) => x.id === todoId);
    if (!t) throw `todo ${todoId} not found`;
    t.done = !t.done;
    save();
    return t;
  },
};

// ---------------------------------------------------------------------------
// Public entry points (mirror `@tauri-apps/api/core` and `/event`)
// ---------------------------------------------------------------------------

export async function stubInvoke<T>(cmd: string, args?: Args): Promise<T> {
  const handler = handlers[cmd];
  if (!handler) {
    throw new Error(
      `stub: unknown command "${cmd}". Add a handler in src/lib/api-stub.ts.`,
    );
  }
  // A small fake IPC delay so async UI states (Searching…, Summarizing…)
  // are observable instead of resolving same-tick.
  await sleep(20);
  return (await handler(args ?? {})) as T;
}

/** Reset the stub's in-memory + persisted state. Exposed on `window` for
 *  console-level "wipe and start over" calls. */
export function resetStub() {
  for (const k of Object.keys(state) as (keyof State)[]) {
    (state as any)[k] = (emptyState() as any)[k];
  }
  if (recording) {
    window.clearInterval(recording.intervalId);
    recording = null;
  }
  save();
}
