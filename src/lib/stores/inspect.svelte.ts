/**
 * Inspection store: debounced URL checking with stale-response protection
 * (spec §6.2).
 *
 * The backend cancels the previous inspection as soon as a new request arrives;
 * this store adds the frontend half of the same rule: a response whose
 * `requestId` is no longer the latest is dropped. Without it, a slow reply
 * about the old URL could paint its title under the new one — the exact bug
 * spec §12.12 records from the original.
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "$lib/stores/settings.svelte";
import {
  inspectDelay,
  MetadataCache,
  normaliseInspected,
  type Inspected,
  type InspectView,
} from "$lib/inspect";
import type { BackendErrorInfo } from "$lib/settings";

export { INSPECT_DEBOUNCE_MS } from "$lib/inspect";

class InspectStore {
  /** What the user typed (not trimmed). */
  url = $state("");
  inspected = $state<Inspected | null>(null);
  loading = $state(false);
  /** The id of the newest request; answers older than this are ignored. */
  latestRequest = $state(0);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);
  #cache = new MetadataCache();
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** One-shot: the next change comes from a paste, so it skips the debounce. */
  #pasted = false;

  /** Called from the input's `paste` event, just before the `input` one. */
  markPaste(): void {
    this.#pasted = true;
  }

  /** Feed the URL input; starts the debounce. */
  setUrl(value: string): void {
    this.url = value;
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    this.latestRequest += 1;
    const requestId = this.latestRequest;
    const url = value.trim();
    const delay = inspectDelay(this.#pasted);
    this.#pasted = false;

    if (url === "") {
      // The box was cleared: cancel what the backend is doing and empty the
      // view. No new request exists to carry the cancellation.
      this.inspected = null;
      this.loading = false;
      this.errorKey = null;
      this.errorDetail = null;
      if (isTauriRuntime()) {
        void invoke("inspect_cancel").catch(() => {});
      }
      return;
    }

    // The original's URL cache: coming back to a URL shows the previous answer
    // without a round-trip.
    const cached = this.#cache.get(url);
    if (cached) {
      this.inspected = cached;
      this.loading = false;
      this.errorKey = null;
      this.errorDetail = null;
      return;
    }

    this.inspected = null;
    this.errorKey = null;
    this.errorDetail = null;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.#run(requestId, url);
    }, delay);
  }

  /** The Clear button: empty everything, including the cache (spec §2.9). */
  clear(): void {
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    this.latestRequest += 1;
    this.url = "";
    this.inspected = null;
    this.loading = false;
    this.errorKey = null;
    this.errorDetail = null;
    this.#cache.clear();
    if (isTauriRuntime()) {
      void invoke("inspect_cancel").catch(() => {});
    }
  }

  /**
   * Ask again **now**, without the debounce and without the cache.
   *
   * This is the "Riprova" button next to a degraded resolution list: the
   * backend already retried once on its own, so a second try is a user
   * decision, and it must not be answered from the cache.
   */
  retry(): void {
    const url = this.url.trim();
    if (url === "") return;
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    this.latestRequest += 1;
    const requestId = this.latestRequest;
    this.inspected = null;
    this.loading = true;
    this.errorKey = null;
    this.errorDetail = null;
    void this.#run(requestId, url);
  }

  async #run(requestId: number, url: string): Promise<void> {
    this.loading = true;
    try {
      if (!isTauriRuntime()) {
        // Plain-browser dev run: there is no backend to ask.
        this.inspected = null;
        return;
      }
      const view = await invoke<InspectView>("inspect_url", { requestId, url });
      if (requestId !== this.latestRequest) return; // stale
      this.inspected = normaliseInspected(view.inspected);
      if (this.inspected.kind === "video") {
        this.#cache.set(url, this.inspected);
      }
    } catch (err) {
      if (requestId !== this.latestRequest) return; // stale or cancelled
      const info = err as Partial<BackendErrorInfo>;
      if (info?.code === "CANCELLED") {
        this.inspected = null;
        return;
      }
      this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
      this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
    } finally {
      if (requestId === this.latestRequest) {
        this.loading = false;
      }
    }
  }
}

export const inspectStore = new InspectStore();
