<script lang="ts">
  /**
   * Application shell: sidebar, content area, status footer (spec §11.2).
   * The window itself is sized in `tauri.conf.json` (980x680, min 640x520).
   */
  import { onMount } from "svelte";
  import FooterStatus from "$lib/components/FooterStatus.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Toast from "$lib/components/Toast.svelte";
  import QueuePage from "$lib/pages/QueuePage.svelte";
  import SettingsPage from "$lib/pages/SettingsPage.svelte";
  import { router } from "$lib/router.svelte";
  import { settingsStore, tt } from "$lib/stores/settings.svelte";
  import { themeStore } from "$lib/stores/theme.svelte";

  onMount(() => {
    router.start();
    // Start listening to the OS theme first, then apply the stored
    // preference on top of it once the settings have been read.
    void (async () => {
      await themeStore.start();
      await settingsStore.load();
    })();
  });
</script>

<div class="shell">
  <a class="skip-link" href="#main">{tt("a11y.skipToContent")}</a>
  <Sidebar />
  <div class="shell__body">
    <Toast />
    <main class="shell__main" id="main" tabindex="-1">
      {#if !settingsStore.loaded}
        <p class="shell__loading">{tt("footer.placeholder")}</p>
      {:else if router.path === "/settings"}
        <SettingsPage />
      {:else if router.path === "/"}
        <QueuePage />
      {:else}
        <p class="shell__loading">{tt("queue.empty.title")}</p>
      {/if}
    </main>
    <FooterStatus />
  </div>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .shell__body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
  }

  .shell__main {
    flex: 1 1 auto;
    overflow: auto;
    padding: var(--space-5);
  }

  .shell__main:focus {
    outline: none;
  }

  .shell__loading {
    color: var(--text-muted);
  }

  .skip-link {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    z-index: 10;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--accent);
    color: var(--on-accent);
    text-decoration: none;
    transform: translateY(-200%);
  }

  .skip-link:focus-visible {
    transform: translateY(0);
  }
</style>