<!--
  The playlist selection panel (spec §6.4, §11.4).

  Opened inline by the Add bar for a playlist, channel tab or Mix. Every entry
  becomes **one queue item with its own URL** (never a "playlist process"), so
  each video has its own progress, retry and cancel and a private one never
  aborts the rest.

  The list is virtualised with the same pure arithmetic as the queue
  (`visibleRange`): a 1 000-entry page is a dozen DOM nodes. All decisions
  (selection, ranges, filters, the requests sent) are the pure, unit-tested
  functions of `$lib/playlist`.
-->
<script lang="ts">
  import { QUALITY_PRESETS, choiceOf } from "$lib/addbar";
  import { secondsToClock, type PlaylistInfo } from "$lib/inspect";
  import {
    buildPlaylistRequests,
    defaultSelection,
    filterEntries,
    invertSelection,
    orderEntries,
    parseRange,
    playlistWarningKey,
    sanitizeFolderName,
    selectAll,
    selectBetween,
    selectNone,
    selectPositions,
    totalDuration,
  } from "$lib/playlist";
  import { qualityLabel, visibleItems, visibleRange, type QualityChoice } from "$lib/queue";
  import { addBarStore, type AddRow } from "$lib/stores/addbar.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { settingsStore, tt, ttCount } from "$lib/stores/settings.svelte";

  let { row, playlist }: { row: AddRow; playlist: PlaylistInfo } = $props();

  /** Fixed row height: the virtualiser needs one, and a title never wraps. */
  const ROW_HEIGHT = 34;
  const VIEWPORT_HEIGHT = 300;

  // The initial selection is taken once, from the playlist as first inspected;
  // "Load more" adds rows without touching what the user already picked.
  // svelte-ignore state_referenced_locally
  let selected = $state<Set<number>>(defaultSelection(playlist));
  let query = $state("");
  let rangeText = $state("");
  let rangeInvalid = $state(false);
  let reversed = $state(false);
  let folderPerPlaylist = $state(true);
  let numbering = $state(true);
  // Spec §6.4: "skip already downloaded" is on by default for playlists, and
  // unticking it is the "ignore the archive for this run" the spec asks for.
  // svelte-ignore state_referenced_locally
  let useArchive = $state(true);
  let presetValue = $state("max1080");
  let anchor = $state<number | null>(null);
  let scrollTop = $state(0);
  let busy = $state(false);

  const visible = $derived(orderEntries(filterEntries(playlist.entries, query), reversed));
  const range = $derived(visibleRange(scrollTop, VIEWPORT_HEIGHT, ROW_HEIGHT, visible.length));
  const shown = $derived(visibleItems(visible, range));
  const warningKey = $derived(playlistWarningKey(playlist));
  const duration = $derived(secondsToClock(totalDuration(playlist.entries)));
  const folderName = $derived(sanitizeFolderName(playlist.title));
  const quality = $derived<QualityChoice>(
    QUALITY_PRESETS.find((preset) => preset.value === presetValue)?.choice ?? {
      kind: "maxHeight",
      height: 1080,
    },
  );

  function qualityText(choice: QualityChoice): string {
    const label = qualityLabel(choice);
    return tt(label.key, label.params);
  }

  function toggle(index: number, checked: boolean, shift: boolean): void {
    if (shift && anchor !== null) {
      selected = selectBetween(selected, visible, anchor, index, checked);
    } else {
      const next = new Set(selected);
      if (checked) next.add(index);
      else next.delete(index);
      selected = next;
    }
    anchor = index;
  }

  function applyRange(): void {
    const max = playlist.entries.reduce((top, entry) => Math.max(top, entry.index), 0);
    const positions = parseRange(rangeText, max);
    rangeInvalid = positions === null && rangeText.trim() !== "";
    if (positions !== null) selected = selectPositions(playlist.entries, positions);
  }

  async function add(): Promise<void> {
    if (selected.size === 0 || busy) return;
    busy = true;
    try {
      const groupId = `g-${Date.now().toString(36)}-${row.key}`;
      const options = {
        groupId,
        quality,
        outputDir: addBarStore.folder,
        folderPerPlaylist,
        numbering,
        useArchive,
      };
      const requests = buildPlaylistRequests(playlist, selected, options);
      if (requests.length === 0) return;
      // The group header (title, folder) goes in first, so the items land
      // under it from their very first event.
      await queueStore.upsertGroup({
        id: groupId,
        title: playlist.title || tt("queue.playlist.title"),
        folder: folderPerPlaylist ? folderName : null,
        index: queueStore.groups.length,
      });
      const result = await queueStore.add(requests);
      if (result.added.length > 0) {
        queueStore.flash(result.added);
        addBarStore.removeRow(row.key);
      } else if (result.duplicates.length === 0) {
        await queueStore.removeGroup(groupId);
      }
    } finally {
      busy = false;
    }
  }
