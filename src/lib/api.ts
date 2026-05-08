import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type MeetingStatus = "idle" | "recording" | "transcribing" | "summarizing" | "done" | "error";

export interface Meeting {
  id: string;
  title: string;
  started_at: string;
  ended_at: string | null;
  status: MeetingStatus;
}

export interface TranscriptSegment {
  start_ms: number;
  end_ms: number;
  speaker: string | null;
  text: string;
}

export interface MeetingDetail extends Meeting {
  segments: TranscriptSegment[];
  summary: string | null;
  todos: Todo[];
}

export interface Todo {
  id: string;
  text: string;
  done: boolean;
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
