<script lang="ts">
  /**
   * First-run Setup wizard (spec §11.1).
   *
   * Every step is optional and the app stays usable if the user skips all of
   * them: the wizard only makes the two things a new user needs — the media
   * tools and an output folder — reachable without hunting through Settings.
   *
   * It is a `role="dialog"` rather than a new window: a modal that traps focus
   * is the one dialog pattern a desktop app can do accessibly without pulling
   * in a component library (spec §1, "no UI framework runtime").
   */
  import { onMount } from "svelte";
  import CheckIcon from "lucide-svelte/icons/check";
  import DownloadIcon from "lucide-svelte/icons/download";
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import RefreshCwIcon from "lucide-svelte/icons/refresh-cw";
  import XIcon from "lucide-svelte/icons/x";
  import { open } from "@tauri-apps/plugin-dialog";
  import { languageName, type LangPreference } from "$lib/i18n";
  import { formatBytes, type ModelViewItem } from "$lib/models";
  import type { ThemePreference } from "$lib/theme";
  import {
    readyToolCount,
    toolState,
    wizardNext,
    wizardPrevious,
    WIZARD_STEPS,
    type ToolStatus,
    type WizardStep,
  } from "$lib/tools";
  import { modelsStore } from "$lib/stores/models.svelte";
  import { errorText, settingsStore, tt } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  interface Props {
    /** Dismiss and remember not to show the wizard again. */
    onfinish: () => void | Promise<void>;
  }

  const { onfinish }: Props = $props();

  /** Tools that are needed for downloading and media processing. */
  const REQUIRED_TOOLS = ["yt-dlp", "qjs", "ffmpeg"] as const;

  let step = $state<WizardStep>("welcome");
  let dialog = $state<HTMLDivElement | null>(null);
  let finishing = $state(false);

  const index = $derived(WIZARD_STEPS.indexOf(step));
  const isLast = $derived(index === WIZARD_STEPS.length - 1);
  const ready = $derived(readyToolCount(toolsStore.view.tools));
  const rows = $derived(
    toolsStore.view.tools
      .slice()
      .sort((left, right) => {
        const order = (tool: ToolStatus): number => {
          const at = REQUIRED_TOOLS.indexOf(tool.name as (typeof REQUIRED_TOOLS)[number]);
          return at === -1 ? REQUIRED_TOOLS.length : at;
        };
        return order(left) - order(right) || left.name.localeCompare(right.name);
      }),
  );
  const missingRequired = $derived(
    rows.filter(
      (tool) =>
        REQUIRED_TOOLS.includes(tool.name as (typeof REQUIRED_TOOLS)[number]) &&
        tool.installable &&
        !tool.path,
    ),
  );
  const recommendedModel = $derived<ModelViewItem | null>(
    modelsStore.view.models.find((model) => model.id === modelsStore.view.recommendedModel) ??
      modelsStore.view.models.find((model) => model.recommended) ??
      null,
  );
  const modelDownloading = $derived(
    recommendedModel !== null &&
      (modelsStore.downloadingModelId === recommendedModel.id ||
        recommendedModel.state.kind === "downloading"),
  );

  onMount(() => {
    void toolsStore.refresh();
    void modelsStore.init();
    // Move focus into the dialog so the keyboard follows the eye (spec §11.9).
    dialog?.focus();
  });

  async function go(target: WizardStep): Promise<void> {
    step = target;
    dialog?.focus();
  }

  async function next(): Promise<void> {
    const target = wizardNext(step);
    if (target) {
      await go(target);
      return;
    }
    await finish();
  }

  /** Every exit path marks the wizard as done, so it does not reappear. */
  async function finish(): Promise<void> {
    if (finishing) return;
    finishing = true;
    try {
      await onfinish();
    } finally {
      finishing = false;
    }
  }

  /** Install the required tools that are missing, one after the other. */
  async function installMissing(): Promise<void> {
    const targets = missingRequired.filter((tool) => tool.name !== "ffprobe").map((tool) => tool.name);
    for (const name of targets) {
      const installed = await toolsStore.install(name);
      // A cancel or a failure stops the batch: the row already explains itself.
      if (!installed) break;
    }
  }

  async function chooseOutputDir(): Promise<void> {
    const picked = await open({
      directory: true,
      multiple: false,
      title: tt("settings.outputDir.chooseTitle"),
    });
    if (typeof picked === "string") {
      await settingsStore.update({ defaultOutputDir: picked });
    }
  }

  function installButtonLabel(tool: ToolStatus): string {
    if (toolsStore.installingTool === tool.name) {
      return toolsStore.installPercent !== null
        ? tt("tools.action.installing", { percent: toolsStore.installPercent })
        : tt("tools.action.installingUnknown");
    }
    return tt("tools.action.install");
  }

  function modelStateLabel(model: ModelViewItem): string {
    if (model.state.kind === "installed") return tt("wizard.transcription.installed");
    if (model.state.kind === "downloading") {
      const percent = modelsStore.downloadPercent;
      return percent !== null
        ? tt("wizard.transcription.downloading", { percent })
        : tt("wizard.transcription.downloadingUnknown");
    }
    if (model.state.kind === "paused") return tt("wizard.transcription.paused");
    return tt("wizard.transcription.notDownloaded");
  }

  /** Keep Tab inside the modal (spec §11.9). */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      void finish();
      return;
    }
    if (event.key !== "Tab" || !dialog) return;
    const focusable = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((element) => element.offsetParent !== null);
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === dialog)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const languageOptions: LangPreference[] = ["system", "it", "en"];
  const themeOptions: { value: ThemePreference; key: string }[] = [
    { value: "system", key: "settings.theme.system" },
    { value: "dark", key: "settings.theme.dark" },
    { value: "light", key: "settings.theme.light" },
  ];
  const engineOptions = [
    { value: "subtitles_whisper_gemini", key: "transcription.engine.subtitlesWhisperGemini" },
    { value: "whisper_only", key: "transcription.engine.whisperOnly" },
    { value: "gemini_only", key: "transcription.engine.geminiOnly" },
    { value: "subtitles_only", key: "transcription.engine.subtitlesOnly" },
  ];
