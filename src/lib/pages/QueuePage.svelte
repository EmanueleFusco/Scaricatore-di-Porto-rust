<script lang="ts">
  /**
   * Temporary single-video screen (M1). The queue UI of spec §11.2–§11.5
   * arrives with M2; what has to work today is the parity path: an inspected
   * link, a resolution (or audio only), a folder, a progress bar with speed and
   * ETA, a Cancel that leaves nothing behind, and an error whose raw detail is
   * one click away.
   */
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import { open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    defaultResolution,
    isDeterminate,
    resolutionOptions,
    secondsToClock,
    speedMbits,
  } from "$lib/inspect";
  import { heightWarning, offersDenoInstall, progressLabel, stageKey, type QualityRequest } from "$lib/job";
  import { activeLang, errorText, settingsStore, tt } from "$lib/stores/settings.svelte";
  import { inspectStore } from "$lib/stores/inspect.svelte";
  import { jobStore } from "$lib/stores/job.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  let folder = $state<string | null>(null);
  let audioOnly = $state(false);
  let resolution = $state("best");
  let copied = $state(false);

  const inspected = $derived(inspectStore.inspected);
  const video = $derived(inspected?.kind === "video" ? inspected : null);
  const playlist = $derived(inspected?.kind === "playlist" ? inspected : null);
  const inspectError = $derived(inspected?.kind === "error" ? inspected : null);
  const options = $derived(video ? resolutionOptions(video) : []);

  // A new video preselects its highest resolution, as the original did.
  $effect(() => {
    if (video) {
      resolution = defaultResolution(video);
      audioOnly = false;
    }
  });

  const folderText = $derived(
    folder ?? settingsStore.settings.defaultOutputDir ?? tt("queue.folder.system"),
  );

  const progress = $derived(jobStore.progress);
  const done = $derived(jobStore.done);
  const doneError = $derived(done?.status === "failed" ? done.error : null);
  const resolutionWarning = $derived(heightWarning(done));
  const running = $derived(jobStore.status === "running");

  async function browse(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: true,
      title: tt("queue.folder.browse"),
    });
    if (typeof picked === "string") {
      folder = picked;
    }
  }

  async function startDownload(): Promise<void> {
    if (!video) return;
    const quality: QualityRequest = audioOnly
      ? { kind: "audioOnly" }
      : resolution === "best"
        ? { kind: "best" }
        : { kind: "exact", height: Number(resolution) };
    await jobStore.startJob(inspectStore.url.trim(), quality, folder);
  }

  async function copyDetails(): Promise<void> {
    const key = doneError?.messageKey ?? inspectError?.messageKey ?? null;
    const detail = doneError?.detail ?? inspectError?.detail ?? null;
    const text = [key ? errorText(key, null) : null, detail].filter(Boolean).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      // Clipboard access can be denied; the detail stays visible either way.
    }
  }

  function reveal(): void {
    if (done?.file) {
      void revealItemInDir(done.file).catch(() => {});
    }
  }

  async function installDeno(): Promise<void> {
    const status = await toolsStore.install("deno");
    if (status) await startDownload();
  }

  function resetForNext(): void {
    jobStore.reset();
  }
</script>

<svelte:head>
  <title>{tt("queue.title")} — {tt("app.name")}</title>
</svelte:head>

