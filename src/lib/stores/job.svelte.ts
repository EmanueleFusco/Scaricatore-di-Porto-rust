/**
 * The temporary single-job store (M1; M2 replaces it with the queue store).
 *
 * Owns the two events (`job:progress`, `job:done`) and the start/cancel IPC.
 * Progress is already coalesced by the backend (spec §7.6), so this store does
 * not throttle: it patches one state object per event.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauriRuntime } from "$lib/stores/settings.svelte";
import {
  normaliseDone,
  normaliseProgress,
  type DownloadStarted,
  type JobDoneEvent,
  type JobProgressEvent,
  type JobStatus,
  type QualityRequest,
} from "$lib/job";
import type { BackendErrorInfo } from "$lib/settings";

export type { JobStatus } from "$lib/job";

class JobStore {
  status = $state<JobStatus>("idle");
  jobId = $state<string | null>(null);
  progress = $state<JobProgressEvent | null>(null);
  done = $state<JobDoneEvent | null>(null);
  outputDir = $state<string | null>(null);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);
  #unlisten: UnlistenFn[] = [];
  #listening = false;

  /** Subscribe to the backend events; safe to call from `onMount`. */
  async start(): Promise<void> {
    if (this.#listening || !isTauriRuntime()) return;
    this.#listening = true;
    this.#unlisten.push(
      await listen<JobProgressEvent>("job:progress", (event) => {
        const progress = normaliseProgress(event.payload);
        if (this.jobId !== null && progress.jobId !== this.jobId) return;
        this.progress = progress;
        if (this.status === "idle") this.status = "running";
      }),
    );
    this.#unlisten.push(
      await listen<JobDoneEvent>("job:done", (event) => {
        const done = normaliseDone(event.payload);
        if (this.jobId !== null && done.jobId !== this.jobId) return;
        this.done = done;
        this.status = done.status;
      }),
    );
  }

  async stop(): Promise<void> {
    for (const unlisten of this.#unlisten) unlisten();
    this.#unlisten = [];
    this.#listening = false;
  }

  /** Start a download. Returns `false` when the backend refused it. */
  async startJob(url: string, quality: QualityRequest, outputDir: string | null): Promise<boolean> {
    if (!isTauriRuntime()) return false;
    this.errorKey = null;
    this.errorDetail = null;
    try {
      const started = await invoke<DownloadStarted>("download_start", {
        url,
        quality,
        outputDir,
      });
      this.jobId = started.jobId;
      this.outputDir = started.outputDir;
      this.progress = null;
      this.done = null;
      this.status = "running";
      return true;
    } catch (err) {
      this.#fail(err);
      return false;
    }
  }

  async cancel(): Promise<void> {
    if (this.jobId === null || !isTauriRuntime()) return;
    try {
      await invoke("download_cancel", { jobId: this.jobId });
    } catch (err) {
      this.#fail(err);
    }
  }

  /** Remember that the Deno offer was declined (spec §5.3.2). */
  async dismissDenoOffer(): Promise<void> {
    try {
      if (isTauriRuntime()) await invoke("download_dismiss_deno_offer");
    } catch (err) {
      this.#fail(err);
    }
  }

  /** Back to the start screen, keeping the URL box for the next download. */
  reset(): void {
    this.status = "idle";
    this.jobId = null;
    this.progress = null;
    this.done = null;
    this.errorKey = null;
    this.errorDetail = null;
  }

  /** `true` while a stage can be cancelled. */
  get cancellable(): boolean {
    return this.status === "running";
  }

  #fail(err: unknown): void {
    const info = err as Partial<BackendErrorInfo>;
    this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
    this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
  }
}

export const jobStore = new JobStore();
