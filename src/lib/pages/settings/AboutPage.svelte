<script lang="ts">
  /**
   * Settings → About (spec §11.8, §13.4).
   *
   * The page answers the three questions support asks first: which build is
   * this, is there an update, and what does the diagnostics report say. The
   * update buttons call Rust commands; the signing key never reaches here.
   */
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { relaunch } from "@tauri-apps/plugin-process";
  import CopyIcon from "lucide-svelte/icons/copy";
  import DownloadIcon from "lucide-svelte/icons/download";
  import ExternalLinkIcon from "lucide-svelte/icons/external-link";
  import FileTextIcon from "lucide-svelte/icons/file-text";
  import RefreshCwIcon from "lucide-svelte/icons/refresh-cw";
  import RotateCcwIcon from "lucide-svelte/icons/rotate-ccw";
  import XIcon from "lucide-svelte/icons/x";
  import { LICENSES, canSelfUpdate, updatePercent } from "$lib/about";
  import { formatBytes } from "$lib/models";
  import { aboutStore } from "$lib/stores/about.svelte";
  import { errorText, tt } from "$lib/stores/settings.svelte";

  const RELEASES_URL =
    "https://github.com/EmanueleFusco/Scaricatore-di-Porto-rust/releases/latest";

  const percent = $derived(aboutStore.progress ? updatePercent(aboutStore.progress) : null);
  const selfUpdate = $derived(canSelfUpdate(aboutStore.bundleType));

  async function install(): Promise<void> {
    await aboutStore.installUpdate();
  }

  async function restart(): Promise<void> {
    await relaunch();
  }
</script>

<svelte:head>
  <title>{tt("about.title")} — {tt("app.name")}</title>
</svelte:head>

