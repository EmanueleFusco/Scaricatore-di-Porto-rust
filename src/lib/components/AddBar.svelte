<!--
  The Add bar (spec §2.2, §11.3).

  One input takes one or many links; inspection starts on its own, debounced
  500 ms as in the original, and every link becomes a row: spinner → title,
  site, duration, quality picker. Errors are inline per row with a Retry (never
  a modal, spec §11.3). Drag-and-drop takes text or a `.txt` file (spec §2.2).

  Two actions, because they are two things: **Scarica** starts the links now
  (a single link is the standard download, priority over what is already
  waiting) and **Aggiungi alla coda** lines them up behind the others.

  M3 (spec §6.4, §11.3, §11.4) adds, per link: the inline *This video only* /
  *Whole playlist* choice for `watch?v=…&list=…`, the playlist selection panel,
  the live-stream *From now* / *From start* choice, the advanced format table,
  a "More options" box (referer, user agent, headers) and the "Use cookies"
  banner on an authentication failure. The transcript options arrive with M4.
-->
<script lang="ts">
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import DownloadIcon from "lucide-svelte/icons/download";
  import PlusIcon from "lucide-svelte/icons/plus";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import CookieBanner from "$lib/components/CookieBanner.svelte";
  import PlaylistPanel from "$lib/components/PlaylistPanel.svelte";
  import { secondsToClock } from "$lib/inspect";
  import {
    AUDIO_CODEC_OPTIONS,
    QUALITY_PRESETS,
    SPONSORBLOCK_CATEGORY_KEYS,
    choiceOf,
    explicitChoiceFor,
    parseHeaderLines,
    qualityValue,
    rowLanguageOptions,
    rowQualityOptions,
    splitLinks,
    tableFormats,
    validateTrimInput,
  } from "$lib/addbar";
  import { addBarStore, type AddRow } from "$lib/stores/addbar.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { errorText, isTauriRuntime, settingsStore, tt, ttCount } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { modelsStore } from "$lib/stores/models.svelte";
  import { formatBytes, qualityLabel, type QualityChoice } from "$lib/queue";

  /** Which of the two actions a click asked for. */
  type AddMode = "download" | "queue";

  let busy = $state<"" | "checking" | "adding">("");
  /** The row whose link was just copied, so only that button says so. */
  let copiedKey = $state<string | null>(null);
  let pasteBlocked = $state(false);
  /** One-shot: the next input comes from a paste, so it skips the debounce. */
  let pasted = $state(false);

  const rows = $derived(addBarStore.rows);
  const readyCount = $derived(addBarStore.readyCount);
  /** A ready row has an invalid trim range: submitting it must be blocked (spec §9.2). */
  const hasTrimError = $derived(
    addBarStore.ready.some(
      (row) =>
        trimError(row, row.inspected?.kind === "video" ? (row.inspected.duration ?? null) : null) !==
        null,
    ),
  );
  /** There is something to act on: a ready row, or text still to be checked. */
  const actionable = $derived(
    !hasTrimError && (readyCount > 0 || addBarStore.links.length > 0),
  );
  const folderText = $derived(
    addBarStore.folder ?? settingsStore.settings.defaultOutputDir ?? tt("queue.folder.system"),
  );
  const duplicates = $derived(queueStore.pendingDuplicates);
  const primaryLabel = $derived(
    busy === "checking"
      ? tt("queue.add.inspecting")
      : busy === "adding"
        ? tt("queue.add.adding")
        : tt("queue.add.download"),
  );

  /**
   * Drag-and-drop has two halves (spec §2.2). Dropped **text** arrives as an
   * HTML5 `drop` on the page; dropped **files** arrive as Tauri's own event,
   * with paths only. Both are needed: a link dragged out of a browser is text,
   * a list saved from a website is a file.
   */
  onMount(() => {
    void toolsStore.probeEncoders();
    void toolsStore.refresh();
    void modelsStore.init();
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

  async function pickLocalFileToTranscribe(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      title: tt("transcribe.pickLocalFile"),
      filters: [
        {
          name: "Media",
          extensions: [
            "mp4", "mkv", "webm", "avi", "mov", "flv", "wmv",
            "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus",
          ],
        },
      ],
    });
    if (typeof picked === "string") {
      await queueStore.addLocalFile(picked);
    }
  }

  /**
   * The two add actions.
   *
   * Whatever is still in the box is checked first and awaited, so the buttons
   * work the moment a link is pasted — never only after the debounce has
   * happened to fire. `download` then moves the new items to the top of the
   * queue; `queue` leaves them at the end.
   */
  async function addAll(mode: AddMode): Promise<void> {
    if (addBarStore.links.length > 0) {
      busy = "checking";
      try {
        await addBarStore.submit();
      } finally {
        busy = "";
      }
    }
    const requests = addBarStore.toRequests();
    if (requests.length === 0) return;

    busy = "adding";
    try {
      const result = await queueStore.add(requests);
      if (result.added.length > 0) {
        addBarStore.clearAdded();
        queueStore.dismissDuplicates();
        queueStore.flash(result.added);
        if (mode === "download") await queueStore.prioritize(result.added);
      }
    } finally {
      busy = "";
    }
  }

  async function addAnyway(): Promise<void> {
    busy = "adding";
    try {
      const result = await queueStore.addAnyway();
      addBarStore.clearAdded();
      queueStore.flash(result.added);
    } finally {
      busy = "";
    }
  }

  /** Ctrl/Cmd+Enter downloads whatever is in the box (spec §11.3). */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter") return;
    if (!(event.metaKey || event.ctrlKey)) return;
    event.preventDefault();
    void addAll("download");
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

  function onDragLeave(event: DragEvent): void {
    const current = event.currentTarget as HTMLElement | null;
    const related = event.relatedTarget as Node | null;
    if (current !== null && related !== null && current.contains(related)) {
      return;
    }
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

  /** The picker of one row: the video's heights, plus the quality in force. */
  function options(row: AddRow): ReturnType<typeof rowQualityOptions> {
    const video = row.inspected?.kind === "video" ? row.inspected : null;
    return rowQualityOptions(video?.heights ?? [], addBarStore.effectiveQuality(row));
  }

  function choiceFor(row: AddRow): QualityChoice {
    return addBarStore.effectiveQuality(row);
  }

  /** The localised text of a choice, heights included. */
  function qualityText(choice: QualityChoice): string {
    const label = qualityLabel(choice);
    return tt(label.key, label.params);
  }

  async function copyLink(row: AddRow): Promise<void> {
    try {
      await navigator.clipboard.writeText(row.url);
      copiedKey = row.key;
      setTimeout(() => {
        if (copiedKey === row.key) copiedKey = null;
      }, 2000);
    } catch {
      // Clipboard access can be denied; the link is on screen anyway.
    }
  }

  function trimError(row: AddRow, duration: number | null): string | null {
    const res = validateTrimInput(row.trimStart, row.trimEnd, duration);
    return res.valid ? null : res.errorKey;
  }
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
      onpaste={() => (pasted = true)}
      oninput={(event) => {
        pasteBlocked = false;
        addBarStore.setText(event.currentTarget.value, pasted);
        pasted = false;
      }}
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
        onchange={(event) => (addBarStore.preset = event.currentTarget.value)}
      >
        {#each QUALITY_PRESETS as preset (preset.value)}
          <option value={preset.value}>{qualityText(preset.choice)}</option>
        {/each}
      </select>
    </div>

    <div class="field">
      <span class="field__label">{tt("queue.folder.label")}</span>
      <div class="add__folder">
        <output class="settings__value">{folderText}</output>
        <button type="button" class="btn" onclick={() => void browse()}>
          <FolderOpenIcon size={16} aria-hidden="true" focusable="false" />
          {tt("queue.folder.browse")}
        </button>
        <button type="button" class="btn" onclick={() => void pickLocalFileToTranscribe()}>
          {tt("transcribe.pickLocalFile")}
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
            {:else if row.state === "choice"}
              <span class="add__row-title">{row.url}</span>
            {:else if row.state === "playlist"}
              <span class="add__row-title">
                {row.inspected?.kind === "playlist" ? row.inspected.title : row.url}
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
                  <option value={option.value}>{qualityText(option.choice)}</option>
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
              {copiedKey === row.key ? tt("queue.copied") : tt("queue.list.copyLink")}
            </button>
            <button
              type="button"
              class="btn btn--small"
              onclick={() => addBarStore.removeRow(row.key)}
            >
              {tt("queue.add.remove")}
            </button>
          </div>

          {#if row.state === "choice"}
            <div class="add__choice" role="group" aria-label={tt("queue.add.choice.title")}>
              <p class="field__hint">{tt("queue.add.choice.title")}</p>
              <div class="add__actions">
                <button
                  type="button"
                  class="btn btn--small btn--primary"
                  onclick={() => void addBarStore.choosePlaylistMode(row.key, "video")}
                >
                  {tt("queue.add.choice.video")}
                </button>
                <button
                  type="button"
                  class="btn btn--small"
                  onclick={() => void addBarStore.choosePlaylistMode(row.key, "playlist")}
                >
                  {tt("queue.add.choice.playlist")}
                </button>
              </div>
              <label class="add__check">
                <input type="checkbox" bind:checked={addBarStore.rememberChoice} />
                {tt("queue.add.choice.remember")}
              </label>
            </div>
          {/if}

          {#if row.state === "playlist" && row.inspected?.kind === "playlist"}
            <div class="add__block">
              <PlaylistPanel {row} playlist={row.inspected} />
            </div>
          {/if}

          {#if row.state === "error" && row.inspected?.kind === "error" && row.inspected.code === "AUTH"}
            <CookieBanner onApplied={() => void addBarStore.retryRow(row.key)} />
          {/if}

          {#if row.state === "ready" && video !== null}
            {#if video.isLive}
              <fieldset class="add__block add__live-mode">
                <legend class="field__label">{tt("queue.add.live.title")}</legend>
                <label class="add__check">
                  <input
                    type="radio"
                    name={`live-${row.key}`}
                    checked={!row.liveFromStart}
                    onchange={() => addBarStore.setLiveFromStart(row.key, false)}
                  />
                  {tt("queue.add.live.now")}
                </label>
                <label class="add__check">
                  <input
                    type="radio"
                    name={`live-${row.key}`}
                    checked={row.liveFromStart}
                    onchange={() => addBarStore.setLiveFromStart(row.key, true)}
                  />
                  {tt("queue.add.live.start")}
                </label>
              </fieldset>
            {/if}

            {#if video.formats.length > 0}
              <details class="add__block">
                <summary>{tt("queue.add.formats.title")}</summary>
                <div class="add__table-wrap">
                  <table class="add__table">
                    <thead>
                      <tr>
                        <th scope="col">{tt("queue.add.formats.id")}</th>
                        <th scope="col">{tt("queue.add.formats.ext")}</th>
                        <th scope="col">{tt("queue.add.formats.resolution")}</th>
                        <th scope="col">{tt("queue.add.formats.codecs")}</th>
                        <th scope="col">{tt("queue.add.formats.size")}</th>
                        <th scope="col"><span class="sr-only">{tt("queue.add.formats.pick")}</span></th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each tableFormats(video.formats) as format (format.formatId)}
                        <tr>
                          <td>{format.formatId}</td>
                          <td>{format.ext}</td>
                          <td>
                            {format.height !== null ? `${format.height}p` : "—"}{format.fps !== null && format.fps > 30 ? ` ${Math.round(format.fps)}` : ""}
                          </td>
                          <td>{[format.vcodec, format.acodec].filter((c) => c !== null && c !== "none").join(" + ") || "—"}</td>
                          <td>{format.filesize !== null ? formatBytes(format.filesize) : "—"}</td>
                          <td>
                            <button
                              type="button"
                              class="btn btn--small"
                              onclick={() => addBarStore.setQuality(row.key, explicitChoiceFor(format))}
                            >
                              {tt("queue.add.formats.pick")}
                            </button>
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
                <p class="field__hint">{tt("queue.add.formats.hint")}</p>
              </details>
            {/if}
          {/if}

          {#if row.state === "ready" || row.state === "error" || row.state === "skipped"}
            <details class="add__block">
              <summary>{tt("queue.add.more.title")}</summary>
              <div class="add__more">
                {#if row.state === "ready"}
                  <!-- Spec §9: Encode and trim -->
                  <div class="field">
                    <span class="field__label">{tt("queue.add.more.encode.title")}</span>
                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.encode}
                        disabled={row.trimStart.trim() !== "" || row.trimEnd.trim() !== ""}
                        aria-describedby={`trim-hint-${row.key}`}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { encode: event.currentTarget.checked })}
                      />
                      {tt("queue.add.more.encode.toggle")}
                    </label>

                    {#if row.encode}
                      <div class="add__controls">
                        {#if choiceFor(row).kind !== "audioOnly"}
                          <div class="field">
                            <label class="field__label" for={`more-vcodec-${row.key}`}>
                              {tt("queue.add.more.encode.videoCodec")}
                            </label>
                            <select
                              class="select"
                              id={`more-vcodec-${row.key}`}
                              value={row.videoCodec ?? settingsStore.settings.defaultVideoCodec ?? "libx264"}
                              onchange={(event) =>
                                addBarStore.setMediaOptions(row.key, { videoCodec: event.currentTarget.value })}
                            >
                              {#if toolsStore.probingEncoders}
                                <option disabled value="">{tt("queue.add.more.encode.probing")}</option>
                              {/if}
                              {#if toolsStore.probedEncoders.length > 0}
                                {#each toolsStore.probedEncoders as encoder (encoder.id)}
                                  <option value={encoder.id}>{encoder.label}</option>
                                {/each}
                              {:else}
                                <option value="libx264">x264 (CPU)</option>
                                <option value="libx265">x265 (CPU)</option>
                              {/if}
                            </select>
                          </div>
                        {/if}

                        <div class="field">
                          <label class="field__label" for={`more-acodec-${row.key}`}>
                            {tt("queue.add.more.encode.audioCodec")}
                          </label>
                          <select
                            class="select"
                            id={`more-acodec-${row.key}`}
                            value={row.audioCodec ?? settingsStore.settings.defaultAudioCodec ?? "aac"}
                            onchange={(event) =>
                              addBarStore.setMediaOptions(row.key, { audioCodec: event.currentTarget.value })}
                          >
                            {#each AUDIO_CODEC_OPTIONS as opt (opt.value)}
                              <option value={opt.value}>{opt.label}</option>
                            {/each}
                          </select>
                        </div>
                      </div>
                    {/if}

                    <div class="field">
                      <span class="field__label">{tt("queue.add.trim.title")}</span>
                      <div class="add__controls">
                        <div class="field">
                          <label class="field__label" for={`trim-start-${row.key}`}>
                            {tt("queue.add.trim.start")}
                          </label>
                          <input
                            class="input"
                            id={`trim-start-${row.key}`}
                            type="text"
                            placeholder="00:00"
                            value={row.trimStart}
                            oninput={(event) =>
                              addBarStore.setMediaOptions(row.key, { trimStart: event.currentTarget.value })}
                          />
                        </div>
                        <div class="field">
                          <label class="field__label" for={`trim-end-${row.key}`}>
                            {tt("queue.add.trim.end")}
                          </label>
                          <input
                            class="input"
                            id={`trim-end-${row.key}`}
                            type="text"
                            placeholder="00:00"
                            value={row.trimEnd}
                            oninput={(event) =>
                              addBarStore.setMediaOptions(row.key, { trimEnd: event.currentTarget.value })}
                          />
                        </div>
                      </div>
                      <p class="field__hint" id={`trim-hint-${row.key}`}>{tt("queue.add.trim.hint")}</p>
                      {#if trimError(row, video?.duration ?? null)}
                        <p class="add__row-error" role="alert">{tt(trimError(row, video?.duration ?? null)!)}</p>
                      {/if}
                    </div>

                    {#if row.encode}
                      <label class="add__check">
                        <input
                          type="checkbox"
                          checked={row.keepOriginal}
                          onchange={(event) =>
                            addBarStore.setMediaOptions(row.key, { keepOriginal: event.currentTarget.checked })}
                        />
                        {tt("queue.add.more.keepOriginal")}
                      </label>
                    {/if}

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.autoPlay}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { autoPlay: event.currentTarget.checked })}
                      />
                      {tt("queue.add.more.autoPlay")}
                    </label>
                  </div>

                  <!-- Spec §8.4: Embed metadata, thumbnail, chapters -->
                  <div class="field">
                    <span class="field__label">{tt("queue.add.embed.title")}</span>
                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.embedMetadata}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { embedMetadata: event.currentTarget.checked })}
                      />
                      {tt("queue.add.embed.metadata")}
                    </label>
                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.embedThumbnail}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { embedThumbnail: event.currentTarget.checked })}
                      />
                      {tt("queue.add.embed.thumbnail")}
                    </label>
                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.embedChapters}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { embedChapters: event.currentTarget.checked })}
                      />
                      {tt("queue.add.embed.chapters")}
                    </label>
                  </div>

                  <!-- Spec §8.4: SponsorBlock -->
                  <div class="field">
                    <span class="field__label">{tt("queue.add.sponsorblock.title")}</span>
                    <div class="field">
                      <label class="field__label" for={`sb-mode-${row.key}`}>
                        {tt("queue.add.sponsorblock.mode")}
                      </label>
                      <select
                        class="select"
                        id={`sb-mode-${row.key}`}
                        value={row.sponsorblockMode ?? "off"}
                        onchange={(event) => {
                          const val = event.currentTarget.value;
                          addBarStore.setMediaOptions(row.key, {
                            sponsorblockMode: val === "mark" || val === "remove" ? val : null,
                          });
                        }}
                      >
                        <option value="off">{tt("queue.add.sponsorblock.mode.off")}</option>
                        <option value="mark">{tt("queue.add.sponsorblock.mode.mark")}</option>
                        <option value="remove">{tt("queue.add.sponsorblock.mode.remove")}</option>
                      </select>
                    </div>

                    {#if row.sponsorblockMode === "remove"}
                      <p class="field__hint add__row-title--warn">{tt("queue.add.sponsorblock.removeWarning")}</p>
                    {/if}

                    {#if row.sponsorblockMode !== null}
                      <div class="field">
                        <span class="field__label">{tt("queue.add.sponsorblock.categories")}</span>
                        {#each SPONSORBLOCK_CATEGORY_KEYS as cat (cat)}
                          <label class="add__check">
                            <input
                              type="checkbox"
                              checked={row.sponsorblockCategories.includes(cat)}
                              disabled={row.sponsorblockCategories.length === 1 && row.sponsorblockCategories.includes(cat)}
                              onchange={(event) => {
                                const checked = event.currentTarget.checked;
                                if (!checked && row.sponsorblockCategories.length <= 1) {
                                  return;
                                }
                                const next = checked
                                  ? [...row.sponsorblockCategories, cat]
                                  : row.sponsorblockCategories.filter((c) => c !== cat);
                                addBarStore.setMediaOptions(row.key, { sponsorblockCategories: next });
                              }}
                            />
                            {tt(`sponsorblock.category.${cat}`)}
                          </label>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}

                <!-- Spec §10.2: Whisper Transcription -->
                <div class="field">
                  <span class="field__label">{tt("transcribe.title")}</span>
                  <label class="add__check">
                    <input
                      type="checkbox"
                      checked={row.transcribe}
                      onchange={(event) =>
                        addBarStore.setMediaOptions(row.key, { transcribe: event.currentTarget.checked })}
                    />
                    {tt("transcribe.toggle")}
                  </label>

                  {#if row.transcribe}
                    <div class="add__controls">
                      <div class="field">
                        <label class="field__label" for={`whisper-model-${row.key}`}>
                          {tt("transcribe.model")}
                        </label>
                        <select
                          class="select"
                          id={`whisper-model-${row.key}`}
                          value={row.whisperModel ?? settingsStore.settings.activeWhisperModel ?? "large-v3-turbo-q5_0"}
                          onchange={(event) =>
                            addBarStore.setMediaOptions(row.key, { whisperModel: event.currentTarget.value })}
                        >
                          {#if modelsStore.view.models.length > 0}
                            {#each modelsStore.view.models as model (model.id)}
                              <option value={model.id}>
                                {model.label} ({model.state.kind === "installed" ? tt("models.installed") : tt("models.notInstalled")})
                              </option>
                            {/each}
                          {:else}
                            <option value="large-v3-turbo-q5_0">Turbo (large-v3-turbo-q5_0)</option>
                            <option value="small-q5_1">Small (small-q5_1)</option>
                            <option value="base-q5_1">Base (base-q5_1)</option>
                            <option value="tiny-q5_1">Tiny (tiny-q5_1)</option>
                          {/if}
                        </select>
                        {#if modelsStore.view.models.length > 0}
                          {@const selectedModel = modelsStore.view.models.find((m) => m.id === (row.whisperModel ?? settingsStore.settings.activeWhisperModel ?? "large-v3-turbo-q5_0"))}
                          {#if selectedModel && selectedModel.state.kind !== "installed"}
                            <button
                              type="button"
                              class="btn btn--small"
                              style="margin-top: 4px;"
                              disabled={modelsStore.busy || (modelsStore.downloadingModelId !== null && modelsStore.downloadingModelId !== selectedModel.id)}
                              onclick={() => void modelsStore.download(selectedModel.id)}
                            >
                              {#if modelsStore.downloadingModelId === selectedModel.id}
                                {modelsStore.downloadPercent !== null
                                  ? tt("models.action.downloading", { percent: modelsStore.downloadPercent })
                                  : tt("models.action.downloadingUnknown")}
                              {:else}
                                {tt("job.whisperModel.download")}
                              {/if}
                            </button>
                          {/if}
                        {/if}
                        {#if toolsStore.view.tools.length > 0}
                          {@const whisperInstalled = toolsStore.view.tools.some((t) => t.name === "sdp-whisper" && t.path !== null)}
                          {#if !whisperInstalled}
                            <button
                              type="button"
                              class="btn btn--small"
                              style="margin-top: 4px;"
                              disabled={toolsStore.installingTool !== null}
                              onclick={() => void toolsStore.install("sdp-whisper")}
                            >
                              {#if toolsStore.installingTool === "sdp-whisper"}
                                {toolsStore.installPercent !== null
                                  ? tt("tools.action.installing", { percent: toolsStore.installPercent })
                                  : tt("tools.action.installingUnknown")}
                              {:else}
                                {tt("job.whisper.install")}
                              {/if}
                            </button>
                          {/if}
                        {/if}
                      </div>

                      <div class="field">
                        <label class="field__label" for={`whisper-lang-${row.key}`}>
                          {tt("transcribe.language")}
                        </label>
                        <select
                          class="select"
                          id={`whisper-lang-${row.key}`}
                          value={row.transcriptLanguage ?? settingsStore.settings.defaultTranscriptLanguage ?? "auto"}
                          onchange={(event) =>
                            addBarStore.setMediaOptions(row.key, { transcriptLanguage: event.currentTarget.value })}
                        >
                          {#each rowLanguageOptions(row.inspected?.kind === "video" ? row.inspected.language : null, row.transcriptLanguage) as opt (opt.value)}
                            <option value={opt.value}>
                              {opt.labelKey ? (opt.value === "auto" ? tt(opt.labelKey) : `${tt(opt.labelKey)} (${opt.value})`) : opt.label}
                            </option>
                          {/each}
                        </select>
                      </div>
                    </div>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.translateToEnglish}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { translateToEnglish: event.currentTarget.checked })}
                      />
                      {tt("transcribe.translateToEnglish")}
                    </label>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.transcriptsSubfolder}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { transcriptsSubfolder: event.currentTarget.checked })}
                      />
                      {tt("transcribe.subfolder")}
                    </label>

                    <div class="field">
                      <label class="field__label" for={`engine-order-${row.key}`}>
                        {tt("addbar.transcribeEngine")}
                      </label>
                      <select
                        class="select"
                        id={`engine-order-${row.key}`}
                        value={row.transcriptionEngine ?? settingsStore.settings.transcriptionEngineOrder}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { transcriptionEngine: event.currentTarget.value })}
                      >
                        <option value="subtitles_whisper_gemini">{tt("transcription.engine.subtitlesWhisperGemini")}</option>
                        <option value="whisper_only">{tt("transcription.engine.whisperOnly")}</option>
                        <option value="gemini_only">{tt("transcription.engine.geminiOnly")}</option>
                        <option value="subtitles_only">{tt("transcription.engine.subtitlesOnly")}</option>
                      </select>
                    </div>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.summarize}
                        onchange={(event) =>
                          addBarStore.setMediaOptions(row.key, { summarize: event.currentTarget.checked })}
                      />
                      {tt("addbar.summarize")}
                    </label>
                  {/if}
                </div>

                <div class="field">
                  <label class="field__label" for={`more-ref-${row.key}`}>{tt("queue.add.more.referer")}</label>
                  <input
                    class="input"
                    id={`more-ref-${row.key}`}
                    type="url"
                    autocomplete="off"
                    spellcheck="false"
                    value={row.referer}
                    oninput={(event) =>
                      addBarStore.setNetworkOptions(row.key, { referer: event.currentTarget.value })}
                  />
                </div>
                <div class="field">
                  <label class="field__label" for={`more-ua-${row.key}`}>{tt("queue.add.more.userAgent")}</label>
                  <input
                    class="input"
                    id={`more-ua-${row.key}`}
                    type="text"
                    autocomplete="off"
                    spellcheck="false"
                    value={row.userAgent}
                    oninput={(event) =>
                      addBarStore.setNetworkOptions(row.key, { userAgent: event.currentTarget.value })}
                  />
                </div>
                <div class="field">
                  <label class="field__label" for={`more-hdr-${row.key}`}>{tt("queue.add.more.headers")}</label>
                  <textarea
                    class="input add__input"
                    id={`more-hdr-${row.key}`}
                    rows="2"
                    spellcheck="false"
                    placeholder="Header-Name: value"
                    value={row.customHeaders}
                    oninput={(event) =>
                      addBarStore.setNetworkOptions(row.key, { customHeaders: event.currentTarget.value })}
                  ></textarea>
                  <p class="field__hint">
                    {ttCount("queue.add.more.headers.count", parseHeaderLines(row.customHeaders).length)}
                  </p>
                </div>
                <button type="button" class="btn btn--small" onclick={() => void addBarStore.retryRow(row.key)}>
                  {tt("queue.add.retry")}
                </button>
              </div>
            </details>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if duplicates.length > 0}
    <div class="add__duplicates" role="alert">
      <p class="add__duplicates-title">{tt("queue.add.duplicate.title")}</p>
      <p class="field__hint">
        {ttCount("queue.add.duplicate.body", duplicates.length)}
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
    <div class="add__status">
      <p class="field__hint" role="status">
        {#if readyCount > 0}
          {ttCount("queue.add.ready", readyCount)}
        {:else if rows.length > 0}
          {tt("queue.add.none")}
        {/if}
      </p>
      <p class="field__hint">{tt("queue.add.actions.hint")}</p>
    </div>
    <div class="add__actions">
      {#if addBarStore.hasRows}
        <button type="button" class="btn" onclick={() => addBarStore.clear()}>
          {tt("queue.clear")}
        </button>
      {/if}
      <button
        type="button"
        class="btn"
        disabled={busy !== "" || !actionable}
        onclick={() => void addAll("queue")}
      >
        <PlusIcon size={16} aria-hidden="true" focusable="false" />
        {tt("queue.add.button")}
      </button>
      <button
        type="button"
        class="btn btn--primary"
        disabled={busy !== "" || !actionable}
        onclick={() => void addAll("download")}
      >
        <DownloadIcon size={16} aria-hidden="true" focusable="false" />
        {primaryLabel}
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
    /* Never squeezed: on a window too small for everything, the page scrolls
       (`.queue`), and the textarea keeps its size. */
    flex: none;
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
    max-height: min(70vh, 720px);
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

  .add__status {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
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

  .add__choice,
  .add__block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 100%;
    min-width: 0;
  }

  .add__live-mode {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .add__check {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: 13px;
  }

  .add__more {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
  }

  .add__table-wrap {
    max-height: 220px;
    overflow: auto;
  }

  .add__table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .add__table th,
  .add__table td {
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border);
    text-align: left;
    white-space: nowrap;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
