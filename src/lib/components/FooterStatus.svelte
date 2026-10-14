<script lang="ts">
  /**
   * Status footer (spec §11.2, §7.5).
   *
   * In M0 this was a static placeholder; with M1 it reads the one job the
   * temporary screen runs, because a footer that says "no process running"
   * while a download is at 73 Mbit/s is not a placeholder, it is a lie. The
   * running/queued counters and the aggregate speed of spec §7.5 arrive with
   * the queue (M2); the shape and the tool warning are already here.
   */
  import { isDeterminate } from "$lib/inspect";
  import { fileName, footerStateKey, progressLabel } from "$lib/job";
  import { router } from "$lib/router.svelte";
  import { activeLang, errorText, tt } from "$lib/stores/settings.svelte";
  import { jobStore } from "$lib/stores/job.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { hasToolProblem } from "$lib/tools";

  const status = $derived(jobStore.status);
  const progress = $derived(jobStore.progress);
  const done = $derived(jobStore.done);
  const toolsProblem = $derived(hasToolProblem(toolsStore.view.tools));

  const stateText = $derived(tt(footerStateKey(status, progress?.stage ?? null)));
</script>

<footer class="status" aria-label={tt("a11y.statusBar")}>
  <span class="status__state" data-state={status}>
    <span class="status__dot" aria-hidden="true"></span>
    {stateText}
  </span>

  {#if status === "running" && progress}
    <!-- A summary, not a copy of the page: the speed and the ETA live there,
         with the bar; here only the stream and how far it is. -->
    <span class="status__detail">{progressLabel(activeLang(), progress)}</span>
    {#if isDeterminate(progress.percent)}
      <span class="status__detail">{tt("footer.percent", { value: Math.round(progress.percent!) })}</span>
    {/if}
  {:else if status === "done" && done?.file}
    <span class="status__detail">{fileName(done.file)}</span>
  {:else if status === "failed" && done?.error}
    <span class="status__detail status__detail--error">
      {errorText(done.error.messageKey, done.error.detail)}
    </span>
  {:else if toolsProblem}
    <button type="button" class="status__link" onclick={() => router.go("/settings/tools")}>
      {tt("footer.toolsPending")}
    </button>
  {:else}
    <span class="status__detail">{tt("footer.placeholder")}</span>
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

  .status__state {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text);
    font-weight: 600;
  }

  /* Decorative only: the state is already spelled out in text (spec §11.9). */
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

  .status__state[data-state="failed"] .status__dot {
    background: var(--danger);
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
    padding: 0;
    border: none;
    background: none;
    color: var(--danger);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
