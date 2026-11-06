import { describe, expect, it } from "vitest";
import { actionLayout, actionsFor, type ItemState } from "./queue";
describe("compact queue actions", () => {
  it("keeps every supported action reachable with at most two primary actions", () => {
    const states: ItemState[] = ["queued", "inspecting", "downloading", "postProcessing", "encoding", "transcribing", "summarizing", "paused", "interrupted", "failed", "cancelled", "done"];
    for (const state of states) for (const hasFile of [true, false]) for (const remote of [true, false]) {
      const layout = actionLayout(state, hasFile, remote);
      expect(layout.primary.length).toBeLessThanOrEqual(2);
      expect([...layout.primary, ...layout.more].sort()).toEqual(actionsFor(state, hasFile, remote).sort());
    }
  });
  it("prioritizes output access, recovery and active transfer controls", () => {
    expect(actionLayout("done", true).primary).toEqual(["open", "reveal"]);
    expect(actionLayout("failed", false).primary).toEqual(["retry", "remove"]);
    expect(actionLayout("downloading", false).primary).toEqual(["pause", "cancel"]);
    expect(actionLayout("paused", false).primary).toEqual(["resume", "cancel"]);
  });
});
