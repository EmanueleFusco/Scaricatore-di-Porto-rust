<script lang="ts">
  /**
   * Transient banner (spec §11.7: toasts for transient info, inline banners
   * for persistent problems). Currently only the settings-recovery notice
   * exists; the queue will emit more.
   */
  import { settingsStore, tt } from "$lib/stores/settings.svelte";

  let dismissed = $state(false);

  const recovery = $derived(settingsStore.recovery);
  const errorKey = $derived(settingsStore.errorKey);
</script>

{#if !dismissed && recovery}
  <div class="toast" role="status" aria-live="polite">
    <p class="toast__text">{tt("toast.settingsRecovered", { path: recovery.backupPath ?? "" })}</p>
    <button type="button" class="toast__close" onclick={() => (dismissed = true)}>
      {tt("toast.dismiss")}
    </button>
  </div>
{:else if !dismissed && errorKey}
  <div class="toast toast--error" role="alert">
    <p class="toast__text">{tt(errorKey)}</p>
    <button type="button" class="toast__close" onclick={() => (dismissed = true)}>
      {tt("toast.dismiss")}
    </button>
  </div>
{/if}

<style>
  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: var(--space-3) var(--space-4) 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    box-shadow: var(--shadow-2);
  }

  /* Error state is carried by the border style and the alert role, not colour. */
  .toast--error {
    border-style: dashed;
    border-color: var(--danger);
    background: var(--danger-surface);
  }

  .toast__text {
    flex: 1 1 auto;
  }

  .toast__close {
    flex: 0 0 auto;
    min-height: var(--control-height);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    cursor: pointer;
  }
</style>