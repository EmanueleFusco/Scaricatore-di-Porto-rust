<script lang="ts">
  /**
   * Settings → Transcription & Models (spec §10.2, §11.6).
   *
   * Manages transcription engine ordering, default language, VAD silence skipping,
   * CPU threads, and the Whisper model catalogue with download, resume, verification,
   * local import, activation and deletion.
   */
  import { onMount, onDestroy } from "svelte";
  import AlertTriangleIcon from "lucide-svelte/icons/alert-triangle";
  import CheckIcon from "lucide-svelte/icons/check";
  import DownloadIcon from "lucide-svelte/icons/download";
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import RefreshCwIcon from "lucide-svelte/icons/refresh-cw";
  import Trash2Icon from "lucide-svelte/icons/trash-2";
  import UploadIcon from "lucide-svelte/icons/upload";
  import XIcon from "lucide-svelte/icons/x";
  import { open } from "@tauri-apps/plugin-dialog";

  import { filterModels, formatBytes, formatWer, isLargeDownload, type ModelViewItem } from "$lib/models";
  import { modelsStore } from "$lib/stores/models.svelte";
  import { settingsStore, tt } from "$lib/stores/settings.svelte";

  onMount(() => {
    void modelsStore.init();
  });

  onDestroy(() => {
    modelsStore.destroy();
  });

  const engineOrderOptions = [
    { value: "subtitles_whisper_gemini", key: "transcription.engine.subtitlesWhisperGemini" },
    { value: "whisper_only", key: "transcription.engine.whisperOnly" },
    { value: "gemini_only", key: "transcription.engine.geminiOnly" },
    { value: "subtitles_only", key: "transcription.engine.subtitlesOnly" },
  ];

  const languageOptions = [
    { value: "auto", key: "transcription.language.auto" },
    { value: "it", key: "transcription.language.it" },
    { value: "en", key: "transcription.language.en" },
    { value: "es", key: "transcription.language.es" },
    { value: "fr", key: "transcription.language.fr" },
    { value: "de", key: "transcription.language.de" },
  ];

  let confirmingDownloadId = $state<string | null>(null);

  const displayedModels = $derived(
    filterModels(modelsStore.view.models, modelsStore.showFullPrecision),
  );

  function startDownload(model: ModelViewItem): void {
    if (isLargeDownload(model.sizeBytes)) {
      confirmingDownloadId = model.id;
    } else {
      void modelsStore.download(model.id);
    }
  }

  function confirmLargeDownload(modelId: string): void {
    confirmingDownloadId = null;
    void modelsStore.download(modelId);
  }

  function cancelConfirmation(): void {
    confirmingDownloadId = null;
  }

  async function importModelFile(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      title: tt("transcription.models.importDialogTitle"),
      filters: [{ name: "Whisper model (*.bin)", extensions: ["bin"] }],
    });
    if (typeof picked === "string") {
      await modelsStore.importFile(picked);
    }
  }

  function onEngineOrderChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    void settingsStore.update({ transcriptionEngineOrder: value });
  }

  function onDefaultLanguageChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    void settingsStore.update({ defaultTranscriptLanguage: value });
  }

  function onSkipSilenceVadChange(event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    void settingsStore.update({ skipSilenceVad: checked });
  }

  function onTranslateToEnglishChange(event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    void settingsStore.update({ whisperTranslateToEnglish: checked });
  }

  function onTranscriptsSubfolderChange(event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    void settingsStore.update({ transcriptsSubfolder: checked });
  }

  function onWhisperThreadsChange(event: Event): void {
    const raw = Number((event.currentTarget as HTMLInputElement).value);
    const value = Number.isFinite(raw) ? Math.max(0, Math.min(128, Math.round(raw))) : 0;
    void settingsStore.update({ whisperThreads: value });
  }

  function onMirrorUrlChange(event: Event): void {
    const value = (event.currentTarget as HTMLInputElement).value.trim();
    void settingsStore.update({ whisperModelsMirrorUrl: value !== "" ? value : null });
  }
</script>

