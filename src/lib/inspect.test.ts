import { describe, expect, it } from "vitest";
import {
  CACHE_LIMIT,
  INSPECT_DEBOUNCE_MS,
  MetadataCache,
  inspectDelay,
  isMixUrl,
  isSearchOrFeedUrl,
  isVideoWithListUrl,
  normaliseInspected,
  secondsToClock,
  type Inspected,
  type VideoInfo,
} from "$lib/inspect";

type VideoInspected = Extract<Inspected, { kind: "video" }>;

function video(partial: Partial<VideoInfo> = {}): VideoInspected {
  return {
    kind: "video",
    id: "x",
    title: "Title",
    duration: 213,
    site: "Youtube",
    webpageUrl: "https://www.youtube.com/watch?v=x",
    isLive: false,
    language: null,
    thumbnailUrl: null,
    formats: [],
    heights: [1080, 720, 360],
    hasVideo: true,
    hasAudio: true,
    degraded: false,
    ...partial,
  };
}

describe("normaliseInspected", () => {
  it("keeps a video and fills the missing fields", () => {
    const inspected = normaliseInspected({ kind: "video", id: "a", heights: [720, "x", 480] });
    expect(inspected.kind).toBe("video");
    if (inspected.kind !== "video") throw new Error("type");
    expect(inspected.heights).toEqual([720, 480]);
    expect(inspected.formats).toEqual([]);
    expect(inspected.duration).toBeNull();
    expect(inspected.isLive).toBe(false);
  });

  it("keeps a playlist stub, its count, webpageUrl and entries", () => {
    const inspected = normaliseInspected({
      kind: "playlist",
      id: "PL1",
      title: "A playlist",
      site: "Youtube",
      webpageUrl: "https://www.youtube.com/playlist?list=PL1",
      playlistKind: "playlist",
      count: 42,
      entries: [
        {
          index: 1,
          id: "v1",
          url: "https://www.youtube.com/watch?v=v1",
          title: "First",
          duration: 120,
          available: true,
        },
        {
          index: 2,
          id: "v2",
          url: "https://www.youtube.com/watch?v=v2",
          title: "[Private video]",
          duration: null,
          available: false,
        },
      ],
    });
    expect(inspected.kind).toBe("playlist");
    if (inspected.kind !== "playlist") throw new Error("type");
    expect(inspected.playlistKind).toBe("playlist");
    expect(inspected.count).toBe(42);
    expect(inspected.webpageUrl).toBe("https://www.youtube.com/playlist?list=PL1");
    expect(inspected.entries).toHaveLength(2);
    expect(inspected.entries[0]?.title).toBe("First");
    expect(inspected.entries[0]?.available).toBe(true);
    expect(inspected.entries[1]?.available).toBe(false);
  });

  it("classifies an unknown playlist kind as other", () => {
    const inspected = normaliseInspected({ kind: "playlist", playlistKind: "strange" });
    if (inspected.kind !== "playlist") throw new Error("type");
    expect(inspected.playlistKind).toBe("other");
  });

  it("turns anything else into an error with a usable key", () => {
    const inspected = normaliseInspected({ code: "AUTH", detail: "sign in" });
    if (inspected.kind !== "error") throw new Error("type");
    expect(inspected.code).toBe("AUTH");
    expect(inspected.messageKey).toBe("error.ytdlp.unknown");
    expect(inspected.detail).toBe("sign in");
  });
});

describe("URL-case classifiers (spec §6.4)", () => {
  it("recognises watch?v=...&list=... URLs", () => {
    expect(isVideoWithListUrl("https://www.youtube.com/watch?v=abc&list=PL123")).toBe(true);
    expect(isVideoWithListUrl("https://youtu.be/abc?list=PL123")).toBe(true);
    expect(isVideoWithListUrl("https://www.youtube.com/playlist?list=PL123")).toBe(false);
    expect(isVideoWithListUrl("https://www.youtube.com/watch?v=abc")).toBe(false);
  });

  it("recognises YouTube Mix/Radio URLs", () => {
    expect(isMixUrl("https://www.youtube.com/watch?v=abc&list=RDabc")).toBe(true);
    expect(isMixUrl("https://www.youtube.com/playlist?list=RDMM")).toBe(true);
    expect(isMixUrl("https://www.youtube.com/playlist?list=PL123")).toBe(false);
  });

  it("recognises YouTube search, feed and home pages", () => {
    expect(isSearchOrFeedUrl("https://www.youtube.com/results?search_query=rust")).toBe(true);
    expect(isSearchOrFeedUrl("https://www.youtube.com/feed/subscriptions")).toBe(true);
    expect(isSearchOrFeedUrl("https://www.youtube.com/")).toBe(true);
    expect(isSearchOrFeedUrl("https://www.youtube.com/@SomeChannel/videos")).toBe(false);
    expect(isSearchOrFeedUrl("https://vimeo.com/")).toBe(false);
  });
});

describe("clocks and speeds (spec §8.3)", () => {
  it("formats durations as m:ss and h:mm:ss", () => {
    expect(secondsToClock(0)).toBe("0:00");
    expect(secondsToClock(59)).toBe("0:59");
    expect(secondsToClock(213)).toBe("3:33");
    expect(secondsToClock(3723)).toBe("1:02:03");
    expect(secondsToClock(null)).toBeNull();
    expect(secondsToClock(-1)).toBeNull();
    expect(secondsToClock(Number.NaN)).toBeNull();
  });
});

describe("inspectDelay() (spec §6.2, D76)", () => {
  it("keeps 500 ms while typing and starts at once after a paste", () => {
    expect(inspectDelay(false)).toBe(INSPECT_DEBOUNCE_MS);
    expect(inspectDelay(true)).toBe(0);
  });
});

describe("the URL metadata cache (spec §2.9)", () => {
  it("remembers a video and forgets an error", () => {
    const cache = new MetadataCache();
    const inspected: Inspected = video();
    cache.set("https://a", inspected);
    expect(cache.get("https://a")).toBe(inspected);
    cache.set("https://b", {
      kind: "error",
      code: "AUTH",
      messageKey: "error.ytdlp.auth",
      detail: null,
    });
    expect(cache.get("https://b")).toBeUndefined();
  });

  it("keeps at most the configured number of URLs, oldest first", () => {
    const cache = new MetadataCache();
    for (let index = 0; index < CACHE_LIMIT + 10; index += 1) {
      cache.set(`https://u${index}`, video({ id: String(index) }));
    }
    expect(cache.size).toBe(CACHE_LIMIT);
    expect(cache.get("https://u0")).toBeUndefined();
    expect(cache.get(`https://u${CACHE_LIMIT + 9}`)).toBeDefined();
  });

  it("clear() empties it, which is what Clear does", () => {
    const cache = new MetadataCache();
    cache.set("https://a", video());
    cache.clear();
    expect(cache.size).toBe(0);
    expect(cache.get("https://a")).toBeUndefined();
  });

  it("never caches a degraded list: the next paste must ask again", () => {
    // Otherwise a degraded answer would stick for the whole session and
    // re-pasting the link would show the same short list.
    const cache = new MetadataCache();
    cache.set("https://a", video({ degraded: true }));
    expect(cache.get("https://a")).toBeUndefined();
    expect(cache.size).toBe(0);

    cache.set("https://a", video({ degraded: false }));
    expect(cache.get("https://a")).toBeDefined();
  });
});
