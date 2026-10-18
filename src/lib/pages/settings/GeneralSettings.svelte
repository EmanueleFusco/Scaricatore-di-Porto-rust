<script lang="ts">
  /**
   * Settings → General (spec §11.8): language, theme, default output folder.
   * Every control writes through the backend immediately, so there is no
   * "unsaved form" state to get wrong.
   */
  import { languageName, type LangPreference } from "$lib/i18n";
  import { onMount } from "svelte";
  import { clampCap, isPlaylistUrlBehavior, type PlaylistUrlBehavior } from "$lib/settings";
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

  const playlistOptions: { value: PlaylistUrlBehavior; key: string }[] = [
    { value: "ask", key: "settings.playlistUrl.ask" },
    { value: "video", key: "settings.playlistUrl.video" },
    { value: "playlist", key: "settings.playlistUrl.playlist" },
  ];

  /** How many videos `archive.txt` remembers (spec §6.4, §11.8). */
  let archiveEntries = $state(0);
  let archiveCleared = $state(false);

  onMount(() => {
    void settingsStore.archiveStatus().then((view) => (archiveEntries = view.entries));
  });

  async function clearArchive(): Promise<void> {
    const view = await settingsStore.archiveClear();
    archiveEntries = view.entries;
    archiveCleared = true;
    setTimeout(() => (archiveCleared = false), 2500);
  }

  /** What a number field can hand over, clamped (spec §7.2). */
  const cap = (value: number, min: number, max: number, fallback: number): number =>
    clampCap(value, min, max, fallback);
</script>

<section class="settings__group" aria-labelledby="heading-lang">
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
  </div>

  <!-- Spec §7.2, §6.4: the two caps the scheduler obeys. Both are in the
       settings because both are what a user on a slow line or a site with bot
       detection reaches for. -->
  <div class="field">
    <label class="field__label" for="setting-parallel">
      {tt("settings.downloads.parallel")}
    </label>
    <input
      class="input"
      id="setting-parallel"
      type="number"
      min="1"
      max="4"
      step="1"
      value={settingsStore.settings.parallelDownloads}
      onchange={(event) =>
        void settingsStore.update({
          parallelDownloads: cap(Number(event.currentTarget.value), 1, 4, 2),
        })}
    />
    <p class="field__hint">{tt("settings.downloads.parallel.hint")}</p>
  </div>

  <div class="field">
    <label class="field__label" for="setting-per-host">{tt("settings.downloads.perHost")}</label>
    <input
      class="input"
      id="setting-per-host"
      type="number"
      min="1"
      max="8"
      step="1"
      value={settingsStore.settings.perHostCap}
      onchange={(event) =>
        void settingsStore.update({ perHostCap: cap(Number(event.currentTarget.value), 1, 8, 2) })}
    />
    <p class="field__hint">{tt("settings.downloads.perHost.hint")}</p>
  </div>
</section>

<section class="card settings__group" aria-labelledby="heading-files">
  <h2 class="settings__group-title" id="heading-files">{tt("settings.section.files")}</h2>

  <!-- Spec §8.2, §11.8: the `[id]` suffix and the H.264 preference. -->
  <label class="settings__check">
    <input
      type="checkbox"
      checked={settingsStore.settings.includeIdInFilename}
      onchange={(event) =>
        void settingsStore.update({ includeIdInFilename: event.currentTarget.checked })}
    />
    {tt("settings.files.includeId")}
  </label>
  <p class="field__hint">{tt("settings.files.includeId.hint")}</p>

  <label class="settings__check">
    <input
      type="checkbox"
      checked={settingsStore.settings.preferH264}
      onchange={(event) => void settingsStore.update({ preferH264: event.currentTarget.checked })}
    />
    {tt("settings.files.preferH264")}
  </label>
  <p class="field__hint">{tt("settings.files.preferH264.hint")}</p>

  <div class="field">
    <label class="field__label" for="setting-limit-rate">{tt("settings.files.limitRate")}</label>
    <input
      class="input"
      id="setting-limit-rate"
      type="text"
      autocomplete="off"
      spellcheck="false"
      placeholder="5M"
      value={settingsStore.settings.limitRate ?? ""}
      onchange={(event) => void settingsStore.update({ limitRate: event.currentTarget.value })}
    />
    <p class="field__hint">{tt("settings.files.limitRate.hint")}</p>
  </div>
</section>

<section class="card settings__group" aria-labelledby="heading-playlists">
  <h2 class="settings__group-title" id="heading-playlists">{tt("settings.section.playlists")}</h2>

  <!-- Spec §6.4: what a `watch?v=…&list=…` link does. -->
  <div class="field">
    <label class="field__label" for="setting-playlist-url">{tt("settings.playlistUrl")}</label>
    <select
      class="select"
      id="setting-playlist-url"
      value={settingsStore.settings.playlistUrlBehavior}
      onchange={(event) => {
        const value = event.currentTarget.value;
        if (isPlaylistUrlBehavior(value)) void settingsStore.update({ playlistUrlBehavior: value });
      }}
    >
      {#each playlistOptions as option (option.value)}
        <option value={option.value}>{tt(option.key)}</option>
      {/each}
    </select>
    <p class="field__hint">{tt("settings.playlistUrl.hint")}</p>
  </div>

  <!-- Spec §6.4: the download archive, with the way to clear it. -->
  <label class="settings__check">
    <input
      type="checkbox"
      checked={settingsStore.settings.useDownloadArchive}
      onchange={(event) =>
        void settingsStore.update({ useDownloadArchive: event.currentTarget.checked })}
    />
    {tt("settings.archive.use")}
  </label>
  <p class="field__hint">{tt("settings.archive.hint")}</p>
  <div class="settings__row">
    <span class="field__hint" role="status">
      {archiveCleared ? tt("settings.archive.cleared") : tt("settings.archive.count", { count: archiveEntries })}
    </span>
    <button
      type="button"
      class="btn"
      disabled={archiveEntries === 0}
      onclick={() => void clearArchive()}
    >
      {tt("settings.archive.clear")}
    </button>
  </div>
</section>

<section class="card settings__group" aria-labelledby="heading-behaviour">
  <h2 class="settings__group-title" id="heading-behaviour">
    {tt("settings.section.behaviour")}
  </h2>

  <!-- Spec §7.7: one notification when the queue goes idle, off by default. -->
  <label class="settings__check">
    <input
      type="checkbox"
      checked={settingsStore.settings.notifyOnIdle}
      onchange={(event) => void settingsStore.update({ notifyOnIdle: event.currentTarget.checked })}
    />
    {tt("settings.general.notifyIdle")}
  </label>

  <!-- Spec §11.3: Ctrl/Cmd+V anywhere on the page, opt-in. -->
  <label class="settings__check">
    <input
      type="checkbox"
      checked={settingsStore.settings.pasteLinksAnywhere}
      onchange={(event) =>
        void settingsStore.update({ pasteLinksAnywhere: event.currentTarget.checked })}
    />
    {tt("settings.general.pasteAnywhere")}
  </label>
  <p class="field__hint">{tt("settings.general.pasteAnywhere.hint")}</p>
</section>

<style>
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

  .settings__check {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: 13px;
  }

  .settings__row {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    flex-wrap: wrap;
  }

  .input[type="number"] {
    max-width: 120px;
  }
</style>