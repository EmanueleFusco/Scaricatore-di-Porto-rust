import type { VisibleRange } from "$lib/queue";

/** Prefix offsets include each card's measured height and its following gap. */
export function rowOffsets(ids: readonly string[], heights: Readonly<Record<string, number>>, fallback: number): number[] {
  const offsets = [0];
  for (const id of ids) {
    const measured = heights[id];
    const height = Number.isFinite(measured) && measured > 0 ? measured : Math.max(1, fallback);
    offsets.push(offsets[offsets.length - 1] + height);
  }
  return offsets;
}

function rowAt(offsets: readonly number[], pixel: number): number {
  let low = 0;
  let high = offsets.length - 1;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (offsets[mid] <= pixel) low = mid;
    else high = mid - 1;
  }
  return Math.min(offsets.length - 2, low);
}

export function variableVisibleRange(scrollTop: number, viewport: number, offsets: readonly number[], overscan = 2): VisibleRange {
  const count = offsets.length - 1;
  if (count <= 0) return { first: 0, last: -1, topPad: 0, bottomPad: 0 };
  const top = Math.max(0, scrollTop);
  const first = Math.max(0, rowAt(offsets, top) - overscan);
  const last = Math.min(count - 1, rowAt(offsets, top + Math.max(0, viewport)) + overscan);
  return { first, last, topPad: offsets[first], bottomPad: offsets[count] - offsets[last + 1] };
}

export function variableScrollOffset(index: number, offsets: readonly number[], scrollTop: number, viewport: number): number | null {
  if (index < 0 || index >= offsets.length - 1) return null;
  const top = offsets[index];
  const bottom = offsets[index + 1];
  if (top < scrollTop || bottom - top > viewport) return top;
  return bottom > scrollTop + viewport ? bottom - viewport : null;
}
