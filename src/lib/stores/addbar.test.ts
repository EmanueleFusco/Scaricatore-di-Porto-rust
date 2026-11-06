import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, Channel: class {
  constructor(public onmessage: (event: unknown) => void) {}
} }));
vi.mock("$lib/stores/settings.svelte", async () => {
  const { DEFAULT_SETTINGS } = await import("$lib/settings");
  const settingsStore = {
    settings: structuredClone(DEFAULT_SETTINGS),
    update: vi.fn(async (patch: Record<string, unknown>) => {
      settingsStore.settings = { ...settingsStore.settings, ...patch };
      return true;
    }),
  };
  return { isTauriRuntime: () => true, settingsStore };
});
vi.mock("$lib/stores/queue.svelte", () => ({ queueStore: { addLocalFile: vi.fn() } }));

import { addBarStore as store } from "./addbar.svelte";
import { settingsStore } from "$lib/stores/settings.svelte";
import { DEFAULT_SETTINGS } from "$lib/settings";

interface Pending {
  requestId: number;
  urls: string[];
  onResult: { onmessage: (event: unknown) => void };
  resolve: (answers: unknown[]) => void;
  reject: (error: unknown) => void;
}
let pending: Pending[];
const first = "https://example.com/first";
const second = "https://example.com/second";
function answer(url: string, title = url) {
  return { url, inspected: { kind: "video", id: url, title }, error: null };
}

beforeEach(async () => {
  store.clear();
  settingsStore.settings = structuredClone(DEFAULT_SETTINGS);
  await store.selectPreset(null);
  store.errorKey = null;
  invoke.mockReset();
  pending = [];
  invoke.mockImplementation((command: string, args: Omit<Pending, "resolve" | "reject">) => {
    if (command !== "inspect_urls") return Promise.resolve();
    return new Promise<unknown[]>((resolve, reject) => pending.push({ ...args, resolve, reject }));
  });
});

