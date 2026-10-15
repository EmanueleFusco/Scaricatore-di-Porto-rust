<!--
  The toast area (spec §11.7).

  Two sources: the settings/queue recovery messages and the `toast` that rides
  on a `queue:update` event (an automatic retry, "the queue went idle with
  errors"). Transient information goes here, persistent problems go inline on
  the item — that is the split §11.7 asks for.
-->
<script lang="ts">
  import { t } from "$lib/i18n";
  import { activeLang, tt } from "$lib/stores/settings.svelte";
  import { queueStore } from "$lib/stores/queue.svelte";

  const toast = $derived(queueStore.toast);
  const text = $derived(toast === null ? "" : t(activeLang(), toast.messageKey, toast.params));

  function dismiss(): void {
    queueStore.dismissToast();
  }
</script>

{#if toast !== null}
  <div class="toast" role="status">
    <span class="toast__text">{text}</span>
    <button type="button" class="btn btn--small" onclick={dismiss}>
      {tt("toast.dismiss")}
    </button>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-6);
    z-index: 20;
    display: flex;
    gap: var(--space-3);
    align-items: center;
    max-width: 420px;
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: var(--shadow-2);
    font-size: 13px;
  }

  .toast__text {
    overflow-wrap: anywhere;
  }

  .btn--small {
    height: 28px;
    padding: 0 var(--space-2);
    font-size: 12px;
    white-space: nowrap;
  }
</style>