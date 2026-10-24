<script lang="ts">
  /**
   * Settings shell (spec §11.8): a tab strip with the pages that exist so far.
   *
   * Tabs rather than a growing single page because spec §11.8 lists six of
   * them and a scroll with five cards in it stops being a settings page. Each
   * tab is a real `href`, so the keyboard, middle-click and "open in new tab"
   * keep working (spec §11.9).
   */
  import AboutPage from "$lib/pages/settings/AboutPage.svelte";
  import GeneralSettings from "$lib/pages/settings/GeneralSettings.svelte";
  import NetworkSettings from "$lib/pages/settings/NetworkSettings.svelte";
  import TranscriptionSettings from "$lib/pages/settings/TranscriptionSettings.svelte";
  import ToolsPage from "$lib/pages/ToolsPage.svelte";
  import { router } from "$lib/router.svelte";
  import { aboutStore } from "$lib/stores/about.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  const tabs = [
    { href: "/settings", key: "settings.tab.general", exact: true },
    { href: "/settings/transcription", key: "settings.tab.transcription", exact: false },
    { href: "/settings/tools", key: "settings.tab.tools", exact: false },
    { href: "/settings/network", key: "settings.tab.network", exact: false },
    { href: "/settings/about", key: "settings.tab.about", exact: false },
  ];
</script>

<svelte:head>
  <title>{tt("settings.title")} — {tt("app.name")}</title>
</svelte:head>

<section class="settings">
  <h1 class="settings__title">{tt("settings.title")}</h1>

  <nav class="settings__tabs" aria-label={tt("settings.title")}>
    {#each tabs as tab (tab.href)}
      <a
        class="settings__tab"
        href={tab.href}
        aria-current={(tab.exact ? router.path === tab.href : router.isActive(tab.href))
          ? "page"
          : undefined}
        onclick={(event) => router.follow(event, tab.href)}
      >
        {tt(tab.key)}
        {#if tab.href === "/settings/about" && aboutStore.updateAvailable}
          <span class="settings__tab-dot" aria-hidden="true"></span>
        {/if}
      </a>
    {/each}
  </nav>

  {#if router.isActive("/settings/tools")}
    <ToolsPage />
  {:else if router.isActive("/settings/transcription")}
    <TranscriptionSettings />
  {:else if router.isActive("/settings/network")}
    <NetworkSettings />
  {:else if router.isActive("/settings/about")}
    <AboutPage />
  {:else}
    <GeneralSettings />
  {/if}
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 720px;
    margin: 0 auto;
  }

  .settings__title {
    font-size: 20px;
  }

  .settings__tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    border-bottom: 1px solid var(--border);
  }

  /* Current tab: bold plus an underline, never colour alone (spec §11.9). */
  .settings__tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    text-decoration: none;
  }

  .settings__tab:hover {
    color: var(--text);
  }

  .settings__tab[aria-current="page"] {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 700;
  }

  .settings__tab-dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-pill);
    background: var(--accent);
  }
</style>