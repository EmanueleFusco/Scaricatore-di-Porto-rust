<script lang="ts">
  /**
   * Application shell: sidebar, content area, status footer (spec §11.2).
   * The window itself is sized in `tauri.conf.json` (980x680, min 640x520).
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import FooterStatus from "$lib/components/FooterStatus.svelte";
  import SetupWizard from "$lib/components/SetupWizard.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Toast from "$lib/components/Toast.svelte";
  import QueuePage from "$lib/pages/QueuePage.svelte";
  import SettingsPage from "$lib/pages/SettingsPage.svelte";
  import { router } from "$lib/router.svelte";
  import { isTauriRuntime, settingsStore, tt, activeLang } from "$lib/stores/settings.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { themeStore } from "$lib/stores/theme.svelte";

  /** Spec §11.1: the wizard shows once. */
  let wizardOpen = $state(false);

  /**
   * Tell the backend which language is actually in effect.
   *
   * The system notification of spec §7.7 is built in Rust, and Tauri v2 exposes
   * no OS locale to Rust (checked against the 2.12 sources), so the webview —
   * which resolves `navigator.languages` properly — is the one that can say.
   * Best effort: a failed call only leaves the notification in the persisted
   * language.
   */
  async function pushLanguage(): Promise<void> {
    if (!isTauriRuntime()) return;
    await invoke("app_set_language", { lang: activeLang() }).catch(() => {});
  }

  onMount(() => {
    router.start();
    // Start listening to the OS theme first, then apply the stored
    // preference on top of it once the settings have been read.
    void (async () => {
      await themeStore.start();
      await settingsStore.load();
      // Only after the settings are known: the wizard must not appear on top
      // of the loading state and must not be re-opened by a failed read.
      if (isTauriRuntime() && !settingsStore.settings.setupWizardCompleted) {
        wizardOpen = true;
      }
      void toolsStore.refresh();
      // Spec §11.9: the backend needs the resolved language for the system
      // notification (spec §7.7), and the webview is the only place that can
      // resolve "System" against `navigator.languages`. Sent after the settings
      // load, because `activeLang()` reads them.
      await pushLanguage();
    })();
    // `queue:update` (spec §7.6) is app-wide, not QueuePage-local: the footer
    // reads the queue, and a component that unmounts must not be able to lose a
    // completion event.
    void queueStore.start();
    return () => {
      void queueStore.stop();
    };
  });
</script>

<div class="shell">
  <a class="skip-link" href="#main" draggable="false">{tt("a11y.skipToContent")}</a>
  <Sidebar />
  <div class="shell__body">
    <Toast />
    <main class="shell__main" id="main" tabindex="-1">
      {#if !settingsStore.loaded}
        <p class="shell__loading">{tt("footer.placeholder")}</p>
      {:else if router.isActive("/settings")}
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

{#if wizardOpen}
  <SetupWizard
    onclose={() => (wizardOpen = false)}
    onfinish={async () => {
      wizardOpen = false;
      await toolsStore.completeSetup();
    }}
  />
{/if}

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