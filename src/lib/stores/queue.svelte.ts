/**
 * The queue store (spec §3, §7.6, §11.7).
 *
 * One event, `queue:update`, arrives already coalesced by the backend to
 * ≤ 10/s per item and already reduced to the items that changed. This store's
 * whole job is to patch **those** rows and nothing else: a tick with one
 * changed item must repaint one row, not the list (spec §11.7). So the state
 * is a keyed map plus an explicit order, and every write is a per-item patch.
 *
 * The M1 single-job store is untouched and still works: the temporary
 * single-video flow is not the queue, and M1's behaviour has to survive M2
 * (spec §14).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isActive,
  isParked,
  normaliseAddResult,
  normaliseSnapshot,
  normaliseUpdate,
  type AddItemRequest,
  type AddResult,
  type ItemAction,
  type QueueGroup,
  type QueueItem,
  type QueueSnapshot,
  type QueueSummary,
  type QueueToast,
  type QueueUpdate,
} from "$lib/queue";
import { isTauriRuntime } from "$lib/stores/settings.svelte";

const EMPTY_SUMMARY: QueueSummary = {
  total: 0,
  queued: 0,
  running: 0,
  paused: 0,
  interrupted: 0,
  done: 0,
  failed: 0,
  cancelled: 0,
  speedBps: null,
  pausedAll: false,
  parallelDownloads: 2,
  perHostCap: 2,
};

class QueueStore {
  /** Keyed by item id: the patch target of every event. */
  items = $state<Record<string, QueueItem>>({});
  /** The queue order, as ids. The backend owns it; we only mirror it. */
  order = $state<string[]>([]);
  groups = $state<QueueGroup[]>([]);
  summary = $state<QueueSummary>({ ...EMPTY_SUMMARY });
  loaded = $state(false);
  /** The last toast, already resolved to a key + params by the backend. */
  toast = $state<QueueToast | null>(null);
  errorKey = $state<string | null>(null);
  errorDetail = $state<string | null>(null);
  /** Duplicates refused by the last `queue_add` (spec §7.3 "add anyway?"). */
  pendingDuplicates = $state<AddResult["duplicates"]>([]);
  /** What the user was adding, kept so "add anyway" can resend it. */
  #lastRequest: AddItemRequest[] = [];
  #unlisten: UnlistenFn[] = [];
  #listening = false;

  /** The items in queue order. Derived in the component, not stored twice. */
  get list(): QueueItem[] {
    return this.order.map((id) => this.items[id]).filter((item): item is QueueItem => item !== undefined);
  }

  async start(): Promise<void> {
    if (this.#listening || !isTauriRuntime()) return;
    this.#listening = true;
    this.#unlisten.push(
      await listen<unknown>("queue:update", (event) => {
        this.apply(normaliseUpdate(event.payload));
      }),
    );
    await this.load();
  }

  async stop(): Promise<void> {
    for (const unlisten of this.#unlisten) unlisten();
    this.#unlisten = [];
    this.#listening = false;
  }

  /** The initial read (spec §3 `queue_list`). */
  async load(): Promise<void> {
    if (!isTauriRuntime()) {
      this.loaded = true;
      return;
    }
    try {
      const snapshot = normaliseSnapshot(await invoke<unknown>("queue_list"));
      this.#applySnapshot(snapshot);
    } catch (err) {
      this.#fail(err);
    } finally {
      this.loaded = true;
    }
  }

  /**
   * Fold one event in.
   *
   * Exported as a plain method (not inline in the listener) so the reducer is
   * testable without a Tauri event bus.
   */
  apply(update: QueueUpdate): void {
    if (update.seq !== 0 && update.seq <= this.summarySeq) return;
    if (update.seq !== 0) this.summarySeq = update.seq;

    if (update.items.length > 0) {
      // Patch only what changed: a new object per changed row, the same object
      // for every other row.
      const next = { ...this.items };
      for (const item of update.items) {
        next[item.id] = item;
      }
      this.items = next;
      // An item the store has never seen arrives without an `order` event on a
      // restart (the list came from `queue_list`), so only add what is missing.
      const known = new Set(this.order);
      const fresh = update.items.filter((item) => !known.has(item.id)).map((item) => item.id);
      if (fresh.length > 0) this.order = [...this.order, ...fresh];
    }

    if (update.removed.length > 0) {
      const gone = new Set(update.removed);
      const next: Record<string, QueueItem> = {};
      for (const [id, item] of Object.entries(this.items)) {
        if (!gone.has(id)) next[id] = item;
      }
      this.items = next;
      this.order = this.order.filter((id) => !gone.has(id));
    }

    if (update.order !== null) {
      this.order = update.order.filter((id) => this.items[id] !== undefined);
    }

    this.summary = update.summary;
    if (update.toast !== null) this.toast = update.toast;
  }

  /** The newest sequence seen, so an event that arrives out of order is ignored. */
  summarySeq = 0;

  #applySnapshot(snapshot: QueueSnapshot): void {
    const next: Record<string, QueueItem> = {};
    for (const item of snapshot.items) next[item.id] = item;
    this.items = next;
    this.order = snapshot.items.map((item) => item.id);
    this.groups = snapshot.groups;
    this.summary = snapshot.summary;
    this.summarySeq = snapshot.seq;
    if (snapshot.toast !== null) this.toast = snapshot.toast;
  }

  /** Add links. Without `force` the duplicates come back to ask about. */
  async add(requests: AddItemRequest[], force = false): Promise<AddResult> {
    if (!isTauriRuntime() || requests.length === 0) {
      return { added: [], duplicates: [] };
    }
    this.#lastRequest = requests;
    try {
      const result = normaliseAddResult(
        await invoke<unknown>("queue_add", { items: requests, force }),
      );
      this.pendingDuplicates = result.duplicates;
      this.errorKey = null;
      this.errorDetail = null;
      if (result.added.length > 0) {
        // The rows themselves arrive as events; this keeps the list from
        // looking empty for one tick.
        await this.load();
      }
      return result;
    } catch (err) {
      this.#fail(err);
      return { added: [], duplicates: [] };
    }
  }

  /** "Add anyway": resend exactly what was refused. */
  async addAnyway(): Promise<AddResult> {
    const requests = this.#lastRequest;
    this.pendingDuplicates = [];
    return this.add(requests, true);
  }

  get hasPendingDuplicates(): boolean {
    return this.pendingDuplicates.length > 0;
  }

  dismissDuplicates(): void {
    this.pendingDuplicates = [];
  }

  dismissToast(): void {
    this.toast = null;
  }

  /** One entry point for every per-item action (spec §11.5). */
  async act(action: ItemAction, id: string): Promise<void> {
    if (!isTauriRuntime()) return;
    const commands: Partial<Record<ItemAction, string>> = {
      pause: "queue_pause",
      resume: "queue_resume",
      cancel: "queue_cancel",
      retry: "queue_retry",
      remove: "queue_remove",
      moveToTop: "queue_move_to_top",
    };
    const command = commands[action];
    if (command !== undefined) {
      await this.#invoke(command, { id });
    }
  }

  async pauseAll(): Promise<void> {
    await this.#invoke("queue_pause_all");
  }

  async resumeAll(): Promise<void> {
    await this.#invoke("queue_resume_all");
  }

  async retryFailed(): Promise<void> {
    await this.#invoke("queue_retry_failed");
  }

  async clearFinished(): Promise<void> {
    await this.#invoke("queue_clear_finished");
  }

  /** Reorder from the visible list: the new order of the ids on screen. */
  async reorder(ids: string[]): Promise<void> {
    if (!isTauriRuntime()) return;
    this.order = ids;
    await this.#invoke("queue_reorder", { ids });
  }

  async #invoke(command: string, args?: Record<string, unknown>): Promise<void> {
    if (!isTauriRuntime()) return;
    try {
      await invoke(command, args);
    } catch (err) {
      this.#fail(err);
    }
  }

  #fail(err: unknown): void {
    const info = err as Partial<{ messageKey?: unknown; detail?: unknown }>;
    this.errorKey = typeof info?.messageKey === "string" ? info.messageKey : "error.tauri";
    this.errorDetail = typeof info?.detail === "string" ? info.detail : null;
  }
}

export const queueStore = new QueueStore();

/** The running items, in queue order (the footer and the tests use it). */
export function runningItems(items: readonly QueueItem[]): QueueItem[] {
  return items.filter((item) => isActive(item.state));
}

/** The parked items (terminal, paused or interrupted) — the history. */
export function parkedItems(items: readonly QueueItem[]): QueueItem[] {
  return items.filter((item) => isParked(item.state));
}