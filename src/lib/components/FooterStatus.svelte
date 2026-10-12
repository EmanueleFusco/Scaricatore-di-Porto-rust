<script lang="ts">
  /**
   * Footer status placeholder (spec §11.2). The real counters come with the
   * queue engine (M2); the shape and the i18n keys are already in place.
   */
  import { tt } from "$lib/stores/settings.svelte";
</script>

<footer class="status" aria-label={tt("a11y.statusBar")}>
  <span class="status__state">
    <span class="status__dot" aria-hidden="true"></span>
    {tt("footer.idle")}
  </span>
  <span class="status__detail">{tt("footer.placeholder")}</span>
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

  /* Decorative only: the state is already spelled out in text. */
  .status__dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-pill);
    background: var(--border-strong);
  }

  .status__detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>