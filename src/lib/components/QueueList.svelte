<!--
  The queue list (spec §11.2, §11.7).

  Virtualised: only the rows in the viewport (plus an overscan) exist in the
  DOM, so a 5 000-item queue scrolls like a 10-item one. The scroll maths is
  the pure `visibleRange()` of `$lib/queue`, which is unit-tested; this
  component only wires it to a scroll event.
-->
<script lang="ts">
  import QueueItemRow from "$lib/components/QueueItem.svelte";
  import { visibleItems, visibleRange } from "$lib/queue";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  /** Row height in px; the cards are fixed-height by construction. */
  const ROW_HEIGHT = 92;
  const VIEWPORT_FALLBACK = 480;

  let scroller = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(VIEWPORT_FALLBACK);

  const items = $derived(queueStore.list);
  const range = $derived(visibleRange(scrollTop, viewportHeight, ROW_HEIGHT, items.length));
  const shown = $derived(visibleItems(items, range));

  function onScroll(): void {
    scrollTop = scroller?.scrollTop ?? 0;
    if (scroller !== null) {
      viewportHeight = scroller.clientHeight || VIEWPORT_FALLBACK;
    }
  }
</script>

<section class="list" aria-label={tt("a11y.queueList")}>
  <div class="list__viewport" bind:this={scroller} onscroll={onScroll} role="list">
    {#if items.length === 0}
      <div class="list__empty">
        <p class="list__empty-title">{tt("queue.list.empty.title")}</p>
        <p class="field__hint">{tt("queue.list.empty.hint")}</p>
      </div>
    {:else}
      <div class="list__pad" style:height={`${range.topPad}px`}></div>
      {#each shown as item (item.id)}
        <QueueItemRow {item} />
      {/each}
      <div class="list__pad" style:height={`${range.bottomPad}px`}></div>
    {/if}
  </div>

  {#if items.length > 0}
    <p class="list__count">{tt("queue.list.count", { visible: shown.length, total: items.length })}</p>
  {/if}
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    flex: 1 1 auto;
  }

  .list__viewport {
    flex: 1 1 auto;
    min-height: 240px;
    max-height: calc(100vh - 320px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-right: var(--space-1);
  }

  .list__pad {
    flex: none;
  }

  .list__empty {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }

  .list__empty-title {
    font-weight: 600;
  }

  .list__count {
    color: var(--text-muted);
    font-size: 11px;
    text-align: right;
  }

  @media (prefers-reduced-motion: reduce) {
    .list__viewport {
      scroll-behavior: auto;
    }
  }
</style>