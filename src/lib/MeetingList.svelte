<script lang="ts">
  import { CLIENT_COLOR_HEX, type Client, type Meeting } from "./api";

  interface Props {
    meetings: Meeting[];
    clientsById: Record<string, Client>;
    onOpen: (id: string) => void;
  }

  let { meetings, clientsById, onOpen }: Props = $props();
</script>

<div class="stack">
  {#if meetings.length === 0}
    <p class="muted">No meetings here yet.</p>
  {/if}
  {#each meetings as m (m.id)}
    {@const client = m.client_id ? clientsById[m.client_id] : null}
    <button class="card" onclick={() => onOpen(m.id)}>
      <div class="title-row">
        <div class="title">{m.title}</div>
        {#if client}
          <span class="tag">
            <span class="dot" style={`background:${CLIENT_COLOR_HEX[client.color]}`}></span>
            {client.name}
          </span>
        {/if}
      </div>
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
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .title { font-weight: 500; }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--fg-muted);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 8px;
    flex: 0 0 auto;
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .meta { font-size: 12px; margin-top: 4px; }
</style>
