import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("$lib/stores/settings.svelte", () => ({ isTauriRuntime: () => true }));
import { queueStore as store } from "./queue.svelte";
import type { AddItemRequest } from "$lib/queue";

const request = (url: string, height = 720): AddItemRequest => ({
  url, title: null, site: null, durationSec: null, kind: "video",
  quality: { kind: "maxHeight", height }, outputDir: null, groupId: null,
});
beforeEach(() => {
  store.dismissDuplicates();
  invoke.mockReset();
  invoke.mockResolvedValue({});
});
describe("duplicate reconciliation", () => {
  it("retains local row ownership in results without sending it to the backend", async () => {
    const item = { ...request("https://example.com/a"), clientKey: "r42" };
    invoke.mockResolvedValueOnce({ added: ["new"], duplicates: [] });
    const result = await store.add([item]);
    expect(result.accepted?.[0].clientKey).toBe("r42");
    expect(invoke).toHaveBeenCalledWith("queue_add", { items: [request(item.url)], force: false });
  });
  it("forces only refused requests, even with identical URLs and different options", async () => {
    const requests = [request("https://example.com/a"), request("https://example.com/a", 1080)];
    invoke.mockResolvedValueOnce({ added: ["new"], duplicates: [{ url: requests[1].url, itemId: "old", requestIndex: 1 }] });
    const result = await store.add(requests);
    expect(result.accepted).toEqual([requests[0]]);
    expect(store.pendingDuplicates).toHaveLength(1);
    invoke.mockResolvedValueOnce({ added: ["forced"], duplicates: [] });
    await store.addAnyway();
    expect(invoke).toHaveBeenCalledWith("queue_add", { items: [requests[1]], force: true });
    expect(store.pendingDuplicates).toEqual([]);
  });
  it("preserves pending confirmations across later additions and failed forced requests", async () => {
    const refused = request("https://example.com/old");
    invoke.mockResolvedValueOnce({ added: [], duplicates: [{ url: refused.url, itemId: "old", requestIndex: 0 }] });
    await store.add([refused]);
    invoke.mockResolvedValueOnce({ added: ["new"], duplicates: [] });
    await store.add([request("https://example.com/new")]);
    expect(store.pendingDuplicates).toHaveLength(1);
    invoke.mockRejectedValueOnce({ messageKey: "error.io" });
    await store.addAnyway();
    expect(store.pendingDuplicates).toHaveLength(1);
  });
});
