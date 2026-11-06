<script lang="ts">
  import { redactDetails } from "$lib/diagnostics";
  import { tt } from "$lib/stores/settings.svelte";
  let { detail, headers = "" }: { detail: string; headers?: string } = $props();
  let copied = $state(false);
  let failed = $state(false);
  const safe = $derived(redactDetails(detail, headers.split(/\r?\n/).map((line) => line.slice(line.indexOf(":") + 1).trim())));
  async function copy(): Promise<void> {
    try { await navigator.clipboard.writeText(safe); copied = true; failed = false; }
    catch { failed = true; }
  }
</script>
<details class="inspection-error">
  <summary>{tt("queue.list.expand")}</summary>
  <pre>{safe}</pre>
  <button type="button" class="btn btn--small" onclick={() => void copy()}>{tt(copied ? "queue.copied" : "queue.copyDetails")}</button>
  {#if failed}<span role="status">{tt("queue.copyFailed")}</span>{/if}
</details>
<style>
  .inspection-error { padding: var(--space-2); border-top: 1px solid var(--border); }
  summary { cursor: pointer; color: var(--text-muted); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 160px; overflow-y: auto; font-size: 12px; user-select: text; }
</style>
