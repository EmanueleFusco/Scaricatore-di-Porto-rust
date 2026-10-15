<!--
  The Add bar (spec §2.2, §11.3).

  One input takes one or many links. Each becomes a row: spinner → title, site,
  duration, quality picker. Errors are inline per row with a Retry (never a
  modal, spec §11.3). Drag-and-drop takes text or a `.txt` file (spec §2.2).

  "More options", presets and the transcript options of spec §11.3 arrive with
  M3/M4: they need the options that do not exist yet.
-->
<script lang="ts">
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import PlusIcon from "lucide-svelte/icons/plus";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { secondsToClock } from "$lib/inspect";
  import { choiceOf, qualityLabel, qualityOptions, qualityValue, splitLinks } from "$lib/addbar";
  import { addBarStore, type AddRow } from "$lib/stores/addbar.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { errorText, isTauriRuntime, settingsStore, tt } from "$lib/stores/settings.svelte";
  import type { QualityChoice } from "$lib/queue";

  let adding = $state(false);
  let copied = $state(false);
  let pasteBlocked = $state(false);

  const rows = $derived(addBarStore.rows);
  const readyCount = $derived(addBarStore.readyCount);
  const folderText = $derived(
    addBarStore.folder ?? settingsStore.settings.defaultOutputDir ?? tt("queue.folder.system"),
  );
  const duplicates = $derived(queueStore.pendingDuplicates);

  /**
   * Drag-and-drop has two halves (spec §2.2). Dropped **text** arrives as an
   * HTML5 `drop` on the page; dropped **files** arrive as Tauri's own event,
   * with paths only. Both are needed: a link dragged out of a browser is text,
   * a list saved from a website is a file.
   */
  onMount(() => {
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") {
          addBarStore.dragging = true;
        } else if (event.payload.type === "drop") {
          addBarStore.dragging = false;
          for (const path of event.payload.paths) {
            void addBarStore.dropFile(path);
          }
        } else {
          addBarStore.dragging = false;
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => unlisten?.();
  });

  async function browse(): Promise<void> {
    const picked = await open({ multiple: false, directory: true, title: tt("queue.folder.browse") });
    if (typeof picked === "string") {
      addBarStore.folder = picked;
    }
  }

  async function addAll(): Promise<void> {
    const requests = addBarStore.toRequests();
    if (requests.length === 0) return;
    adding = true;
    try {
      const result = await queueStore.add(requests);
      if (result.added.length > 0) {
        addBarStore.clearAdded();
        queueStore.dismissDuplicates();
      }
    } finally {
      adding = false;
    }
  }

  async function addAnyway(): Promise<void> {
    adding = true;
    try {
      await queueStore.addAnyway();
      addBarStore.clearAdded();
    } finally {
      adding = false;
    }
  }

  /**
   * Ctrl/Cmd+Enter adds whatever is in the box (spec §11.3), and Enter alone on
   * an empty line does nothing: a paste of five links is already there.
   */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter") return;
    if (!(event.metaKey || event.ctrlKey)) return;
    event.preventDefault();
    void addBarStore.submit();
  }

  function onDrop(event: DragEvent): void {
    event.preventDefault();
    addBarStore.dragging = false;
    const text = event.dataTransfer?.getData("text/plain");
    if (text !== undefined && text !== null && text.trim() !== "") {
      pasteBlocked = splitLinks(text).length === 0;
      void addBarStore.dropText(text);
    }
  }

  function onDragOver(event: DragEvent): void {
    event.preventDefault();
    addBarStore.dragging = true;
  }

  function onDragLeave(): void {
    addBarStore.dragging = false;
  }

  /** Ctrl/Cmd+V anywhere on the page, when the setting says so (§11.3). */
  function onWindowPaste(event: ClipboardEvent): void {
    if (!settingsStore.settings.pasteLinksAnywhere) return;
    const target = event.target as HTMLElement | null;
    if (target !== null && /^(INPUT|TEXTAREA)$/.test(target.tagName)) return;
    const text = event.clipboardData?.getData("text");
    if (text === undefined || text === null || text.trim() === "") return;
    if (splitLinks(text).length === 0) return;
    event.preventDefault();
    void addBarStore.dropText(text);
  }

  onMount(() => {
    window.addEventListener("paste", onWindowPaste);
    return () => window.removeEventListener("paste", onWindowPaste);
  });

  function options(row: AddRow): ReturnType<typeof qualityOptions> {
    const video = row.inspected?.kind === "video" ? row.inspected : null;
    return qualityOptions(video?.heights ?? [], addBarStore.audioOnly);
  }

  function choiceFor(row: AddRow): QualityChoice {
    return addBarStore.effectiveQuality(row);
  }

  async function copyLink(row: AddRow): Promise<void> {
    try {
      await navigator.clipboard.writeText(row.url);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      // Clipboard access can be denied; the link is on screen anyway.
    }
  }

  const PRESETS = [
    { value: "best", key: "queue.add.quality.best" },
    { value: "max1080", key: "queue.add.quality.max", params: { height: "1080" } },
    { value: "max720", key: "queue.add.quality.max", params: { height: "720" } },
    { value: "max480", key: "queue.add.quality.max", params: { height: "480" } },
  ];
