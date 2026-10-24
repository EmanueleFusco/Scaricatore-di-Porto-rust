<!--
  The Queue page (spec §11.2, §11.5).

  Add bar on top, the virtualised list below, and the global actions in the
  footer: Pause all / Resume all, Retry failed, Clear finished.
-->
<script lang="ts">
  import AddBar from "$lib/components/AddBar.svelte";
  import QueueList from "$lib/components/QueueList.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  const summary = $derived(queueStore.summary);
  const hasFinished = $derived(summary.done + summary.failed + summary.cancelled > 0);
  const hasFailed = $derived(summary.failed > 0);
  const hasParked = $derived(summary.paused + summary.interrupted > 0);
</script>

<svelte:head>
  <title>{tt("queue.title")} — {tt("app.name")}</title>
</svelte:head>

<section class="queue">
  <h1 class="queue__title">{tt("queue.title")}</h1>

  <AddBar />

  <QueueList />

  <div class="queue__actions">
    {#if summary.pausedAll || hasParked}
      <button type="button" class="btn" onclick={() => void queueStore.resumeAll()}>
        {tt("queue.list.resumeAll")}
      </button>
    {:else}
      <button type="button" class="btn" disabled={summary.running === 0} onclick={() => void queueStore.pauseAll()}>
        {tt("queue.list.pauseAll")}
      </button>
    {/if}
    <button type="button" class="btn" disabled={!hasFailed} onclick={() => void queueStore.retryFailed()}>
      {tt("queue.list.retryFailed")}
    </button>
    <button type="button" class="btn" disabled={!hasFinished} onclick={() => void queueStore.clearFinished()}>
      {tt("queue.list.clearFinished")}
    </button>
  </div>
</section>

<style>
  .queue {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    height: 100%;
    min-height: 0;
    /* The page is exactly as tall as the window; when even the minimum of the
       add bar plus the list does not fit (a very small window, or a long list
       of pasted links), this scrolls vertically instead of letting a child
       paint over the actions and the footer. `overflow-y: auto` uses the full
       height of `.shell__main`, while small horizontal padding keeps focus
       rings and item-flash shadows from being clipped at the container edge. */
    overflow-y: auto;
    padding: 0 2px;
    margin: 0 -2px;
  }

  .queue__title {
    font-size: 20px;
    flex: none;
  }

  .queue__actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    flex: none;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }
</style>