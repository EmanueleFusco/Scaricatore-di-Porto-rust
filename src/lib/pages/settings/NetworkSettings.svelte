<script lang="ts">
  /**
   * Settings → Network & Accounts (spec §11.8): the JavaScript runtime strategy
   * (spec §5.3.3) and the cookies two ways (spec §8.6).
   *
   * Proxy, custom headers and impersonation are M3; the page says so instead of
   * leaving three empty fields.
   */
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    browserNamesForTriple,
    sanitizePlayerClients,
    type JsRuntimeMode,
  } from "$lib/settings";
  import { settingsStore, tt } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  const modeOptions: { value: JsRuntimeMode; key: string }[] = [
    { value: "auto", key: "settings.jsRuntime.auto" },
    { value: "quickjs", key: "settings.jsRuntime.quickjs" },
    { value: "deno", key: "settings.jsRuntime.deno" },
  ];

  // Safari exists only on macOS; the triple comes from the backend (spec §8.6).
  const browsers = $derived(browserNamesForTriple(toolsStore.view.triple));

  async function chooseCookiesFile(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      title: tt("settings.cookies.file.choose"),
    });
    if (typeof picked === "string") {
      await settingsStore.update({ cookiesFile: picked });
    }
  }
</script>

<section class="settings__group" aria-labelledby="heading-runtime">
  <h2 class="settings__group-title" id="heading-runtime">
    {tt("settings.section.network")}
  </h2>

  <div class="field">
    <label class="field__label" for="setting-js-runtime">{tt("settings.jsRuntime")}</label>
    <select
      class="select"
      id="setting-js-runtime"
      value={settingsStore.settings.jsRuntimeMode}
      onchange={(event) =>
        void settingsStore.update({
          jsRuntimeMode: (event.currentTarget as HTMLSelectElement).value as JsRuntimeMode,
        })}
    >
      {#each modeOptions as option (option.value)}
        <option value={option.value}>{tt(option.key)}</option>
      {/each}
    </select>
    <p class="field__hint">{tt("settings.jsRuntime.hint")}</p>
  </div>

  <div class="field">
    <label class="field__label" for="setting-youtube-clients">
      {tt("settings.youtubeClients")}
    </label>
    <input
      class="input"
      id="setting-youtube-clients"
      type="text"
      autocomplete="off"
      spellcheck="false"
      placeholder={tt("settings.youtubeClients.placeholder")}
      value={settingsStore.settings.youtubePlayerClients ?? ""}
      onchange={(event) =>
        void settingsStore.update({
          youtubePlayerClients: sanitizePlayerClients(
            (event.currentTarget as HTMLInputElement).value,
          ),
        })}
    />
    <p class="field__hint">{tt("settings.youtubeClients.hint")}</p>
  </div>

  <div class="field">
    <label class="field__label" for="setting-cookies-browser">
      {tt("settings.cookies.browser")}
    </label>
    <select
      class="select"
      id="setting-cookies-browser"
      value={settingsStore.settings.cookiesFromBrowser ?? ""}
      onchange={(event) =>
        void settingsStore.update({
          cookiesFromBrowser: (event.currentTarget as HTMLSelectElement).value || null,
        })}
    >
      <option value="">{tt("settings.cookies.none")}</option>
      {#each browsers as browser (browser)}
        <option value={browser}>{browser}</option>
      {/each}
    </select>
    <p class="field__hint">{tt("settings.cookies.hint")}</p>
  </div>

  <div class="field">
    <span class="field__label">{tt("settings.cookies.file")}</span>
    <div class="settings__row">
      <output class="settings__value">
        {settingsStore.settings.cookiesFile ?? tt("settings.cookies.file.none")}
      </output>
      <button type="button" class="btn" onclick={() => void chooseCookiesFile()}>
        {tt("settings.cookies.file.choose")}
      </button>
      {#if settingsStore.settings.cookiesFile}
        <button
          type="button"
          class="btn"
          onclick={() => void settingsStore.update({ cookiesFile: null })}
        >
          {tt("settings.cookies.file.clear")}
        </button>
      {/if}
    </div>
  </div>

  <p class="field__hint">{tt("settings.accounts.comingSoon")}</p>
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

  .settings__row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .settings__value {
    flex: 1 1 240px;
    padding: var(--space-2) var(--space-3);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    font-size: 12px;
    word-break: break-all;
  }
</style>
