/**
 * Store for the Tools page and the Setup wizard (spec §5.2, §11.1, §11.8).
 *
 * Owns the IPC and nothing else: every rule lives in `$lib/tools`, which is
 * pure and unit tested. `invoke` is skipped when the UI runs in a plain browser
 * (`npm run dev`), exactly like the settings store does, so the page renders
 * with the "not installed" state instead of an error banner.
 */
import { invoke } from "@tauri-apps/api/core";
import {
  normaliseToolStatus,
  normaliseToolsView,
  type ToolStatus,
  type ToolsView,
  type UpdateOutcome,
} from "$lib/tools";
import { isTauriRuntime } from "$lib/stores/settings.svelte";

const EMPTY_VIEW: ToolsView = {
  tools: [],
  triple: "",
  toolsDir: "",
  bundledDir: "",
  tempDir: null,
  setupWizardCompleted: false,
};

export interface UpdateReport extends UpdateOutcome {
  /** i18n key describing what happened. */
  key: string;
}

class ToolsStore {
  view = $state<ToolsView>(EMPTY_VIEW);
  loading = $state(false);
  /** `true` while an update or a path change is in flight. */
  busy = $state(false);
  lastUpdate = $state<UpdateReport | null>(null);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  /** Read every tool. Safe to call from `onMount` and from a Refresh button. */
  async refresh(): Promise<void> {
    this.loading = true;
    try {
      if (!isTauriRuntime()) {
        this.view = EMPTY_VIEW;
        return;
      }
      this.view = normaliseToolsView(await invoke<ToolsView>("tools_status"));
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.loading = false;
    }
  }

  /** Re-probe one tool, for the per-row "Test" button. */
  async test(tool: string): Promise<ToolStatus | null> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return null;
      const status = normaliseToolStatus(await invoke<ToolStatus>("tools_test", { tool }));
      this.view = {
        ...this.view,
        tools: this.view.tools.map((row) => (row.name === status.name ? status : row)),
      };
      this.clearError();
      return status;
    } catch (err) {
      this.#fail(err);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /**
   * Run the yt-dlp updater (spec §5.5).
   *
   * The outcome wording is decided here rather than in the backend so the UI
   * never has to parse an English sentence: the backend says *what happened*,
   * this says which message to show.
   */
  async updateYtdlp(): Promise<UpdateReport | null> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return null;
      const outcome = await invoke<UpdateOutcome>("tools_update_ytdlp");
      const report: UpdateReport = { ...outcome, key: updateMessageKey(outcome) };
      this.lastUpdate = report;
      await this.refresh();
      this.clearError();
      return report;
    } catch (err) {
      this.#fail(err);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** `null` clears the override; anything else sets it. */
  async setToolPath(tool: string, path: string | null): Promise<void> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return;
      await invoke("tools_set_path", { tool, path });
      await this.refresh();
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.busy = false;
    }
  }

  async setYtdlpChannel(channel: string): Promise<void> {
    this.busy = true;
    try {
      if (!isTauriRuntime()) return;
      await invoke("tools_set_ytdlp_channel", { channel });
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.busy = false;
    }
  }

  /** Mark the wizard as done so it stops appearing at every launch. */
  async completeSetup(): Promise<void> {
    try {
      if (!isTauriRuntime()) return;
      await invoke("tools_complete_setup");
      this.view = { ...this.view, setupWizardCompleted: true };
    } catch (err) {
      this.#fail(err);
    }
  }

  dismissUpdate(): void {
    this.lastUpdate = null;
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
}

/** Which message the update outcome maps to (spec §5.5). */
export function updateMessageKey(outcome: UpdateOutcome): string {
  if (outcome.updated) return "tools.update.done";
  if (outcome.version && !outcome.previousVersion) return "tools.update.done";
  if (outcome.previousVersion && outcome.version === outcome.previousVersion) {
    return "tools.update.upToDate";
  }
  if (outcome.previousVersion && !outcome.version) return "tools.update.broken";
  return "tools.update.notChanged";
}

export const toolsStore = new ToolsStore();