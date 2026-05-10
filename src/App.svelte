<script lang="ts">
  import { onMount } from "svelte";
  import MeetingList from "./lib/MeetingList.svelte";
  import ActiveMeeting from "./lib/ActiveMeeting.svelte";
  import MeetingDetail from "./lib/MeetingDetail.svelte";
  import Sidebar, { type Filter } from "./lib/Sidebar.svelte";
  import StartDialog from "./lib/StartDialog.svelte";
  import SearchResults from "./lib/SearchResults.svelte";
  import { api, events, type Client, type Meeting, type NoteHit } from "./lib/api";

  let meetings = $state<Meeting[]>([]);
  let clients = $state<Client[]>([]);
  let active = $state<Meeting | null>(null);
  let viewing = $state<string | null>(null);
  let filter = $state<Filter>({ kind: "all" });
  let showStartDialog = $state(false);

  // Notes search state. Empty query → list view; non-empty → search results.
  // Debounced via $effect so we don't fire a query per keystroke.
  let query = $state("");
  let debouncedQuery = $state("");
  let searchHits = $state<NoteHit[]>([]);
  let searching = $state(false);

  $effect(() => {
    const q = query;
    const id = setTimeout(() => { debouncedQuery = q; }, 200);
    return () => clearTimeout(id);
  });

  $effect(() => {
    const q = debouncedQuery.trim();
    if (!q) { searchHits = []; return; }
    const clientFilter = filter.kind === "client" ? filter.id : null;
    let cancelled = false;
    searching = true;
    api.searchNotes(q, clientFilter)
      .then((hits) => { if (!cancelled) searchHits = hits; })
      .catch(() => { if (!cancelled) searchHits = []; })
      .finally(() => { if (!cancelled) searching = false; });
    return () => { cancelled = true; };
  });

  const isSearching = $derived(debouncedQuery.trim().length > 0);

  let clientsById = $derived(
    Object.fromEntries(clients.map((c) => [c.id, c])) as Record<string, Client>,
  );

  let counts = $derived.by(() => {
    const perClient: Record<string, number> = {};
    let unassigned = 0;
    for (const m of meetings) {
      if (m.client_id) {
        perClient[m.client_id] = (perClient[m.client_id] ?? 0) + 1;
      } else {
        unassigned += 1;
      }
    }
    return { all: meetings.length, unassigned, perClient };
  });

  let visibleMeetings = $derived(
    meetings.filter((m) => {
      if (filter.kind === "all") return true;
      if (filter.kind === "unassigned") return !m.client_id;
      return m.client_id === filter.id;
    }),
  );

  async function refresh() {
    meetings = await api.listMeetings();
    active = meetings.find((m) => m.status === "recording") ?? null;
  }

  async function refreshClients() {
    clients = await api.listClients();
  }

  onMount(() => {
    refresh();
    refreshClients();
    const unlisten = events.onMeetingStatus(async (_m) => { await refresh(); });
    return () => { unlisten.then((fn) => fn()); };
  });

  function openStart() { showStartDialog = true; }

  async function confirmStart(args: { title: string | null; clientId: string | null; newClientName?: string }) {
    let clientId = args.clientId;
    if (args.newClientName) {
      const created = await api.createClient(args.newClientName);
      clientId = created.id;
      await refreshClients();
    }
    const m = await api.startMeeting(args.title ?? undefined, clientId);
    active = m;
    showStartDialog = false;
    await refresh();
  }

  async function handleStop() {
    if (!active) return;
    await api.stopMeeting(active.id);
    await refresh();
  }

  async function createClientFromSidebar(name: string) {
    const created = await api.createClient(name);
    await refreshClients();
    filter = { kind: "client", id: created.id };
  }

  // The default client for the start dialog: if a specific client is
  // currently selected in the sidebar, pre-fill it; otherwise leave empty.
  let defaultClientId = $derived(filter.kind === "client" ? filter.id : null);
</script>

<main>
  <Sidebar
    {clients}
    {counts}
    {filter}
    onSelect={(f) => (filter = f)}
    onCreateClient={createClientFromSidebar}
  />

  <section class="pane">
    <header>
      <h1>
        {#if filter.kind === "all"}All meetings
        {:else if filter.kind === "unassigned"}Unassigned
        {:else}{clientsById[filter.id]?.name ?? "Client"}
        {/if}
      </h1>
      <input
        class="search"
        type="search"
        placeholder={filter.kind === "client"
          ? `Search notes in ${clientsById[filter.id]?.name ?? "client"}…`
          : "Search notes…"}
        bind:value={query}
      />
      <div class="row actions">
        {#if active}
          <button class="danger" onclick={handleStop}>Stop meeting</button>
        {:else}
          <button class="primary" onclick={openStart}>Start meeting</button>
        {/if}
      </div>
    </header>

    <section class="content">
      {#if active}
        <ActiveMeeting meeting={active} />
      {:else if viewing}
        <MeetingDetail
          id={viewing}
          {clients}
          onBack={() => (viewing = null)}
        />
      {:else if isSearching}
        <SearchResults
          hits={searchHits}
          query={debouncedQuery}
          {searching}
          {clientsById}
          onOpen={(id) => (viewing = id)}
        />
      {:else}
        <MeetingList
          meetings={visibleMeetings}
          {clientsById}
          onOpen={(id) => (viewing = id)}
        />
      {/if}
    </section>
  </section>

  {#if showStartDialog}
    <StartDialog
      {clients}
      {defaultClientId}
      onConfirm={confirmStart}
      onCancel={() => (showStartDialog = false)}
    />
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: row;
    height: 100%;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  h1 { font-size: 16px; margin: 0; font-weight: 600; flex: 0 0 auto; }
  .search {
    flex: 1;
    max-width: 320px;
    background: var(--bg-elev);
    color: inherit;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
  }
  .actions { margin-left: auto; }
  .content { flex: 1; overflow: auto; padding: 16px; }
</style>