<section class="queue">
  <h1 class="queue__title">{tt("queue.title")}</h1>
  <p class="queue__temp-note">{tt("queue.comingSoon")}</p>

  <div class="card queue__add">
    <div class="field">
      <label class="field__label" for="queue-url">{tt("queue.url.label")}</label>
      <div class="queue__url-row">
        <input
          class="input"
          id="queue-url"
          type="url"
          inputmode="url"
          autocomplete="off"
          spellcheck="false"
          placeholder={tt("queue.url.placeholder")}
          value={inspectStore.url}
          disabled={running}
          oninput={(event) => inspectStore.setUrl((event.currentTarget as HTMLInputElement).value)}
        />
        <button
          type="button"
          class="btn"
          disabled={running || inspectStore.url === ""}
          onclick={() => inspectStore.clear()}
        >
          {tt("queue.clear")}
        </button>
      </div>
      {#if inspectStore.loading}
        <p class="field__hint" role="status">{tt("queue.inspecting")}</p>
      {/if}
    </div>

    {#if inspectError}
      <div class="queue__error" role="alert">
        <p>{errorText(inspectError.messageKey, inspectError.detail)}</p>
        {#if inspectError.detail}
          <button type="button" class="btn" onclick={() => void copyDetails()}>
            {copied ? tt("queue.copied") : tt("queue.copyDetails")}
          </button>
        {/if}
      </div>
    {/if}

    {#if playlist}
      <div class="queue__playlist" role="status">
        <p class="queue__playlist-title">{tt("queue.playlist.title")}: {playlist.title}</p>
        <p class="field__hint">
          {playlist.count !== null
            ? tt("queue.playlist.stub", { count: playlist.count })
            : tt("queue.playlist.countUnknown")}
        </p>
      </div>
    {/if}

    {#if video}
      <div class="queue__meta">
        <p class="queue__video-title">{video.title}</p>
        <p class="field__hint">
          <span class="queue__site">{video.site}</span>
          {#if video.duration !== null}
            · {tt("queue.result.duration")}: {secondsToClock(video.duration)}
          {/if}
          {#if video.isLive}
            · <span class="queue__live">{tt("queue.badge.live")}</span>
          {/if}
        </p>
      </div>

      <div class="queue__controls">
        <div class="field">
          <label class="field__label" for="queue-resolution">{tt("queue.resolution.label")}</label>
          <select
            class="select"
            id="queue-resolution"
            value={resolution}
            disabled={audioOnly || running}
            onchange={(event) => (resolution = (event.currentTarget as HTMLSelectElement).value)}
          >
            {#each options as option (option.value)}
              <option value={option.value}>
                {option.height !== null ? `${option.height}p` : tt("queue.resolution.best")}
              </option>
            {/each}
          </select>
        </div>

        <label class="queue__switch">
          <input
            type="checkbox"
            checked={audioOnly}
            disabled={running}
            onchange={(event) => (audioOnly = (event.currentTarget as HTMLInputElement).checked)}
          />
          {tt("queue.audioOnly")}
        </label>
      </div>

      <div class="field">
        <span class="field__label">{tt("queue.folder.label")}</span>
        <div class="queue__folder">
          <output class="settings__value">{folderText}</output>
          <button type="button" class="btn" disabled={running} onclick={() => void browse()}>
            <FolderOpenIcon size={16} aria-hidden="true" focusable="false" />
            {tt("queue.folder.browse")}
          </button>
        </div>
      </div>

      <div class="queue__actions">
        {#if running}
          <button type="button" class="btn btn--danger" onclick={() => void jobStore.cancel()}>
            {tt("queue.cancel")}
          </button>
        {:else}
          <button type="button" class="btn btn--primary" onclick={() => void startDownload()}>
            {tt("queue.download")}
          </button>
        {/if}
      </div>
    {/if}
  </div>

  {#if running || done}
    <div class="card queue__progress">
      {#if running && progress}
        <p class="queue__stage">
          <span class="queue__chip">{tt(stageKey(progress.stage))}</span>
          {progressLabel(activeLang(), progress)}
        </p>
        <div
          class="queue__bar"
          role="progressbar"
          aria-label={tt(stageKey(progress.stage))}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={isDeterminate(progress.percent) ? Math.round(progress.percent!) : undefined}
          data-indeterminate={!isDeterminate(progress.percent)}
        >
          <div
            class="queue__bar-fill"
            style:width={isDeterminate(progress.percent) ? `${progress.percent}%` : "100%"}
          ></div>
        </div>
        <p class="queue__detail">
          {#if progress.speedBps !== null && speedMbits(progress.speedBps) !== null}
            <span>{tt("queue.speed", { value: speedMbits(progress.speedBps)! })}</span>
          {/if}
          {#if progress.etaSec !== null && secondsToClock(progress.etaSec)}
            <span>{tt("queue.eta", { value: secondsToClock(progress.etaSec)! })}</span>
          {/if}
        </p>
      {/if}

      {#if jobStore.status === "done" && done}
        <p class="queue__stage">
          <span class="queue__chip queue__chip--ok">{tt("job.stage.done")}</span>
          {tt("queue.done.title")}
        </p>
        {#if done.file}
          <p class="queue__file">{done.file}</p>
        {/if}
        {#if done.runtime}
          <p class="field__hint">{tt("queue.done.runtime", { runtime: done.runtime })}</p>
        {/if}
        {#if resolutionWarning}
          <p class="queue__warning" role="status">
            {tt("queue.warning.heightLower", {
              actual: resolutionWarning.actual,
              requested: resolutionWarning.requested,
            })}
          </p>
        {/if}
        <div class="queue__actions">
          <button type="button" class="btn" onclick={reveal}>{tt("queue.done.reveal")}</button>
          <button type="button" class="btn" onclick={resetForNext}>{tt("queue.done.again")}</button>
        </div>
      {/if}

      {#if jobStore.status === "cancelled"}
        <p class="queue__stage">
          <span class="queue__chip">{tt("job.stage.cancelled")}</span>
          {tt("queue.cancelled")}
        </p>
        <div class="queue__actions">
          <button type="button" class="btn" onclick={resetForNext}>{tt("queue.done.again")}</button>
        </div>
      {/if}

      {#if jobStore.status === "failed" && done}
        <div class="queue__error" role="alert">
          <p>
            {doneError
              ? errorText(doneError.messageKey, doneError.detail)
              : tt("error.ytdlp.unknown")}
          </p>
          <div class="queue__actions">
            {#if doneError?.detail}
              <button type="button" class="btn" onclick={() => void copyDetails()}>
                {copied ? tt("queue.copied") : tt("queue.copyDetails")}
              </button>
            {/if}
            <button type="button" class="btn" onclick={resetForNext}>{tt("queue.done.again")}</button>
          </div>
        </div>

        {#if offersDenoInstall(done, settingsStore.settings.denoOfferDismissed)}
          <div class="queue__deno">
            <p class="queue__deno-title">{tt("job.deno.title")}</p>
            <p class="field__hint">{tt("job.deno.body")}</p>
            <div class="queue__actions">
              <button
                type="button"
                class="btn btn--primary"
                disabled={toolsStore.busy && toolsStore.installingTool !== "deno"}
                onclick={() => void installDeno()}
              >
                {#if toolsStore.installingTool === "deno"}
                  {toolsStore.installPercent !== null
                    ? tt("tools.action.installing", { percent: toolsStore.installPercent })
                    : tt("tools.action.installingUnknown")}
                {:else}
                  {tt("job.deno.install")}
                {/if}
              </button>
              {#if toolsStore.installingTool === "deno"}
                <button
                  type="button"
                  class="btn"
                  onclick={() => void toolsStore.cancelInstall()}
                >
                  {tt("tools.action.cancel")}
                </button>
              {/if}
              <button
                type="button"
                class="btn"
                disabled={toolsStore.busy}
                onclick={() => void jobStore.dismissDenoOffer()}
              >
                {tt("job.deno.dismiss")}
              </button>
            </div>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</section>

<style>
  .queue {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 720px;
    margin: 0 auto;
  }

  .queue__title {
    font-size: 20px;
  }

  .queue__temp-note {
    margin-top: calc(-1 * var(--space-3));
    color: var(--text-muted);
    font-size: 12px;
  }

  .queue__add,
  .queue__progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }

  .queue__url-row,
  .queue__folder {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .queue__meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .queue__video-title {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .queue__site {
    padding: 0 var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
  }

  .queue__live {
    color: var(--danger);
    font-weight: 700;
  }

  .queue__controls {
    display: flex;
    gap: var(--space-4);
    flex-wrap: wrap;
    align-items: flex-end;
  }

  .queue__switch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
  }

  .queue__actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .queue__stage {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 14px;
  }

  .queue__chip {
    padding: 0 var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .queue__chip--ok {
    border-color: var(--accent);
    color: var(--accent);
  }

  .queue__bar {
    height: 8px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    overflow: hidden;
  }

  .queue__bar-fill {
    height: 100%;
    background: var(--accent);
    transition: width 120ms linear;
  }

  .queue__bar[data-indeterminate="true"] .queue__bar-fill {
    animation: queue-slide 1.2s ease-in-out infinite alternate;
    opacity: 0.6;
  }

  @keyframes queue-slide {
    from {
      transform: translateX(-30%);
    }
    to {
      transform: translateX(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .queue__bar-fill {
      transition: none;
      animation: none !important;
    }
  }

  .queue__detail {
    display: flex;
    gap: var(--space-3);
    color: var(--text-muted);
    font-size: 12px;
  }

  .queue__file {
    font-family: var(--font-mono);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .queue__error {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--danger);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: 13px;
  }

  .queue__deno {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    font-size: 13px;
  }

  .queue__deno-title {
    font-weight: 700;
  }

  .queue__warning {
    padding: var(--space-2) var(--space-3);
    border-left: 3px solid var(--warning, var(--accent));
    color: var(--text);
    font-size: 12px;
  }

  .queue__playlist {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: 13px;
  }

  .queue__playlist-title {
    font-weight: 600;
  }
</style>
