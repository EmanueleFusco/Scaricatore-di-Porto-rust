<script lang="ts">
  /**
   * Settings → General (spec §11.8): language, theme, default output folder.
   * Every control writes through the backend immediately, so there is no
   * "unsaved form" state to get wrong.
   */
  import { languageName, type LangPreference } from "$lib/i18n";
  import { settingsStore, activeLang, tt } from "$lib/stores/settings.svelte";
  import type { ThemePreference } from "$lib/theme";

  const languageOptions: LangPreference[] = ["system", "it", "en"];

  const themeOptions: { value: ThemePreference; key: string }[] = [
    { value: "system", key: "settings.theme.system" },
    { value: "dark", key: "settings.theme.dark" },
    { value: "light", key: "settings.theme.light" },
  ];

  const onLanguageChange = (event: Event) => {
    const value = (event.currentTarget as HTMLSelectElement).value as LangPreference;
    void settingsStore.update({ language: value });
  };

  const onThemeChange = (event: Event) => {
    const value = (event.currentTarget as HTMLSelectElement).value as ThemePreference;
    void settingsStore.update({ theme: value });
  };
</script>

<svelte:head>
  <title>{tt("settings.title")} — {tt("app.name")}</title>
</svelte:head>

<section class="settings">
  <h1 class="settings__title">{tt("settings.title")}</h1>

  <section class="card settings__group" aria-labelledby="heading-lang">
    <h2 class="settings__group-title" id="heading-lang">{tt("settings.section.language")}</h2>

    <div class="field">
      <label class="field__label" for="setting-language">{tt("settings.language")}</label>
      <select
        class="select"
        id="setting-language"
        value={settingsStore.settings.language}
        onchange={onLanguageChange}
      >
        {#each languageOptions as option (option)}
          <!-- Language names are written in the language itself. -->
          <option value={option}>
            {option === "system" ? tt("settings.language.system") : languageName(option)}
          </option>
        {/each}
      </select>
      <p class="field__hint">
        {tt("settings.language.resolved", { lang: languageName(activeLang()) })}
      </p>
    </div>

    <div class="field">
      <label class="field__label" for="setting-theme">{tt("settings.theme")}</label>
      <select
        class="select"
        id="setting-theme"
        value={settingsStore.settings.theme}
        onchange={onThemeChange}
      >
        {#each themeOptions as option (option.value)}
          <option value={option.value}>{tt(option.key)}</option>
        {/each}
      </select>
    </div>
  </section>

  <section class="card settings__group" aria-labelledby="heading-downloads">
    <h2 class="settings__group-title" id="heading-downloads">{tt("settings.section.downloads")}</h2>

    <div class="field">
      <span class="field__label" id="label-output-dir">{tt("settings.outputDir")}</span>
      <output class="settings__value" aria-labelledby="label-output-dir">
        {settingsStore.settings.defaultOutputDir ?? tt("settings.outputDir.system")}
      </output>
      <p class="field__hint">{tt("settings.outputDir.hint")}</p>
      <p class="field__hint">{tt("settings.downloads.comingSoon")}</p>
    </div>
  </section>
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 720px;
    margin: 0 auto;
  }

  .settings__title {
    font-size: 20px;
  }

  .settings__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }

  .settings__group-title {
    font-size: 15px;
  }

  .settings__value {
    display: block;
    padding: var(--space-2) var(--space-3);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    word-break: break-all;
  }
</style>