</script>

<section class="pl" aria-label={tt("queue.playlist.title")}>
  <header class="pl__head">
    <h3 class="pl__title">{playlist.title || tt("queue.playlist.title")}</h3>
    <p class="pl__meta">
      {#if playlist.site}<span class="pl__site">{playlist.site}</span>{/if}
      {#if playlist.count !== null}
        {ttCount("playlist.count", playlist.count)}
      {:else}
        {ttCount("playlist.count", playlist.entries.length)}
      {/if}
      {#if duration !== null}· {tt("playlist.duration", { duration })}{/if}
    </p>
  </header>

  {#if warningKey !== null}
    <p class="pl__warning" role="status">{tt(warningKey, { count: selected.size })}</p>
  {/if}

  <div class="pl__tools">
    <button type="button" class="btn btn--small" onclick={() => (selected = selectAll(selected, visible))}>
      {tt("playlist.select.all")}
    </button>
    <button type="button" class="btn btn--small" onclick={() => (selected = selectNone(selected, visible))}>
      {tt("playlist.select.none")}
    </button>
    <button type="button" class="btn btn--small" onclick={() => (selected = invertSelection(selected, visible))}>
      {tt("playlist.select.invert")}
    </button>
    <label class="pl__check">
      <input type="checkbox" bind:checked={reversed} />
      {tt("playlist.reverse")}
    </label>
  </div>

  <div class="pl__filters">
    <div class="field">
      <label class="field__label" for={`pl-filter-${row.key}`}>{tt("playlist.filter.label")}</label>
      <input
        class="input"
        id={`pl-filter-${row.key}`}
        type="search"
        autocomplete="off"
        placeholder={tt("playlist.filter.placeholder")}
        bind:value={query}
        oninput={() => (scrollTop = 0)}
      />
    </div>
    <div class="field">
      <label class="field__label" for={`pl-range-${row.key}`}>{tt("playlist.range.label")}</label>
      <div class="pl__range">
        <input
          class="input"
          id={`pl-range-${row.key}`}
          type="text"
          autocomplete="off"
          spellcheck="false"
          placeholder={tt("playlist.range.placeholder")}
          aria-invalid={rangeInvalid}
          bind:value={rangeText}
          onkeydown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              applyRange();
            }
          }}
        />
        <button type="button" class="btn btn--small" onclick={applyRange}>
          {tt("playlist.range.apply")}
        </button>
      </div>
      {#if rangeInvalid}
        <p class="add__error" role="alert">{tt("playlist.range.invalid")}</p>
      {/if}
    </div>
  </div>

  <div
    class="pl__list"
    role="group"
    aria-label={tt("playlist.list.label")}
    style:height={`${VIEWPORT_HEIGHT}px`}
    onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
  >
    <div style:height={`${range.topPad}px`}></div>
    {#each shown as entry (entry.index)}
      <label class="pl__row" class:pl__row--off={!entry.available} style:height={`${ROW_HEIGHT}px`}>
        <input
          type="checkbox"
          disabled={!entry.available}
          checked={entry.available && selected.has(entry.index)}
          onclick={(event) => toggle(entry.index, event.currentTarget.checked, event.shiftKey)}
        />
        <span class="pl__num">{entry.index}</span>
        <span class="pl__name" title={entry.title}>{entry.title || entry.url}</span>
        {#if !entry.available}
          <span class="pl__badge">{tt("playlist.unavailable")}</span>
        {:else if entry.duration !== null}
          <span class="pl__dur">{secondsToClock(entry.duration)}</span>
        {/if}
      </label>
    {/each}
    <div style:height={`${range.bottomPad}px`}></div>
  </div>

  {#if playlist.truncated}
    <div class="pl__more">
      <p class="field__hint">{tt("playlist.truncated")}</p>
      <button
        type="button"
        class="btn btn--small"
        disabled={row.loadingMore}
        onclick={() => void addBarStore.loadMore(row.key)}
      >
        {row.loadingMore ? tt("playlist.loadingMore") : tt("playlist.loadMore")}
      </button>
    </div>
  {/if}

  <div class="pl__options">
    <div class="field">
      <label class="field__label" for={`pl-quality-${row.key}`}>{tt("playlist.option.quality")}</label>
      <select
        class="select"
        id={`pl-quality-${row.key}`}
        value={presetValue}
        onchange={(event) => {
          const value = event.currentTarget.value;
          if (choiceOf(QUALITY_PRESETS, value) !== null) presetValue = value;
        }}
      >
        {#each QUALITY_PRESETS as preset (preset.value)}
          <option value={preset.value}>{qualityText(preset.choice)}</option>
        {/each}
      </select>
      <p class="field__hint">{tt("playlist.option.quality.hint")}</p>
    </div>

    <div class="pl__checks">
      <label class="pl__check">
        <input type="checkbox" bind:checked={folderPerPlaylist} />
        {tt("playlist.option.folder")}
      </label>
      {#if folderPerPlaylist}
        <p class="field__hint">{tt("playlist.option.folder.preview", { name: folderName })}</p>
      {/if}
      <label class="pl__check">
        <input type="checkbox" bind:checked={numbering} />
        {tt("playlist.option.numbering")}
      </label>
      <label class="pl__check">
        <input type="checkbox" bind:checked={useArchive} />
        {tt("playlist.option.archive")}
      </label>
      {#if useArchive && !settingsStore.settings.useDownloadArchive}
        <p class="field__hint">{tt("playlist.option.archive.hint")}</p>
      {/if}
    </div>
  </div>

  <footer class="pl__foot">
    <p class="field__hint" role="status">
      {tt("playlist.selected", { selected: selected.size, total: playlist.entries.length })}
    </p>
    <div class="pl__actions">
      <button type="button" class="btn" onclick={() => addBarStore.removeRow(row.key)}>
        {tt("playlist.cancel")}
      </button>
      <button
        type="button"
        class="btn btn--primary"
        disabled={selected.size === 0 || busy}
        onclick={() => void add()}
      >
        {ttCount("playlist.add", selected.size)}
      </button>
    </div>
  </footer>
</section>

<style>
  .pl {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
  }

  .pl__title {
    font-size: 15px;
  }

  .pl__meta {
    color: var(--text-muted);
    font-size: 12px;
  }

  .pl__site {
    margin-right: var(--space-1);
    padding: 0 var(--space-1);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
  }

  .pl__warning {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    font-size: 13px;
  }

  .pl__tools,
  .pl__filters,
  .pl__options,
  .pl__foot,
  .pl__more,
  .pl__actions,
  .pl__range {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: flex-end;
  }

  .pl__foot,
  .pl__more {
    justify-content: space-between;
    align-items: center;
  }

  .pl__checks {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .pl__check {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: 13px;
  }

  .pl__list {
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .pl__row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: 0 var(--space-2);
    box-sizing: border-box;
    cursor: pointer;
  }

  .pl__row:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .pl__row--off {
    color: var(--text-muted);
    cursor: not-allowed;
  }

  .pl__num {
    min-width: 3ch;
    color: var(--text-muted);
    font-size: 12px;
    text-align: right;
  }

  .pl__name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pl__dur,
  .pl__badge {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }
</style>
