import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { stubInvoke, stubListen, resetStub } from "./api-stub";

// Pick the IPC layer once at module load. In `pnpm dev` (browser, no Tauri
// shell) we route to the in-memory stub so UI iteration works without
// macOS. In the real Tauri app `__TAURI_INTERNALS__` is set and the real
// `invoke` / `listen` are used. CLAUDE.md → "UI-only iteration" describes
// the rationale.
const useStub =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  !(window as any).__TAURI_INTERNALS__;

const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  useStub ? stubInvoke<T>(cmd, args) : tauriInvoke<T>(cmd, args);

const listen = <T>(
  event: string,
  cb: (e: { payload: T }) => void,
): Promise<UnlistenFn> =>
  useStub
    ? (stubListen(event, cb as (e: { payload: unknown }) => void) as Promise<UnlistenFn>)
    : tauriListen<T>(event, cb);

if (useStub && typeof window !== "undefined") {
  // eslint-disable-next-line no-console
  console.info(
    "[meetior] Tauri shell not detected — using in-memory stub. " +
      "Run `window.meetiorResetStub()` to wipe state.",
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).meetiorResetStub = resetStub;
}

export type MeetingStatus = "idle" | "recording" | "transcribing" | "summarizing" | "done" | "error";

export type MeetingSource = "live" | "imported";

export type SpeakerSource = "mic" | "system" | "unknown";

export type SpeakerHint = "you" | "them" | "unknown";

export type ClientColor =
  | "red" | "orange" | "amber" | "green" | "teal"
  | "blue" | "indigo" | "purple" | "gray";

/** Curated palette mapped to hex values. Order matches the auto-assign order
 *  in `ClientColor::PALETTE` in the Rust backend. */
export const CLIENT_COLOR_HEX: Record<ClientColor, string> = {
  red: "#ef4444",
  orange: "#f97316",
  amber: "#f59e0b",
  green: "#22c55e",
  teal: "#14b8a6",
  blue: "#3b82f6",
  indigo: "#6366f1",
  purple: "#a855f7",
  gray: "#6b7280",
};

export interface Client {
  id: string;
  name: string;
  color: ClientColor;
  created_at: string;
}

export interface Meeting {
  id: string;
  title: string;
  started_at: string;
  ended_at: string | null;
  status: MeetingStatus;
  client_id: string | null;
  source: MeetingSource;
}

export interface TranscriptSegment {
  start_ms: number;
  end_ms: number;
  speaker: string | null;
  speaker_source: SpeakerSource;
  text: string;
}

export interface Note {
  idx: number;
  t_ms: number;
  speaker_hint: SpeakerHint;
  text: string;
}

export interface Todo {
  id: string;
  text: string;
  done: boolean;
}

export interface NoteHit {
  meeting_id: string;
  meeting_title: string;
  meeting_started_at: string;
  client_id: string | null;
  idx: number;
  t_ms: number;
  speaker_hint: SpeakerHint;
  text: string;
}

export interface MeetingDetail extends Meeting {
  client: Client | null;
  segments: TranscriptSegment[];
  notes: Note[];
  summary: string | null;
  todos: Todo[];
}

export const api = {
  // meetings
  startMeeting: (title?: string, clientId?: string | null) =>
    invoke<Meeting>("start_meeting", { title, clientId }),
  stopMeeting: (id: string) => invoke<Meeting>("stop_meeting", { id }),
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
  getMeeting: (id: string) => invoke<MeetingDetail>("get_meeting", { id }),
  summarize: (id: string) => invoke<MeetingDetail>("summarize_meeting", { id }),
  updateMeetingTitle: (id: string, title: string) =>
    invoke<Meeting>("update_meeting_title", { id, title }),
  setMeetingClient: (meetingId: string, clientId: string | null) =>
    invoke<Meeting>("set_meeting_client", { meetingId, clientId }),

  // clients
  createClient: (name: string) => invoke<Client>("create_client", { name }),
  listClients: () => invoke<Client[]>("list_clients"),
  updateClient: (id: string, name: string, color: ClientColor) =>
    invoke<Client>("update_client", { id, name, color }),
  deleteClient: (id: string) => invoke<void>("delete_client", { id }),

  // notes
  appendNote: (meetingId: string, text: string, speakerHint?: SpeakerHint) =>
    invoke<Note>("append_note", { meetingId, text, speakerHint }),
  searchNotes: (query: string, clientId: string | null) =>
    invoke<NoteHit[]>("search_notes", { query, clientId }),

  // summary + todos
  updateSummary: (meetingId: string, summary: string) =>
    invoke<void>("update_summary", { meetingId, summary }),
  addTodo: (meetingId: string, text: string) =>
    invoke<Todo>("add_todo", { meetingId, text }),
  updateTodoText: (meetingId: string, todoId: string, text: string) =>
    invoke<Todo>("update_todo_text", { meetingId, todoId, text }),
  deleteTodo: (meetingId: string, todoId: string) =>
    invoke<void>("delete_todo", { meetingId, todoId }),
  toggleTodo: (meetingId: string, todoId: string) =>
    invoke<Todo>("toggle_todo", { meetingId, todoId }),
};

export const events = {
  onTranscriptSegment: (cb: (s: TranscriptSegment & { meeting_id: string }) => void): Promise<UnlistenFn> =>
    listen("meetior://transcript-segment", (e) => cb(e.payload as TranscriptSegment & { meeting_id: string })),
  onMeetingStatus: (cb: (m: Meeting) => void): Promise<UnlistenFn> =>
    listen("meetior://meeting-status", (e) => cb(e.payload as Meeting)),
};
