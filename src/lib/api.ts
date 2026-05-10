import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export interface MeetingDetail extends Meeting {
  client: Client | null;
  segments: TranscriptSegment[];
  notes: Note[];
  summary: string | null;
  todos: Todo[];
}

export const api = {
  startMeeting: (title?: string) => invoke<Meeting>("start_meeting", { title }),
  stopMeeting: (id: string) => invoke<Meeting>("stop_meeting", { id }),
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
  getMeeting: (id: string) => invoke<MeetingDetail>("get_meeting", { id }),
  summarize: (id: string) => invoke<MeetingDetail>("summarize_meeting", { id }),
  toggleTodo: (meetingId: string, todoId: string) =>
    invoke<Todo>("toggle_todo", { meetingId, todoId }),
};

export const events = {
  onTranscriptSegment: (cb: (s: TranscriptSegment & { meeting_id: string }) => void): Promise<UnlistenFn> =>
    listen("meetior://transcript-segment", (e) => cb(e.payload as TranscriptSegment & { meeting_id: string })),
  onMeetingStatus: (cb: (m: Meeting) => void): Promise<UnlistenFn> =>
    listen("meetior://meeting-status", (e) => cb(e.payload as Meeting)),
};
