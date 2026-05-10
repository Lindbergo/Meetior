<script lang="ts">
  import { untrack } from "svelte";
  import { CLIENT_COLOR_HEX, type Client } from "./api";

  interface Props {
    clients: Client[];
    /** Pre-fill the picker with this client (e.g. the active sidebar filter). */
    defaultClientId?: string | null;
    onConfirm: (args: { title: string | null; clientId: string | null; newClientName?: string }) => Promise<void>;
    onCancel: () => void;
  }

  let { clients, defaultClientId = null, onConfirm, onCancel }: Props = $props();

  // "" → No client, "__new__" → create new client by name, otherwise an existing id.
  const NEW = "__new__";
  const NONE = "";
  let title = $state("");
  // Read the prop once at mount; subsequent changes shouldn't move the user's
  // pick out from under them while the dialog is open.
  let pick = $state<string>(untrack(() => defaultClientId ?? NONE));
  let newName = $state("");
  let submitting = $state(false);
  let error = $state<string | null>(null);
  let titleEl: HTMLInputElement | null = $state(null);

  $effect(() => {
    titleEl?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function submit(e?: Event) {
    e?.preventDefault();
    if (submitting) return;
    submitting = true;
    error = null;
    try {
      const args = {
        title: title.trim() || null,
        clientId: pick === NEW || pick === NONE ? null : pick,
        newClientName: pick === NEW ? newName.trim() : undefined,
      };
      if (pick === NEW && !args.newClientName) {
        error = "Enter a name for the new client.";
        submitting = false;
        return;
      }
      await onConfirm(args);
    } catch (err) {
      error = String(err);
      submitting = false;
    }
  }
</script>

<div
  class="backdrop"
  role="dialog"
  aria-modal="true"
  aria-label="Start meeting"
  tabindex="-1"
>
  <button
    class="close-overlay"
    aria-label="Close"
    onclick={onCancel}
  ></button>

  <form class="card" onsubmit={submit}>
    <h2>Start meeting</h2>

    <label>
      <span>Title</span>
      <input
        bind:this={titleEl}
        bind:value={title}
        placeholder="Defaults to “Meeting <date>”"
      />
    </label>

    <label>
      <span>Client</span>
      <select bind:value={pick}>
        <option value={NONE}>No client</option>
        {#each clients as c (c.id)}
          <option value={c.id}>{c.name}</option>
        {/each}
        <option value={NEW}>+ New client…</option>
      </select>
    </label>

    {#if pick === NEW}
      <label>
        <span>New client name</span>
        <input bind:value={newName} placeholder="e.g. Acme Corp" />
      </label>
    {:else if pick && pick !== NONE}
      {@const picked = clients.find((c) => c.id === pick)}
      {#if picked}
        <div class="picked muted">
          <span class="dot" style={`background:${CLIENT_COLOR_HEX[picked.color]}`}></span>
          {picked.name}
        </div>
      {/if}
    {/if}

    {#if error}<div class="error">{error}</div>{/if}

    <div class="actions">
      <button type="button" onclick={onCancel}>Cancel</button>
      <button type="submit" class="primary" disabled={submitting}>
        {submitting ? "Starting…" : "Start"}
      </button>
    </div>
  </form>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    z-index: 50;
  }
  .close-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    border: 0;
    padding: 0;
    cursor: default;
  }
  .card {
    position: relative;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 16px;
    width: 360px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
  }
  h2 { margin: 0; font-size: 14px; font-weight: 600; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  label input,
  label select {
    background: var(--bg);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    font: inherit;
  }
  .picked { display: flex; align-items: center; gap: 6px; font-size: 12px; padding: 0 2px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .error { color: var(--danger); font-size: 12px; }
</style>
