<!--
  The queue list (spec §11.2, §11.7).

  Virtualised: only the rows in the viewport (plus an overscan) exist in the
  DOM, so a 5 000-item queue scrolls like a 10-item one. The scroll maths is
  the pure `visibleRange()` of `$lib/queue`, which is unit-tested; this
  component only wires it to a scroll event.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import QueueGroups from "$lib/components/QueueGroups.svelte";
  import QueueItemRow from "$lib/components/QueueItem.svelte";
  import {
    actionsFor,
    filterQueueItems,
    itemsOutsideCollapsed,
    scrollOffsetFor,
    selectRange,
    toggleSelected,
    visibleItems,
    visibleRange,
    type ItemAction,
    type QueueItem,
  } from "$lib/queue";
  import { isTypingTarget, shortcutAction } from "$lib/shortcuts";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { tt, ttCount } from "$lib/stores/settings.svelte";

  /** Fallback row height, for the first paint; the real one is measured. */
  const ROW_HEIGHT_FALLBACK = 132;
  const VIEWPORT_FALLBACK = 480;

  let scroller = $state<HTMLElement | null>(null);
  let filterInput = $state<HTMLInputElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(VIEWPORT_FALLBACK);
  let rowHeight = $state(ROW_HEIGHT_FALLBACK);

  /** Groups the user folded away (spec §11.5): their items leave the list. */
  let collapsed = $state<Set<string>>(new Set());
  /** A find-as-you-type filter over the visible rows (spec §11.5). */
  let filter = $state("");
  let selectedIds = $state<Set<string>>(new Set());
  let selectionAnchor = $state<string | null>(null);

  const allItems = $derived(queueStore.list);
  const matching = $derived(filterQueueItems(allItems, filter));
  const items = $derived(itemsOutsideCollapsed(matching, collapsed));
  const selectedItems = $derived(items.filter((item) => selectedIds.has(item.id)));
  const allSelected = $derived(items.length > 0 && items.every((item) => selectedIds.has(item.id)));
  const canPause = $derived(selectedItems.some((item) => hasAction(item, "pause")));
  const canResume = $derived(selectedItems.some((item) => hasAction(item, "resume")));
  const canCancel = $derived(selectedItems.some((item) => hasAction(item, "cancel")));

  const range = $derived(visibleRange(scrollTop, viewportHeight, rowHeight, items.length));
  const shown = $derived(visibleItems(items, range));

  function toggleGroup(id: string): void {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
    scrollTop = 0;
    if (scroller !== null) scroller.scrollTop = 0;
  }

  function resetScroll(): void {
    scrollTop = 0;
    if (scroller !== null) scroller.scrollTop = 0;
  }

  function onFilterInput(): void {
    resetScroll();
    selectionAnchor = null;
  }

  function onSelect(id: string, event: MouseEvent): void {
    if (event.shiftKey && selectionAnchor !== null) {
      selectedIds = selectRange(
        selectedIds,
        items.map((item) => item.id),
        selectionAnchor,
        id,
      );
    } else {
      selectedIds = toggleSelected(selectedIds, id);
    }
    selectionAnchor = id;
  }

  function toggleSelectAll(): void {
    selectedIds = allSelected ? new Set() : new Set(items.map((item) => item.id));
    selectionAnchor = null;
  }

  function clearSelection(): void {
    selectedIds = new Set();
    selectionAnchor = null;
  }

  function hasAction(item: QueueItem, action: ItemAction): boolean {
    return actionsFor(item.state, item.file !== null, !item.url.startsWith("local://")).includes(
      action,
    );
  }

  /** Apply one per-item action to every selected row that supports it. */
  async function bulk(action: ItemAction): Promise<void> {
    const targets = selectedItems.filter((item) => hasAction(item, action));
    for (const item of targets) {
      await queueStore.act(action, item.id);
    }
    if (action === "remove" || action === "cancel") clearSelection();
  }

  /** Space on a selection: resume parked rows, pause the running ones. */
  async function togglePauseSelected(): Promise<void> {
    const parked = selectedItems.every(
      (item) => item.state === "paused" || item.state === "interrupted",
    );
    await bulk(parked ? "resume" : "pause");
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented) return;
    const action = shortcutAction(event, isTypingTarget(event.target));
    if (action === "filter") {
      event.preventDefault();
      filterInput?.focus();
      filterInput?.select();
    } else if (action === "removeSelected" && selectedItems.length > 0) {
      event.preventDefault();
      void bulk("remove");
    } else if (action === "togglePause" && selectedItems.length > 0) {
      event.preventDefault();
      void togglePauseSelected();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", onWindowKeydown);
    return () => window.removeEventListener("keydown", onWindowKeydown);
  });

  // Drop ids that left the queue (removed, cleared) so the bulk bar cannot
  // keep acting on rows that no longer exist.
  $effect(() => {
    const live = new Set(allItems.map((item) => item.id));
    const kept = [...selectedIds].filter((id) => live.has(id));
    if (kept.length !== selectedIds.size) {
      selectedIds = new Set(kept);
      if (selectionAnchor !== null && !live.has(selectionAnchor)) selectionAnchor = null;
    }
  });

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
  {#if allItems.length > 0}
    <div class="list__toolbar">
      <div class="list__filter">
        <label class="visually-hidden" for="queue-filter">{tt("queue.filter.label")}</label>
        <input
          class="input list__filter-input"
          id="queue-filter"
          type="search"
          bind:this={filterInput}
          bind:value={filter}
          oninput={onFilterInput}
          placeholder={tt("queue.filter.placeholder")}
          autocomplete="off"
        />
        {#if filter !== ""}
          <button
            type="button"
            class="btn btn--small"
            onclick={() => {
              filter = "";
              onFilterInput();
              filterInput?.focus();
            }}
          >
            {tt("queue.filter.clear")}
          </button>
        {/if}
      </div>
      <div class="list__toolbar-actions">
        <button
          type="button"
          class="btn btn--small"
          disabled={items.length === 0}
          onclick={toggleSelectAll}
        >
          {allSelected ? tt("queue.selection.clear") : tt("queue.selection.selectAll")}
        </button>
        {#if filter !== ""}
          <span class="list__filter-count" role="status">
            {tt("queue.filter.count", { shown: items.length, total: allItems.length })}
          </span>
        {/if}
      </div>
    </div>
  {/if}

  {#if selectedItems.length > 0}
    <div class="list__bulk" role="toolbar" aria-label={tt("queue.selection.toolbar")}>
      <span class="list__bulk-count" role="status">
        {tt("queue.selection.count", { count: selectedItems.length })}
      </span>
      {#if canPause}
        <button type="button" class="btn btn--small" onclick={() => void bulk("pause")}>
          {tt("queue.selection.pause")}
        </button>
      {/if}
      {#if canResume}
        <button type="button" class="btn btn--small" onclick={() => void bulk("resume")}>
          {tt("queue.selection.resume")}
        </button>
      {/if}
      {#if canCancel}
        <button type="button" class="btn btn--small" onclick={() => void bulk("cancel")}>
          {tt("queue.selection.cancel")}
        </button>
      {/if}
      <button
        type="button"
        class="btn btn--small btn--danger"
        onclick={() => void bulk("remove")}
      >
        {tt("queue.selection.remove")}
      </button>
      <button type="button" class="btn btn--small" onclick={clearSelection}>
        {tt("queue.selection.clear")}
      </button>
    </div>
  {/if}

  <QueueGroups {collapsed} ontoggle={toggleGroup} items={matching} />
  <div class="list__viewport" bind:this={scroller} onscroll={onScroll} role="list">
    {#if allItems.length === 0}
      <div class="list__empty">
        <p class="list__empty-title">{tt("queue.list.empty.title")}</p>
        <p class="field__hint">{tt("queue.list.empty.hint")}</p>
      </div>
    {:else if items.length === 0}
      <div class="list__empty">
        <p class="list__empty-title">
          {filter !== "" ? tt("queue.filter.empty") : tt("queue.list.allCollapsed")}
        </p>
        {#if filter !== ""}
          <button
            type="button"
            class="btn"
            onclick={() => {
              filter = "";
              onFilterInput();
            }}
          >
            {tt("queue.filter.clear")}
          </button>
        {/if}
      </div>
    {:else}
      <div class="list__pad" style:height={`${range.topPad}px`}></div>
      {#each shown as item (item.id)}
        <QueueItemRow
          {item}
          selected={selectedIds.has(item.id)}
          onselect={onSelect}
        />
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

  .list__toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex: none;
  }

  .list__filter {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: 1 1 240px;
  }

  .list__filter-input {
    flex: 1 1 auto;
    min-width: 120px;
  }

  .list__toolbar-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .list__filter-count {
    color: var(--text-muted);
    font-size: 11px;
  }

  .list__bulk {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    flex: none;
  }

  .list__bulk-count {
    font-weight: 600;
    font-size: 12px;
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