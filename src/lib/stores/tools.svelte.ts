/**
 * Store for the Tools page and the Setup wizard (spec §5.2, §11.1, §11.8).
 *
 * Owns the IPC and nothing else: every rule lives in `$lib/tools`, which is
 * pure and unit tested. `invoke` is skipped when the UI runs in a plain browser
 * (`npm run dev`), exactly like the settings store does, so the page renders
 * with the "not installed" state instead of an error banner.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  normaliseSelfTest,
  normaliseToolStatus,
  normaliseToolsView,
  InFlightDeduplicator,
  type SelfTestReport,
  type ToolProgressEvent,
  type ToolStatus,
  type ToolsView,
  type UpdateOutcome,
} from "$lib/tools";
import type { EncoderInfo } from "$lib/queue";
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
  /** `true` while an update, an install or a path change is in flight. */
  busy = $state(false);
  lastUpdate = $state<UpdateReport | null>(null);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);
  /** Which tool an install is downloading right now, if any. */
  installingTool = $state<string | null>(null);
  /** 0–100 while the size is known; `null` otherwise. */
  installPercent = $state<number | null>(null);
  /** Set when the user pressed Cancel, so the cancelled download is not
   * reported as a failure (spec §5.2: Cancel is a normal outcome). */
  #installCancelled = false;
  /** Last "Test YouTube extraction" run (spec §5.3.6). */
  selfTest = $state<SelfTestReport | null>(null);
  selfTestRunning = $state(false);
  /** Probed video encoders (spec §9.3). */
  probedEncoders = $state<EncoderInfo[]>([]);
  probingEncoders = $state(false);
  #encoderDeduplicator = new InFlightDeduplicator<EncoderInfo[]>();

  /** Probe video encoders (spec §9.3: run in the background, cache by path/mtime/version). */
  async probeEncoders(force = false): Promise<EncoderInfo[]> {
    if (!isTauriRuntime()) return [];
    return this.#encoderDeduplicator.run(async () => {
      this.probingEncoders = true;
      try {
        const res = await invoke<EncoderInfo[]>("encoders_probe", { force });
        this.probedEncoders = res;
        return res;
      } catch {
        return [];
      } finally {
        this.probingEncoders = false;
      }
    }, force);
  }

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

  /**
   * Install a tool on demand (spec §5.3.2).
   *
   * `tool:progress` is listened to for the duration of the call only: installs
   * are rare, and a permanent listener would keep a reference to a closed view.
   */
  async install(tool: string): Promise<ToolStatus | null> {
    this.busy = true;
    this.installingTool = tool;
    this.installPercent = null;
    this.#installCancelled = false;
    let unlisten: UnlistenFn | null = null;
    try {
      if (!isTauriRuntime()) return null;
      unlisten = await listen<ToolProgressEvent>("tool:progress", (event) => {
        const payload = event.payload;
        if (payload.tool !== tool) return;
        this.installPercent =
          typeof payload.total === "number" && payload.total > 0
            ? Math.min(100, Math.round((payload.received / payload.total) * 100))
            : null;
      });
      const status = normaliseToolStatus(await invoke<ToolStatus>("tools_install", { tool }));
      this.view = {
        ...this.view,
        tools: this.view.tools.map((row) => (row.name === status.name ? status : row)),
      };
      this.clearError();
      return status;
    } catch (err) {
      // A cancel is an answer, not a failure; the backend only says the
      // download stopped.
      if (!this.#installCancelled) {
        this.#fail(err);
      }
      return null;
    } finally {
      if (unlisten) unlisten();
      this.installingTool = null;
      this.installPercent = null;
      this.#installCancelled = false;
      this.busy = false;
    }
  }

  /** "Test YouTube extraction" (spec §5.3.6). */
  async runSelfTest(): Promise<SelfTestReport | null> {
    this.selfTestRunning = true;
    try {
      if (!isTauriRuntime()) return null;
      const report = normaliseSelfTest(await invoke("tools_self_test"));
      this.selfTest = report;
      this.clearError();
      return report;
    } catch (err) {
      this.#fail(err);
      return null;
    } finally {
      this.selfTestRunning = false;
    }
  }

  /** Cancel the install in flight (spec §5.2: progress **and** Cancel). */
  async cancelInstall(): Promise<void> {
    this.#installCancelled = true;
    try {
      if (isTauriRuntime()) await invoke("tools_cancel_install");
    } catch (err) {
      this.#fail(err);
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