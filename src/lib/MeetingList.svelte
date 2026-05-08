<script lang="ts">
  import type { Meeting } from "./api";

  interface Props {
    meetings: Meeting[];
    onOpen: (id: string) => void;
  }

  let { meetings, onOpen }: Props = $props();
</script>

<div class="stack">
  {#if meetings.length === 0}
    <p class="muted">No meetings yet. Click <strong>Start meeting</strong> to record one.</p>
  {/if}
  {#each meetings as m (m.id)}
    <button class="card" onclick={() => onOpen(m.id)}>
      <div class="title">{m.title}</div>
      <div class="meta muted">
        {new Date(m.started_at).toLocaleString()} · {m.status}
      </div>
    </button>
  {/each}
</div>

<style>
  .card {
    text-align: left;
    padding: 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    cursor: pointer;
  }
  .title { font-weight: 500; }
  .meta { font-size: 12px; margin-top: 4px; }
</style>