</script>

<!--
    A plain `div`, not a `<form>`: the keyboard shortcut is handled on the
    textarea itself, and a form carrying drag handlers is a non-interactive
    element with interactive listeners (an accessibility warning, not a rule to
    break for).
  -->
  <div
  class="card add"
  class:add--dragging={addBarStore.dragging}
  role="group"
  aria-label={tt("queue.add.label")}
  ondrop={onDrop}
  ondragover={onDragOver}
  ondragleave={onDragLeave}
>
  <div class="field">
    <label class="field__label" for="add-links">{tt("queue.add.label")}</label>
    <textarea
      class="input add__input"
      id="add-links"
      rows="2"
      spellcheck="false"
      autocomplete="off"
      placeholder={tt("queue.add.placeholder")}
      value={addBarStore.text}
      onkeydown={onKeydown}
      oninput={(event) => addBarStore.setText(event.currentTarget.value)}
    ></textarea>
    <p class="field__hint">
      {addBarStore.dragging ? tt("queue.add.drop") : tt("queue.add.hint")}
    </p>
    {#if pasteBlocked}
      <p class="field__hint" role="status">{tt("queue.add.none")}</p>
    {/if}
    {#if addBarStore.errorKey}
      <p class="add__error" role="alert">
        {errorText(addBarStore.errorKey, addBarStore.errorDetail)}
      </p>
    {/if}
  </div>

  <div class="add__controls">
    <div class="field">
      <label class="field__label" for="add-preset">{tt("queue.add.quality")}</label>
      <select
        class="select"
        id="add-preset"
        value={addBarStore.preset}
        disabled={addBarStore.audioOnly}
        onchange={(event) => (addBarStore.preset = event.currentTarget.value)}
      >
        {#each PRESETS as preset (preset.value)}
          <option value={preset.value}>
            {tt(preset.key, preset.params ?? undefined)}
          </option>
        {/each}
      </select>
    </div>

    <label class="add__switch">
      <input
        type="checkbox"
        checked={addBarStore.audioOnly}
        onchange={(event) => (addBarStore.audioOnly = event.currentTarget.checked)}
      />
      {tt("queue.add.audio")}
    </label>

    <div class="field">
      <span class="field__label">{tt("queue.folder.label")}</span>
      <div class="add__folder">
        <output class="settings__value">{folderText}</output>
        <button type="button" class="btn" onclick={() => void browse()}>
          <FolderOpenIcon size={16} aria-hidden="true" focusable="false" />
          {tt("queue.folder.browse")}
        </button>
      </div>
    </div>
  </div>

  {#if rows.length > 0}
    <ul class="add__rows" aria-label={tt("queue.add.label")}>
      {#each rows as row (row.key)}
        {@const video = row.inspected?.kind === "video" ? row.inspected : null}
        <li class="add__row" data-state={row.state}>
          <div class="add__row-main">
            {#if row.state === "checking" || row.state === "pending"}
              <span class="add__spinner" aria-hidden="true"></span>
              <span class="add__row-title">{tt("queue.add.inspecting")}</span>
            {:else if row.state === "ready"}
              <span class="add__row-title">{video?.title ?? row.url}</span>
              <span class="add__row-meta">
                {#if video !== null && video.site}
                  <span class="add__site">{video.site}</span>
                {/if}
                {#if video !== null && video.duration !== null}
                  · {secondsToClock(video.duration)}
                {/if}
                {#if video?.isLive}
                  · <span class="add__live">{tt("queue.badge.live")}</span>
                {/if}
              </span>
            {:else if row.state === "skipped"}
              <span class="add__row-title add__row-title--warn">{row.url}</span>
            {:else}
              <span class="add__row-title add__row-title--error">{row.url}</span>
            {/if}
          </div>

          <div class="add__row-actions">
            {#if row.state === "ready"}
              <select
                class="select select--small"
                aria-label={tt("queue.add.quality")}
                value={qualityValue(choiceFor(row))}
                onchange={(event) =>
                  addBarStore.setQuality(
                    row.key,
                    choiceOf(options(row), event.currentTarget.value),
                  )}
              >
                {#each options(row) as option (option.value)}
                  <option value={option.value}>{tt(qualityLabel(option.choice))}</option>
                {/each}
              </select>
            {/if}
            {#if row.state === "error" || row.state === "skipped"}
              <button type="button" class="btn btn--small" onclick={() => void addBarStore.retryRow(row.key)}>
                {tt("queue.add.retry")}
              </button>
            {/if}
            {#if row.errorKey}
              <span class="add__row-error">{errorText(row.errorKey, null)}</span>
            {/if}
            <button type="button" class="btn btn--small" onclick={() => void copyLink(row)}>
              {copied ? tt("queue.copied") : tt("queue.list.copyLink")}
            </button>
            <button
              type="button"
              class="btn btn--small"
              onclick={() => addBarStore.removeRow(row.key)}
            >
              {tt("queue.add.remove")}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  {#if duplicates.length > 0}
    <div class="add__duplicates" role="alert">
      <p class="add__duplicates-title">{tt("queue.add.duplicate.title")}</p>
      <p class="field__hint">
        {tt("queue.add.duplicate.body", { count: duplicates.length })}
      </p>
      <div class="add__actions">
        <button type="button" class="btn btn--primary" onclick={() => void addAnyway()}>
          {tt("queue.add.duplicate.confirm")}
        </button>
        <button type="button" class="btn" onclick={() => queueStore.dismissDuplicates()}>
          {tt("queue.add.duplicate.cancel")}
        </button>
      </div>
    </div>
  {/if}

  <div class="add__footer">
    <p class="field__hint" role="status">
      {#if readyCount > 0}
        {tt("queue.add.ready", { count: readyCount })}
      {:else if rows.length > 0}
        {tt("queue.add.none")}
      {/if}
    </p>
    <div class="add__actions">
      {#if addBarStore.hasRows}
        <button type="button" class="btn" onclick={() => addBarStore.clear()}>
          {tt("queue.clear")}
        </button>
      {/if}
      <button
        type="button"
        class="btn btn--primary"
        disabled={readyCount === 0 || adding}
        onclick={() => void addAll()}
      >
        <PlusIcon size={16} aria-hidden="true" focusable="false" />
        {adding ? tt("tools.action.installingUnknown") : tt("queue.add.button")}
      </button>
    </div>
  </div>
</div>

<style>
  .add {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }

  .add--dragging {
    border-color: var(--accent);
  }

  .add__input {
    min-height: 64px;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: 13px;
  }

  .add__controls {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    align-items: flex-end;
  }

  .add__switch {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: 13px;
  }

  .add__folder {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .add__rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 260px;
    overflow-y: auto;
  }

  .add__row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .add__row[data-state="error"] {
    border-color: var(--danger);
  }

  .add__row-main {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    min-width: 0;
    flex: 1 1 auto;
  }

  .add__row-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .add__row-title--error {
    color: var(--danger);
  }

  .add__row-title--warn {
    color: var(--text-muted);
  }

  .add__row-meta {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .add__row-error {
    color: var(--danger);
    font-size: 11px;
  }

  .add__row-actions {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    flex-wrap: wrap;
  }

  .add__site {
    padding: 0 var(--space-1);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
  }

  .add__live {
    color: var(--danger);
    font-weight: 700;
  }

  .add__spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: var(--radius-pill);
    animation: add-spin 700ms linear infinite;
    flex: none;
  }

  @keyframes add-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .add__spinner {
      animation: none;
    }
  }

  .select--small {
    height: 28px;
    font-size: 12px;
    padding: 0 var(--space-2);
  }

  .btn--small {
    height: 28px;
    font-size: 12px;
    padding: 0 var(--space-2);
  }

  .add__duplicates {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
  }

  .add__duplicates-title {
    font-weight: 700;
  }

  .add__footer {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    justify-content: space-between;
  }

  .add__actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .add__error {
    color: var(--danger);
    font-size: 12px;
  }
</style>
