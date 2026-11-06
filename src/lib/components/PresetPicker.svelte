<script lang="ts">
  import { addBarStore } from "$lib/stores/addbar.svelte";
  import { errorText, settingsStore, tt } from "$lib/stores/settings.svelte";
  let name = $state("");
  let busy = $state(false);
  let failed = $state(false);
  const selected = $derived(settingsStore.settings.lastPreset);
  const presets = $derived(settingsStore.settings.savedPresets);
  const knownName = $derived(presets.some((preset) => preset.name.toLocaleLowerCase() === name.trim().toLocaleLowerCase()));
  $effect(() => { name = presets.find((preset) => preset.id === selected)?.name ?? ""; });
  async function run(action: () => Promise<boolean>): Promise<void> {
    busy = true;
    try { failed = !await action(); } finally { busy = false; }
  }
</script>
<details class="presets">
  <summary>{tt("presets.title")}{#if selected} · {presets.find((preset) => preset.id === selected)?.name}{/if}</summary>
  <div class="presets__controls">
    <div class="field">
      <label class="field__label" for="saved-preset">{tt("presets.select")}</label>
      <select class="select" id="saved-preset" disabled={busy} value={selected ?? ""} onchange={(event) => void run(() => addBarStore.selectPreset(event.currentTarget.value || null))}>
        <option value="">{tt("presets.defaults")}</option>
        {#each presets as preset (preset.id)}<option value={preset.id}>{preset.name}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label class="field__label" for="preset-name">{tt("presets.name")}</label>
      <input class="input" id="preset-name" maxlength="80" bind:value={name} disabled={busy} />
    </div>
    <button type="button" class="btn btn--small" disabled={busy || !name.trim() || (!knownName && presets.length >= 20)} onclick={() => void run(() => addBarStore.savePreset(name))}>{tt(knownName ? "presets.update" : "presets.save")}</button>
    {#if selected}<button type="button" class="btn btn--small btn--danger" disabled={busy} onclick={() => void run(() => addBarStore.deletePreset(selected))}>{tt("presets.delete")}</button>{/if}
  </div>
  <p class="field__hint">{tt("presets.hint")}</p>
  {#if failed}<p role="alert">{errorText(settingsStore.errorKey ?? "error.io", null)}</p>{/if}
</details>
<style>
  summary { cursor: pointer; color: var(--text-muted); font-size: 12px; }
  .presets__controls { display: flex; align-items: end; flex-wrap: wrap; gap: var(--space-2); padding-top: var(--space-2); }
</style>
