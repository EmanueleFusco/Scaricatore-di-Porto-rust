/**
 * Store for Whisper model management and downloads (spec §10.2, §11.6).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type ModelsView,
  type ModelProgressEvent,
  type ModelViewItem,
} from "$lib/models";
import { isTauriRuntime } from "$lib/stores/settings.svelte";

const EMPTY_MODELS_VIEW: ModelsView = {
  models: [],
  activeModel: null,
  recommendedModel: "large-v3-turbo-q5_0",
  lowCoreSystem: false,
  freeSpaceBytes: 0,
  vad: {
    id: "silero-v5.1.2",
    installed: false,
    file: "ggml-silero-v5.1.2.bin",
    sizeBytes: 885098,
  },
  modelsDir: "",
};

class ModelsStore {
  view = $state<ModelsView>(EMPTY_MODELS_VIEW);
  loading = $state(false);
  busy = $state(false);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  /** Currently downloading model ID if any. */
  downloadingModelId = $state<string | null>(null);
  downloadPercent = $state<number | null>(null);
  downloadPhase = $state<string | null>(null); // "vad" or "model"
  downloadReceived = $state<number>(0);
  downloadTotal = $state<number | null>(null);

  /** UI toggle for full precision models under Advanced section */
  showFullPrecision = $state(false);

  #unlistenProgress: UnlistenFn | null = null;
  #unlistenCompleted: UnlistenFn | null = null;
  #unlistenCancelled: UnlistenFn | null = null;
  #unlistenError: UnlistenFn | null = null;

  async init(): Promise<void> {
    await this.setupEventListeners();
    await this.refresh();
  }

  async setupEventListeners(): Promise<void> {
    if (!isTauriRuntime()) return;
    if (this.#unlistenProgress) return;

    this.#unlistenProgress = await listen<ModelProgressEvent>(
      "models:progress",
      (event) => {
        const payload = event.payload;
        this.downloadingModelId = payload.id === this.view.vad.id ? (this.downloadingModelId ?? payload.id) : payload.id;
        this.downloadPhase = payload.phase;
        this.downloadPercent = payload.percent ?? null;
        this.downloadReceived = payload.receivedBytes;
        this.downloadTotal = payload.totalBytes ?? null;

        // Patch model progress in memory
        this.view = {
          ...this.view,
          models: this.view.models.map((m) => {
            if (m.id === payload.id) {
              return {
                ...m,
                state: {
                  kind: "downloading",
                  receivedBytes: payload.receivedBytes,
                  totalBytes: payload.totalBytes,
                },
              };
            }
            return m;
          }),
        };
      },
    );

    this.#unlistenCompleted = await listen<string>(
      "models:completed",
      async () => {
        this.downloadingModelId = null;
        this.downloadPercent = null;
        this.downloadPhase = null;
        await this.refresh();
      },
    );

    this.#unlistenCancelled = await listen<string>(
      "models:cancelled",
      async () => {
        this.downloadingModelId = null;
        this.downloadPercent = null;
        this.downloadPhase = null;
        await this.refresh();
      },
    );

    this.#unlistenError = await listen<{ modelId: string; error: { messageKey?: string; detail?: string } }>(
      "models:error",
      (event) => {
        this.downloadingModelId = null;
        this.downloadPercent = null;
        this.downloadPhase = null;
        const err = event.payload.error;
        this.errorKey = err.messageKey ?? "error.tauri";
        this.errorDetail = err.detail ?? null;
        void this.refresh();
      },
    );
  }

  async refresh(): Promise<void> {
    this.loading = true;
    try {
      if (!isTauriRuntime()) {
        this.view = EMPTY_MODELS_VIEW;
        return;
      }
      this.view = await invoke<ModelsView>("models_list");
      if (this.downloadingModelId === null) {
        const active = this.view.models.find((m) => m.state.kind === "downloading");
        if (active && active.state.kind === "downloading") {
          this.downloadingModelId = active.id;
          if (active.state.totalBytes && active.state.totalBytes > 0) {
            this.downloadPercent = Math.round(
              (active.state.receivedBytes / active.state.totalBytes) * 100,
            );
          }
        }
      }
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.loading = false;
    }
  }

  async download(modelId: string): Promise<void> {
    this.busy = true;
    this.downloadingModelId = modelId;
    this.downloadPercent = 0;
    this.clearError();
    try {
      if (!isTauriRuntime()) return;
      await invoke("models_download", { modelId });
    } catch (err) {
      this.downloadingModelId = null;
      this.downloadPercent = null;
      this.#fail(err);
    } finally {
      this.busy = false;
    }
  }

  async pauseDownload(modelId: string): Promise<void> {
    try {
      if (isTauriRuntime()) {
        await invoke("models_pause_download", { modelId });
      }
      this.downloadingModelId = null;
      this.downloadPercent = null;
      this.downloadPhase = null;
      await this.refresh();
    } catch (err) {
      this.#fail(err);
    }
  }

  async cancelDownload(modelId: string): Promise<void> {
    try {
      if (isTauriRuntime()) {
        await invoke("models_cancel_download", { modelId });
      }
      this.downloadingModelId = null;
      this.downloadPercent = null;
      this.downloadPhase = null;
      await this.refresh();
    } catch (err) {
      this.#fail(err);
    }
  }

  async deleteModel(modelId: string): Promise<void> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return;
      this.view = await invoke<ModelsView>("models_delete", { modelId });
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.busy = false;
    }
  }

  async setActive(modelId: string): Promise<void> {
    try {
      if (!isTauriRuntime()) return;
      await invoke("models_set_active", { modelId });
      this.view = {
        ...this.view,
        activeModel: modelId,
      };
      this.clearError();
    } catch (err) {
      this.#fail(err);
    }
  }

  async importFile(sourcePath: string, targetModelId?: string): Promise<ModelViewItem | null> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return null;
      await invoke("models_import", {
        sourcePath,
        targetModelId: targetModelId ?? null,
      });
      await this.refresh();
      return this.view.models.find((m) => m.id === targetModelId) ?? null;
    } catch (err) {
      this.#fail(err);
      return null;
    } finally {
      this.busy = false;
    }
  }

  async openFolder(): Promise<void> {
    try {
      if (isTauriRuntime()) {
        await invoke("models_open_folder");
      }
    } catch (err) {
      this.#fail(err);
    }
  }

  dismissError(): void {
    this.clearError();
  }

  #fail(err: unknown): void {
    const info = err as { messageKey?: unknown; detail?: unknown };
    this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
    this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
  }

  private clearError(): void {
    this.errorKey = null;
    this.errorDetail = null;
  }

  destroy(): void {
    if (this.#unlistenProgress) {
      this.#unlistenProgress();
      this.#unlistenProgress = null;
    }
    if (this.#unlistenCompleted) {
      this.#unlistenCompleted();
      this.#unlistenCompleted = null;
    }
    if (this.#unlistenCancelled) {
      this.#unlistenCancelled();
      this.#unlistenCancelled = null;
    }
    if (this.#unlistenError) {
      this.#unlistenError();
      this.#unlistenError = null;
    }
  }
}

export const modelsStore = new ModelsStore();
