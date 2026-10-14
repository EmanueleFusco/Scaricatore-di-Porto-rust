<script lang="ts">
  /**
   * First-run Setup wizard (spec §11.1).
   *
   * Steps 1, 2 and 4 exist in M0; step 3 (Transcription) is a skippable
   * placeholder because the model manager arrives in M5, and spec §11.1 is
   * explicit that every step is skippable and that the app stays usable if the
   * user skips everything.
   *
   * It is a `role="dialog"` rather than a new window: a modal that traps focus
   * is the one dialog pattern a desktop app can do accessibly without pulling
   * in a component library (spec §1, "no UI framework runtime").
   */
  import { onMount } from "svelte";
  import XIcon from "lucide-svelte/icons/x";
  import { languageName, type LangPreference } from "$lib/i18n";
  import type { ThemePreference } from "$lib/theme";
  import {
    readyToolCount,
    toolState,
    wizardNext,
    wizardPrevious,
    WIZARD_STEPS,
    type WizardStep,
  } from "$lib/tools";
  import { settingsStore, tt } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  interface Props {
    /** Dismiss without marking the wizard as done. */
    onclose: () => void;
    /** Dismiss and remember not to show it again. */
    onfinish: () => void | Promise<void>;
  }

  const { onclose, onfinish }: Props = $props();

  let step = $state<WizardStep>("welcome");
  let dialog = $state<HTMLDivElement | null>(null);

  const index = $derived(WIZARD_STEPS.indexOf(step));
  const isLast = $derived(index === WIZARD_STEPS.length - 1);
  const ready = $derived(readyToolCount(toolsStore.view.tools));

  onMount(() => {
    void toolsStore.refresh();
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
    await onfinish();
  }

  const languageOptions: LangPreference[] = ["system", "it", "en"];
  const themeOptions: { value: ThemePreference; key: string }[] = [
    { value: "system", key: "settings.theme.system" },
    { value: "dark", key: "settings.theme.dark" },
    { value: "light", key: "settings.theme.light" },
  ];
</script>

<div
  class="wizard__backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.target === event.currentTarget) onclose();
  }}
>
  <div
    class="wizard card"
    role="dialog"
    aria-modal="true"
    aria-labelledby="wizard-title"
    tabindex="-1"
    bind:this={dialog}
    onkeydown={(event) => {
      // Escape closes: a wizard you cannot get out of is a trap (spec §11.9).
      if (event.key === "Escape") onclose();
    }}
  >
    <header class="wizard__header">
      <h2 class="wizard__title" id="wizard-title">
        {tt("wizard.title")}
        <span class="wizard__counter">{index + 1} / {WIZARD_STEPS.length}</span>
      </h2>
      <button type="button" class="btn" onclick={onclose} aria-label={tt("common.close")}>
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
      {:else if step === "tools"}
        <p>{tt("wizard.tools.body", { count: ready })}</p>
        <ul class="wizard__tools">
          {#each toolsStore.view.tools as tool (tool.name)}
            {@const state = toolState(tool)}
            <li>
              <span class="wizard__tool-name">{tool.name}</span>
              <span class="wizard__tool-state">{tt(state.key)}</span>
              {#if tool.version}
                <span class="field__hint">{tool.version}</span>
              {/if}
            </li>
          {/each}
        </ul>
        <p class="field__hint">{tt("wizard.tools.bundled")}</p>
        <p class="field__hint">{tt("wizard.tools.ffmpeg")}</p>
        <p class="field__hint">{tt("wizard.tools.deno")}</p>
      {:else if step === "transcription"}
        <p>{tt("wizard.transcription.body")}</p>
      {:else}
        <p>{tt("wizard.output.body")}</p>
        <div class="field">
          <span class="field__label">{tt("settings.outputDir")}</span>
          <output class="wizard__output">
            {settingsStore.settings.defaultOutputDir ?? tt("settings.outputDir.system")}
          </output>
          <p class="field__hint">{tt("wizard.output.comingSoon")}</p>
        </div>
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
      <button type="button" class="btn" onclick={() => void onfinish()}>
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
    width: min(560px, 100%);
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

  .wizard__tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .wizard__tools li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }

  .wizard__tool-name {
    font-weight: 600;
    min-width: 72px;
  }

  .wizard__output {
    display: block;
    padding: var(--space-2) var(--space-3);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    word-break: break-all;
  }

  .wizard__footer {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }
</style>