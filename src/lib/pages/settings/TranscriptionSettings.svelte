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
  import EyeIcon from "lucide-svelte/icons/eye";
  import EyeOffIcon from "lucide-svelte/icons/eye-off";
  import FolderOpenIcon from "lucide-svelte/icons/folder-open";
  import PauseIcon from "lucide-svelte/icons/pause";
  import PlayIcon from "lucide-svelte/icons/play";
  import RefreshCwIcon from "lucide-svelte/icons/refresh-cw";
  import Trash2Icon from "lucide-svelte/icons/trash-2";
  import UploadIcon from "lucide-svelte/icons/upload";
  import XIcon from "lucide-svelte/icons/x";
  import { open } from "@tauri-apps/plugin-dialog";

  import {
    deleteGeminiKey,
    geminiModelsStore,
    getGeminiKeyStatus,
    saveGeminiKey,
    testGeminiKey,
    type GeminiKeyStatus,
  } from "$lib/gemini.svelte";
  import { hasKey } from "$lib/i18n";
  import {
    filterModels,
    formatBytes,
    formatWer,
    isLargeDownload,
    modelLabelKey,
    modelNotesKey,
    type ModelViewItem,
  } from "$lib/models";
  import { modelsStore } from "$lib/stores/models.svelte";
  import { activeLang, errorText, settingsStore, tt } from "$lib/stores/settings.svelte";

  let geminiStatus = $state<GeminiKeyStatus | null>(null);
  let geminiInputKey = $state("");
  let showGeminiKey = $state(false);
  let testingGeminiKey = $state(false);
  let geminiTestSuccess = $state<boolean | null>(null);
  let geminiTestMessage = $state<string | null>(null);
  let geminiTestDetail = $state<string | null>(null);

  async function refreshGeminiStatus(): Promise<void> {
    try {
      geminiStatus = await getGeminiKeyStatus();
    } catch {
      geminiStatus = null;
    }
  }

  async function refreshGeminiModels(): Promise<void> {
    await geminiModelsStore.refresh();
  }

  async function handleSaveGeminiKey(): Promise<void> {
    const keyToSave = geminiInputKey.trim();
    if (!keyToSave) return;
    try {
      await saveGeminiKey(keyToSave);
      geminiInputKey = "";
      geminiTestSuccess = null;
      geminiTestMessage = null;
      geminiTestDetail = null;
      await refreshGeminiStatus();
      await refreshGeminiModels();
    } catch (e: any) {
      geminiTestSuccess = false;
      geminiTestMessage = e?.message ?? String(e);
      geminiTestDetail = null;
    }
  }

  async function handleDeleteGeminiKey(): Promise<void> {
    try {
      await deleteGeminiKey();
      geminiInputKey = "";
      geminiTestSuccess = null;
      geminiTestMessage = null;
      geminiTestDetail = null;
      await refreshGeminiStatus();
    } catch (e: any) {
      geminiTestSuccess = false;
      geminiTestMessage = e?.message ?? String(e);
      geminiTestDetail = null;
    }
  }

  async function handleTestGeminiKey(): Promise<void> {
    testingGeminiKey = true;
    geminiTestSuccess = null;
    geminiTestMessage = null;
    geminiTestDetail = null;
    try {
      const ok = await testGeminiKey(geminiInputKey.trim() || undefined);
      geminiTestSuccess = ok;
      if (ok) {
        geminiTestMessage = tt("transcription.gemini.testSuccess");
        void refreshGeminiModels();
      }
    } catch (e: any) {
      geminiTestSuccess = false;
      // The localized key is what the user reads; the raw provider response
      // stays available under "Technical details" for support.
      geminiTestMessage =
        typeof e?.messageKey === "string"
          ? errorText(e.messageKey, null)
          : (e?.message ?? String(e));
      geminiTestDetail = typeof e?.detail === "string" ? e.detail : null;
    } finally {
      testingGeminiKey = false;
    }
  }

  function onGeminiModelChange(event: Event): void {
    const value = (event.currentTarget as HTMLInputElement).value.trim();
    if (value) {
      void settingsStore.update({ defaultGeminiModel: value });
    }
  }

  function onDefaultSummarizeChange(event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    void settingsStore.update({ defaultSummarize: checked });
  }

  function onSummaryLanguageChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    void settingsStore.update({ defaultSummaryLanguage: value });
  }

  onMount(() => {
    void modelsStore.init();
    void refreshGeminiStatus();
    void refreshGeminiModels();
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

  function localizedModelLabel(model: ModelViewItem): string {
    const key = modelLabelKey(model.id);
    return hasKey(activeLang(), key) ? tt(key) : model.label;
  }

  function localizedModelNotes(model: ModelViewItem): string | null {
    const key = modelNotesKey(model.id);
    return hasKey(activeLang(), key) ? tt(key) : model.notes;
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
          class="btn btn--secondary btn--small"
          disabled={modelsStore.busy}
          onclick={importModelFile}
        >
          <UploadIcon size={14} aria-hidden="true" />
          {tt("transcription.models.importButton")}
        </button>

        <button
          type="button"
          class="btn btn--secondary btn--small"
          onclick={() => void modelsStore.openFolder()}
        >
          <FolderOpenIcon size={14} aria-hidden="true" />
          {tt("transcription.models.openFolder")}
        </button>

        <button
          type="button"
          class="btn btn--ghost btn--icon"
          disabled={modelsStore.loading}
          onclick={() => void modelsStore.refresh()}
          aria-label={tt("common.refresh")}
          title={tt("common.refresh")}
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
            {@const isDownloadingThis =
              modelsStore.downloadingModelId === model.id ||
              (modelsStore.downloadingModelId === null && model.state.kind === "downloading")}
            {@const isPaused = !isDownloadingThis && model.state.kind === "paused"}
            {@const isInstalled = model.state.kind === "installed"}
            {@const isActive = modelsStore.view.activeModel === model.id}
            {@const currentPercent = isDownloadingThis
              ? modelsStore.downloadingModelId === model.id && modelsStore.downloadPercent !== null
                ? modelsStore.downloadPercent
                : model.state.kind === "downloading" && model.state.totalBytes
                  ? Math.round((model.state.receivedBytes / model.state.totalBytes) * 100)
                  : null
              : null}
            {@const pausedPercent =
              isPaused && model.state.kind === "paused" && model.state.totalBytes
                ? Math.round((model.state.receivedBytes / model.state.totalBytes) * 100)
                : null}

            <tr class:row--active={isActive} class:row--recommended={model.recommended}>
              <!-- Active Radio -->
              <td class="col-active">
                <input
                  type="radio"
                  name="active-whisper-model"
                  value={model.id}
                  checked={isActive}
                  disabled={!isInstalled}
                  aria-label={tt("transcription.models.setActive", { name: localizedModelLabel(model) })}
                  onchange={() => void modelsStore.setActive(model.id)}
                />
              </td>

              <!-- Model info -->
              <td class="col-model">
                <div class="model-name-group">
                  <span class="model-name">{localizedModelLabel(model)}</span>
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
                {#if localizedModelNotes(model)}
                  <span class="model-notes">{localizedModelNotes(model)}</span>
                {/if}
              </td>

              <!-- Size -->
              <td class="col-size">
                {#if isInstalled && model.state.kind === "installed"}
                  <span>{formatBytes(model.state.sizeBytes)}</span>
                {:else if isPaused && model.state.kind === "paused"}
                  <span>{formatBytes(model.state.receivedBytes)} / {formatBytes(model.sizeBytes)}</span>
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
                        style="width: {currentPercent ?? 100}%"
                      ></div>
                    </div>
                    <div class="downloading-widget__meta">
                      <span>
                        {#if modelsStore.downloadPhase === "vad"}
                          {tt("transcription.models.downloadingVad")}
                        {:else}
                          {currentPercent !== null ? `${currentPercent}%` : "..."}
                        {/if}
                      </span>
                      <div class="downloading-widget__actions">
                        <button
                          type="button"
                          class="btn-icon"
                          title={tt("transcription.models.pauseButton")}
                          onclick={() => void modelsStore.pauseDownload(model.id)}
                        >
                          <PauseIcon size={14} aria-hidden="true" />
                        </button>
                        <button
                          type="button"
                          class="btn-icon btn-icon--danger"
                          title={tt("common.cancel")}
                          onclick={() => void modelsStore.cancelDownload(model.id)}
                        >
                          <XIcon size={14} aria-hidden="true" />
                        </button>
                      </div>
                    </div>
                  </div>
                {:else if isPaused}
                  <div class="downloading-widget downloading-widget--paused">
                    <div class="downloading-widget__progress">
                      <div
                        class="downloading-widget__bar downloading-widget__bar--paused"
                        style="width: {pausedPercent ?? 0}%"
                      ></div>
                    </div>
                    <div class="downloading-widget__meta">
                      <span>
                        {tt("transcription.models.statePaused")}{pausedPercent !== null
                          ? ` (${pausedPercent}%)`
                          : ""}
                      </span>
                      <div class="downloading-widget__actions">
                        <button
                          type="button"
                          class="btn-icon btn-icon--accent"
                          title={tt("transcription.models.resumeButton")}
                          disabled={modelsStore.busy || modelsStore.downloadingModelId !== null}
                          onclick={() => startDownload(model)}
                        >
                          <PlayIcon size={14} aria-hidden="true" />
                        </button>
                        <button
                          type="button"
                          class="btn-icon btn-icon--danger"
                          title={tt("common.cancel")}
                          onclick={() => void modelsStore.cancelDownload(model.id)}
                        >
                          <XIcon size={14} aria-hidden="true" />
                        </button>
                      </div>
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

  <!-- 3. Cloud Engine: Gemini & Summaries (spec §10.3, §10.4, §10.5, §11.6) -->
  <section class="card settings__group" aria-labelledby="heading-gemini">
    <div class="models-header">
      <div class="models-header__title-block">
        <h2 class="settings__group-title" id="heading-gemini">
          {tt("transcription.section.gemini")}
        </h2>
        {#if geminiStatus?.hasKey}
          <span class="badge badge--recommended">
            {tt("transcription.gemini.statusConfigured", {
              source:
                geminiStatus.source === "environment"
                  ? tt("transcription.gemini.sourceEnv")
                  : geminiStatus.source === "keyring"
                    ? tt("transcription.gemini.sourceKeyring")
                    : tt("transcription.gemini.sourceFile"),
            })}
          </span>
        {:else}
          <span class="badge badge--muted">
            {tt("transcription.gemini.statusNotConfigured")}
          </span>
        {/if}
      </div>
    </div>

    <!-- Privacy Notice -->
    <div class="banner banner--info" role="note">
      <p>{tt("transcription.gemini.privacyNotice")}</p>
    </div>

    {#if geminiStatus?.hasKey && geminiStatus.source === "file"}
      <div class="banner banner--info" role="note">
        <p>{tt("transcription.gemini.keyringFallbackNote")}</p>
      </div>
    {/if}

    <!-- API Key Input & Actions -->
    <div class="field">
      <label class="field__label" for="setting-gemini-key">
        {tt("transcription.gemini.keyInput")}
      </label>
      <div class="input-action-group">
        <div class="password-wrapper">
          <input
            id="setting-gemini-key"
            type={showGeminiKey ? "text" : "password"}
            class="input input--password"
            placeholder={geminiStatus?.hasKey
              ? "••••••••••••••••••••••••••••••••"
              : tt("transcription.gemini.keyInputPlaceholder")}
            bind:value={geminiInputKey}
          />
          <button
            type="button"
            class="password-toggle"
            onclick={() => (showGeminiKey = !showGeminiKey)}
            aria-label={showGeminiKey
              ? tt("transcription.gemini.hideKey")
              : tt("transcription.gemini.showKey")}
          >
            {#if showGeminiKey}
              <EyeOffIcon size={16} aria-hidden="true" />
            {:else}
              <EyeIcon size={16} aria-hidden="true" />
            {/if}
          </button>
        </div>

        <button
          type="button"
          class="btn btn--secondary"
          disabled={!geminiInputKey.trim()}
          onclick={handleSaveGeminiKey}
        >
          {tt("transcription.gemini.saveKey")}
        </button>

        <button
          type="button"
          class="btn btn--secondary"
          disabled={testingGeminiKey || (!geminiInputKey.trim() && !geminiStatus?.hasKey)}
          onclick={handleTestGeminiKey}
        >
          {tt("transcription.gemini.testKey")}
        </button>

        {#if geminiStatus?.hasKey}
          <button
            type="button"
            class="btn btn--ghost text-danger"
            onclick={handleDeleteGeminiKey}
          >
            <Trash2Icon size={16} aria-hidden="true" />
            {tt("transcription.gemini.deleteKey")}
          </button>
        {/if}
      </div>
      <p class="field__hint">{tt("transcription.gemini.keyStorageNote")}</p>
    </div>

    {#if geminiTestMessage}
      <div
        class="banner {geminiTestSuccess ? 'banner--success' : 'banner--error'}"
        role="alert"
      >
        <div class="banner__body">
          <span>{geminiTestMessage}</span>
          {#if geminiTestDetail}
            <details class="banner__details">
              <summary>{tt("common.technicalDetails")}</summary>
              <pre>{geminiTestDetail}</pre>
            </details>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Model selection -->
    <div class="field">
      <div class="field__header-row">
        <label class="field__label" for="setting-gemini-model">
          {tt("transcription.gemini.model")}
        </label>
        <button
          type="button"
          class="btn btn--ghost btn--sm"
          disabled={geminiModelsStore.loading || !geminiStatus?.hasKey}
          onclick={refreshGeminiModels}
          aria-label={tt("transcription.gemini.refreshModels")}
        >
          <RefreshCwIcon size={14} class={geminiModelsStore.loading ? "spin" : ""} aria-hidden="true" />
          {tt("transcription.gemini.refreshModels")}
        </button>
      </div>

      <div class="model-combobox-wrapper">
        <input
          id="setting-gemini-model"
          type="text"
          class="input"
          list="gemini-models-list"
          value={settingsStore.settings.defaultGeminiModel}
          onchange={onGeminiModelChange}
          placeholder="gemini-flash-latest"
          autocomplete="off"
        />
        <datalist id="gemini-models-list">
          {#each geminiModelsStore.models as model (model.id)}
            <option value={model.id}>{model.displayName} ({model.id})</option>
          {/each}
        </datalist>
      </div>
      <p class="field__hint">{tt("transcription.gemini.modelHint")}</p>
    </div>

    <!-- Summaries Configuration -->
    <div class="toggle-list">
      <label class="toggle-item">
        <input
          type="checkbox"
          checked={settingsStore.settings.defaultSummarize}
          onchange={onDefaultSummarizeChange}
        />
        <div class="toggle-item__text">
          <span class="toggle-item__title">{tt("transcription.gemini.defaultSummarize")}</span>
          <span class="toggle-item__hint">{tt("transcription.gemini.defaultSummarizeHint")}</span>
          <span class="toggle-item__hint">{tt("settings.transcription.geminiSummaryPrivacyHint")}</span>
        </div>
      </label>
    </div>

    {#if settingsStore.settings.defaultSummarize}
      <div class="field">
        <label class="field__label" for="setting-summary-language">
          {tt("transcription.gemini.summaryLanguage")}
        </label>
        <select
          class="select"
          id="setting-summary-language"
          value={settingsStore.settings.defaultSummaryLanguage}
          onchange={onSummaryLanguageChange}
        >
          {#each languageOptions as option (option.value)}
            <option value={option.value}>{tt(option.key)}</option>
          {/each}
        </select>
        <p class="field__hint">{tt("transcription.gemini.summaryLanguageHint")}</p>
      </div>
    {/if}
  </section>

  <!-- 4. Advanced Configuration Drawer -->
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
  }

  .models-header__title-block {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .models-header__actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .disk-space-badge {
    font-size: 12px;
    color: var(--text-muted);
    background: var(--surface-2);
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }

  .vad-banner {
    display: flex;
    align-items: center;
    padding: var(--space-2) var(--space-3);
    background: var(--surface-2);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    font-size: 13px;
  }

  .vad-banner__info {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .models-table-wrapper {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
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
    vertical-align: middle;
  }

  .models-table tbody tr:last-child td {
    border-bottom: none;
  }

  .models-table th {
    background: var(--surface-2);
    color: var(--text-muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
    white-space: nowrap;
  }

  .row--active {
    background-color: var(--accent-surface);
  }

  .model-name-group {
    display: flex;
    flex-wrap: wrap;
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
    margin-top: 2px;
  }

  .model-notes {
    display: block;
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 2px;
  }

  .badge {
    display: inline-block;
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    font-weight: 600;
    white-space: nowrap;
  }

  .badge--recommended {
    background: var(--accent);
    color: var(--on-accent);
  }

  .badge--advanced {
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .badge--success {
    background: var(--success-surface);
    color: var(--success);
  }

  .badge--neutral {
    background: var(--surface-2);
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
    white-space: nowrap;
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

  .downloading-widget__bar--paused {
    background: var(--text-muted);
  }

  .downloading-widget__actions {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }

  .btn-icon--accent:hover {
    color: var(--accent);
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
    background: var(--surface-hover);
    color: var(--text);
  }

  .btn-icon--danger:hover {
    color: var(--danger);
  }

  .confirm-download-row {
    background: var(--warning-surface);
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
    gap: var(--space-2);
    cursor: pointer;
  }

  .toggle-item input[type="checkbox"] {
    margin-top: 3px;
    flex: none;
  }

  .toggle-item__text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .toggle-item__title {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.4;
  }

  .toggle-item__hint {
    font-size: 12px;
    color: var(--text-muted);
  }

  .ram-text {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }

  .text-muted {
    color: var(--text-muted);
  }

  .text-danger {
    color: var(--danger);
  }

  .badge--muted {
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .banner--info {
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    font-size: 13px;
  }

  .banner--success {
    background: var(--success-surface);
    border: 1px solid var(--success);
    color: var(--success);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    font-size: 13px;
  }

  .input-action-group {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .password-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1 1 240px;
    min-width: 200px;
  }

  .input--password {
    width: 100%;
    padding-right: 36px;
  }

  .password-toggle {
    position: absolute;
    right: 8px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
  }

  .password-toggle:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .field__header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .input[type="number"] {
    max-width: 120px;
  }

  @keyframes spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }
</style>
