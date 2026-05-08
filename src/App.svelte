<script lang="ts">
  import { onMount } from "svelte";
  import MeetingList from "./lib/MeetingList.svelte";
  import ActiveMeeting from "./lib/ActiveMeeting.svelte";
  import MeetingDetail from "./lib/MeetingDetail.svelte";
  import { api, events, type Meeting } from "./lib/api";

  let meetings = $state<Meeting[]>([]);
  let active = $state<Meeting | null>(null);
  let viewing = $state<string | null>(null);

  async function refresh() {
    meetings = await api.listMeetings();
    active = meetings.find((m) => m.status === "recording") ?? null;
  }

  onMount(() => {
    refresh();
    const unlisten = events.onMeetingStatus(async (_m) => { await refresh(); });
    return () => { unlisten.then((fn) => fn()); };
  });

  async function handleStart() {
    const m = await api.startMeeting();
    active = m;
    await refresh();
  }

  async function handleStop() {
    if (!active) return;
    await api.stopMeeting(active.id);
    await refresh();
  }
</script>

<main>
  <header>
    <h1>Meetior</h1>
    <div class="row">
      {#if active}
        <button class="danger" onclick={handleStop}>Stop meeting</button>
      {:else}
        <button class="primary" onclick={handleStart}>Start meeting</button>
      {/if}
    </div>
  </header>

  <section class="content">
    {#if active}
      <ActiveMeeting meeting={active} />
    {:else if viewing}
      <MeetingDetail id={viewing} onBack={() => (viewing = null)} />
    {:else}
      <MeetingList {meetings} onOpen={(id) => (viewing = id)} />
    {/if}
  </section>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  h1 { font-size: 16px; margin: 0; font-weight: 600; }
  .content { flex: 1; overflow: auto; padding: 16px; }
</style>
