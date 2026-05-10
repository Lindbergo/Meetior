<script lang="ts">
  import { CLIENT_COLOR_HEX, type Client } from "./api";

  // --------------------------------------------------------------------
  // Filter is the union of "all meetings", "unassigned", or a client id.
  // Lifted to App.svelte so the meeting list can filter against it.
  // --------------------------------------------------------------------
  export type Filter =
    | { kind: "all" }
    | { kind: "unassigned" }
    | { kind: "client"; id: string };

  interface Props {
    clients: Client[];
    counts: { all: number; unassigned: number; perClient: Record<string, number> };
    filter: Filter;
    onSelect: (filter: Filter) => void;
    onCreateClient: (name: string) => Promise<void>;
  }

  let { clients, counts, filter, onSelect, onCreateClient }: Props = $props();

  let creating = $state(false);
  let newName = $state("");
  let creatingError = $state<string | null>(null);
  let inputEl: HTMLInputElement | null = $state(null);

  function startCreate() {
    creating = true;
    newName = "";
    creatingError = null;
    queueMicrotask(() => inputEl?.focus());
  }

  function cancelCreate() {
    creating = false;
    newName = "";
    creatingError = null;
  }

  async function submitCreate() {
    const trimmed = newName.trim();
    if (!trimmed) {
      cancelCreate();
      return;
    }
    try {
      await onCreateClient(trimmed);
      cancelCreate();
    } catch (e) {
      creatingError = String(e);
    }
  }

  function isActive(f: Filter): boolean {
    if (f.kind !== filter.kind) return false;
    if (f.kind === "client" && filter.kind === "client") return f.id === filter.id;
    return true;
  }
</script>

<aside>
  <div class="brand">Meetior</div>

  <nav class="group">
    <button
      class="row"
      class:active={isActive({ kind: "all" })}
      onclick={() => onSelect({ kind: "all" })}
    >
      <span class="label">All meetings</span>
      <span class="count">{counts.all}</span>
    </button>
    <button
      class="row"
      class:active={isActive({ kind: "unassigned" })}
      onclick={() => onSelect({ kind: "unassigned" })}
    >
      <span class="label">Unassigned</span>
      <span class="count">{counts.unassigned}</span>
    </button>
  </nav>

  <div class="group">
    <div class="group-header">
      <span>Clients</span>
      <button
        class="icon"
        title="New client"
        aria-label="New client"
        onclick={startCreate}
      >+</button>
    </div>

    {#if creating}
      <div class="row create">
        <input
          bind:this={inputEl}
          bind:value={newName}
          placeholder="Client name"
          onkeydown={(e) => {
            if (e.key === "Enter") submitCreate();
            else if (e.key === "Escape") cancelCreate();
          }}
          onblur={() => { if (!newName.trim()) cancelCreate(); }}
        />
      </div>
      {#if creatingError}
        <div class="error">{creatingError}</div>
      {/if}
    {/if}

    {#if clients.length === 0 && !creating}
      <div class="empty muted">No clients yet.</div>
    {/if}

    {#each clients as c (c.id)}
      <button
        class="row"
        class:active={isActive({ kind: "client", id: c.id })}
        onclick={() => onSelect({ kind: "client", id: c.id })}
      >
        <span class="dot" style={`background:${CLIENT_COLOR_HEX[c.color]}`}></span>
        <span class="label">{c.name}</span>
        <span class="count">{counts.perClient[c.id] ?? 0}</span>
      </button>
    {/each}
  </div>
</aside>

<style>
  aside {
    width: 220px;
    flex: 0 0 220px;
    height: 100%;
    border-right: 1px solid var(--border);
    background: var(--bg);
    padding: 12px 8px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
  }
  .brand {
    font-size: 16px;
    font-weight: 600;
    padding: 4px 8px 0;
  }
  .group { display: flex; flex-direction: column; gap: 2px; }
  .group-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
    margin-bottom: 4px;
  }
  .icon {
    width: 20px;
    height: 20px;
    padding: 0;
    line-height: 1;
    border-radius: 4px;
    font-size: 14px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    text-align: left;
    cursor: pointer;
    width: 100%;
    color: inherit;
    font: inherit;
  }
  .row:hover { background: var(--bg-elev); }
  .row.active {
    background: var(--bg-elev);
    border-color: var(--border);
    font-weight: 500;
  }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count {
    font-size: 11px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: 0 0 10px;
  }
  .row.create input {
    flex: 1;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: inherit;
    font: inherit;
    padding: 4px 6px;
    min-width: 0;
  }
  .empty { padding: 4px 8px; font-size: 12px; }
  .error {
    color: var(--danger);
    font-size: 11px;
    padding: 0 8px;
  }
</style>
