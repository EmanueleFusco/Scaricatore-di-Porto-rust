<!--
  Status footer (spec §11.2, §7.5).

  With M2 this reads the queue: how many are running and queued, the aggregate
  speed, and whether everything is paused. It does not repeat what the cards
  already say (D72): this is a glance, the detail is where the bar is.
-->
<script lang="ts">
  import { fileName, speedMbits, type QueueItem } from "$lib/queue";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { errorText, tt } from "$lib/stores/settings.svelte";
  import { hasToolProblem } from "$lib/tools";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { router } from "$lib/router.svelte";

  const summary = $derived(queueStore.summary);
  const toolsProblem = $derived(hasToolProblem(toolsStore.view.tools));

  /** The item to name in the footer: the only one running, or the last one. */
  const focus = $derived<QueueItem | null>(focusItem(queueStore.list));

  const speed = $derived(speedMbits(summary.speedBps));

  function focusItem(items: QueueItem[]): QueueItem | null {
    const running = items.filter((item) => item.state === "downloading" || item.state === "postProcessing");
    if (running.length === 1) return running[0];
    if (running.length > 1) return null;
    // Nothing running: name the failure if there is one, else the newest done.
    const failed = items.filter((item) => item.state === "failed");
    if (failed.length === 1) return failed[0];
    const done = items.filter((item) => item.state === "done");
    return done.length > 0 ? done[done.length - 1] : null;
  }
</script>

<footer class="status" aria-label={tt("a11y.statusBar")}>
  <span class="status__counts">
    {#if summary.pausedAll}
      <span class="status__state" data-state="paused">
        <span class="status__dot" aria-hidden="true"></span>
        {tt("footer.pausedAll")}
      </span>
    {:else if summary.running > 0}
      <span class="status__state" data-state="running">
        <span class="status__dot" aria-hidden="true"></span>
        {tt("footer.running", { count: summary.running })}
      </span>
    {:else if summary.queued > 0}
      <span class="status__state" data-state="queued">
        <span class="status__dot" aria-hidden="true"></span>
        {tt("footer.queued", { count: summary.queued })}
      </span>
    {:else if summary.total === 0}
      <span class="status__state" data-state="idle">
        <span class="status__dot" aria-hidden="true"></span>
        {tt("footer.idle")}
      </span>
    {:else}
      <span class="status__state" data-state="done">
        <span class="status__dot" aria-hidden="true"></span>
        {tt("queue.state.done")}
      </span>
    {/if}

    {#if summary.queued > 0 && (summary.running > 0 || summary.pausedAll)}
      <span class="status__detail">· {tt("footer.queued", { count: summary.queued })}</span>
    {/if}

    {#if speed !== null}
      <span class="status__detail">{tt("footer.speed", { value: speed })}</span>
    {/if}
  </span>

  {#if focus !== null && (focus.file !== null || focus.error !== null)}
    <span class="status__detail" class:status__detail--error={focus.error !== null}>
      {#if focus.error !== null}
        {errorText(focus.error.messageKey, focus.error.detail)}
      {:else if focus.file !== null}
        {fileName(focus.file)}
      {/if}
    </span>
  {/if}

  {#if toolsProblem}
    <button type="button" class="status__link" onclick={() => router.go("/settings/tools")}>
      {tt("footer.toolsPending")}
    </button>
  {/if}
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--row-height);
    padding: 0 var(--space-4);
    background: var(--surface);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 12px;
  }

  .status__counts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .status__state {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text);
    font-weight: 600;
  }

  /* Decorative only: the state is spelled out in text (spec §11.9). */
  .status__dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-pill);
    background: var(--border-strong);
  }

  .status__state[data-state="running"] .status__dot {
    background: var(--accent);
  }

  .status__state[data-state="done"] .status__dot {
    background: var(--accent);
  }

  .status__state[data-state="paused"] .status__dot,
  .status__state[data-state="queued"] .status__dot {
    background: var(--warning, var(--border-strong));
  }

  .status__detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status__detail--error {
    color: var(--danger);
  }

  .status__link {
    margin-left: auto;
    padding: 0;
    border: none;
    background: none;
    color: var(--danger);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>