describe("Add bar inspection lifecycle", () => {
  it("does not clear a replacement row when an older add response names the same URL", async () => {
    const initial = store.addLinks([first]);
    pending[0].resolve([answer(first)]);
    await initial;
    const old = store.toRequests()[0];
    store.removeRow(store.rows[0].key);
    await store.addLinks([first]);
    store.clearAdded([old.clientKey!]);
    expect(store.rows).toHaveLength(1);
    expect(store.rows[0].key).not.toBe(old.clientKey);
  });
  it("saves media options, applies them to new links and deletes the selected preset", async () => {
    const initial = store.addLinks([first]);
    pending[0].resolve([answer(first)]);
    await initial;
    store.preset = "audio";
    store.setMediaOptions(store.rows[0].key, { encode: true, audioCodec: "opus", transcribe: true, transcriptionEngine: "whisper_only" });
    store.setNetworkOptions(store.rows[0].key, { customHeaders: "Authorization: fake-private-header" });
    expect(await store.savePreset("Podcast")).toBe(true);
    const saved = settingsStore.settings.savedPresets[0];
    expect(saved.options).not.toHaveProperty("customHeaders");
    expect(settingsStore.settings.lastPreset).toBe(saved.id);
    store.clear();
    const fresh = store.addLinks([second]);
    pending[1].resolve([answer(second)]);
    await fresh;
    expect(store.rows[0]).toMatchObject({ encode: true, audioCodec: "opus", transcribe: true, transcriptionEngine: "whisper_only" });
    expect(store.toRequests()[0].kind).toBe("audio");
    expect(await store.deletePreset(saved.id)).toBe(true);
    expect(settingsStore.settings.savedPresets).toEqual([]);
    expect(settingsStore.settings.lastPreset).toBeNull();
    expect(store.rows[0].transcribe).toBe(false);
  });
  it("preserves streamed successes if the remaining batch fails", async () => {
    const work = store.addLinks([first, second]);
    pending[0].onResult.onmessage({ requestId: pending[0].requestId, answer: answer(second) });
    pending[0].reject({ messageKey: "error.io" });
    await work;
    expect(store.rows.map((row) => row.state)).toEqual(["error", "ready"]);
  });
  it("shows a fast row before its batch completes and ignores late stream events", async () => {
    const work = store.addLinks([first, second]);
    pending[0].onResult.onmessage({ requestId: pending[0].requestId, answer: answer(second) });
    expect(store.rows.map((row) => row.state)).toEqual(["checking", "ready"]);
    store.clear();
    pending[0].onResult.onmessage({ requestId: pending[0].requestId, answer: answer(first) });
    pending[0].resolve([answer(first), answer(second)]);
    await work;
    expect(store.rows).toEqual([]);
  });
  it.each([false, true])("keeps overlapping batches independent (newer first: %s)", async (newerFirst) => {
    const old = store.addLinks([first]);
    const fresh = store.addLinks([second]);
    const order = newerFirst ? [1, 0] : [0, 1];
    pending[order[0]].resolve([answer(pending[order[0]].urls[0])]);
    await (newerFirst ? fresh : old);
    expect(store.checking).toBe(true);
    pending[order[1]].resolve([answer(pending[order[1]].urls[0])]);
    await Promise.all([old, fresh]);
    expect(store.rows.map((row) => row.state)).toEqual(["ready", "ready"]);
    expect(store.checking).toBe(false);
  });

  it("cancels only the removed row and lets its batch peers finish", async () => {
    const work = store.addLinks([first, second]);
    store.removeRow(store.rows[0].key);
    expect(invoke).toHaveBeenCalledWith("inspect_cancel", { requestId: pending[0].requestId, urls: [first] });
    pending[0].resolve([answer(first), answer(second)]);
    await work;
    expect(store.rows.map((row) => [row.url, row.state])).toEqual([[second, "ready"]]);
  });

  it.each([false, true])("ignores cleared batch replies, including failures (%s)", async (failure) => {
    const old = store.addLinks([first]);
    store.clear();
    const fresh = store.addLinks([first]);
    expect(pending[1].requestId).not.toBe(pending[0].requestId);
    if (failure) pending[0].reject({ messageKey: "error.old" });
    else pending[0].resolve([answer(first, "Old title")]);
    await old;
    expect(store.rows[0].state).toBe("checking");
    pending[1].resolve([answer(first, "New title")]);
    await fresh;
    expect(store.rows[0].inspected).toMatchObject({ title: "New title" });
  });

  it("keeps a retry result when the superseded request finishes later", async () => {
    const old = store.addLinks([first]);
    const retry = store.retryRow(store.rows[0].key);
    pending[1].resolve([answer(first, "Retried")]);
    await retry;
    pending[0].resolve([answer(first, "Stale")]);
    await old;
    expect(store.rows[0].inspected).toMatchObject({ title: "Retried" });
    expect(store.checking).toBe(false);
  });

  it("turns missing backend answers into retryable errors", async () => {
    const work = store.addLinks([first]);
    pending[0].resolve([]);
    await work;
    expect(store.rows[0].state).toBe("error");
    expect(store.checking).toBe(false);
  });

  it("does not invalidate another batch while loading a playlist page", async () => {
    const playlist = store.addLinks([first]);
    const inspected = { kind: "playlist", id: "list", entries: [], count: 2000 };
    pending[0].resolve([{ url: first, inspected }]);
    await playlist;
    const video = store.addLinks([second]);
    const page = store.loadMore(store.rows[0].key);
    pending[1].resolve([answer(second)]);
    await video;
    pending[2].resolve([{ url: first, inspected }]);
    await page;
    expect(store.rows.map((row) => row.state)).toEqual(["playlist", "ready"]);
    expect(store.rows[0].loadingMore).toBe(false);
  });

  it("ignores pagination failures after clearing the rows", async () => {
    const initial = store.addLinks([first]);
    pending[0].resolve([{ url: first, inspected: { kind: "playlist", id: "list", entries: [] } }]);
    await initial;
    const page = store.loadMore(store.rows[0].key);
    store.clear();
    pending[1].reject({ messageKey: "error.old" });
    await page;
    expect(store.rows).toEqual([]);
    expect(store.errorKey).toBeNull();
  });
});
