<script lang="ts">
  import { rowLanguageOptions } from "$lib/addbar";
  import { hasKey } from "$lib/i18n";
  import { modelLabelKey } from "$lib/models";
  import { modelsStore } from "$lib/stores/models.svelte";
  import { toolsStore } from "$lib/stores/tools.svelte";
  import { activeLang, settingsStore, tt } from "$lib/stores/settings.svelte";
  import type { AddRow } from "$lib/stores/addbar.svelte";
  let { row, onchange }: { row: AddRow; onchange: (patch: Partial<AddRow>) => void } = $props();
</script>
                <!-- Spec §10.2: Whisper Transcription -->
                <div class="field">
                  <span class="field__label">{tt("transcribe.title")}</span>
                  <label class="add__check">
                    <input
                      type="checkbox"
                      checked={row.transcribe}
                      onchange={(event) =>
                        onchange({ transcribe: event.currentTarget.checked })}
                    />
                    {tt("transcribe.toggle")}
                  </label>

                  {#if row.transcribe}
                    {@const activeEngine = row.transcriptionEngine ?? settingsStore.settings.transcriptionEngineOrder}
                    {@const usesWhisper = activeEngine !== "gemini_only" && activeEngine !== "subtitles_only"}

                    <div class="field">
                      <label class="field__label" for={`engine-order-${row.key}`}>
                        {tt("addbar.transcribeEngine")}
                      </label>
                      <select
                        class="select"
                        id={`engine-order-${row.key}`}
                        value={activeEngine}
                        onchange={(event) =>
                          onchange({ transcriptionEngine: event.currentTarget.value })}
                      >
                        <option value="subtitles_whisper_gemini">{tt("transcription.engine.subtitlesWhisperGemini")}</option>
                        <option value="whisper_only">{tt("transcription.engine.whisperOnly")}</option>
                        <option value="gemini_only">{tt("transcription.engine.geminiOnly")}</option>
                        <option value="subtitles_only">{tt("transcription.engine.subtitlesOnly")}</option>
                      </select>
                      {#if activeEngine === "gemini_only" || activeEngine === "subtitles_whisper_gemini"}
                        <p class="field__hint">{tt("transcription.gemini.privacyNotice")}</p>
                      {/if}
                    </div>

                    <div class="add__controls">
                      <div class="field" class:field--disabled={!usesWhisper}>
                        <label class="field__label" for={`whisper-model-${row.key}`}>
                          {tt("transcribe.model")}
                        </label>
                        <select
                          class="select"
                          id={`whisper-model-${row.key}`}
                          disabled={!usesWhisper}
                          value={row.whisperModel ?? settingsStore.settings.activeWhisperModel ?? "large-v3-turbo-q5_0"}
                          onchange={(event) =>
                            onchange({ whisperModel: event.currentTarget.value })}
                        >
                          {#if modelsStore.view.models.length > 0}
                            {#each modelsStore.view.models as model (model.id)}
                              {@const mKey = modelLabelKey(model.id)}
                              <option value={model.id}>
                                {hasKey(activeLang(), mKey) ? tt(mKey) : model.label} ({model.state.kind === "installed" ? tt("models.installed") : tt("models.notInstalled")})
                              </option>
                            {/each}
                          {:else}
                            <option value="large-v3-turbo-q5_0">Turbo (large-v3-turbo-q5_0)</option>
                            <option value="small-q5_1">Small (small-q5_1)</option>
                            <option value="base-q5_1">Base (base-q5_1)</option>
                            <option value="tiny-q5_1">Tiny (tiny-q5_1)</option>
                          {/if}
                        </select>
                        {#if modelsStore.view.models.length > 0}
                          {@const selectedModel = modelsStore.view.models.find((m) => m.id === (row.whisperModel ?? settingsStore.settings.activeWhisperModel ?? "large-v3-turbo-q5_0"))}
                          {#if selectedModel && selectedModel.state.kind !== "installed"}
                            <button
                              type="button"
                              class="btn btn--small"
                              style="margin-top: 4px;"
                              disabled={!usesWhisper || modelsStore.busy || (modelsStore.downloadingModelId !== null && modelsStore.downloadingModelId !== selectedModel.id)}
                              onclick={() => void modelsStore.download(selectedModel.id)}
                            >
                              {#if modelsStore.downloadingModelId === selectedModel.id}
                                {modelsStore.downloadPercent !== null
                                  ? tt("models.action.downloading", { percent: modelsStore.downloadPercent })
                                  : tt("models.action.downloadingUnknown")}
                              {:else}
                                {tt("job.whisperModel.download")}
                              {/if}
                            </button>
                          {/if}
                        {/if}
                        {#if toolsStore.view.tools.length > 0}
                          {@const whisperInstalled = toolsStore.view.tools.some((t) => t.name === "sdp-whisper" && t.path !== null)}
                          {#if !whisperInstalled}
                            <button
                              type="button"
                              class="btn btn--small"
                              style="margin-top: 4px;"
                              disabled={!usesWhisper || toolsStore.installingTool !== null}
                              onclick={() => void toolsStore.install("sdp-whisper")}
                            >
                              {#if toolsStore.installingTool === "sdp-whisper"}
                                {toolsStore.installPercent !== null
                                  ? tt("tools.action.installing", { percent: toolsStore.installPercent })
                                  : tt("tools.action.installingUnknown")}
                              {:else}
                                {tt("job.whisper.install")}
                              {/if}
                            </button>
                          {/if}
                        {/if}
                      </div>

                      <div class="field">
                        <label class="field__label" for={`whisper-lang-${row.key}`}>
                          {tt("transcribe.language")}
                        </label>
                        <select
                          class="select"
                          id={`whisper-lang-${row.key}`}
                          value={row.transcriptLanguage ?? settingsStore.settings.defaultTranscriptLanguage ?? "auto"}
                          onchange={(event) =>
                            onchange({ transcriptLanguage: event.currentTarget.value })}
                        >
                          {#each rowLanguageOptions(row.inspected?.kind === "video" ? row.inspected.language : null, row.transcriptLanguage) as opt (opt.value)}
                            <option value={opt.value}>
                              {opt.labelKey ? (opt.value === "auto" ? tt(opt.labelKey) : `${tt(opt.labelKey)} (${opt.value})`) : opt.label}
                            </option>
                          {/each}
                        </select>
                      </div>
                    </div>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.translateToEnglish}
                        disabled={!usesWhisper}
                        onchange={(event) =>
                          onchange({ translateToEnglish: event.currentTarget.checked })}
                      />
                      {tt("transcribe.translateToEnglish")}
                    </label>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.transcriptsSubfolder}
                        onchange={(event) =>
                          onchange({ transcriptsSubfolder: event.currentTarget.checked })}
                      />
                      {tt("transcribe.subfolder")}
                    </label>

                    <label class="add__check">
                      <input
                        type="checkbox"
                        checked={row.summarize}
                        onchange={(event) =>
                          onchange({ summarize: event.currentTarget.checked })}
                      />
                      {tt("addbar.summarize")}
                    </label>
                    {#if row.summarize}
                      <p class="field__hint">{tt("settings.transcription.geminiSummaryPrivacyHint")}</p>
                      <div class="field">
                        <label class="field__label" for={`summary-language-${row.key}`}>{tt("transcription.gemini.summaryLanguage")}</label>
                        <select class="select" id={`summary-language-${row.key}`} value={row.summaryLanguage ?? "auto"} onchange={(event) => onchange({ summaryLanguage: event.currentTarget.value })}>
                          {#each rowLanguageOptions(null, row.summaryLanguage) as opt (opt.value)}
                            <option value={opt.value}>{opt.labelKey ? tt(opt.labelKey) : opt.label}</option>
                          {/each}
                        </select>
                      </div>
                    {/if}
                    {#if activeEngine === "gemini_only" || activeEngine === "subtitles_whisper_gemini" || row.summarize}
                      <div class="field">
                        <label class="field__label" for={`gemini-model-${row.key}`}>{tt("transcribe.geminiModel")}</label>
                        <input class="input" id={`gemini-model-${row.key}`} value={row.geminiModel ?? settingsStore.settings.defaultGeminiModel} onchange={(event) => onchange({ geminiModel: event.currentTarget.value.trim() || null })} />
                      </div>
                    {/if}
                    {#if !settingsStore.settings.geminiPrivacyHintDismissed && ((row.transcriptionEngine ?? settingsStore.settings.transcriptionEngineOrder) === "gemini_only" || (row.transcriptionEngine ?? settingsStore.settings.transcriptionEngineOrder) === "subtitles_whisper_gemini" || row.summarize)}
                      <div class="add__duplicates" role="status">
                        <p class="field__hint">{tt("transcription.gemini.privacyNotice")}</p>
                        <p class="field__hint">{tt("settings.transcription.geminiSummaryPrivacyHint")}</p>
                        <div class="add__actions">
                          <button
                            type="button"
                            class="btn btn--small"
                            onclick={() => void settingsStore.update({ geminiPrivacyHintDismissed: true })}
                          >
                            {tt("common.dismiss")}
                          </button>
                        </div>
                      </div>
                    {/if}
                  {/if}
                </div>

<style>
  .add__check { display: flex; align-items: center; gap: var(--space-2); }
  .add__controls, .add__actions { display: flex; flex-wrap: wrap; align-items: end; gap: var(--space-3); }
  .add__duplicates { padding: var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-md); }
  .field--disabled { opacity: .6; }
</style>
