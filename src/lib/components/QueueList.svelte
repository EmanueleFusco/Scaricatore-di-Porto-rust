<!--
  The queue list (spec §11.2, §11.7).

  Virtualised: only the rows in the viewport (plus an overscan) exist in the
  DOM, so a 5 000-item queue scrolls like a 10-item one. The scroll maths is
  the pure `visibleRange()` of `$lib/queue`, which is unit-tested; this
  component only wires it to a scroll event.
-->
<script lang="ts">
  import QueueGroups from "$lib/components/QueueGroups.svelte";
  import QueueItemRow from "$lib/components/QueueItem.svelte";
  import { untrack } from "svelte";
  import { itemsOutsideCollapsed, scrollOffsetFor, visibleItems, visibleRange } from "$lib/queue";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { tt, ttCount } from "$lib/stores/settings.svelte";

  /** Fallback row height, for the first paint; the real one is measured. */
  const ROW_HEIGHT_FALLBACK = 132;
  const VIEWPORT_FALLBACK = 480;

  let scroller = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(VIEWPORT_FALLBACK);
  let rowHeight = $state(ROW_HEIGHT_FALLBACK);

  /** Groups the user folded away (spec §11.5): their items leave the list. */
  let collapsed = $state<Set<string>>(new Set());
  const items = $derived(itemsOutsideCollapsed(queueStore.list, collapsed));

  function toggleGroup(id: string): void {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
    scrollTop = 0;
    if (scroller !== null) scroller.scrollTop = 0;
  }
  const range = $derived(visibleRange(scrollTop, viewportHeight, rowHeight, items.length));
  const shown = $derived(visibleItems(items, range));

  function onScroll(): void {
    scrollTop = scroller?.scrollTop ?? 0;
    if (scroller !== null) {
      viewportHeight = scroller.clientHeight || VIEWPORT_FALLBACK;
    }
  }

  /**
   * The distance between two rows, in px: the height of a card plus the gap.
   *
   * The cards have the same shape whatever the state (head, bar, one status
   * line, actions), so one pitch fits them all; the tallest covers a narrow
   * window where one state's buttons wrap and another's do not. Measured rather
   * than taken from a constant: the padding rows have to match the rows that
   * are really drawn, or the window scrolls to the wrong place.
   */
  function measureRow(): void {
    if (scroller === null) return;
    const rows = scroller.querySelectorAll<HTMLElement>(".item");
    if (rows.length === 0) return;
    const first = rows[0].getBoundingClientRect();
    const gap = Number.parseFloat(getComputedStyle(scroller).rowGap) || 0;
    const pitch =
      rows.length >= 2
        ? Math.round(rows[1].getBoundingClientRect().top - first.top)
        : Math.round(first.height + gap);
    if (pitch > 0 && Math.abs(pitch - rowHeight) > 2) {
      rowHeight = pitch;
    }
  }

  $effect(() => {
    const el = scroller;
    if (el === null) return;
    const observer = new ResizeObserver(() => measureRow());
    observer.observe(el);
    return () => observer.disconnect();
  });

  // Re-measure whenever the rendered slice changes: a row that went from
  // downloading to done has a different height, and so does a window resize.
  $effect(() => {
    void shown;
    untrack(() => measureRow());
  });

  /**
   * Bring a freshly added row into view (D104).
   *
   * New items keep their place at the end of the queue — FIFO, so a batch is
   * never starved by the links added after it — which means the row can land
   * below the visible ones and look like it went nowhere. The row is usually
   * not rendered yet (the list is virtualised), so the offset comes from the
   * same numbers the pads use, not from an element.
   */
  $effect(() => {
    const ids = queueStore.flashIds;
    if (ids.length === 0) return;
    untrack(() => {
      const element = scroller;
      if (element === null) return;
      const index = items.findIndex((item) => item.id === ids[0]);
      const offset = scrollOffsetFor(index, rowHeight, element.scrollTop, element.clientHeight);
      if (offset === null) return;
      const reduced =
        typeof window !== "undefined" &&
        window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      element.scrollTo({ top: offset, behavior: reduced ? "auto" : "smooth" });
    });
  });
</script>

<section class="list" aria-label={tt("a11y.queueList")}>
  <QueueGroups {collapsed} ontoggle={toggleGroup} />
  <div class="list__viewport" bind:this={scroller} onscroll={onScroll} role="list">
    {#if items.length === 0 && queueStore.list.length === 0}
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
    <p class="list__count" role="status">{ttCount("queue.list.count", items.length)}</p>
  {/if}
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 auto;
    /* A floor, not a fixed height: on a window too small for everything, the
       page scrolls (see `.queue`) instead of the list painting over the
       actions below it. */
    min-height: 104px;
  }

  .list__viewport {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-right: var(--space-1);
  }

  .list__count {
    flex: none;
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