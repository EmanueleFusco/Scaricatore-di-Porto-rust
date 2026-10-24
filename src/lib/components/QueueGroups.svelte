<!--
  Playlist group headers (spec §6.4, §11.5).

  One compact row per playlist that still has items: a collapse toggle, the
  title and folder, "7/42" with a thin progress bar, and the group actions
  (Pause, Resume, Retry failed, Open folder, Remove). The items themselves stay
  in the virtualised list below; collapsing a group only filters them out, so
  the virtualiser's one-height arithmetic never has to know about headers.
-->
<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { groupPercent, groupStats, nonEmptyGroups, type QueueItem } from "$lib/queue";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  let {
    collapsed,
    ontoggle,
    items = queueStore.list,
  }: {
    collapsed: ReadonlySet<string>;
    ontoggle: (id: string) => void;
    /** Rows left after the filter, so a group with no match disappears too. */
    items?: QueueItem[];
  } = $props();

  const groups = $derived(nonEmptyGroups(queueStore.groups, items));
  const stats = $derived(new Map(groups.map((group) => [group.id, groupStats(group.id, items)])));
</script>

{#if groups.length > 0}
  <ul class="groups" aria-label={tt("queue.group.list")}>
    {#each groups as group (group.id)}
      {@const s = stats.get(group.id)}
      {#if s !== undefined}
        <li class="group">
          <button
            type="button"
            class="group__toggle"
            aria-expanded={!collapsed.has(group.id)}
            aria-label={collapsed.has(group.id) ? tt("queue.group.expand") : tt("queue.group.collapse")}
            onclick={() => ontoggle(group.id)}
          >
            <span aria-hidden="true">{collapsed.has(group.id) ? "▸" : "▾"}</span>
          </button>
          <div class="group__main">
            <span class="group__title">{group.title}</span>
            <span class="group__meta">
              {tt("queue.group.progress", { done: s.done, total: s.total })}
              {#if s.failed > 0}· {tt("queue.group.failed", { count: s.failed })}{/if}
              {#if group.folder}· {group.folder}{/if}
            </span>
            <div
              class="group__bar"
              role="progressbar"
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={groupPercent(s)}
              aria-label={group.title}
            >
              <div class="group__fill" style:width={`${groupPercent(s)}%`}></div>
            </div>
          </div>
          <div class="group__actions">
            {#if s.active > 0}
              <button type="button" class="btn btn--small" onclick={() => void queueStore.pauseGroup(group.id)}>
                {tt("queue.group.pause")}
              </button>
            {/if}
            {#if s.paused > 0}
              <button type="button" class="btn btn--small" onclick={() => void queueStore.resumeGroup(group.id)}>
                {tt("queue.group.resume")}
              </button>
            {/if}
            {#if s.failed > 0}
              <button
                type="button"
                class="btn btn--small"
                onclick={() => void queueStore.retryGroupFailed(group.id)}
              >
                {tt("queue.group.retryFailed")}
              </button>
            {/if}
            {#if s.firstFile !== null}
              <button
                type="button"
                class="btn btn--small"
                onclick={() => s.firstFile !== null && void revealItemInDir(s.firstFile).catch(() => {})}
              >
                {tt("queue.group.openFolder")}
              </button>
            {/if}
            <button type="button" class="btn btn--small btn--danger" onclick={() => void queueStore.removeGroup(group.id)}>
              {tt("queue.group.remove")}
            </button>
          </div>
        </li>
      {/if}
    {/each}
  </ul>
{/if}

<style>
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: none;
    max-height: 168px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .group {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    flex-wrap: wrap;
  }

  .group__toggle {
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .group__main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1 1 200px;
    min-width: 0;
  }

  .group__title {
    overflow: hidden;
    font-weight: 700;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .group__meta {
    overflow: hidden;
    color: var(--text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .group__bar {
    height: 4px;
    border-radius: var(--radius-pill);
    background: var(--surface-2);
    overflow: hidden;
  }

  .group__fill {
    height: 100%;
    background: var(--accent);
  }

  .group__actions {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
</style>
