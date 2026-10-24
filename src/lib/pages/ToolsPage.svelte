<script lang="ts">
  /**
   * Settings → Tools (spec §11.8): one row per tool with its source, version,
   * status and the actions that apply to it.
   *
   * The composable-download rule of spec §5.2.1 is what decides the actions: a
   * tool that is missing must never be a dead end, so every available row
   * carries at least "Use my own path", and the ones the app cannot install yet
   * say so in words rather than showing a button that does nothing.
   */
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import PlayIcon from "lucide-svelte/icons/play";
  import RefreshCwIcon from "lucide-svelte/icons/refresh-cw";
  import WrenchIcon from "lucide-svelte/icons/wrench";
  import { open } from "@tauri-apps/plugin-dialog";
  import SetupWizard from "$lib/components/SetupWizard.svelte";
  import { hasKey } from "$lib/i18n";
  import {
    canUpdate,
    canUseOwnPath,
    canUseSystemPath,
    splitToolBanner,
    toolRoleKey,
    toolState,
    wizardToolNames,
    type ToolStatus,
  } from "$lib/tools";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { activeLang, errorText, settingsStore, tt } from "$lib/stores/settings.svelte";

  let wizardOpen = $state(false);

  /** Manifest order for the bundled tools, then everything else. */
  const rows = $derived(
    toolsStore.view.tools
      .slice()
      .sort(
        (left, right) =>
          wizardToolNames(toolsStore.view.tools).indexOf(left.name) -
          wizardToolNames(toolsStore.view.tools).indexOf(right.name),
      ),
  );

  async function chooseOwnPath(name: string): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      title: tt("tools.choosePath.title", { tool: name }),
    });
    // `open` returns null when the dialog is cancelled: that must not clear an
    // override the user already has.
    if (typeof picked === "string") {
      await toolsStore.setToolPath(name, picked);
    }
  }

  function roleLabel(tool: ToolStatus): string {
    const key = toolRoleKey(tool.name);
    return hasKey(activeLang(), key) ? tt(key) : tool.role;
  }
</script>

