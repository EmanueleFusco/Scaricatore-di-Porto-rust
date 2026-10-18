<!--
  The "Use cookies" banner of an authentication failure (spec §6.4, §11.4).

  Shown right on the link that needs a session (Watch Later, a private video,
  an age gate…), with the two ways the spec names: the browser's cookie
  database, or a Netscape cookies file. Choosing one saves the setting and
  re-checks the link, so the user never has to find Settings first.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { browserNamesForTriple } from "$lib/settings";
  import { settingsStore, tt } from "$lib/stores/settings.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";

  let { onApplied }: { onApplied: () => void } = $props();

  const browsers = $derived(browserNamesForTriple(toolsStore.view.triple));

  async function useBrowser(browser: string): Promise<void> {
    if (browser === "") return;
    if (await settingsStore.update({ cookiesFromBrowser: browser, cookiesFile: null })) {
      onApplied();
    }
  }

  async function useFile(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      title: tt("settings.cookies.file.choose"),
    });
    if (typeof picked !== "string") return;
    if (await settingsStore.update({ cookiesFile: picked, cookiesFromBrowser: null })) {
      onApplied();
    }
  }
</script>

<div class="cookies" role="group" aria-label={tt("playlist.cookies.title")}>
  <p class="cookies__title">{tt("playlist.cookies.title")}</p>
  <p class="field__hint">{tt("playlist.cookies.hint")}</p>
  <div class="cookies__actions">
    <select
      class="select select--small"
      aria-label={tt("settings.cookies.browser")}
      value=""
      onchange={(event) => void useBrowser(event.currentTarget.value)}
    >
      <option value="">{tt("playlist.cookies.browser")}</option>
      {#each browsers as browser (browser)}
        <option value={browser}>{browser}</option>
      {/each}
    </select>
    <button type="button" class="btn btn--small" onclick={() => void useFile()}>
      {tt("playlist.cookies.file")}
    </button>
  </div>
</div>

<style>
  .cookies {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1 1 100%;
    padding: var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
  }

  .cookies__title {
    font-weight: 700;
    font-size: 13px;
  }

  .cookies__actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