</script>

<div
  class="wizard__backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.target === event.currentTarget) void finish();
  }}
>
  <div
    class="wizard card"
    role="dialog"
    aria-modal="true"
    aria-labelledby="wizard-title"
    tabindex="-1"
    bind:this={dialog}
    onkeydown={onKeydown}
  >
    <header class="wizard__header">
      <h2 class="wizard__title" id="wizard-title">
        {tt("wizard.title")}
        <span class="wizard__counter">{index + 1} / {WIZARD_STEPS.length}</span>
      </h2>
      <button type="button" class="btn" onclick={() => void finish()} aria-label={tt("common.close")}>
        <XIcon size={16} aria-hidden="true" focusable="false" />
      </button>
    </header>

    <ol class="wizard__dots" aria-hidden="true">
      {#each WIZARD_STEPS as name, position (name)}
        <li class="wizard__dot" class:wizard__dot--current={position === index}></li>
      {/each}
    </ol>

    <div class="wizard__body">
      {#if step === "welcome"}
        <p>{tt("wizard.welcome.body")}</p>
        <p class="field__hint">{tt("wizard.welcome.hint")}</p>
        <div class="wizard__grid">
          <div class="field">
            <label class="field__label" for="wizard-language">{tt("settings.language")}</label>
            <select
              class="select"
              id="wizard-language"
              value={settingsStore.settings.language}
              onchange={(event) =>
                void settingsStore.update({
                  language: (event.currentTarget as HTMLSelectElement).value as LangPreference,
                })}
            >
              {#each languageOptions as option (option)}
                <option value={option}>
                  {option === "system" ? tt("settings.language.system") : languageName(option)}
                </option>
              {/each}
            </select>
          </div>
          <div class="field">
            <label class="field__label" for="wizard-theme">{tt("settings.theme")}</label>
            <select
              class="select"
              id="wizard-theme"
              value={settingsStore.settings.theme}
              onchange={(event) =>
                void settingsStore.update({
                  theme: (event.currentTarget as HTMLSelectElement).value as ThemePreference,
                })}
            >
              {#each themeOptions as option (option.value)}
                <option value={option.value}>{tt(option.key)}</option>
              {/each}
            </select>
          </div>
        </div>
      {:else if step === "tools"}
        <p>{tt("wizard.tools.body", { count: ready })}</p>
        {#if toolsStore.errorKey}
          <p class="wizard__error" role="alert">
            {errorText(toolsStore.errorKey, toolsStore.errorDetail)}
          </p>
        {/if}
        <ul class="wizard__tools">
          {#each rows as tool (tool.name)}
            {@const state = toolState(tool)}
            <li class="wizard__tool" class:wizard__tool--problem={state.problem}>
              <span class="wizard__tool-name">
                {tool.name}
                {#if tool.role}<span class="field__hint">{tool.role}</span>{/if}
              </span>
              <span class="wizard__tool-state" data-state={state.state}>{tt(state.key)}</span>
              {#if tool.version}
                <span class="field__hint">{tool.version}</span>
              {/if}
              {#if tool.installable && !tool.path}
                <button
                  type="button"
                  class="btn"
                  disabled={toolsStore.busy}
                  onclick={() => void toolsStore.install(tool.name)}
                >
                  {installButtonLabel(tool)}
                </button>
              {/if}
            </li>
          {/each}
        </ul>
        <div class="wizard__actions">
          {#if missingRequired.length > 0}
            <button
              type="button"
              class="btn btn--primary"
              disabled={toolsStore.busy}
              onclick={() => void installMissing()}
            >
              <DownloadIcon size={16} aria-hidden="true" focusable="false" />
              {toolsStore.busy
                ? tt("wizard.tools.installing")
                : tt("wizard.tools.installMissing")}
            </button>
          {:else}
            <p class="wizard__ok" role="status">
              <CheckIcon size={16} aria-hidden="true" focusable="false" />
              {tt("wizard.tools.allReady")}
            </p>
          {/if}
          <button
            type="button"
            class="btn"
            disabled={toolsStore.busy}
            onclick={() => void toolsStore.refresh()}
          >
            <RefreshCwIcon size={16} aria-hidden="true" focusable="false" />
            {tt("tools.refresh")}
          </button>
          {#if toolsStore.installingTool}
            <button type="button" class="btn" onclick={() => void toolsStore.cancelInstall()}>
              {tt("tools.action.cancel")}
            </button>
          {/if}
        </div>
        <p class="field__hint">{tt("wizard.tools.bundled")}</p>
        <p class="field__hint">{tt("wizard.tools.ffmpeg")}</p>
        <p class="field__hint">{tt("wizard.tools.deno")}</p>
      {:else if step === "transcription"}
        <p>{tt("wizard.transcription.body")}</p>
        <div class="field">
          <label class="field__label" for="wizard-engine">
            {tt("wizard.transcription.engine")}
          </label>
          <select
            class="select"
            id="wizard-engine"
            value={settingsStore.settings.transcriptionEngineOrder}
            onchange={(event) =>
              void settingsStore.update({
                transcriptionEngineOrder: (event.currentTarget as HTMLSelectElement).value,
              })}
          >
            {#each engineOptions as option (option.value)}
              <option value={option.value}>{tt(option.key)}</option>
            {/each}
          </select>
          <p class="field__hint">{tt("transcription.engineOrder.hint")}</p>
        </div>

        {#if modelsStore.errorKey}
          <p class="wizard__error" role="alert">
            {errorText(modelsStore.errorKey, modelsStore.errorDetail)}
          </p>
        {/if}

        {#if recommendedModel}
          <div class="wizard__model">
            <div>
              <span class="wizard__model-name">{recommendedModel.label}</span>
              <span class="field__hint">
                {tt("wizard.transcription.modelHint", {
                  size: formatBytes(recommendedModel.sizeBytes),
                })}
              </span>
            </div>
            <span class="wizard__tool-state">{modelStateLabel(recommendedModel)}</span>
            {#if modelDownloading}
              <div
                class="wizard__progress"
                role="progressbar"
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow={modelsStore.downloadPercent ?? undefined}
                aria-label={tt("wizard.transcription.downloadingUnknown")}
              >
                <div
                  class="wizard__progress-bar"
                  style="width: {modelsStore.downloadPercent ?? 100}%"
                ></div>
              </div>
              <button
                type="button"
                class="btn"
                onclick={() => void modelsStore.cancelDownload(recommendedModel.id)}
              >
                {tt("tools.action.cancel")}
              </button>
            {:else if recommendedModel.state.kind === "installed"}
              <p class="wizard__ok">
                <CheckIcon size={16} aria-hidden="true" focusable="false" />
                {tt("wizard.transcription.ready")}
              </p>
            {:else if recommendedModel.state.kind === "paused"}
              <button
                type="button"
                class="btn"
                onclick={() => void modelsStore.download(recommendedModel.id)}
              >
                {tt("wizard.transcription.resume")}
              </button>
            {:else}
              <button
                type="button"
                class="btn btn--primary"
                disabled={modelsStore.busy}
                onclick={() => void modelsStore.download(recommendedModel.id)}
              >
                <DownloadIcon size={16} aria-hidden="true" focusable="false" />
                {tt("wizard.transcription.download")}
              </button>
            {/if}
          </div>
        {/if}
        <p class="field__hint">{tt("wizard.transcription.gemini")}</p>
        <p class="field__hint">{tt("wizard.transcription.later")}</p>
      {:else}
        <p>{tt("wizard.output.body")}</p>
        <div class="field">
          <span class="field__label">{tt("settings.outputDir")}</span>
          <output class="wizard__output">
            {settingsStore.settings.defaultOutputDir ?? tt("settings.outputDir.system")}
          </output>
          <div class="wizard__actions">
            <button type="button" class="btn" onclick={() => void chooseOutputDir()}>
              <FolderOpenIcon size={16} aria-hidden="true" focusable="false" />
              {tt("settings.outputDir.choose")}
            </button>
            {#if settingsStore.settings.defaultOutputDir}
              <button
                type="button"
                class="btn"
                onclick={() => void settingsStore.update({ defaultOutputDir: null })}
              >
                {tt("settings.outputDir.clear")}
              </button>
            {/if}
          </div>
          <p class="field__hint">{tt("wizard.output.hint")}</p>
        </div>
        <p class="wizard__summary">
          {tt("wizard.output.summary", {
            ready,
            total: rows.length,
          })}
        </p>
      {/if}
    </div>

    <footer class="wizard__footer">
      <button
        type="button"
        class="btn"
        disabled={wizardPrevious(step) === null}
        onclick={() => {
          const previous = wizardPrevious(step);
          if (previous) void go(previous);
        }}
      >
        {tt("wizard.previous")}
      </button>
      <button type="button" class="btn" onclick={() => void finish()}>
        {tt("wizard.skip")}
      </button>
      <button type="button" class="btn btn--primary" onclick={() => void next()}>
        {isLast ? tt("wizard.finish") : tt("wizard.next")}
      </button>
    </footer>
  </div>
</div>

<style>
  .wizard__backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-5);
    background: rgb(0 0 0 / 45%);
    z-index: 20;
  }

  .wizard {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: min(600px, 100%);
    max-height: 100%;
    overflow: auto;
  }

  .wizard:focus {
    outline: none;
  }

  .wizard__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .wizard__title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: 16px;
  }

  .wizard__counter {
    font-size: 12px;
    font-weight: 400;
    color: var(--text-muted);
  }

  .wizard__dots {
    display: flex;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .wizard__dot {
    width: 18px;
    height: 4px;
    border-radius: 2px;
    background: var(--border-strong);
  }

  .wizard__dot--current {
    background: var(--accent);
  }

  .wizard__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    font-size: 13px;
  }

  .wizard__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-3);
  }

  .wizard__tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .wizard__tool {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .wizard__tool--problem {
    border-color: var(--danger);
  }

  .wizard__tool-name {
    display: flex;
    flex-direction: column;
    font-weight: 600;
    min-width: 120px;
  }

  .wizard__tool-state {
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 11px;
  }

  .wizard__tool-state[data-state="notInstalled"],
  .wizard__tool-state[data-state="belowMinVersion"] {
    border-color: var(--danger);
    color: var(--danger);
  }

  .wizard__actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .wizard__ok {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--accent);
  }

  .wizard__error {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--danger);
    border-radius: var(--radius-md);
    color: var(--danger);
  }

  .wizard__model {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
  }

  .wizard__model > div {
    display: flex;
    flex-direction: column;
    flex: 1 1 220px;
  }

  .wizard__model-name {
    font-weight: 600;
  }

  .wizard__progress {
    flex: 1 1 100%;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--border);
  }

  .wizard__progress-bar {
    height: 100%;
    background: var(--accent);
    transition: width 150ms linear;
  }

  .wizard__output {
    display: block;
    padding: var(--space-2) var(--space-3);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    word-break: break-all;
  }

  .wizard__summary {
    color: var(--text-muted);
  }

  .wizard__footer {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }
</style>