<div class="about">
  {#if aboutStore.errorKey}
    <div class="banner banner--error" role="alert">
      <div class="banner__body">
        <strong>{errorText(aboutStore.errorKey, null)}</strong>
        {#if aboutStore.errorDetail}
          <details class="banner__details">
            <summary>{tt("common.technicalDetails")}</summary>
            <pre>{aboutStore.errorDetail}</pre>
          </details>
        {/if}
      </div>
      <button
        type="button"
        class="banner__close"
        onclick={() => aboutStore.dismissError()}
        aria-label={tt("common.dismiss")}
      >
        <XIcon size={16} aria-hidden="true" />
      </button>
    </div>
  {/if}

  <section class="card about__hero" aria-labelledby="about-heading">
    <div>
      <h2 class="about__name" id="about-heading">{tt("app.name")}</h2>
      <p class="about__tagline">{tt("about.tagline")}</p>
    </div>
    <p class="about__version">
      {tt("about.version", { version: aboutStore.info?.version ?? "—" })}
    </p>
    <dl class="about__facts">
      <dt>{tt("about.platform")}</dt>
      <dd>{aboutStore.info ? `${aboutStore.info.os} ${aboutStore.info.arch}` : "—"}</dd>
      <dt>{tt("about.target")}</dt>
      <dd>{aboutStore.info?.triple || "—"}</dd>
      <dt>{tt("about.bundle")}</dt>
      <dd>{aboutStore.bundleType ?? tt("about.bundle.unknown")}</dd>
    </dl>
  </section>

  <section class="card about__group" aria-labelledby="about-updates">
    <h3 class="about__group-title" id="about-updates">{tt("about.updates.title")}</h3>
    <p class="field__hint">{tt("about.updates.hint")}</p>

    {#if aboutStore.installedVersion}
      <p class="about__ok" role="status">
        {tt("about.updates.installed", { version: aboutStore.installedVersion })}
      </p>
      <button type="button" class="btn btn--primary" onclick={() => void restart()}>
        <RotateCcwIcon size={16} aria-hidden="true" focusable="false" />
        {tt("about.updates.restart")}
      </button>
    {:else if aboutStore.installing}
      <p role="status">{tt("about.updates.installing")}</p>
      <div
        class="about__progress"
        role="progressbar"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={percent ?? undefined}
        aria-label={tt("about.updates.installing")}
      >
        <div class="about__progress-bar" style:width={`${percent ?? 100}%`}></div>
      </div>
      {#if aboutStore.progress}
        <p class="field__hint">
          {aboutStore.progress.total
            ? tt("about.updates.progress", {
                done: formatBytes(aboutStore.progress.downloaded),
                total: formatBytes(aboutStore.progress.total),
              })
            : tt("about.updates.progressUnknown", {
                done: formatBytes(aboutStore.progress.downloaded),
              })}
        </p>
      {/if}
    {:else if aboutStore.update?.available && aboutStore.update.version}
      <p class="about__available" role="status">
        {tt("about.updates.available", { version: aboutStore.update.version })}
      </p>
      {#if aboutStore.update.date}
        <p class="field__hint">{tt("about.updates.date", { date: aboutStore.update.date })}</p>
      {/if}
      {#if aboutStore.update.notes}
        <p class="about__notes">{aboutStore.update.notes}</p>
      {/if}
      {#if selfUpdate}
        <button
          type="button"
          class="btn btn--primary"
          disabled={aboutStore.installing}
          onclick={() => void install()}
        >
          <DownloadIcon size={16} aria-hidden="true" focusable="false" />
          {tt("about.updates.install")}
        </button>
      {:else}
        <p class="field__hint">{tt("about.updates.managed")}</p>
        <button type="button" class="btn" onclick={() => void openUrl(RELEASES_URL)}>
          <ExternalLinkIcon size={16} aria-hidden="true" focusable="false" />
          {tt("about.updates.releases")}
        </button>
      {/if}
    {:else if aboutStore.update && !aboutStore.update.available}
      <p class="about__ok" role="status">{tt("about.updates.upToDate")}</p>
    {:else}
      <p class="field__hint">{tt("about.updates.never")}</p>
    {/if}

    <div class="about__actions">
      <button
        type="button"
        class="btn"
        disabled={aboutStore.checking || aboutStore.installing}
        onclick={() => void aboutStore.checkUpdate()}
      >
        <RefreshCwIcon size={16} aria-hidden="true" focusable="false" />
        {aboutStore.checking ? tt("about.updates.checking") : tt("about.updates.check")}
      </button>
      <button type="button" class="btn" onclick={() => void openUrl(RELEASES_URL)}>
        <ExternalLinkIcon size={16} aria-hidden="true" focusable="false" />
        {tt("about.updates.releases")}
      </button>
    </div>
  </section>

  <section class="card about__group" aria-labelledby="about-diagnostics">
    <h3 class="about__group-title" id="about-diagnostics">
      {tt("about.diagnostics.title")}
    </h3>
    <p class="field__hint">{tt("about.diagnostics.hint")}</p>
    <div class="about__actions">
      <button type="button" class="btn" onclick={() => void aboutStore.copyDiagnostics()}>
        <CopyIcon size={16} aria-hidden="true" focusable="false" />
        {aboutStore.copied ? tt("about.diagnostics.copied") : tt("about.diagnostics.copy")}
      </button>
      <button type="button" class="btn" onclick={() => void aboutStore.openLogs()}>
        <FileTextIcon size={16} aria-hidden="true" focusable="false" />
        {tt("about.diagnostics.openLogs")}
      </button>
    </div>
    <details class="about__paths">
      <summary>{tt("about.paths.title")}</summary>
      <dl class="about__facts">
        <dt>{tt("about.paths.settings")}</dt>
        <dd>{aboutStore.info?.settingsPath || "—"}</dd>
        <dt>{tt("about.paths.tools")}</dt>
        <dd>{aboutStore.info?.toolsDir || "—"}</dd>
        <dt>{tt("about.paths.models")}</dt>
        <dd>{aboutStore.info?.modelsDir || "—"}</dd>
        <dt>{tt("about.paths.log")}</dt>
        <dd>{aboutStore.info?.logPath || "—"}</dd>
      </dl>
    </details>
  </section>

  <section class="card about__group" aria-labelledby="about-licenses">
    <h3 class="about__group-title" id="about-licenses">{tt("about.licenses.title")}</h3>
    <p class="field__hint">{tt("about.licenses.hint")}</p>
    <ul class="about__licenses">
      {#each LICENSES as entry (entry.name)}
        <li class="about__license">
          <div>
            <span class="about__license-name">{entry.name}</span>
            <span class="field__hint">{tt(entry.roleKey)}</span>
          </div>
          <span class="about__license-badge">{entry.license}</span>
          <button type="button" class="btn btn--small" onclick={() => void openUrl(entry.url)}>
            <ExternalLinkIcon size={14} aria-hidden="true" focusable="false" />
            {tt("about.licenses.open")}
          </button>
        </li>
      {/each}
    </ul>
  </section>
</div>

<style>
  .about {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
  }

  .about__hero {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .about__name {
    font-size: 20px;
  }

  .about__tagline {
    color: var(--text-muted);
    margin-top: 2px;
  }

  .about__version {
    display: inline-block;
    align-self: flex-start;
    padding: 2px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .about__facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    font-size: 12px;
  }

  .about__facts dt {
    color: var(--text-muted);
  }

  .about__facts dd {
    margin: 0;
    font-family: var(--font-mono);
    word-break: break-all;
  }

  .about__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    align-items: flex-start;
  }

  .about__group-title {
    font-size: 15px;
    font-weight: 700;
  }

  .about__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .about__ok {
    color: var(--accent);
  }

  .about__available {
    font-weight: 600;
  }

  .about__notes {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: 12px;
    white-space: pre-wrap;
    max-height: 180px;
    overflow: auto;
  }

  .about__progress {
    width: 100%;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--border);
  }

  .about__progress-bar {
    height: 100%;
    background: var(--accent);
    transition: width 150ms linear;
  }

  .about__paths {
    width: 100%;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
    font-size: 12px;
  }

  .about__paths summary {
    cursor: pointer;
    color: var(--text-muted);
    font-weight: 600;
  }

  .about__paths dl {
    margin-top: var(--space-2);
  }

  .about__licenses {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    width: 100%;
  }

  .about__license {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
  }

  .about__license > div {
    display: flex;
    flex-direction: column;
    flex: 1 1 220px;
  }

  .about__license-name {
    font-weight: 600;
  }

  .about__license-badge {
    padding: 0 var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