<section class="tools">
  <header class="tools__header">
    <h2 class="tools__title">{tt("tools.title")}</h2>
    <button
      type="button"
      class="btn"
      onclick={() => void toolsStore.refresh()}
      disabled={toolsStore.loading}
    >
      <RefreshCwIcon size={16} aria-hidden="true" focusable="false" />
      {tt("tools.refresh")}
    </button>
  </header>

  {#if toolsStore.errorKey}
    <p class="tools__error" role="alert">
      {errorText(toolsStore.errorKey, toolsStore.errorDetail)}
    </p>
  {/if}

  {#if toolsStore.lastUpdate}
    <p class="tools__note" role="status">
      {tt(toolsStore.lastUpdate.key, {
        version: toolsStore.lastUpdate.version ?? tt("tools.version.unknown"),
      })}
      {#if toolsStore.lastUpdate.message}
        <span class="tools__note-detail">{toolsStore.lastUpdate.message}</span>
      {/if}
      <button type="button" class="btn" onclick={() => toolsStore.dismissUpdate()}>
        {tt("toast.dismiss")}
      </button>
    </p>
  {/if}

  <div class="card tools__table-wrap">
    <table class="tools__table">
      <caption class="visually-hidden">{tt("tools.table.caption")}</caption>
      <thead>
        <tr>
          <th scope="col">{tt("tools.column.tool")}</th>
          <th scope="col">{tt("tools.column.source")}</th>
          <th scope="col">{tt("tools.column.version")}</th>
          <th scope="col">{tt("tools.column.status")}</th>
          <th scope="col">{tt("tools.column.actions")}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as tool (tool.name)}
          {@const state = toolState(tool)}
          <tr class:tools__row--has-detail={Boolean(tool.path)}>
            <th scope="row" class="tools__name">
              <div class="tools__name-inner">
                <div class="tools__name-head">
                  <span>{tool.name}</span>
                  {#if tool.bundled}<span class="tools__badge">{tt("tools.badge.bundled")}</span>{/if}
                </div>
                {#if tool.role}<span class="field__hint">{roleLabel(tool)}</span>{/if}
              </div>
            </th>
            <td>{tool.source ? tt(`tools.source.${tool.source}`) : "—"}</td>
            <td>
              {#if tool.version}
                {tool.version}
              {:else if tool.path}
                <span class="field__hint">{tt("tools.version.unknown")}</span>
              {:else}
                —
              {/if}
            </td>
            <td>
              <!-- The state is in the text too, never colour alone (spec §11.9). -->
              <span class="tools__status" data-state={state.state}>{tt(state.key)}</span>
            </td>
            <td class="tools__actions">
              <div class="tools__actions-inner">
                {#if canUpdate(tool)}
                  <button
                    type="button"
                    class="btn btn--small"
                    disabled={toolsStore.busy}
                    onclick={() => void toolsStore.updateYtdlp()}
                  >
                    {tt("tools.action.update")}
                  </button>
                {/if}
                {#if tool.installable && !tool.path}
                  <button
                    type="button"
                    class="btn btn--small btn--primary"
                    disabled={toolsStore.busy}
                    onclick={() => void toolsStore.install(tool.name)}
                  >
                    {#if toolsStore.installingTool === tool.name}
                      {toolsStore.installPercent !== null
                        ? tt("tools.action.installing", { percent: toolsStore.installPercent })
                        : tt("tools.action.installingUnknown")}
                    {:else}
                      {tt("tools.action.install")}
                    {/if}
                  </button>
                  {#if toolsStore.installingTool === tool.name}
                    <button
                      type="button"
                      class="btn btn--small"
                      onclick={() => void toolsStore.cancelInstall()}
                    >
                      {tt("tools.action.cancel")}
                    </button>
                  {/if}
                {:else if state.state === "notYetAvailable"}
                  <span class="field__hint">{tt("tools.install.unavailable")}</span>
                {/if}
                <button
                  type="button"
                  class="btn btn--small"
                  disabled={toolsStore.busy}
                  onclick={() => void toolsStore.test(tool.name)}
                >
                  {tt("tools.action.test")}
                </button>
                {#if canUseOwnPath(tool)}
                  <button
                    type="button"
                    class="btn btn--small"
                    disabled={toolsStore.busy}
                    onclick={() => void chooseOwnPath(tool.name)}
                  >
                    <FolderOpenIcon size={14} aria-hidden="true" focusable="false" />
                    {tt("tools.action.ownPath")}
                  </button>
                {/if}
                {#if canUseSystemPath(tool)}
                  <button
                    type="button"
                    class="btn btn--small"
                    disabled={toolsStore.busy}
                    onclick={() => void toolsStore.useSystemPath(tool.name)}
                  >
                    {tt("tools.action.useSystemPath")}
                  </button>
                {/if}
                {#if tool.source === "override" || Boolean(settingsStore.settings.toolPaths[tool.name])}
                  <button
                    type="button"
                    class="btn btn--small"
                    disabled={toolsStore.busy}
                    onclick={() => void toolsStore.setToolPath(tool.name, null)}
                  >
                    {tt("tools.action.clearPath")}
                  </button>
                {/if}
              </div>
            </td>
          </tr>
          {#if tool.path}
            {@const bannerParts = splitToolBanner(tool.banner)}
            <tr>
              <td class="tools__detail" colspan="5">
                <span class="tools__path">{tool.path}</span>
                {#if bannerParts}
                  <span class="tools__banner-head field__hint">{bannerParts.headline}</span>
                  {#if bannerParts.details}
                    <details class="tools__banner-details">
                      <summary>{tt("common.technicalDetails")}</summary>
                      <pre>{bannerParts.details}</pre>
                    </details>
                  {/if}
                {/if}
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
  </div>

  <section class="card tools__group" aria-labelledby="heading-channel">
    <h3 class="tools__group-title" id="heading-channel">{tt("tools.channel.title")}</h3>
    <div class="field">
      <label class="field__label" for="ytdlp-channel">{tt("tools.channel.label")}</label>
      <select
        class="select"
        id="ytdlp-channel"
        value={settingsStore.settings.ytdlpChannel}
        disabled={toolsStore.busy}
        onchange={(event) =>
          void toolsStore.setYtdlpChannel((event.currentTarget as HTMLSelectElement).value)}
      >
        <option value="stable">{tt("tools.channel.stable")}</option>
        <option value="nightly">{tt("tools.channel.nightly")}</option>
      </select>
      <p class="field__hint">{tt("tools.channel.hint")}</p>
    </div>
  </section>

  <section class="card tools__group" aria-labelledby="heading-paths">
    <h3 class="tools__group-title" id="heading-paths">{tt("tools.paths.title")}</h3>
    <dl class="tools__paths">
      <dt>{tt("tools.paths.target")}</dt>
      <dd>{toolsStore.view.triple || "—"}</dd>
      <dt>{tt("tools.paths.tools")}</dt>
      <dd>{toolsStore.view.toolsDir || "—"}</dd>
      <dt>{tt("tools.paths.bundled")}</dt>
      <dd>{toolsStore.view.bundledDir || "—"}</dd>
      <dt>{tt("tools.paths.temp")}</dt>
      <dd>{toolsStore.view.tempDir ?? tt("tools.paths.temp.unavailable")}</dd>
    </dl>
    <p class="field__hint">{tt("tools.paths.hint")}</p>
  </section>

  <section class="card tools__group" aria-labelledby="heading-selftest">
    <h3 class="tools__group-title" id="heading-selftest">{tt("tools.selfTest.title")}</h3>
    <p class="field__hint">{tt("tools.selfTest.hint")}</p>
    <button
      type="button"
      class="btn"
      disabled={toolsStore.selfTestRunning}
      onclick={() => void toolsStore.runSelfTest()}
    >
      <PlayIcon size={16} aria-hidden="true" focusable="false" />
      {toolsStore.selfTestRunning
        ? tt("tools.selfTest.running")
        : tt("tools.selfTest.button")}
    </button>
    <p class="tools__note" role="status">
      {#if toolsStore.selfTest === null}
        {tt("tools.selfTest.never")}
      {:else if toolsStore.selfTest.ok}
        {tt("tools.selfTest.ok", {
          runtime: toolsStore.selfTest.runtime ?? "—",
          seconds: (toolsStore.selfTest.elapsedMs / 1000).toFixed(1),
          formats: toolsStore.selfTest.formats,
        })}
      {:else}
        {tt("tools.selfTest.failed", {
          message: toolsStore.selfTest.error
            ? errorText(toolsStore.selfTest.error.messageKey, toolsStore.selfTest.error.detail)
            : tt("error.ytdlp.unknown"),
        })}
      {/if}
    </p>
  </section>

  <section class="card tools__group" aria-labelledby="heading-wizard">
    <h3 class="tools__group-title" id="heading-wizard">{tt("tools.wizard.title")}</h3>
    <p class="field__hint">{tt("tools.wizard.hint")}</p>
    <button type="button" class="btn" onclick={() => (wizardOpen = true)}>
      <WrenchIcon size={16} aria-hidden="true" focusable="false" />
      {tt("tools.wizard.open")}
    </button>
  </section>

  {#if wizardOpen}
    <SetupWizard
      onfinish={async () => {
        wizardOpen = false;
        await toolsStore.completeSetup();
      }}
    />
  {/if}
</section>

<style>
  .tools {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .tools__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .tools__title {
    font-size: 16px;
  }

  .tools__error {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--danger);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: 13px;
  }

  .tools__note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    font-size: 13px;
  }

  .tools__note-detail {
    color: var(--text-muted);
  }

  .tools__table-wrap {
    padding: 0;
    overflow-x: auto;
  }

  .tools__table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .tools__table th,
  .tools__table td {
    padding: var(--space-2) var(--space-3);
    text-align: left;
    vertical-align: middle;
    border-bottom: 1px solid var(--border);
  }

  .tools__table tr.tools__row--has-detail > th,
  .tools__table tr.tools__row--has-detail > td {
    border-bottom: none;
  }

  .tools__table tbody tr:last-child > th,
  .tools__table tbody tr:last-child > td {
    border-bottom: none;
  }

  .tools__table thead th {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }

  .tools__name {
    font-weight: 600;
  }

  .tools__name-inner {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tools__name-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .tools__badge {
    padding: 1px var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-size: 10px;
    text-transform: uppercase;
    color: var(--text-muted);
    white-space: nowrap;
  }

  .tools__status {
    display: inline-block;
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    font-size: 11px;
    border: 1px solid var(--border-strong);
    white-space: nowrap;
  }

  .tools__status[data-state="notInstalled"],
  .tools__status[data-state="belowMinVersion"] {
    border-color: var(--danger);
    color: var(--danger);
  }

  .tools__actions-inner {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    align-items: center;
  }

  .tools__detail {
    background: var(--surface-2);
    padding: var(--space-2) var(--space-3);
  }

  .tools__path {
    display: block;
    font-family: var(--font-mono);
    font-size: 11px;
    word-break: break-all;
  }

  .tools__banner-head {
    display: block;
    font-family: var(--font-mono);
    font-size: 11px;
    margin-top: 2px;
  }

  .tools__banner-details {
    margin-top: var(--space-1);
    font-size: 11px;
  }

  .tools__banner-details summary {
    cursor: pointer;
    color: var(--text-muted);
  }

  .tools__banner-details pre {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text-muted);
  }

  .tools__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    align-items: flex-start;
  }

  .tools__group .field {
    width: 100%;
    max-width: 360px;
  }

  .tools__group .tools__note {
    width: 100%;
  }

  .tools__group-title {
    font-size: 15px;
    font-weight: 700;
  }

  .tools__paths {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    font-size: 12px;
  }

  .tools__paths dt {
    color: var(--text-muted);
  }

  .tools__paths dd {
    margin: 0;
    font-family: var(--font-mono);
    word-break: break-all;
  }
</style>