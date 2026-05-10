<script lang="ts">
  import { CLIENT_COLOR_HEX, type Client, type NoteHit, type SpeakerHint } from "./api";

  interface Props {
    hits: NoteHit[];
    query: string;
    searching: boolean;
    clientsById: Record<string, Client>;
    onOpen: (meetingId: string) => void;
  }

  let { hits, query, searching, clientsById, onOpen }: Props = $props();

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

  function escapeRegex(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  // Splits the text on case-insensitive matches of the query so we can
  // render `<mark>` around matches in the template (Svelte escapes the
  // text parts automatically — no innerHTML).
  function highlightParts(text: string, q: string): { text: string; match: boolean }[] {
    const trimmed = q.trim();
    if (!trimmed) return [{ text, match: false }];
    const re = new RegExp(`(${escapeRegex(trimmed)})`, "gi");
    return text
      .split(re)
      .filter((p) => p !== "")
      .map((p, i) => ({ text: p, match: i % 2 === 1 }));
  }
</script>

<div class="stack">
  {#if searching && hits.length === 0}
    <p class="muted">Searching…</p>
  {:else if hits.length === 0}
    <p class="muted">No notes match “{query}”.</p>
  {:else}
    <div class="muted count">{hits.length} {hits.length === 1 ? "match" : "matches"}</div>
    {#each hits as h (`${h.meeting_id}:${h.idx}`)}
      {@const client = h.client_id ? clientsById[h.client_id] : null}
      <button class="card" onclick={() => onOpen(h.meeting_id)}>
        <div class="head">
          <span class="meeting">{h.meeting_title}</span>
          {#if client}
            <span class="tag">
              <span class="dot" style={`background:${CLIENT_COLOR_HEX[client.color]}`}></span>
              {client.name}
            </span>
          {/if}
          <span class="when muted">
            {new Date(h.meeting_started_at).toLocaleDateString()}
            · {fmtTimestamp(h.t_ms)}
            {#if hintLabel(h.speaker_hint)}
              · <span class="hint-tag" data-hint={h.speaker_hint}>{hintLabel(h.speaker_hint)}</span>
            {/if}
          </span>
        </div>
        <div class="text">
          {#each highlightParts(h.text, query) as part}
            {#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}
          {/each}
        </div>
      </button>
    {/each}
  {/if}
</div>

<style>
  .count { font-size: 11px; }
  .card {
    display: block;
    width: 100%;
    text-align: left;
    padding: 10px 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    cursor: pointer;
    color: inherit;
  }
  .card:hover { border-color: var(--fg-muted); }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    margin-bottom: 4px;
  }
  .meeting { font-weight: 500; font-size: 13px; }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--fg-muted);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .when { font-variant-numeric: tabular-nums; }
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
  .text { font-size: 13px; line-height: 1.45; white-space: pre-wrap; }
  mark { background: rgba(255, 220, 0, 0.35); color: inherit; padding: 0 1px; border-radius: 2px; }
</style>
