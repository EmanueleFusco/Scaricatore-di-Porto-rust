<script lang="ts">
  /** Queue page placeholder (spec §11.2 empty state). The engine lands in M2. */
  import { tt } from "$lib/stores/settings.svelte";
</script>

<svelte:head>
  <title>{tt("queue.title")} — {tt("app.name")}</title>
</svelte:head>

<section class="queue">
  <h1 class="queue__title">{tt("queue.title")}</h1>

  <div class="queue__empty card">
    <p class="queue__empty-title">{tt("queue.empty.title")}</p>
    <p class="queue__empty-hint">{tt("queue.empty.hint")}</p>
    <p class="queue__empty-note">{tt("queue.comingSoon")}</p>
  </div>
</section>

<style>
  .queue {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 720px;
    margin: 0 auto;
  }

  .queue__title {
    font-size: 20px;
  }

  .queue__empty {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-6);
    text-align: center;
  }

  .queue__empty-title {
    font-size: 16px;
    font-weight: 700;
  }

  .queue__empty-hint,
  .queue__empty-note {
    color: var(--text-muted);
  }
</style>