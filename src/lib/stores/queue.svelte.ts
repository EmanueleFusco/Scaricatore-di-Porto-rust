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
  applySnapshot,
  applyUpdate,
  emptyQueueState,
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
  type QueueState,
  type QueueSummary,
  type QueueToast,
  type QueueUpdate,
} from "$lib/queue";
import { isTauriRuntime } from "$lib/stores/settings.svelte";

class QueueStore {
  /**
   * The queue as **one value**, so the reducer in `$lib/queue` can stay pure
   * and be tested without a Svelte compiler — the store is a shell that hands
   * the value in and takes the result out.
   *
   * The components still read single fields (`summary`, `items`, `order`),
   * through the getters below, and Svelte 5's fine-grained reactivity follows
   * the reads: a tick that changes one item gives that item a new object and
   * leaves the others identical, so one row repaints and the list does not
   * (spec §11.7).
   */
  #state = $state<QueueState>(emptyQueueState());
  #loaded = $state(false);

  /** Keyed by item id: the patch target of every event. */
  get items(): Record<string, QueueItem> {
    return this.#state.items;
  }
  /** The queue order, as ids. The backend owns it; we only mirror it. */
  get order(): string[] {
    return this.#state.order;
  }
  get groups(): QueueGroup[] {
    return this.#state.groups;
  }
  get summary(): QueueSummary {
    return this.#state.summary;
  }
  /** The newest `seq` already shown, so an older event is dropped. */
  get summarySeq(): number {
    return this.#state.seq;
  }
  get loaded(): boolean {
    return this.#loaded;
  }
  /** The last toast, already resolved to a key + params by the backend. */
  get toast(): QueueToast | null {
    return this.#state.toast;
  }

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
      this.#loaded = true;
      return;
    }
    // What the UI already knows when the request goes out. A `queue_list` that
    // comes back having been overtaken by events describes a queue that has
    // already moved on, and applying it wholesale would put a row back to
    // `queued` — possibly for good, if the queue went idle before the next
    // event (D97).
    const seq = this.#state.seq;
    try {
      const snapshot = normaliseSnapshot(await invoke<unknown>("queue_list"));
      const previous = this.#state;
      this.#write(applySnapshot(previous, snapshot, previous.seq > seq), previous);
    } catch (err) {
      this.#fail(err);
    } finally {
      this.#loaded = true;
    }
  }

  /**
   * Write a new state back, **field by field**.
   *
   * Assigning `this.#state = next` would replace the whole object and
   * invalidate every reader of every field, which is exactly the granularity
   * spec §11.7 asks the UI not to lose: a tick that changes one item would
   * repaint the footer too. Only the fields the reducer actually changed are
   * assigned, so the reactivity stays per field.
   */
  #write(next: QueueState, previous: QueueState): void {
    if (next.items !== previous.items) this.#state.items = next.items;
    if (next.order !== previous.order) this.#state.order = next.order;
    if (next.groups !== previous.groups) this.#state.groups = next.groups;
    if (next.summary !== previous.summary) this.#state.summary = next.summary;
    if (next.seq !== previous.seq) this.#state.seq = next.seq;
    if (next.toast !== previous.toast) this.#state.toast = next.toast;
  }

  /**
   * Fold one event in.
   *
   * A method (not the listener body) so the reducer in `$lib/queue` is the one
   * thing that decides, and it is tested there.
   */
  apply(update: QueueUpdate): void {
    const previous = this.#state;
    this.#write(applyUpdate(previous, update), previous);
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

  /**
   * Put freshly added items at the top of the queue.
   *
   * This is what makes the add bar's "Download" the standard download: the
   * links run now, ahead of whatever is still waiting (spec §11.3, the
   * "Start immediately" side of the two actions). When nothing else is queued
   * they are next anyway, and no call is made at all.
   */
  async prioritize(ids: readonly string[]): Promise<void> {
    if (!isTauriRuntime() || ids.length === 0) return;
    const fresh = new Set(ids);
    const behind = this.#state.order.some(
      (id) => !fresh.has(id) && this.items[id]?.state === "queued",
    );
    if (!behind) return;
    // Back to front: each call inserts at the top, so the batch keeps its order.
    for (const id of [...ids].reverse()) {
      await this.#invoke("queue_move_to_top", { id });
    }
  }

  get hasPendingDuplicates(): boolean {
    return this.pendingDuplicates.length > 0;
  }

  /**
   * Ids that were just added, for the list to scroll to and flash (D104).
   *
   * The queue keeps the order the items were added in — FIFO, because a batch
   * (a playlist, in M3) must not be starved by every link added after it — so a
   * new item can land below the visible rows and look like it went nowhere.
   * This is the feedback that fixes that, without touching the order.
   */
  flashIds = $state<string[]>([]);
  #flashTimer: ReturnType<typeof setTimeout> | null = null;

  flash(ids: readonly string[]): void {
    if (ids.length === 0) return;
    this.flashIds = [...ids];
    if (this.#flashTimer !== null) clearTimeout(this.#flashTimer);
    this.#flashTimer = setTimeout(() => {
      this.flashIds = [];
      this.#flashTimer = null;
    }, 2500);
  }

  isFlashing(id: string): boolean {
    return this.flashIds.includes(id);
  }

  dismissDuplicates(): void {
    this.pendingDuplicates = [];
  }

  dismissToast(): void {
    this.#state.toast = null;
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

  /** Create or update a playlist group header (spec §6.4, §11.5). */
  async upsertGroup(group: QueueGroup): Promise<void> {
    await this.#invoke("queue_upsert_group", { group });
  }

  async pauseGroup(groupId: string): Promise<void> {
    await this.#invoke("queue_pause_group", { groupId });
  }

  async resumeGroup(groupId: string): Promise<void> {
    await this.#invoke("queue_resume_group", { groupId });
  }

  async retryGroupFailed(groupId: string): Promise<void> {
    await this.#invoke("queue_retry_group_failed", { groupId });
  }

  async removeGroup(groupId: string): Promise<void> {
    await this.#invoke("queue_remove_group", { groupId });
  }

  /** Reorder from the visible list: the new order of the ids on screen. */
  async reorder(ids: string[]): Promise<void> {
    if (!isTauriRuntime()) return;
    // Shown immediately, the way a drag should feel; the backend owns the real
    // order and sends it back in the next event.
    this.#state.order = ids;
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