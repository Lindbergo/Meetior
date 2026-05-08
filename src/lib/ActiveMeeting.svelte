<script lang="ts">
  import { onMount } from "svelte";
  import { events, type Meeting, type TranscriptSegment } from "./api";

  interface Props { meeting: Meeting }
  let { meeting }: Props = $props();

  let segments = $state<TranscriptSegment[]>([]);

  onMount(() => {
    const unlisten = events.onTranscriptSegment((s) => {
      if (s.meeting_id !== meeting.id) return;
      segments = [...segments, s];
    });
    return () => { unlisten.then((fn) => fn()); };
  });
</script>

<div class="stack">
  <h2>{meeting.title}</h2>
  <div class="muted">Started {new Date(meeting.started_at).toLocaleTimeString()} · {meeting.status}</div>

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
</div>

<style>
  h2 { margin: 0; font-size: 18px; }
  .transcript {
    margin-top: 12px;
    padding: 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    min-height: 240px;
    font-family: ui-monospace, SF Mono, Menlo, monospace;
    font-size: 13px;
    line-height: 1.5;
  }
  .seg { margin-bottom: 6px; }
  .ts { display: inline-block; min-width: 48px; }
  .speaker { font-weight: 600; margin-right: 4px; }
</style>
