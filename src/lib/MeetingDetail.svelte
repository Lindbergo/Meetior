<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    CLIENT_COLOR_HEX,
    type Client,
    type MeetingDetail,
    type SpeakerHint,
  } from "./api";

  interface Props {
    id: string;
    clients: Client[];
    onBack: () => void;
  }

  let { id, clients, onBack }: Props = $props();
  let detail = $state<MeetingDetail | null>(null);
  let summarizing = $state(false);

  // ----- title editing ------------------------------------------------
  let editingTitle = $state(false);
  let titleDraft = $state("");
  let titleError = $state<string | null>(null);

  // ----- summary editing ----------------------------------------------
  let editingSummary = $state(false);
  let summaryDraft = $state("");
  let summaryError = $state<string | null>(null);

  // ----- todos --------------------------------------------------------
  let editingTodoId = $state<string | null>(null);
  let todoDraft = $state("");
  let newTodoText = $state("");
  let todoError = $state<string | null>(null);

  onMount(async () => {
    detail = await api.getMeeting(id);
  });

  async function summarize() {
    summarizing = true;
    try {
      detail = await api.summarize(id);
    } finally {
      summarizing = false;
    }
  }

  async function toggle(todoId: string) {
    if (!detail) return;
    const updated = await api.toggleTodo(detail.id, todoId);
    detail = {
      ...detail,
      todos: detail.todos.map((t) => (t.id === todoId ? updated : t)),
    };
  }

  // -------- title -----------------------------------------------------

  function startEditTitle() {
    if (!detail) return;
    titleDraft = detail.title;
    titleError = null;
    editingTitle = true;
  }

  async function saveTitle() {
    if (!detail) return;
    const trimmed = titleDraft.trim();
    if (!trimmed) { titleError = "Title cannot be empty"; return; }
    try {
      const m = await api.updateMeetingTitle(detail.id, trimmed);
      detail = { ...detail, title: m.title };
      editingTitle = false;
    } catch (e) {
      titleError = String(e);
    }
  }

  // -------- client ----------------------------------------------------

  async function changeClient(clientId: string | null) {
    if (!detail) return;
    try {
      const m = await api.setMeetingClient(detail.id, clientId);
      detail = {
        ...detail,
        client_id: m.client_id,
        client: clientId ? clients.find((c) => c.id === clientId) ?? null : null,
      };
    } catch (e) {
      // Surface inline; non-fatal.
      console.error("setMeetingClient failed:", e);
    }
  }

  // -------- summary ---------------------------------------------------

  function startEditSummary() {
    if (!detail) return;
    summaryDraft = detail.summary ?? "";
    summaryError = null;
    editingSummary = true;
  }

  async function saveSummary() {
    if (!detail) return;
    try {
      await api.updateSummary(detail.id, summaryDraft);
      detail = { ...detail, summary: summaryDraft };
      editingSummary = false;
    } catch (e) {
      summaryError = String(e);
    }
  }

  // -------- todos -----------------------------------------------------

  function startEditTodo(todoId: string, text: string) {
    editingTodoId = todoId;
    todoDraft = text;
    todoError = null;
  }

  async function saveTodo(todoId: string) {
    if (!detail) return;
    const trimmed = todoDraft.trim();
    if (!trimmed) { todoError = "Todo cannot be empty"; return; }
    try {
      const updated = await api.updateTodoText(detail.id, todoId, trimmed);
      detail = {
        ...detail,
        todos: detail.todos.map((t) => (t.id === todoId ? updated : t)),
      };
      editingTodoId = null;
    } catch (e) {
      todoError = String(e);
    }
  }

  async function deleteTodo(todoId: string) {
    if (!detail) return;
    try {
      await api.deleteTodo(detail.id, todoId);
      detail = { ...detail, todos: detail.todos.filter((t) => t.id !== todoId) };
    } catch (e) {
      todoError = String(e);
    }
  }

  async function addTodo() {
    if (!detail) return;
    const trimmed = newTodoText.trim();
    if (!trimmed) return;
    try {
      const created = await api.addTodo(detail.id, trimmed);
      detail = { ...detail, todos: [...detail.todos, created] };
      newTodoText = "";
    } catch (e) {
      todoError = String(e);
    }
  }

  // -------- helpers ---------------------------------------------------

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

