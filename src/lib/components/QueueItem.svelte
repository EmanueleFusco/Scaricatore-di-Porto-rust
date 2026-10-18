<!--
  One queue item (spec §11.5).

  A card with the title, the site badge, the **stage chip**, a determinate or
  indeterminate progress bar, speed and ETA, and the actions the state offers.
  The details panel is expandable and carries the chosen options, the output
  path, the error with **Copy details** and the last log lines.
-->
<script lang="ts">
  import { openPath, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    actionsFor,
    actionKey,
    barPercentFor,
    detailsText,
    fileName,
    formatBytes,
    heightWarning,
    isActive,
    wasSkippedByArchive,
    itemLabel,
    qualityKey,
    speedMbits,
    stageDetail,
    stateKey,
    statusKindFor,
    type ItemAction,
    type QueueItem,
  } from "$lib/queue";
  import { secondsToClock } from "$lib/inspect";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { errorText, settingsStore, tt } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  let { item }: { item: QueueItem } = $props();

  let expanded = $state(false);
  let copied = $state(false);

  const hasFile = $derived(item.file !== null);
  const actions = $derived(actionsFor(item.state, hasFile));
  const warning = $derived(heightWarning(item));
  const speed = $derived(speedMbits(item.progress.speedBps));
  const eta = $derived(secondsToClock(item.progress.etaSec));
  const percent = $derived(barPercentFor(item));
  const indeterminate = $derived(isActive(item.state) && item.progress.percent === null);
  const quality = $derived(qualityKey(item));
  const detail = $derived(tt(quality.key, quality.params));
  /** "480 MB", once the file exists (spec §11.5). */
  const size = $derived(formatBytes(item.sizeBytes));
  const stageKey = $derived(stateKey(item.state));
  /**
   * One status line, whatever the state.
   *
   * The card keeps the same shape while a download runs: the slot is always
   * there, so a speed that arrives, disappears (yt-dlp reports `NA` between
   * fragments) and comes back cannot resize the card. While it runs, the line
   * opens with the stage detail — "Video 1/2", "Merging…" (spec §11.5). The
   * full text lives in the details panel; this line is clamped with a `title`
   * for the same reason.
   */
  const statusKind = $derived(statusKindFor(item));
  const stage = $derived(stageDetail(item));
  const statusText = $derived.by(() => {
    if (statusKind === "error" && item.error !== null) {
      return errorText(item.error.messageKey, null);
    }
    if (statusKind === "warning" && warning !== null) {
      return tt("queue.warning.heightLower.short", {
        actual: warning.actual,
        requested: warning.requested,
      });
    }
    if (statusKind === "progress") {
      const parts = [
        stage !== null ? tt(stage.key, stage.params) : null,
        speed !== null ? tt("queue.speed", { value: speed }) : null,
        eta !== null ? tt("queue.eta", { value: eta }) : null,
      ].filter((part) => part !== null);
      return parts.length > 0 ? parts.join(" · ") : tt("queue.status.waitingData");
    }
    // Spec §6.4: finished without downloading because the archive has it.
    if (wasSkippedByArchive(item)) return tt("queue.item.skippedArchive");
    return size !== null ? `${detail} · ${size}` : detail;
  });

  /**
   * The Deno install offer of spec §5.3.2 ("ask once"), on the card that failed
   * for it.
   *
   * M1 showed it on its own screen; the queue has to show it now that the
   * screen is gone, or a real download stays blocked with a message and no way
   * out but Settings → Tools. Deno is the one tool the app installs for the
   * user, and the item's options froze `auto`, which is exactly the mode that
   * wanted it.
   */
  const denoAvailable = $derived(
    toolsStore.view.tools.some((tool) => tool.name === "deno" && tool.path !== null),
  );
  const denoOffer = $derived(
    item.state === "failed" &&
      item.error?.code === "DENO_MISSING" &&
      !settingsStore.settings.denoOfferDismissed &&
      !denoAvailable,
  );

  async function installDeno(): Promise<void> {
    await toolsStore.install("deno");
  }

  async function dismissDeno(): Promise<void> {
    await settingsStore.update({ denoOfferDismissed: true });
  }

  async function act(action: ItemAction): Promise<void> {
    switch (action) {
      case "reveal":
        if (item.file !== null) {
          await revealItemInDir(item.file).catch(() => {});
        }
        return;
      case "open":
        if (item.file !== null) {
          await openPath(item.file).catch(() => {});
        }
        return;
      case "copyLink":
        await navigator.clipboard.writeText(item.url).catch(() => {});
        copied = true;
        setTimeout(() => (copied = false), 1500);
        return;
      case "openPage":
        await openUrl(item.url).catch(() => {});
        return;
      default:
        await queueStore.act(action, item.id);
    }
  }

  async function copyDetails(): Promise<void> {
    const message = item.error !== null ? errorText(item.error.messageKey, null) : null;
    await navigator.clipboard.writeText(detailsText(item, message)).catch(() => {});
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  /** Drag and drop to reorder (spec §7.3), pointer-free on touch. */
  function onDragStart(event: DragEvent): void {
    const target = event.target as HTMLElement | null;
    if (target?.closest("button, a, input, select, textarea")) {
      event.preventDefault();
      return;
    }
    event.dataTransfer?.setData("text/plain", item.id);
    if (event.dataTransfer !== null) event.dataTransfer.effectAllowed = "move";
  }

  async function onDrop(event: DragEvent): Promise<void> {
    event.preventDefault();
    const moved = event.dataTransfer?.getData("text/plain");
    if (!moved || moved === item.id) return;
    const ids = queueStore.list.map((entry) => entry.id).filter((id) => id !== moved);
    const at = ids.indexOf(item.id);
    ids.splice(at < 0 ? 0 : at, 0, moved);
    await queueStore.reorder(ids);
  }
</script>

<li
  class="item"
  class:item--expanded={expanded}
  class:item--new={queueStore.isFlashing(item.id)}
  data-state={item.state}
  draggable="true"
  ondragstart={onDragStart}
  ondragover={(event) => event.preventDefault()}
  ondrop={onDrop}
>
  <div class="item__head">
    <span class="item__chip" data-state={item.state}>{tt(stageKey)}</span>

    <div class="item__title-block">
      <span class="item__title">{itemLabel(item)}</span>
      <span class="item__meta">
        {#if item.site !== null}
          <span class="item__site">{item.site}</span>
        {/if}
        {#if item.durationSec !== null}
          · {secondsToClock(item.durationSec)}
        {/if}
        {#if item.state === "paused" || item.state === "interrupted"}
          · {tt("queue.item.resumed")}
        {/if}
        {#if item.retryAt !== null}
          · {tt("queue.item.retrying", { attempt: item.autoRetries })}
        {/if}
      </span>
    </div>

    <div class="item__actions">
      {#each actions as action (action)}
        <button
          type="button"
          class="btn btn--small"
          class:btn--danger={action === "cancel"}
          onclick={() => void act(action)}
        >
          {#if action === "copyLink" && copied}
            {tt("queue.copied")}
          {:else}
            {tt(actionKey(action))}
          {/if}
        </button>
      {/each}
      <button
        type="button"
        class="btn btn--small"
        aria-expanded={expanded}
        onclick={() => (expanded = !expanded)}
      >
        {tt(expanded ? "queue.list.collapse" : "queue.list.expand")}
      </button>
    </div>
  </div>

  {#if isActive(item.state)}
    <div
      class="item__bar"
      data-state={item.state}
      data-indeterminate={indeterminate}
      role="progressbar"
      aria-label={tt(stageKey)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : Math.round(percent)}
    >
      <div class="item__bar-fill" style:width={`${percent}%`}></div>
    </div>
  {:else}
    <div class="item__bar" data-state={item.state} aria-hidden="true">
      <div class="item__bar-fill" style:width={`${percent}%`}></div>
    </div>
  {/if}

  <p
    class="item__status"
    data-kind={statusKind}
    role={statusKind === "error" ? "alert" : statusKind === "warning" ? "status" : undefined}
    title={statusText}
  >
    {statusText}
  </p>

  {#if item.transcriptPreview}
    <p class="item__preview" title={item.transcriptPreview}>
      <span class="item__preview-badge">{tt("transcribe.livePreview")}:</span>
      {item.transcriptPreview}
    </p>
  {/if}

  {#if denoOffer}
    <div class="item__offer" role="status">
      <p class="item__offer-title">{tt("job.deno.title")}</p>
      <p class="item__offer-body">{tt("job.deno.body")}</p>
      <div class="item__offer-actions">
        <button
          type="button"
          class="btn btn--small"
          disabled={toolsStore.installingTool !== null}
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
            class="btn btn--small"
            onclick={() => void toolsStore.cancelInstall()}
          >
            {tt("tools.action.cancel")}
          </button>
        {/if}
        <button
          type="button"
          class="btn btn--small"
          disabled={toolsStore.installingTool === "deno"}
          onclick={() => void dismissDeno()}
        >
          {tt("job.deno.dismiss")}
        </button>
      </div>
      {#if toolsStore.errorKey !== null}
        <p class="item__offer-error">
          {errorText(toolsStore.errorKey, toolsStore.errorDetail)}
        </p>
      {/if}
    </div>
  {/if}

  {#if expanded}
    <div class="item__details">
      <dl class="item__facts">
        <dt>{tt("queue.item.options")}</dt>
        <dd>{detail}</dd>
        {#if item.runtime !== null}
          <dt>{tt("queue.item.runtime", { runtime: "" }).replace(": ", "")}</dt>
          <dd>{item.runtime}</dd>
        {/if}
        {#if item.file !== null}
          <dt>{tt("queue.item.output")}</dt>
          <dd class="item__path">{item.file}</dd>
        {/if}
        {#if size !== null}
          <dt>{tt("queue.item.size")}</dt>
          <dd>{size}</dd>
        {/if}
        {#if item.transcriptFiles.length > 0}
          <dt>{tt("transcribe.outputFiles")}</dt>
          <dd class="item__transcripts">
            {#each item.transcriptFiles as path}
              <button
                type="button"
                class="btn btn--small"
                onclick={() => void openPath(path)}
                title={path}
              >
                {fileName(path)}
              </button>
            {/each}
          </dd>
        {/if}
      </dl>

      {#if item.error !== null}
        <p class="item__note item__note--error">
          {errorText(item.error.messageKey, item.error.detail)}
        </p>
      {/if}

      {#if warning !== null}
        <p class="item__note item__note--warning">
          {tt("queue.warning.heightLower", {
            actual: warning.actual,
            requested: warning.requested,
          })}
        </p>
      {/if}

      {#if item.logTail.length > 0}
        <p class="item__log-label">{tt("queue.item.log")}</p>
        <pre class="item__log">{item.logTail.slice(-12).join("\n")}</pre>
      {/if}

      <div class="item__details-actions">
        <button type="button" class="btn btn--small" onclick={() => void copyDetails()}>
          {#if copied}{tt("queue.copied")}{:else}{tt("queue.copyDetails")}{/if}
        </button>
      </div>
    </div>
  {/if}
</li>

<style>
  .item {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .item--expanded {
    border-color: var(--border-strong);
  }

  /* Just added: a pulse the eye catches where the row appears (D104). */
  .item--new {
    border-color: var(--accent);
    animation: item-flash 1.1s ease-in-out 2;
  }

  @keyframes item-flash {
    0%,
    100% {
      box-shadow: 0 0 0 2px var(--accent);
    }
    50% {
      box-shadow: 0 0 0 2px transparent;
    }
  }

  .item__head {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex-wrap: wrap;
  }

  .item__chip {
    padding: 0 var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .item__chip[data-state="done"] {
    border-color: var(--accent);
    color: var(--accent);
  }

  .item__chip[data-state="failed"] {
    border-color: var(--danger);
    color: var(--danger);
  }

  .item__chip[data-state="paused"],
  .item__chip[data-state="interrupted"] {
    color: var(--text-muted);
  }

  .item__title-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    flex: 1 1 200px;
  }

  .item__title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item__meta {
    color: var(--text-muted);
    font-size: 12px;
  }

  .item__site {
    padding: 0 var(--space-1);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
  }

  .item__actions {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
    margin-left: auto;
  }

  .item__bar {
    height: 8px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    overflow: hidden;
  }

  .item__bar-fill {
    height: 100%;
    background: var(--accent);
    transition: width 120ms linear;
  }

  .item__bar[data-state="queued"] .item__bar-fill {
    background: transparent;
  }

  .item__bar[data-state="failed"] .item__bar-fill {
    background: var(--danger);
  }

  .item__bar[data-state="cancelled"] .item__bar-fill,
  .item__bar[data-state="paused"] .item__bar-fill,
  .item__bar[data-state="interrupted"] .item__bar-fill {
    background: var(--border-strong);
  }

  .item__bar[data-indeterminate="true"] .item__bar-fill {
    animation: item-slide 1.2s ease-in-out infinite alternate;
    opacity: 0.6;
  }

  @keyframes item-slide {
    from {
      transform: translateX(-30%);
    }
    to {
      transform: translateX(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .item--new {
      animation: none;
    }

    .item__bar-fill {
      transition: none;
      animation: none !important;
    }
  }

  /*
    The one status line of a card: progress while it runs, the quality when it
    waits, the problem when there is one. Always present and always one line,
    so the card's height never changes (the virtualiser relies on that too).
  */
  .item__status {
    min-height: 16px;
    color: var(--text-muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item__status[data-kind="error"] {
    color: var(--danger);
  }

  .item__status[data-kind="warning"] {
    color: var(--accent);
  }

  .item__note {
    margin: 0;
  }

  .item__note--error {
    color: var(--danger);
  }

  .item__note--warning {
    color: var(--accent);
  }

  /* The Deno offer of spec §5.3.2: a way out, right where the failure is. */
  .item__offer {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    font-size: 12px;
  }

  .item__offer-title {
    font-weight: 600;
  }

  .item__offer-body {
    color: var(--text-muted);
  }

  .item__offer-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .item__offer-error {
    color: var(--danger);
  }

  .item__details {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
    font-size: 12px;
  }

  .item__facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }

  .item__facts dt {
    color: var(--text-muted);
  }

  .item__facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .item__path {
    font-family: var(--font-mono);
  }

  .item__log-label {
    color: var(--text-muted);
  }

  .item__log {
    margin: 0;
    padding: var(--space-2);
    max-height: 140px;
    overflow-y: auto;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }

  .item__details-actions {
    display: flex;
    gap: var(--space-2);
  }

  .btn--small {
    height: 28px;
    padding: 0 var(--space-2);
    font-size: 12px;
  }

  .item__preview {
    margin: 0;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    font-size: 12px;
    font-family: var(--font-mono);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item__preview-badge {
    color: var(--text-muted);
    font-weight: 500;
  }

  .item__transcripts {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
</style>