import { describe, expect, it } from "vitest";
import { rowOffsets, variableScrollOffset, variableVisibleRange } from "./virtual-list";
describe("variable-height queue virtualization", () => {
  it("uses expanded and wrapped row heights without overlapping or missing padding", () => {
    const offsets = rowOffsets(["a", "b", "c", "d"], { a: 80, b: 420, c: 120, d: 90 }, 132);
    expect(offsets).toEqual([0, 80, 500, 620, 710]);
    const range = variableVisibleRange(480, 150, offsets, 0);
    expect(range).toEqual({ first: 1, last: 3, topPad: 80, bottomPad: 0 });
    expect(variableScrollOffset(2, offsets, 0, 200)).toBe(420);
    expect(variableScrollOffset(1, offsets, 300, 200)).toBe(80);
    expect(variableScrollOffset(0, offsets, 0, 200)).toBeNull();
  });
  it("keeps the rendered slice bounded across 5000 mixed-height items", () => {
    const ids = Array.from({ length: 5000 }, (_, i) => `q${i}`);
    const heights = Object.fromEntries(ids.map((id, i) => [id, i % 7 === 0 ? 600 : 90]));
    const offsets = rowOffsets(ids, heights, 132);
    for (let scroll = 0; scroll < offsets[5000]; scroll += 1257) {
      const range = variableVisibleRange(scroll, 480, offsets);
      expect(range.last - range.first + 1).toBeLessThanOrEqual(12);
      expect(range.topPad + (offsets[range.last + 1] - offsets[range.first]) + range.bottomPad).toBe(offsets[5000]);
      expect(offsets[range.first]).toBeLessThanOrEqual(scroll);
      expect(offsets[range.last + 1]).toBeGreaterThanOrEqual(Math.min(scroll + 480, offsets[5000]));
    }
  });
  it("handles empty, unknown, invalid and reordered measurements", () => {
    expect(variableVisibleRange(10, 480, [0])).toEqual({ first: 0, last: -1, topPad: 0, bottomPad: 0 });
    expect(rowOffsets(["b", "a", "bad"], { a: 80, b: 120, bad: NaN }, 132)).toEqual([0, 120, 200, 332]);
    expect(variableScrollOffset(-1, [0, 100], 0, 100)).toBeNull();
    expect(variableScrollOffset(2, [0, 100], 0, 100)).toBeNull();
  });
});