<div class="stack">
  <button onclick={onBack}>← Back</button>

  {#if !detail}
    <p class="muted">Loading…</p>
  {:else}
    <header class="head">
      {#if editingTitle}
        <div class="title-edit">
          <input
            bind:value={titleDraft}
            onkeydown={(e) => {
              if (e.key === "Enter") saveTitle();
              else if (e.key === "Escape") editingTitle = false;
            }}
          />
          <button class="primary" onclick={saveTitle}>Save</button>
          <button onclick={() => (editingTitle = false)}>Cancel</button>
        </div>
        {#if titleError}<div class="error">{titleError}</div>{/if}
      {:else}
        <div class="title-row">
          <h2>{detail.title}</h2>
          <button class="link" onclick={startEditTitle}>Edit</button>
        </div>
      {/if}

      <div class="muted meta">
        {new Date(detail.started_at).toLocaleString()}
        {#if detail.ended_at}· ended {new Date(detail.ended_at).toLocaleTimeString()}{/if}
      </div>

      <label class="client-row muted">
        Client
        <select
          value={detail.client_id ?? ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            changeClient(v === "" ? null : v);
          }}
        >
          <option value="">Unassigned</option>
          {#each clients as c (c.id)}
            <option value={c.id}>{c.name}</option>
          {/each}
        </select>
        {#if detail.client}
          <span
            class="client-dot"
            style={`background:${CLIENT_COLOR_HEX[detail.client.color]}`}
            aria-hidden="true"
          ></span>
        {/if}
      </label>
    </header>

    <section>
      <h3>Summary</h3>
      {#if editingSummary}
        <textarea
          bind:value={summaryDraft}
          rows="6"
          placeholder="Write a summary…"
        ></textarea>
        {#if summaryError}<div class="error">{summaryError}</div>{/if}
        <div class="row gap">
          <button class="primary" onclick={saveSummary}>Save</button>
          <button onclick={() => (editingSummary = false)}>Cancel</button>
        </div>
      {:else if detail.summary}
        <p class="summary">{detail.summary}</p>
        <div class="row gap">
          <button onclick={startEditSummary}>Edit summary</button>
          <button onclick={summarize} disabled={summarizing}>
            {summarizing ? "Regenerating…" : "Regenerate"}
          </button>
        </div>
      {:else}
        <div class="row gap">
          <button class="primary" onclick={summarize} disabled={summarizing}>
            {summarizing ? "Summarizing…" : "Generate summary & todos"}
          </button>
          <button onclick={startEditSummary}>Write your own</button>
        </div>
      {/if}
    </section>

    <section>
      <h3>Todos</h3>
      <ul class="todos">
        {#each detail.todos as t (t.id)}
          <li>
            <input type="checkbox" checked={t.done} onchange={() => toggle(t.id)} />
            {#if editingTodoId === t.id}
              <input
                class="todo-edit"
                bind:value={todoDraft}
                onkeydown={(e) => {
                  if (e.key === "Enter") saveTodo(t.id);
                  else if (e.key === "Escape") editingTodoId = null;
                }}
              />
              <button class="link" onclick={() => saveTodo(t.id)}>Save</button>
              <button class="link" onclick={() => (editingTodoId = null)}>Cancel</button>
            {:else}
              <button
                class="todo-text"
                class:done={t.done}
                onclick={() => startEditTodo(t.id, t.text)}
                title="Click to edit"
              >{t.text}</button>
              <button
                class="link delete"
                aria-label="Delete todo"
                onclick={() => deleteTodo(t.id)}
              >×</button>
            {/if}
          </li>
        {/each}
        <li class="add-row">
          <input
            placeholder="Add a todo…"
            bind:value={newTodoText}
            onkeydown={(e) => { if (e.key === "Enter") addTodo(); }}
          />
          <button onclick={addTodo} disabled={!newTodoText.trim()}>Add</button>
        </li>
      </ul>
      {#if todoError}<div class="error">{todoError}</div>{/if}
    </section>

    {#if detail.notes.length > 0}
      <section>
        <h3>Notes</h3>
        <div class="notes">
          {#each detail.notes as n (n.idx)}
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
      </section>
    {/if}

    <section>
      <h3>Transcript</h3>
      <div class="transcript">
        {#each detail.segments as s, i (i)}
          <div class="seg">
            <span class="ts muted">{(s.start_ms / 1000).toFixed(1)}s</span>
            {#if s.speaker}<span class="speaker">{s.speaker}:</span>{/if}
            <span>{s.text}</span>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .head { display: flex; flex-direction: column; gap: 6px; }
  h2 { margin: 0; font-size: 18px; }
  h3 { margin: 0 0 6px; font-size: 14px; }
  section { margin-top: 16px; }
  .meta { font-size: 12px; }
  .gap { gap: 8px; margin-top: 8px; }

  .title-row { display: flex; align-items: center; gap: 8px; }
  .title-edit { display: flex; gap: 6px; align-items: center; }
  .title-edit input {
    flex: 1;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    font: inherit;
    font-size: 16px;
    font-weight: 600;
  }

  .client-row { display: flex; gap: 8px; align-items: center; font-size: 12px; }
  .client-row select {
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 8px;
    font: inherit;
    font-size: 12px;
  }
  .client-dot { width: 10px; height: 10px; border-radius: 50%; }

  .summary { margin: 0; white-space: pre-wrap; }
  textarea {
    width: 100%;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px;
    font: inherit;
    font-size: 13px;
    line-height: 1.5;
    resize: vertical;
  }

  .todos { list-style: none; padding: 0; margin: 0; }
  .todos li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .todo-text {
    flex: 1;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    padding: 2px 4px;
    border-radius: 4px;
    cursor: text;
    color: inherit;
    font: inherit;
  }
  .todo-text:hover { background: var(--bg-elev); border-color: var(--border); }
  .todo-text.done { text-decoration: line-through; color: var(--fg-muted); }
  .todo-edit {
    flex: 1;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 2px 6px;
    font: inherit;
  }
  .add-row { margin-top: 4px; }
  .add-row input {
    flex: 1;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 8px;
    font: inherit;
  }

  button.link {
    background: transparent;
    border: none;
    padding: 0 4px;
    color: var(--fg-muted);
    font-size: 12px;
    cursor: pointer;
  }
  button.link:hover { color: var(--fg); }
  button.link.delete:hover { color: var(--danger); }

  .notes { display: flex; flex-direction: column; gap: 6px; }
  .note {
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
  }
  .note-meta { display: flex; gap: 8px; font-size: 11px; align-items: center; margin-bottom: 2px; }
  .note-meta .ts { font-variant-numeric: tabular-nums; }
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
  .note-text { font-size: 13px; white-space: pre-wrap; }

  .transcript {
    padding: 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-family: ui-monospace, SF Mono, Menlo, monospace;
    font-size: 13px;
    line-height: 1.5;
  }
  .seg { margin-bottom: 6px; }
  .ts { display: inline-block; min-width: 48px; }
  .speaker { font-weight: 600; margin-right: 4px; }

  .error { color: var(--danger); font-size: 12px; margin-top: 4px; }
</style>
