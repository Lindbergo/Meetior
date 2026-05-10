<script lang="ts">
  import { onMount, tick } from "svelte";
  import {
    api,
    events,
    type Meeting,
    type Note,
    type SpeakerHint,
    type TranscriptSegment,
  } from "./api";

  interface Props { meeting: Meeting }
  let { meeting }: Props = $props();

  let segments = $state<TranscriptSegment[]>([]);
  let notes = $state<Note[]>([]);
  let draft = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);
  let notesScrollEl: HTMLDivElement | null = $state(null);
  let textareaEl: HTMLTextAreaElement | null = $state(null);

  onMount(() => {
    // Fetch any notes already saved (e.g. after a window reopen mid-recording).
    api.getMeeting(meeting.id).then((d) => { notes = d.notes; }).catch(() => {});

    const unlisten = events.onTranscriptSegment((s) => {
      if (s.meeting_id !== meeting.id) return;
      segments = [...segments, s];
    });
    return () => { unlisten.then((fn) => fn()); };
  });

  // Speaker-hint inference is a real feature: for now we don't have audio
  // source data on the frontend, so every flushed line is `unknown`. M2a
  // will pipe a hint in here once mic/system tagging exists.
  function inferHint(): SpeakerHint | undefined {
    return undefined;
  }

  async function flush(text: string) {
    const trimmed = text.trim();
    if (!trimmed || saving) return;
    saving = true;
    error = null;
    try {
      const note = await api.appendNote(meeting.id, trimmed, inferHint());
      notes = [...notes, note];
      // Scroll the saved-notes column to the new note.
      await tick();
      notesScrollEl?.scrollTo({ top: notesScrollEl.scrollHeight });
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  // Enter flushes the current draft. Shift+Enter (or Cmd/Alt+Enter)
  // inserts a literal newline if someone wants a multi-line note.
  async function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      const text = draft;
      draft = "";
      await flush(text);
      // Refocus is automatic; just defensive.
      textareaEl?.focus();
    }
  }

  function fmtTimestamp(ms: number): string {
    const total = Math.floor(ms / 1000);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function hintLabel(h: SpeakerHint): string | null {
    if (h === "you") return "you";
    if (h === "them") return "them";
    return null;
  }
</script>

<div class="active">
  <header>
    <h2>{meeting.title}</h2>
    <div class="muted">
      Started {new Date(meeting.started_at).toLocaleTimeString()} · {meeting.status}
    </div>
  </header>

  <div class="panes">
    <section class="pane transcript-pane">
      <div class="pane-header">Transcript</div>
      <div class="transcript">
        {#if segments.length === 0}
          <p class="muted">Listening… transcript will appear here.</p>
        {/if}
        {#each segments as s, i (i)}
          <div class="seg">
            <span class="ts muted">{(s.start_ms / 1000).toFixed(1)}s</span>
            {#if s.speaker}<span class="speaker">{s.speaker}:</span>{/if}
            <span>{s.text}</span>
          </div>
        {/each}
      </div>
    </section>

    <section class="pane notes-pane">
      <div class="pane-header">
        Notes
        <span class="hint muted">Enter to save · Shift+Enter for newline</span>
      </div>

      <div class="notes" bind:this={notesScrollEl}>
        {#if notes.length === 0}
          <p class="muted">Jot quick notes here. Each line is timestamped.</p>
        {/if}
        {#each notes as n (n.idx)}
          <div class="note">
            <div class="note-meta muted">
              <span class="ts">{fmtTimestamp(n.t_ms)}</span>
              {#if hintLabel(n.speaker_hint)}
                <span class="hint-tag" data-hint={n.speaker_hint}>{hintLabel(n.speaker_hint)}</span>
              {/if}
            </div>
            <div class="note-text">{n.text}</div>
          </div>
        {/each}
      </div>

      <div class="composer">
        <textarea
          bind:this={textareaEl}
          bind:value={draft}
          onkeydown={onKey}
          placeholder="Type a note and press Enter…"
          rows="2"
          disabled={saving}
        ></textarea>
        {#if error}<div class="error">{error}</div>{/if}
      </div>
    </section>
  </div>
</div>

<style>
  .active { display: flex; flex-direction: column; gap: 12px; height: 100%; }
  header h2 { margin: 0; font-size: 18px; }

  .panes {
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
    gap: 12px;
    flex: 1;
    min-height: 0;
  }
  .pane {
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    min-height: 0;
    overflow: hidden;
  }
  .pane-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
  }
  .pane-header .hint { font-weight: 400; text-transform: none; letter-spacing: 0; font-size: 11px; }

  .transcript {
    flex: 1;
    padding: 12px;
    overflow-y: auto;
    font-family: ui-monospace, SF Mono, Menlo, monospace;
    font-size: 13px;
    line-height: 1.5;
  }
  .seg { margin-bottom: 6px; }
  .ts { display: inline-block; min-width: 48px; }
  .speaker { font-weight: 600; margin-right: 4px; }

  .notes {
    flex: 1;
    padding: 8px 12px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .note { padding: 6px 8px; border-radius: 6px; background: var(--bg); }
  .note-meta {
    display: flex;
    gap: 8px;
    font-size: 11px;
    margin-bottom: 2px;
    align-items: center;
  }
  .note-meta .ts { font-variant-numeric: tabular-nums; min-width: 0; }
  .hint-tag {
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 10px;
    padding: 0 4px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .hint-tag[data-hint="you"] { color: var(--accent); border-color: var(--accent); }
  .hint-tag[data-hint="them"] { color: var(--good); border-color: var(--good); }
  .note-text { font-size: 13px; line-height: 1.45; white-space: pre-wrap; }

  .composer {
    border-top: 1px solid var(--border);
    padding: 8px;
    background: var(--bg);
  }
  .composer textarea {
    width: 100%;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    font: inherit;
    font-size: 13px;
    resize: vertical;
    min-height: 36px;
  }
  .error { color: var(--danger); font-size: 11px; margin-top: 4px; }
</style>