<div class="transcription-page">
  {#if modelsStore.errorKey}
    <div class="banner banner--error" role="alert">
      <div class="banner__body">
        <strong>{tt(modelsStore.errorKey)}</strong>
        {#if modelsStore.errorDetail}
          <p class="banner__detail">{modelsStore.errorDetail}</p>
        {/if}
      </div>
      <button
        type="button"
        class="banner__close"
        onclick={() => modelsStore.dismissError()}
        aria-label={tt("common.dismiss")}
      >
        <XIcon size={16} aria-hidden="true" />
      </button>
    </div>
  {/if}

  <!-- 1. General Transcription Pipeline Configuration -->
  <section class="card settings__group" aria-labelledby="heading-pipeline">
    <h2 class="settings__group-title" id="heading-pipeline">
      {tt("transcription.section.pipeline")}
    </h2>

    <div class="field">
      <label class="field__label" for="setting-engine-order">
        {tt("transcription.engineOrder")}
      </label>
      <select
        class="select"
        id="setting-engine-order"
        value={settingsStore.settings.transcriptionEngineOrder}
        onchange={onEngineOrderChange}
      >
        {#each engineOrderOptions as option (option.value)}
          <option value={option.value}>{tt(option.key)}</option>
        {/each}
      </select>
      <p class="field__hint">{tt("transcription.engineOrder.hint")}</p>
    </div>

    <div class="field">
      <label class="field__label" for="setting-default-language">
        {tt("transcription.defaultLanguage")}
      </label>
      <select
        class="select"
        id="setting-default-language"
        value={settingsStore.settings.defaultTranscriptLanguage}
        onchange={onDefaultLanguageChange}
      >
        {#each languageOptions as option (option.value)}
          <option value={option.value}>{tt(option.key)}</option>
        {/each}
      </select>
      <p class="field__hint">{tt("transcription.defaultLanguage.hint")}</p>
    </div>

    <div class="toggle-list">
      <label class="toggle-item">
        <input
          type="checkbox"
          checked={settingsStore.settings.skipSilenceVad}
          onchange={onSkipSilenceVadChange}
        />
        <div class="toggle-item__text">
          <span class="toggle-item__title">{tt("transcription.skipSilenceVad")}</span>
          <span class="toggle-item__hint">{tt("transcription.skipSilenceVad.hint")}</span>
        </div>
      </label>

      <label class="toggle-item">
        <input
          type="checkbox"
          checked={settingsStore.settings.whisperTranslateToEnglish}
          onchange={onTranslateToEnglishChange}
        />
        <div class="toggle-item__text">
          <span class="toggle-item__title">{tt("transcription.translateToEnglish")}</span>
          <span class="toggle-item__hint">{tt("transcription.translateToEnglish.hint")}</span>
        </div>
      </label>

      <label class="toggle-item">
        <input
          type="checkbox"
          checked={settingsStore.settings.transcriptsSubfolder}
          onchange={onTranscriptsSubfolderChange}
        />
        <div class="toggle-item__text">
          <span class="toggle-item__title">{tt("transcription.transcriptsSubfolder")}</span>
          <span class="toggle-item__hint">{tt("transcription.transcriptsSubfolder.hint")}</span>
        </div>
      </label>
    </div>
  </section>

  <!-- 2. Whisper Models Management -->
  <section class="card settings__group" aria-labelledby="heading-models">
    <div class="models-header">
      <div class="models-header__title-block">
        <h2 class="settings__group-title" id="heading-models">
          {tt("transcription.section.models")}
        </h2>
        <span class="disk-space-badge">
          {tt("transcription.models.freeSpace", { space: formatBytes(modelsStore.view.freeSpaceBytes) })}
        </span>
      </div>

      <div class="models-header__actions">
        <button
          type="button"
          class="btn btn--secondary"
          disabled={modelsStore.busy}
          onclick={importModelFile}
        >
          <UploadIcon size={16} aria-hidden="true" />
          {tt("transcription.models.importButton")}
        </button>

        <button
          type="button"
          class="btn btn--secondary"
          onclick={() => void modelsStore.openFolder()}
        >
          <FolderOpenIcon size={16} aria-hidden="true" />
          {tt("transcription.models.openFolder")}
        </button>

        <button
          type="button"
          class="btn btn--ghost"
          disabled={modelsStore.loading}
          onclick={() => void modelsStore.refresh()}
          aria-label={tt("common.refresh")}
        >
          <RefreshCwIcon size={16} aria-hidden="true" />
        </button>
      </div>
    </div>

    {#if modelsStore.view.lowCoreSystem}
      <div class="banner banner--info" role="status">
        <AlertTriangleIcon size={16} aria-hidden="true" />
        <p>{tt("transcription.models.lowCoreWarning")}</p>
      </div>
    {/if}

    <!-- VAD status summary note -->
    <div class="vad-banner">
      <div class="vad-banner__info">
        <strong>{tt("transcription.models.vadTitle")}:</strong>
        <span>
          {#if modelsStore.view.vad.installed}
            <span class="badge badge--success">{tt("transcription.models.vadInstalled")}</span>
            ({formatBytes(modelsStore.view.vad.sizeBytes)})
          {:else}
            <span class="badge badge--neutral">{tt("transcription.models.vadNotInstalled")}</span>
            — {tt("transcription.models.vadCoDownloadHint")}
          {/if}
        </span>
      </div>
    </div>

    <!-- Models Table -->
    <div class="models-table-wrapper">
      <table class="models-table">
        <thead>
          <tr>
            <th class="col-active">{tt("transcription.models.colActive")}</th>
            <th class="col-model">{tt("transcription.models.colModel")}</th>
            <th class="col-size">{tt("transcription.models.colSize")}</th>
            <th class="col-ram">{tt("transcription.models.colRam")}</th>
            <th class="col-wer">{tt("transcription.models.colWer")}</th>
            <th class="col-actions">{tt("transcription.models.colActions")}</th>
          </tr>
        </thead>
        <tbody>
          {#each displayedModels as model (model.id)}
            {@const isDownloadingThis = modelsStore.downloadingModelId === model.id}
            {@const isInstalled = model.state.kind === "installed"}
            {@const isActive = modelsStore.view.activeModel === model.id}

            <tr class:row--active={isActive} class:row--recommended={model.recommended}>
              <!-- Active Radio -->
              <td class="col-active">
                <input
                  type="radio"
                  name="active-whisper-model"
                  value={model.id}
                  checked={isActive}
                  disabled={!isInstalled}
                  aria-label={tt("transcription.models.setActive", { name: model.label })}
                  onchange={() => void modelsStore.setActive(model.id)}
                />
              </td>

              <!-- Model info -->
              <td class="col-model">
                <div class="model-name-group">
                  <span class="model-name">{model.label}</span>
                  {#if model.recommended}
                    <span class="badge badge--recommended">
                      {tt("transcription.models.recommendedBadge")}
                    </span>
                  {/if}
                  {#if model.advanced}
                    <span class="badge badge--advanced">
                      {tt("transcription.models.advancedBadge")}
                    </span>
                  {/if}
                </div>
                <span class="model-filename">{model.file}</span>
                {#if model.notes}
                  <span class="model-notes">{model.notes}</span>
                {/if}
              </td>

              <!-- Size -->
              <td class="col-size">
                {#if isInstalled && model.state.kind === "installed"}
                  <span>{formatBytes(model.state.sizeBytes)}</span>
                {:else}
                  <span class="text-muted">{formatBytes(model.sizeBytes)}</span>
                {/if}
              </td>

              <!-- RAM -->
              <td class="col-ram">
                <span class="ram-text">{model.ramEstimate}</span>
              </td>

              <!-- WER -->
              <td class="col-wer">
                <span>{formatWer(model.indicativeWer)}</span>
              </td>

              <!-- Actions & State -->
              <td class="col-actions">
                {#if isDownloadingThis}
                  <div class="downloading-widget">
                    <div class="downloading-widget__progress">
                      <div
                        class="downloading-widget__bar"
                        style="width: {modelsStore.downloadPercent ?? 100}%"
                      ></div>
                    </div>
                    <div class="downloading-widget__meta">
                      <span>
                        {#if modelsStore.downloadPhase === "vad"}
                          {tt("transcription.models.downloadingVad")}
                        {:else}
                          {modelsStore.downloadPercent ? `${modelsStore.downloadPercent}%` : "..."}
                        {/if}
                      </span>
                      <button
                        type="button"
                        class="btn-icon"
                        title={tt("common.cancel")}
                        onclick={() => void modelsStore.cancelDownload(model.id)}
                      >
                        <XIcon size={14} aria-hidden="true" />
                      </button>
                    </div>
                  </div>
                {:else if isInstalled}
                  <div class="installed-actions">
                    {#if !isActive}
                      <button
                        type="button"
                        class="btn btn--sm btn--secondary"
                        onclick={() => void modelsStore.setActive(model.id)}
                      >
                        <CheckIcon size={14} aria-hidden="true" />
                        {tt("transcription.models.useButton")}
                      </button>
                    {:else}
                      <span class="active-pill">{tt("transcription.models.activeBadge")}</span>
                    {/if}

                    <button
                      type="button"
                      class="btn-icon btn-icon--danger"
                      title={tt("transcription.models.deleteButton")}
                      onclick={() => void modelsStore.deleteModel(model.id)}
                    >
                      <Trash2Icon size={14} aria-hidden="true" />
                    </button>
                  </div>
                {:else}
                  <button
                    type="button"
                    class="btn btn--sm btn--primary"
                    disabled={modelsStore.busy || modelsStore.downloadingModelId !== null}
                    onclick={() => startDownload(model)}
                  >
                    <DownloadIcon size={14} aria-hidden="true" />
                    {tt("transcription.models.downloadButton")}
                  </button>
                {/if}
              </td>
            </tr>

            <!-- Large download confirmation row -->
            {#if confirmingDownloadId === model.id}
              <tr class="confirm-download-row">
                <td colspan="6">
                  <div class="confirm-download-banner">
                    <AlertTriangleIcon size={18} aria-hidden="true" />
                    <span>
                      {tt("transcription.models.largeDownloadWarning", {
                        size: formatBytes(model.sizeBytes),
                      })}
                    </span>
                    <div class="confirm-download-banner__actions">
                      <button
                        type="button"
                        class="btn btn--sm btn--primary"
                        onclick={() => confirmLargeDownload(model.id)}
                      >
                        {tt("common.confirm")}
                      </button>
                      <button
                        type="button"
                        class="btn btn--sm btn--secondary"
                        onclick={cancelConfirmation}
                      >
                        {tt("common.cancel")}
                      </button>
                    </div>
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>
  </section>

  <!-- 3. Advanced Configuration Drawer -->
  <section class="card settings__group" aria-labelledby="heading-advanced">
    <h2 class="settings__group-title" id="heading-advanced">
      {tt("transcription.section.advanced")}
    </h2>

    <div class="toggle-list">
      <label class="toggle-item">
        <input
          type="checkbox"
          bind:checked={modelsStore.showFullPrecision}
        />
        <div class="toggle-item__text">
          <span class="toggle-item__title">{tt("transcription.advanced.showFullPrecision")}</span>
          <span class="toggle-item__hint">{tt("transcription.advanced.showFullPrecision.hint")}</span>
        </div>
      </label>
    </div>

    <div class="field">
      <label class="field__label" for="setting-cpu-threads">
        {tt("transcription.advanced.cpuThreads")}
      </label>
      <input
        type="number"
        class="input"
        id="setting-cpu-threads"
        min="0"
        max="128"
        value={settingsStore.settings.whisperThreads}
        onchange={onWhisperThreadsChange}
      />
      <p class="field__hint">{tt("transcription.advanced.cpuThreads.hint")}</p>
    </div>

    <div class="field">
      <label class="field__label" for="setting-mirror-url">
        {tt("transcription.advanced.mirrorUrl")}
      </label>
      <input
        type="text"
        class="input"
        id="setting-mirror-url"
        placeholder="https://hf-mirror.com/models"
        value={settingsStore.settings.whisperModelsMirrorUrl ?? ""}
        onchange={onMirrorUrlChange}
      />
      <p class="field__hint">{tt("transcription.advanced.mirrorUrl.hint")}</p>
    </div>
  </section>
</div>

<style>
  .transcription-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .models-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }

  .models-header__title-block {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .models-header__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .disk-space-badge {
    font-size: 12px;
    color: var(--text-muted);
    background: var(--bg-surface);
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }

  .vad-banner {
    display: flex;
    align-items: center;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-surface);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    font-size: 13px;
    margin-bottom: var(--space-3);
  }

  .vad-banner__info {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .models-table-wrapper {
    overflow-x: auto;
  }

  .models-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    text-align: left;
  }

  .models-table th,
  .models-table td {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
  }

  .models-table th {
    color: var(--text-muted);
    font-weight: 600;
  }

  .row--active {
    background-color: rgba(var(--accent-rgb, 59, 130, 246), 0.08);
  }

  .model-name-group {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .model-name {
    font-weight: 600;
  }

  .model-filename {
    display: block;
    font-size: 11px;
    color: var(--text-muted);
    font-family: var(--font-mono, monospace);
  }

  .model-notes {
    display: block;
    font-size: 11px;
    color: var(--text-muted);
  }

  .badge {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    font-weight: 500;
  }

  .badge--recommended {
    background: var(--accent);
    color: white;
  }

  .badge--advanced {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .badge--success {
    background: rgba(34, 197, 94, 0.15);
    color: #16a34a;
  }

  .badge--neutral {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .active-pill {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
  }

  .downloading-widget {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 110px;
  }

  .downloading-widget__progress {
    height: 6px;
    background: var(--border);
    border-radius: 3px;
    overflow: hidden;
  }

  .downloading-widget__bar {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }

  .downloading-widget__meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    color: var(--text-muted);
  }

  .installed-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .btn-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    cursor: pointer;
  }

  .btn-icon:hover {
    background: var(--bg-surface);
    color: var(--text);
  }

  .btn-icon--danger:hover {
    color: #ef4444;
  }

  .confirm-download-row {
    background: rgba(245, 158, 11, 0.1);
  }

  .confirm-download-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2);
    font-size: 13px;
    color: var(--text);
  }

  .confirm-download-banner__actions {
    display: flex;
    gap: var(--space-2);
  }

  .toggle-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .toggle-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    cursor: pointer;
  }

  .toggle-item__text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .toggle-item__title {
    font-size: 14px;
    font-weight: 500;
  }

  .toggle-item__hint {
    font-size: 12px;
    color: var(--text-muted);
  }

  .ram-text {
    font-size: 12px;
    color: var(--text-muted);
  }

  .text-muted {
    color: var(--text-muted);
  }
</style>
