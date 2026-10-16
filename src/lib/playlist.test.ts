import { describe, expect, it } from "vitest";
import type { EntryLite, PlaylistInfo } from "$lib/inspect";
import {
  buildPlaylistRequests,
  defaultSelection,
  filterEntries,
  formatPlaylistPrefix,
  invertSelection,
  mergeEntries,
  nextPageRange,
  orderEntries,
  parseRange,
  playlistWarningKey,
  sanitizeFolderName,
  selectAll,
  selectBetween,
  selectNone,
  selectPositions,
  totalDuration,
} from "$lib/playlist";

function entry(index: number, overrides: Partial<EntryLite> = {}): EntryLite {
  return {
    index,
    id: `id${index}`,
    url: `https://www.youtube.com/watch?v=id${index}`,
    title: `Video ${index}`,
    duration: 60,
    available: true,
    ...overrides,
  };
}

function playlist(entries: EntryLite[], overrides: Partial<PlaylistInfo> = {}): PlaylistInfo {
  return {
    id: "PL1",
    title: "My: playlist?",
    site: "Youtube",
    webpageUrl: null,
    playlistKind: "playlist",
    count: entries.length,
    entries,
    truncated: false,
    ...overrides,
  };
}

describe("parseRange()", () => {
  it("reads numbers, ranges and open ends", () => {
    expect(parseRange("1-3, 7, 9-", 10)).toEqual([1, 2, 3, 7, 9, 10]);
    expect(parseRange("-2", 5)).toEqual([1, 2]);
    expect(parseRange("3 5", 9)).toEqual([3, 5]);
  });

  it("clips to the playlist and reads a reversed pair the right way round", () => {
    expect(parseRange("8-20", 10)).toEqual([8, 9, 10]);
    expect(parseRange("4-2", 10)).toEqual([2, 3, 4]);
    expect(parseRange("50", 10)).toEqual([]);
  });

  it("refuses anything it cannot read instead of selecting half", () => {
    expect(parseRange("", 10)).toBeNull();
    expect(parseRange("a-3", 10)).toBeNull();
    expect(parseRange("1-2-3", 10)).toBeNull();
    expect(parseRange("-", 10)).toBeNull();
    expect(parseRange("1-3", 0)).toBeNull();
  });
});

describe("filtering and selecting", () => {
  const entries = [
    entry(1, { title: "Perché no" }),
    entry(2, { title: "[Private video]", available: false }),
    entry(3, { title: "Terzo" }),
  ];

  it("filters by title, folding case and accents", () => {
    expect(filterEntries(entries, "perche").map((e) => e.index)).toEqual([1]);
    expect(filterEntries(entries, "  ").length).toBe(3);
    expect(filterEntries(entries, "zzz")).toEqual([]);
  });

  it("never selects an unavailable entry", () => {
    expect([...selectAll(new Set(), entries)]).toEqual([1, 3]);
    expect([...selectPositions(entries, [1, 2, 3])]).toEqual([1, 3]);
  });

  it("selects none and inverts only inside the view", () => {
    const all = selectAll(new Set(), entries);
    expect([...selectNone(all, [entries[0]])]).toEqual([3]);
    expect([...invertSelection(new Set([1]), entries)]).toEqual([3]);
  });

  it("shift-click covers the rows between, skipping disabled ones", () => {
    const next = selectBetween(new Set(), entries, 1, 3, true);
    expect([...next].sort()).toEqual([1, 3]);
    const cleared = selectBetween(next, entries, 3, 1, false);
    expect(cleared.size).toBe(0);
    expect(selectBetween(new Set([9]), entries, 99, 1, true)).toEqual(new Set([9]));
  });
});

