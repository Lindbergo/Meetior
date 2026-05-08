<script lang="ts">
  import { onMount } from "svelte";
  import { api, type MeetingDetail } from "./api";

  interface Props {
    id: string;
    onBack: () => void;
  }

  let { id, onBack }: Props = $props();
  let detail = $state<MeetingDetail | null>(null);
  let summarizing = $state(false);

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
</script>

<div class="stack">
  <button onclick={onBack}>← Back</button>

  {#if !detail}
    <p class="muted">Loading…</p>
  {:else}
    <h2>{detail.title}</h2>
    <div class="muted">
      {new Date(detail.started_at).toLocaleString()}
      {#if detail.ended_at}· ended {new Date(detail.ended_at).toLocaleTimeString()}{/if}
    </div>

    <section>
      <h3>Summary</h3>
      {#if detail.summary}
        <p>{detail.summary}</p>
      {:else}
        <button class="primary" onclick={summarize} disabled={summarizing}>
          {summarizing ? "Summarizing…" : "Generate summary & todos"}
        </button>
      {/if}
    </section>

    {#if detail.todos.length > 0}
      <section>
        <h3>Todos</h3>
        <ul class="todos">
          {#each detail.todos as t (t.id)}
            <li>
              <label>
                <input type="checkbox" checked={t.done} onchange={() => toggle(t.id)} />
                <span class:done={t.done}>{t.text}</span>
              </label>
            </li>
          {/each}
        </ul>
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
  h2 { margin: 0; font-size: 18px; }
  h3 { margin: 0 0 6px; font-size: 14px; }
  section { margin-top: 16px; }
  .todos { list-style: none; padding: 0; }
  .todos li { padding: 4px 0; }
  .todos label { display: flex; gap: 8px; align-items: center; }
  .done { text-decoration: line-through; color: var(--fg-muted); }
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
</style>
