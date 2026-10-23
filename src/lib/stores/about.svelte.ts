/**
 * Store for the About page: build info, updates, diagnostics and logs
 * (spec §11.8, §13.4).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getBundleType } from "@tauri-apps/api/app";
import {
  normaliseAppInfo,
  normaliseUpdateCheck,
  type AppInfo,
  type UpdateCheck,
  type UpdateProgress,
} from "$lib/about";
import { isTauriRuntime } from "$lib/stores/settings.svelte";

const UPDATE_CHECK_KEY = "sdp.updateCheckedAt";
const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

class AboutStore {
  info = $state<AppInfo | null>(null);
  bundleType = $state<string | null>(null);
  loading = $state(false);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  update = $state<UpdateCheck | null>(null);
  checking = $state(false);
  installing = $state(false);
  progress = $state<UpdateProgress | null>(null);
  installedVersion = $state<string | null>(null);
  /** Set by the daily check so the About tab can show a dot without opening. */
  updateAvailable = $state(false);

  copied = $state(false);

  async init(): Promise<void> {
    if (!isTauriRuntime()) return;
    this.loading = true;
    try {
      this.info = normaliseAppInfo(await invoke<AppInfo>("app_info"));
      this.bundleType = await getBundleType().catch(() => null);
      this.clearError();
    } catch (err) {
      this.#fail(err);
    } finally {
      this.loading = false;
    }
  }

  /**
   * One check per day, from the shell (spec §13.4).
   *
   * The timestamp lives in `localStorage`: it is a UI preference, not a
   * setting the backend needs, and bumping the settings schema for it would
   * migrate every existing install for no gain.
   */
  async maybeDailyCheck(): Promise<void> {
    if (!isTauriRuntime()) return;
    if (this.bundleType === "deb" || this.bundleType === "rpm") return;
    const last = Number(localStorage.getItem(UPDATE_CHECK_KEY) ?? 0);
    if (Number.isFinite(last) && Date.now() - last < UPDATE_CHECK_INTERVAL_MS) return;
    await this.checkUpdate(true);
  }

  async checkUpdate(silent = false): Promise<void> {
    if (!isTauriRuntime() || this.checking) return;
    this.checking = true;
    if (!silent) this.clearError();
    try {
      const check = normaliseUpdateCheck(await invoke<UpdateCheck>("app_update_check"));
      this.update = check;
      this.updateAvailable = check.available;
      localStorage.setItem(UPDATE_CHECK_KEY, String(Date.now()));
      if (!silent) this.clearError();
    } catch (err) {
      if (!silent) this.#fail(err);
    } finally {
      this.checking = false;
    }
  }

  async installUpdate(): Promise<boolean> {
    if (!isTauriRuntime() || this.installing) return false;
    this.installing = true;
    this.progress = null;
    this.installedVersion = null;
    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listen<UpdateProgress>("update:progress", (event) => {
        this.progress = event.payload;
      });
      const result = normaliseUpdateCheck(await invoke<UpdateCheck>("app_update_install"));
      this.update = result;
      this.installedVersion = result.version;
      this.updateAvailable = false;
      this.clearError();
      return true;
    } catch (err) {
      this.#fail(err);
      return false;
    } finally {
      if (unlisten) unlisten();
      this.installing = false;
      this.progress = null;
    }
  }

  /** Copy the plain-text report; returns the text so the page can confirm. */
  async copyDiagnostics(): Promise<string | null> {
    if (!isTauriRuntime()) return null;
    try {
      const text = await invoke<string>("app_diagnostics", {
        bundleType: this.bundleType,
      });
      await navigator.clipboard.writeText(text);
      this.copied = true;
      setTimeout(() => (this.copied = false), 2000);
      this.clearError();
      return text;
    } catch (err) {
      this.#fail(err);
      return null;
    }
  }

  async openLogs(): Promise<void> {
    if (!isTauriRuntime()) return;
    try {
      await invoke<string>("app_open_logs");
      this.clearError();
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
}

export const aboutStore = new AboutStore();