describe("the default selection and warnings", () => {
  it("takes everything available from a normal playlist", () => {
    const list = playlist([entry(1), entry(2, { available: false }), entry(3)]);
    expect([...defaultSelection(list)]).toEqual([1, 3]);
    expect(playlistWarningKey(list)).toBeNull();
  });

  it("caps a Mix at 50 and warns", () => {
    const list = playlist(
      Array.from({ length: 80 }, (_, i) => entry(i + 1)),
      { playlistKind: "mix" },
    );
    expect(defaultSelection(list).size).toBe(50);
    expect(playlistWarningKey(list)).toBe("playlist.warning.mix");
  });

  it("caps a huge channel at 100 and warns, but not a small one", () => {
    const big = playlist(
      Array.from({ length: 250 }, (_, i) => entry(i + 1)),
      { playlistKind: "channel" },
    );
    expect(defaultSelection(big).size).toBe(100);
    expect(playlistWarningKey(big)).toBe("playlist.warning.channel");
    const small = playlist(
      Array.from({ length: 30 }, (_, i) => entry(i + 1)),
      { playlistKind: "channel" },
    );
    expect(defaultSelection(small).size).toBe(30);
    expect(playlistWarningKey(small)).toBeNull();
  });
});

describe("totals, order and paging", () => {
  it("sums the known durations", () => {
    expect(totalDuration([entry(1), entry(2, { duration: null })])).toBe(60);
    expect(totalDuration([entry(1, { duration: null })])).toBeNull();
  });

  it("reverses without changing positions", () => {
    expect(orderEntries([entry(2), entry(1), entry(3)], true).map((e) => e.index)).toEqual([3, 2, 1]);
    expect(orderEntries([entry(2), entry(1)], false).map((e) => e.index)).toEqual([1, 2]);
  });

  it("merges a page by position so a repeat cannot duplicate rows", () => {
    const merged = mergeEntries([entry(1), entry(2)], [entry(2, { title: "new" }), entry(3)]);
    expect(merged.map((e) => e.index)).toEqual([1, 2, 3]);
    expect(merged[1].title).toBe("new");
    expect(nextPageRange(1000, 1000)).toEqual([1001, 2000]);
  });
});

describe("names", () => {
  it("makes a safe folder name", () => {
    expect(sanitizeFolderName("My: playlist?")).toBe("My_ playlist_");
    expect(sanitizeFolderName("name.  ")).toBe("name");
    expect(sanitizeFolderName("...")).toBe("Playlist");
    expect(sanitizeFolderName("CON")).toBe("_CON");
    expect(sanitizeFolderName("Perché ☃")).toBe("Perché ☃");
    expect(sanitizeFolderName("x".repeat(300)).length).toBe(100);
  });

  it("pads the numbering to the playlist size, at least three digits", () => {
    expect(formatPlaylistPrefix(1, 42)).toBe("001 - ");
    expect(formatPlaylistPrefix(7, 1500)).toBe("0007 - ");
  });
});

describe("buildPlaylistRequests()", () => {
  const list = playlist([entry(1), entry(2, { available: false }), entry(3), entry(4)]);
  const options = {
    groupId: "g1",
    quality: { kind: "maxHeight" as const, height: 1080 },
    outputDir: null,
    folderPerPlaylist: true,
    numbering: true,
    useArchive: true,
  };

  it("makes one request per selected, available entry with its own position", () => {
    const requests = buildPlaylistRequests(list, new Set([1, 2, 4]), options);
    expect(requests.map((r) => r.url)).toEqual([list.entries[0].url, list.entries[3].url]);
    expect(requests.map((r) => r.filenamePrefix)).toEqual(["001 - ", "004 - "]);
    expect(requests[0].groupId).toBe("g1");
    expect(requests[0].outputSubdir).toBe("My_ playlist_");
    expect(requests[0].useArchive).toBe(true);
    expect(requests[0].sleepInterval).toBeNull();
  });

  it("can drop the folder and the numbering", () => {
    const [request] = buildPlaylistRequests(list, new Set([1]), {
      ...options,
      folderPerPlaylist: false,
      numbering: false,
    });
    expect(request.outputSubdir).toBeNull();
    expect(request.filenamePrefix).toBeNull();
  });

  it("paces big batches and marks audio-only as audio", () => {
    const many = playlist(Array.from({ length: 25 }, (_, i) => entry(i + 1)));
    const requests = buildPlaylistRequests(many, new Set(many.entries.map((e) => e.index)), {
      ...options,
      quality: { kind: "audioOnly" },
    });
    expect(requests).toHaveLength(25);
    expect(requests[0].sleepInterval).toEqual([1, 4]);
    expect(requests[0].kind).toBe("audio");
  });
});
