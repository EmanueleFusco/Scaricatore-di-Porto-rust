/**
 * Tests for the queue's pure helpers (`$lib/queue`, `$lib/stores/addbar`).
 *
 * Spec §15 asks for unit tests of the state machine, the retry policy, the
 * serialisation and the virtualisation arithmetic; the ones that live in Rust
 * are there, and these are the frontend half: what the payloads become, what
 * each state offers, and which rows a long list renders.
 */
import { describe, expect, it } from "vitest";
import {
  actionsFor,
  applySnapshot,
  applyUpdate,
  barPercentFor,
  detailsText,
  emptyQueueState,
  fileName,
  filterQueueItems,
  groupPercent,
  groupStats,
  itemsOutsideCollapsed,
  nonEmptyGroups,
  wasSkippedByArchive,
  formatBytes,
  heightWarning,
  isActive,
  isParked,
  isTerminal,
  itemLabel,
  normaliseAddResult,
  normaliseItem,
  normaliseSnapshot,
  normaliseState,
  normaliseSummary,
  normaliseUpdate,
  progressLabelKey,
  progressLine,
  qualityKey,
  qualityLabel,
  secondsToClock,
  scrollOffsetFor,
  selectRange,
  speedMbits,
  toggleSelected,
  stageDetail,
  stateKey,
  statusKindFor,
  visibleItems,
  visibleRange,
  type ItemState,
  type QueueItem,
  type QueueState,
  type QueueSummary,
  type QueueUpdate,
} from "$lib/queue";
import {
  choiceOf,
  explicitChoiceFor,
  newLinks,
  parseHeaderLines,
  tableFormats,
  qualityValue,
  rowQualityOptions,
  splitLinks,
} from "$lib/addbar";

/** A backend item with sensible defaults, for the tests to bend. */
function item(overrides: Partial<QueueItem> = {}): QueueItem {
  return {
    id: "q1",
    url: "https://www.youtube.com/watch?v=abc",
    title: "A video",
    site: "Youtube",
    durationSec: 212,
    state: "queued",
    options: {
      kind: "video",
      quality: { kind: "best" },
      outputDir: "/home/user/Videos",
      cookies: { kind: "none" },
      runtimeMode: "auto",
      filenamePrefix: null,
      includeIdInFilename: true,
      preferH264: false,
      archiveFile: null,
      sleepInterval: null,
      liveFromStart: null,
      proxy: null,
      limitRate: null,
      referer: null,
      userAgent: null,
      customHeaders: [],
      impersonateRetry: true,
      encode: false,
      videoCodec: null,
      audioCodec: null,
      trimStart: null,
      trimEnd: null,
      keepOriginal: false,
      autoPlay: false,
      embedMetadata: false,
      embedThumbnail: false,
      embedChapters: false,
      sponsorblockMode: null,
      sponsorblockCategories: [],
      transcribe: false,
      whisperModel: null,
      transcriptLanguage: null,
      translateToEnglish: null,
      transcriptsSubfolder: null,
      transcriptionEngine: null,
      geminiModel: null,
      summarize: null,
      summaryLanguage: null,
    },
    groupId: null,
    progress: { percent: null, speedBps: null, etaSec: null, label: "", subIndex: null, subTotal: null },
    file: null,
    sizeBytes: null,
    runtime: null,
    requestedHeight: null,
    actualHeight: null,
    error: null,
    logTail: [],
    autoRetries: 0,
    retryAt: null,
    pausedFrom: null,
    addedAt: 0,
    updatedAt: 0,
    startedAt: null,
    finishedAt: null,
    transcriptFiles: [],
    transcriptPreview: null,
    transcriptOrigin: null,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

describe("item states", () => {
  it("separates running, terminal and parked, like the Rust state machine", () => {
    const active: ItemState[] = [
      "inspecting",
      "downloading",
      "postProcessing",
      "encoding",
      "transcribing",
      "summarizing",
    ];
    for (const state of active) expect(isActive(state)).toBe(true);
    for (const state of active) expect(isTerminal(state)).toBe(false);

    for (const state of ["done", "failed", "cancelled"] as ItemState[]) {
      expect(isTerminal(state)).toBe(true);
      expect(isParked(state)).toBe(true);
    }
    for (const state of ["paused", "interrupted"] as ItemState[]) {
      expect(isTerminal(state)).toBe(false);
      expect(isParked(state)).toBe(true);
    }
    for (const state of ["queued"] as ItemState[]) {
      expect(isActive(state)).toBe(false);
      expect(isParked(state)).toBe(false);
    }
  });

  it("names an unknown state interrupted rather than trusting it", () => {
    expect(normaliseState("downloading")).toBe("downloading");
    expect(normaliseState("writtenByANewerBuild")).toBe("interrupted");
    expect(normaliseState(undefined)).toBe("interrupted");
  });

  it("has one i18n key per state", () => {
    expect(stateKey("postProcessing")).toBe("queue.state.postProcessing");
    expect(stateKey("interrupted")).toBe("queue.state.interrupted");
  });
});

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

describe("the actions a state offers", () => {
  it("offers pause and cancel while something runs", () => {
    const actions = actionsFor("downloading", false);
    expect(actions).toContain("pause");
    expect(actions).toContain("cancel");
    expect(actions).not.toContain("retry");
  });

  it("offers resume for paused and interrupted, retry only for failures", () => {
    expect(actionsFor("paused", false)).toContain("resume");
    expect(actionsFor("interrupted", false)).toContain("resume");
    expect(actionsFor("failed", false)).toContain("retry");
    expect(actionsFor("cancelled", false)).toContain("retry");
    // A done item has nothing to retry.
    expect(actionsFor("done", false)).not.toContain("retry");
    expect(actionsFor("done", false)).not.toContain("resume");
  });

  it("adds open and reveal only when there is a file", () => {
    expect(actionsFor("done", false)).not.toContain("reveal");
    expect(actionsFor("done", true)).toContain("open");
    expect(actionsFor("done", true)).toContain("reveal");
  });

  it("always offers copy link and open page for remote items", () => {
    for (const state of ["queued", "downloading", "done", "failed"] as ItemState[]) {
      const actions = actionsFor(state, false);
      expect(actions).toContain("copyLink");
      expect(actions).toContain("openPage");
    }
  });

  it("omits copy link and open page for local items", () => {
    for (const state of ["queued", "transcribing", "done", "failed"] as ItemState[]) {
      const actions = actionsFor(state, false, false);
      expect(actions).not.toContain("copyLink");
      expect(actions).not.toContain("openPage");
    }
  });
});

// ---------------------------------------------------------------------------
// Normalisers
// ---------------------------------------------------------------------------

describe("normalising the backend payloads", () => {
  it("fills an item with safe defaults when the backend sends almost nothing", () => {
    const parsed = normaliseItem({});
    expect(parsed.id).toBe("");
    expect(parsed.state).toBe("interrupted");
    expect(parsed.options.quality).toEqual({ kind: "best" });
    expect(parsed.options.kind).toBe("video");
    expect(parsed.error).toBeNull();
    expect(parsed.logTail).toEqual([]);
  });

  it("keeps what a real item carries", () => {
    const parsed = normaliseItem({
      id: "q7",
      url: "https://a.example/1",
      title: "T",
      site: "Vimeo",
      durationSec: 12,
      state: "downloading",
      options: {
        kind: "audio",
        quality: { kind: "maxHeight", height: 720 },
        outputDir: "/out",
        cookies: { kind: "browser", browser: "firefox" },
        runtimeMode: "deno",
      },
      progress: { percent: 42, speedBps: 3e6, etaSec: 9, label: "video", subIndex: 1, subTotal: 2 },
      file: "/out/a.mkv",
      sizeBytes: 1234567,
      runtime: "deno",
      error: { code: "NETWORK", messageKey: "error.ytdlp.network", detail: "boom" },
      logTail: ["line one", 42],
    });
    expect(parsed.state).toBe("downloading");
    expect(parsed.options.kind).toBe("audio");
    expect(parsed.options.quality).toEqual({ kind: "maxHeight", height: 720 });
    expect(parsed.options.cookies).toEqual({ kind: "browser", browser: "firefox" });
    expect(parsed.progress.percent).toBe(42);
    expect(parsed.sizeBytes).toBe(1234567);
    expect(parsed.error?.code).toBe("NETWORK");
    // A non-string line in the tail is dropped rather than rendered.
    expect(parsed.logTail).toEqual(["line one"]);
  });

  it("preserves typed WHISPER_MISSING and WHISPER_MODEL_MISSING errors", () => {
    const whisperMissingItem = normaliseItem({
      id: "q_whisper",
      state: "failed",
      url: "https://a.example/1",
      error: { code: "WHISPER_MISSING", messageKey: "error.ytdlp.whisperMissing", detail: "sdp-whisper not found" },
    });
    expect(whisperMissingItem.error?.code).toBe("WHISPER_MISSING");
    expect(whisperMissingItem.error?.messageKey).toBe("error.ytdlp.whisperMissing");

    const modelMissingItem = normaliseItem({
      id: "q_model",
      state: "failed",
      url: "https://a.example/2",
      error: { code: "WHISPER_MODEL_MISSING", messageKey: "error.ytdlp.whisperModelMissing", detail: "small-q5_1" },
    });
    expect(modelMissingItem.error?.code).toBe("WHISPER_MODEL_MISSING");
    expect(modelMissingItem.error?.messageKey).toBe("error.ytdlp.whisperModelMissing");
    expect(modelMissingItem.error?.detail).toBe("small-q5_1");
  });

  it("preserves M6 transcription options and transcript origin", () => {
    const parsed = normaliseItem({
      id: "q_m6",
      options: {
        transcriptionEngine: "subtitles_whisper_gemini",
        geminiModel: "gemini-flash-latest",
        summarize: true,
        summaryLanguage: "it",
      },
      transcriptFiles: ["/tmp/a.txt", "/tmp/a.summary.md"],
      transcriptOrigin: "subtitles_manual",
    });
    expect(parsed.options.transcriptionEngine).toBe("subtitles_whisper_gemini");
    expect(parsed.options.geminiModel).toBe("gemini-flash-latest");
    expect(parsed.options.summarize).toBe(true);
    expect(parsed.options.summaryLanguage).toBe("it");
    expect(parsed.transcriptFiles).toEqual(["/tmp/a.txt", "/tmp/a.summary.md"]);
    expect(parsed.transcriptOrigin).toBe("subtitles_manual");
  });

  it("preserves typed GEMINI_KEY_MISSING and NO_SUBTITLES error codes", () => {
    const keyMissing = normaliseItem({
      id: "q_gem",
      state: "failed",
      error: { code: "GEMINI_KEY_MISSING", messageKey: "error.ytdlp.geminiKeyMissing" },
    });
    expect(keyMissing.error?.code).toBe("GEMINI_KEY_MISSING");

    const noSubs = normaliseItem({
      id: "q_subs",
      state: "failed",
      error: { code: "NO_SUBTITLES", messageKey: "error.ytdlp.noSubtitles" },
    });
    expect(noSubs.error?.code).toBe("NO_SUBTITLES");
  });

  it("gives the summary the documented zeros rather than undefined", () => {
    const summary = normaliseSummary({ running: 2 });
    expect(summary.running).toBe(2);
    expect(summary.total).toBe(0);
    expect(summary.speedBps).toBeNull();
    expect(summary.parallelDownloads).toBe(2);
  });

  it("reads an update with items, removals, order and a toast", () => {
    const update = normaliseUpdate({
      seq: 12,
      items: [{ id: "q1", state: "queued", url: "https://a.example/1" }],
      removed: ["q9"],
      order: ["q1"],
      summary: { total: 1 },
      toast: { messageKey: "toast.queueRetrying", params: { count: "1" } },
    });
    expect(update.seq).toBe(12);
    expect(update.items).toHaveLength(1);
    expect(update.removed).toEqual(["q9"]);
    expect(update.order).toEqual(["q1"]);
    expect(update.toast?.messageKey).toBe("toast.queueRetrying");
    expect(update.toast?.params.count).toBe("1");
  });

  it("treats a missing order as 'unchanged' and a missing toast as none", () => {
    const update = normaliseUpdate({ seq: 1, items: [], removed: [] });
    expect(update.order).toBeNull();
    expect(update.toast).toBeNull();
  });

  it("reads the initial snapshot with its groups", () => {
    const snapshot = normaliseSnapshot({
      seq: 3,
      items: [{ id: "q1", state: "done" }],
      groups: [{ id: "g1", title: "A playlist", folder: "/out/A playlist", index: 0 }],
      summary: { total: 1, done: 1 },
    });
    expect(snapshot.items).toHaveLength(1);
    expect(snapshot.groups[0].title).toBe("A playlist");
    expect(snapshot.summary.done).toBe(1);
  });

  it("reads the add result with its duplicates", () => {
    const result = normaliseAddResult({
      added: ["q1"],
      duplicates: [{ url: "https://a.example/1", itemId: "q0", title: "Already" }],
    });
    expect(result.added).toEqual(["q1"]);
    expect(result.duplicates[0].itemId).toBe("q0");
    expect(normaliseAddResult(null).added).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

describe("the text the cards show", () => {
  it("converts speed to Mbit/s and refuses the impossible", () => {
    expect(speedMbits(1024 * 1024 / 8)).toBe(1);
    expect(speedMbits(5e6)).toBe(38);
    // One decimal below 1 Mbit/s: a slow download must not read as `0`.
    expect(speedMbits(50_000)).toBe(0.4);
    expect(speedMbits(0)).toBeNull();
    expect(speedMbits(null)).toBeNull();
  });

  it("writes a file size in the unit a person reads", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 kB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5 MB");
    expect(formatBytes(1536 * 1024 * 1024)).toBe("1.5 GB");
    // Nothing produced, nothing shown: the card asks for `null`, not zero.
    expect(formatBytes(null)).toBeNull();
    expect(formatBytes(-1)).toBeNull();
  });

  it("formats a duration and refuses the empty ones", () => {
    expect(secondsToClock(9)).toBe("0:09");
    expect(secondsToClock(212)).toBe("3:32");
    expect(secondsToClock(3725)).toBe("1:02:05");
    expect(secondsToClock(0)).toBeNull();
    expect(secondsToClock(null)).toBeNull();
  });

  it("names an item by its title, then its file, then the url", () => {
    expect(itemLabel(item())).toBe("A video");
    expect(itemLabel(item({ title: null }))).toBe("watch?v=abc");
    expect(itemLabel(item({ title: "  " }))).toBe("watch?v=abc");
  });

  it("cuts a path on either separator", () => {
    expect(fileName("/home/user/Videos/a.mkv")).toBe("a.mkv");
    expect(fileName("C:\\Users\\me\\Videos\\a.mkv")).toBe("a.mkv");
    expect(fileName("a.mkv")).toBe("a.mkv");
  });

  it("names the chosen quality", () => {
    expect(qualityKey(item())).toEqual({ key: "queue.item.quality.best" });
    expect(
      qualityKey(
        item({
          options: { ...item().options, quality: { kind: "exact", height: 1080 } },
        }),
      ),
    ).toEqual({ key: "queue.item.quality.exact", params: { height: 1080 } });
    expect(
      qualityKey(
        item({
          options: { ...item().options, quality: { kind: "maxHeight", height: 720 } },
        }),
      ),
    ).toEqual({ key: "queue.item.quality.max", params: { height: 720 } });
  });

  it("warns only when a finished item is lower than asked", () => {
    expect(heightWarning(item({ state: "done" }))).toBeNull();
    expect(
      heightWarning(
        item({ state: "done", requestedHeight: 1440, actualHeight: 360 }),
      ),
    ).toEqual({ requested: 1440, actual: 360 });
    expect(
      heightWarning(item({ state: "done", requestedHeight: 720, actualHeight: 1080 })),
    ).toBeNull();
    expect(
      heightWarning(item({ state: "downloading", requestedHeight: 1080, actualHeight: 360 })),
    ).toBeNull();
  });

  it("gives every state a status line: the problem, the progress, or the quality", () => {
    // The card must never lose its status line: that is the height it keeps.
    for (const state of Object.keys({
      queued: 1,
      inspecting: 1,
      downloading: 1,
      postProcessing: 1,
      encoding: 1,
      transcribing: 1,
      summarizing: 1,
      done: 1,
      failed: 1,
      cancelled: 1,
      paused: 1,
      interrupted: 1,
    }) as ItemState[]) {
      expect(["error", "warning", "progress", "quality"]).toContain(
        statusKindFor(item({ state })),
      );
    }
    expect(statusKindFor(item({ state: "downloading" }))).toBe("progress");
    expect(statusKindFor(item({ state: "queued" }))).toBe("quality");
    expect(
      statusKindFor(
        item({
          state: "failed",
          error: { code: "NETWORK", messageKey: "error.ytdlp.network", detail: "raw" },
        }),
      ),
    ).toBe("error");
    expect(
      statusKindFor(item({ state: "done", requestedHeight: 1440, actualHeight: 360 })),
    ).toBe("warning");
  });

  it("always has a progress width, even when yt-dlp reported none", () => {
    expect(barPercentFor(item({ progress: { ...item().progress, percent: 42 } }))).toBe(42);
    expect(barPercentFor(item({ state: "queued" }))).toBe(0);
    expect(barPercentFor(item({ state: "done" }))).toBe(100);
    // A failed item keeps the last width it had, so the bar never jumps.
    expect(
      barPercentFor(
        item({ state: "failed", progress: { ...item().progress, percent: 37 } }),
      ),
    ).toBe(37);
  });

  it("names the stream of a running item, index and total", () => {
    const running = item({
      state: "downloading",
      progress: {
        percent: 42,
        speedBps: 1_000_000,
        etaSec: 30,
        label: "video",
        subIndex: 1,
        subTotal: 2,
      },
    });
    expect(stageDetail(running)).toEqual({
      key: "job.label.video",
      params: { index: 1, total: 2 },
    });
    expect(
      stageDetail(
        item({
          state: "downloading",
          progress: { ...item().progress, label: "audio", subIndex: 2, subTotal: 2 },
        }),
      ),
    ).toEqual({ key: "job.label.audio", params: { index: 2, total: 2 } });
  });

  it("says nothing before the tool reports a stage", () => {
    expect(stageDetail(item({ state: "queued" }))).toBeNull();
    expect(stageDetail(item({ progress: { ...item().progress, label: "" } }))).toBeNull();
    // A bare token with no index is a whole step, not "Video 1/1".
    expect(
      stageDetail(
        item({ state: "postProcessing", progress: { ...item().progress, label: "merging" } }),
      ),
    ).toEqual({ key: "job.label.merging" });
  });

  it("sends an unknown token to the generic step", () => {
    expect(progressLabelKey("somethingNew")).toBe("job.label.postProcessing");
    expect(progressLabelKey("verifying")).toBe("job.label.preparing");
    expect(progressLabelKey("extractingAudio")).toBe("job.label.extractingAudio");
    expect(progressLabelKey("transcribing")).toBe("job.label.transcribing");
    expect(progressLabelKey("transcribingWhisper")).toBe("job.label.transcribingWhisper");
    expect(progressLabelKey("transcribingGemini")).toBe("job.label.transcribingGemini");
    expect(progressLabelKey("transcribingSubtitles")).toBe("job.label.transcribingSubtitles");
  });

  it("builds the copy-details text out of the message, the detail and the log", () => {
    const text = detailsText(
      item({
        error: { code: "NETWORK", messageKey: "error.ytdlp.network", detail: "raw detail" },
        logTail: ["last line"],
      }),
      "Network problem",
    );
    expect(text.split("\n")).toEqual(["Network problem", "raw detail", "last line"]);
    expect(detailsText(item(), null)).toBe("");
  });

  it("folds transcript preview into progress line while transcribing", () => {
    // While transcribing, if transcriptPreview is present, it folds into the single status line
    expect(
      progressLine("Transcribing…", null, null, {
        isTranscribing: true,
        transcriptPreview: "Hello world live segment",
      }),
    ).toBe("Transcribing… · Hello world live segment");

    // When transcribing but no stage text, shows preview
    expect(
      progressLine(null, null, null, {
        isTranscribing: true,
        transcriptPreview: "Hello world live segment",
      }),
    ).toBe("Hello world live segment");

    // When transcribing but no preview yet, shows stage text
    expect(
      progressLine("Transcribing…", null, null, {
        isTranscribing: true,
        transcriptPreview: null,
      }),
    ).toBe("Transcribing…");

    // When not transcribing, does not fold transcript preview
    expect(
      progressLine("Downloading…", "5.0 MB/s", "00:10", {
        isTranscribing: false,
        transcriptPreview: "stale preview",
      }),
    ).toBe("Downloading… · 5.0 MB/s · 00:10");

    // When empty, falls back to waitingDataText or empty string
    expect(progressLine(null, null, null, { waitingDataText: "Waiting for data…" })).toBe(
      "Waiting for data…",
    );
    expect(progressLine(null, null, null)).toBe("");
  });
});

// ---------------------------------------------------------------------------
// Virtualisation (spec §11.7)
// ---------------------------------------------------------------------------

describe("the virtualised list", () => {
  const ROW = 40;
  const HEIGHT = 400;

  it("renders the first screen plus the overscan", () => {
    const range = visibleRange(0, HEIGHT, ROW, 1000);
    expect(range.first).toBe(0);
    // 400 / 40 = 10 visible rows, +1 for the partial one, +2 overscan.
    expect(range.last).toBe(12);
    expect(range.topPad).toBe(0);
  });

  it("scrolls without losing a row at the edges", () => {
    // scrollTop 400 = row 10; two rows of overscan above, and the viewport shows
    // rows 10..20, two more below.
    const range = visibleRange(400, HEIGHT, ROW, 1000);
    expect(range.first).toBe(8);
    expect(range.last).toBe(22);
    // The pads are what keeps the scrollbar honest.
    expect(range.topPad).toBe(8 * ROW);
    expect(range.bottomPad).toBe((1000 - 1 - 22) * ROW);
    expect(visibleItems(Array.from({ length: 1000 }), range)).toHaveLength(15);
  });

  it("never runs past the end of a short list", () => {
    const range = visibleRange(0, HEIGHT, ROW, 3);
    expect(range.last).toBe(2);
    expect(range.bottomPad).toBe(0);
  });

  it("renders nothing for an empty list and survives a zero height", () => {
    const empty = visibleRange(0, HEIGHT, ROW, 0);
    expect(empty.last).toBe(-1);
    expect(visibleItems([1, 2, 3], empty)).toEqual([]);
    expect(visibleRange(0, HEIGHT, 0, 10).last).toBe(-1);
  });

  it("keeps a 5 000-item list down to a dozen nodes", () => {
    const items = Array.from({ length: 5000 }, (_, index) => index);
    const range = visibleRange(100_000, HEIGHT, ROW, items.length);
    const shown = visibleItems(items, range);
    expect(shown.length).toBeLessThan(20);
    expect(shown[0]).toBe(range.first);
    expect(shown[shown.length - 1]).toBe(range.last);
  });

  it("does not go below the start when scrolled to the top", () => {
    const range = visibleRange(-50, HEIGHT, ROW, 10);
    expect(range.first).toBe(0);
  });

  it("scrolls to a new row only when it is out of sight, and to the nearest edge", () => {
    // Already fully visible: the list must not move under the user's eyes.
    expect(scrollOffsetFor(2, 100, 150, 400)).toBeNull();
    // Below the viewport: the row's bottom lands on the viewport's bottom.
    expect(scrollOffsetFor(6, 100, 0, 400)).toBe(300);
    // Above it: the row's top goes to the top.
    expect(scrollOffsetFor(1, 100, 500, 400)).toBe(100);
    // The first row can never ask for a negative offset.
    expect(scrollOffsetFor(0, 100, 500, 400)).toBe(0);
    // An item that rolled off the history cap is nowhere, not row zero.
    expect(scrollOffsetFor(-1, 100, 0, 400)).toBeNull();
    expect(scrollOffsetFor(3, 0, 0, 400)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// The Add bar
// ---------------------------------------------------------------------------

describe("filtering and selecting rows", () => {
  const rows = [
    item({ id: "a", title: "Live concert", site: "Youtube" }),
    item({ id: "b", title: "Cooking", site: "Vimeo", url: "https://vimeo.com/1" }),
    item({ id: "c", title: "Podcast", site: "Soundcloud", url: "https://soundcloud.com/1" }),
  ];

  it("matches title, site and link, case-insensitively", () => {
    expect(filterQueueItems(rows, "LIVE").map((row) => row.id)).toEqual(["a"]);
    expect(filterQueueItems(rows, "vimeo").map((row) => row.id)).toEqual(["b"]);
    expect(filterQueueItems(rows, "soundcloud.com").map((row) => row.id)).toEqual(["c"]);
  });

  it("returns the whole list for an empty or whitespace query", () => {
    expect(filterQueueItems(rows, "")).toEqual(rows);
    expect(filterQueueItems(rows, "   ")).toEqual(rows);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterQueueItems(rows, "no-such-row")).toEqual([]);
  });

  it("filters a 5 000-item queue without copying unrelated rows into the result", () => {
    const many = Array.from({ length: 5000 }, (_, index) =>
      item({ id: `row-${index}`, title: `Item-${index}` }),
    );
    const filtered = filterQueueItems(many, "Item-4999");
    expect(filtered.map((row) => row.id)).toEqual(["row-4999"]);
    expect(filterQueueItems(many, "").length).toBe(5000);
  });

  it("toggles one id without touching the others", () => {
    const selected = toggleSelected(new Set(["a"]), "b");
    expect([...selected].sort()).toEqual(["a", "b"]);
    expect([...toggleSelected(selected, "a")]).toEqual(["b"]);
  });

  it("extends a selection across the visible range, in either direction", () => {
    expect([...selectRange(new Set(), ["a", "b", "c", "d"], "b", "d")]).toEqual(["b", "c", "d"]);
    expect([...selectRange(new Set(), ["a", "b", "c", "d"], "d", "b")]).toEqual(["b", "c", "d"]);
    expect([...selectRange(new Set(["a"]), ["a", "b"], "a", "b")]).toEqual(["a", "b"]);
  });

  it("ignores anchors that are no longer visible", () => {
    const selected = new Set(["a"]);
    expect(selectRange(selected, ["b", "c"], "a", "c")).toEqual(selected);
  });
});

describe("splitting a paste into links", () => {
  it("keeps http(s) links, deduped and in order", () => {
    expect(
      splitLinks(
        "https://a.example/1\nhttps://b.example/2 https://a.example/1\nftp://c.example/3\nnot a url",
      ),
    ).toEqual(["https://a.example/1", "https://b.example/2"]);
  });

  it("strips the punctuation a copy brings along", () => {
    expect(splitLinks('"https://a.example/1", https://b.example/2;')).toEqual([
      "https://a.example/1",
      "https://b.example/2",
    ]);
  });

  it("finds nothing in text that is not links", () => {
    expect(splitLinks("hello world")).toEqual([]);
    expect(splitLinks("")).toEqual([]);
  });
});

describe("the links not seen yet", () => {
  it("keeps the order and drops what already has a row", () => {
    expect(
      newLinks("https://a.example/1\nhttps://b.example/2", ["https://a.example/1"]),
    ).toEqual(["https://b.example/2"]);
  });

  it("finds nothing new in a text that is not links", () => {
    expect(newLinks("hello world", [])).toEqual([]);
  });
});

describe("the quality options of a link", () => {
  it("offers the video's own heights as exact choices, plus Best and Audio only", () => {
    const options = rowQualityOptions([1080, 720], { kind: "best" });
    expect(options.map((option) => option.value)).toEqual([
      "best",
      "exact1080",
      "exact720",
      "audio",
    ]);
  });

  it("shows the preset in force while the row has its own choice to make", () => {
    // A cap is not one of the heights: it has to be in the list, or the
    // <select> would show nothing for a row that follows the preset.
    const options = rowQualityOptions([720], { kind: "maxHeight", height: 1080 });
    expect(options.map((option) => option.value)).toEqual([
      "best",
      "max1080",
      "exact720",
      "audio",
    ]);
  });

  it("does not repeat a preset that is already one of the heights", () => {
    const options = rowQualityOptions([1080], { kind: "exact", height: 1080 });
    expect(options.map((option) => option.value)).toEqual(["best", "exact1080", "audio"]);
  });

  it("still offers Best and Audio only when there are no heights", () => {
    // Spec §6.2: a source without heights is not a failure.
    expect(rowQualityOptions([], { kind: "audioOnly" }).map((option) => option.value)).toEqual([
      "best",
      "audio",
    ]);
  });

  it("round-trips a choice through a <select> value", () => {
    const options = rowQualityOptions(
      [720],
      { kind: "explicit", formatId: "137", hasAudio: false },
    );
    for (const option of options) {
      expect(choiceOf(options, qualityValue(option.choice))).toEqual(option.choice);
    }
    expect(choiceOf(options, "nonsense")).toBeNull();
  });

  it("names every choice with an i18n key and its parameters", () => {
    expect(qualityLabel({ kind: "best" })).toEqual({ key: "queue.item.quality.best" });
    expect(qualityLabel({ kind: "audioOnly" })).toEqual({ key: "queue.item.quality.audio" });
    expect(qualityLabel({ kind: "exact", height: 720 })).toEqual({
      key: "queue.item.quality.exact",
      params: { height: 720 },
    });
    expect(qualityLabel({ kind: "maxHeight", height: 720 })).toEqual({
      key: "queue.item.quality.max",
      params: { height: 720 },
    });
  });
});

// ---------------------------------------------------------------------------
// The reducer (spec §7.6, §11.7)
// ---------------------------------------------------------------------------

/** A summary with the documented zeros, to bend. */
function summary(overrides: Partial<QueueSummary> = {}): QueueSummary {
  return {
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
    ...overrides,
  };
}

function update(overrides: Partial<QueueUpdate> = {}): QueueUpdate {
  return {
    seq: 1,
    items: [],
    removed: [],
    order: null,
    groups: null,
    summary: summary(),
    toast: null,
    ...overrides,
  };
}

describe("folding an event into what the UI shows", () => {
  it("patches one row and leaves every other row's object alone", () => {
    // The point of the keyed map: a tick with one changed item must repaint
    // one row, not the list (spec §11.7).
    const first = item({ id: "a", state: "downloading" });
    const second = item({ id: "b", state: "queued" });
    const before: QueueState = {
      ...emptyQueueState(),
      items: { a: first, b: second },
      order: ["a", "b"],
      seq: 1,
    };

    const after = applyUpdate(before, update({ seq: 2, items: [item({ id: "b", state: "done" })] }));

    expect(after.order).toEqual(["a", "b"]);
    expect(after.items.b.state).toBe("done");
    // Same object: a row that did not change cannot have re-rendered.
    expect(after.items.a).toBe(first);
    expect(after.seq).toBe(2);
  });

  it("adds an item it has never seen, and forgets one that was removed", () => {
    const before: QueueState = { ...emptyQueueState(), items: { a: item({ id: "a" }) }, order: ["a"] };
    const added = applyUpdate(before, update({ seq: 2, items: [item({ id: "c" })] }));
    expect(added.order).toEqual(["a", "c"]);

    const removed = applyUpdate(added, update({ seq: 3, removed: ["a"] }));
    expect(removed.order).toEqual(["c"]);
    expect(removed.items.a).toBeUndefined();
  });

  it("takes the order when the event carries one, and ignores unknown ids", () => {
    const before: QueueState = {
      ...emptyQueueState(),
      items: { a: item({ id: "a" }), b: item({ id: "b" }) },
      order: ["a", "b"],
    };
    const after = applyUpdate(before, update({ seq: 2, order: ["b", "ghost", "a"] }));
    expect(after.order).toEqual(["b", "a"]);
  });

  it("drops an event that is not newer than what is already shown", () => {
    // The window between the initial read and the first event can deliver them
    // out of order, and a stale tick must not walk a row backwards.
    const before: QueueState = { ...emptyQueueState(), seq: 7, items: { a: item({ id: "a", state: "done" }) } };
    const after = applyUpdate(before, update({ seq: 3, items: [item({ id: "a", state: "queued" })] }));
    expect(after).toBe(before);
    expect(after.items.a.state).toBe("done");
  });

  it("keeps the last toast when an event carries none", () => {
    const toast = { messageKey: "toast.queueRecovered", params: { count: "2" } };
    const before = applyUpdate(emptyQueueState(), update({ seq: 1, toast }));
    const after = applyUpdate(before, update({ seq: 2 }));
    expect(after.toast).toBe(toast);
  });
});

describe("groups in events (M3)", () => {
  it("replaces the groups only when the event carries them", () => {
    const group = { id: "g1", title: "PL", folder: null, index: 0 };
    const withGroups = applyUpdate(emptyQueueState(), update({ seq: 1, groups: [group] }));
    expect(withGroups.groups).toEqual([group]);
    const unchanged = applyUpdate(withGroups, update({ seq: 2 }));
    expect(unchanged.groups).toBe(withGroups.groups);
    expect(applyUpdate(unchanged, update({ seq: 3, groups: [] })).groups).toEqual([]);
  });

  it("reads M3 item options with safe defaults", () => {
    const parsed = normaliseItem({ options: { filenamePrefix: "01 - ", sleepInterval: [1, 4], liveFromStart: true } });
    expect(parsed.options.filenamePrefix).toBe("01 - ");
    expect(parsed.options.sleepInterval).toEqual([1, 4]);
    expect(parsed.options.liveFromStart).toBe(true);
    expect(parsed.options.includeIdInFilename).toBe(true);
    expect(parsed.options.customHeaders).toEqual([]);
  });
});

describe("folding the initial read in", () => {
  it("replaces everything when nothing newer has arrived", () => {
    const before = applyUpdate(emptyQueueState(), update({ seq: 4, items: [item({ id: "a" })] }));
    const after = applySnapshot(before, {
      seq: 1,
      items: [item({ id: "a" }), item({ id: "b" })],
      groups: [{ id: "g1", title: "A playlist", folder: null, index: 0 }],
      summary: summary({ total: 2, queued: 2 }),
      toast: null,
    });
    expect(after.order).toEqual(["a", "b"]);
    expect(after.summary.total).toBe(2);
    expect(after.seq).toBe(1);
    expect(after.groups).toHaveLength(1);
  });

  it("never walks a row back when events overtook the read", () => {
    // A `queue_list` that was in flight while a download started describes a
    // queue that has already moved on. Applying it wholesale would show
    // `queued` again — and if the queue then goes idle, no event may ever
    // come to correct it.
    const moved = applyUpdate(
      emptyQueueState(),
      update({ seq: 9, items: [item({ id: "a", state: "downloading" })], summary: summary({ total: 1, running: 1 }) }),
    );
    const after = applySnapshot(
      moved,
      {
        seq: 3,
        items: [item({ id: "a", state: "queued" })],
        groups: [],
        summary: summary({ total: 1, queued: 1 }),
        toast: null,
      },
      true,
    );

    expect(after.items.a.state).toBe("downloading");
    expect(after.summary.running).toBe(1);
    expect(after.seq).toBe(9);
  });

  it("still takes the rows the read has and the UI does not", () => {
    // The read is the only source for an item that no event has mentioned yet.
    const after = applySnapshot(
      emptyQueueState(),
      { seq: 2, items: [item({ id: "a" }), item({ id: "b" })], groups: [], summary: summary(), toast: null },
      true,
    );
    expect(after.order).toEqual(["a", "b"]);
  });

  it("takes the groups, which only ever travel in the read", () => {
    const after = applySnapshot(
      emptyQueueState(),
      {
        seq: 1,
        items: [],
        groups: [{ id: "g1", title: "A playlist", folder: "/home/user/A playlist", index: 0 }],
        summary: summary(),
        toast: null,
      },
      true,
    );
    expect(after.groups[0].title).toBe("A playlist");
  });
});

describe("the custom header lines (M3)", () => {
  it("keeps well-formed `Name: value` lines and drops the rest", () => {
    expect(parseHeaderLines("X-One: 1\n  X-Two:  2  \nno colon\n: empty name\nEmpty:\n")).toEqual([
      "X-One: 1",
      "X-Two:  2",
    ]);
    expect(parseHeaderLines("")).toEqual([]);
  });
});

describe("the advanced format table (M3)", () => {
  const fmt = (formatId: string, over: Record<string, unknown> = {}) => ({
    formatId,
    ext: "mp4",
    height: null,
    width: null,
    fps: null,
    vcodec: null,
    acodec: null,
    filesize: null,
    protocol: "https",
    note: null,
    ...over,
  });

  it("drops storyboards and lists video tallest first, then audio", () => {
    const rows = tableFormats([
      fmt("sb0", { ext: "mhtml", protocol: "mhtml" }),
      fmt("251", { ext: "webm", vcodec: "none", acodec: "opus", filesize: 10 }),
      fmt("136", { height: 720, vcodec: "avc1", acodec: "none" }),
      fmt("137", { height: 1080, vcodec: "avc1", acodec: "none" }),
      fmt("140", { ext: "m4a", vcodec: "none", acodec: "mp4a", filesize: 20 }),
    ]);
    expect(rows.map((row) => row.formatId)).toEqual(["137", "136", "140", "251"]);
  });

  it("tells whether a stream already has sound", () => {
    expect(explicitChoiceFor(fmt("18", { acodec: "mp4a" }))).toEqual({
      kind: "explicit",
      formatId: "18",
      hasAudio: true,
    });
    expect(explicitChoiceFor(fmt("137", { acodec: "none" }))).toMatchObject({ hasAudio: false });
  });
});

describe("playlist groups (M3)", () => {
  const inGroup = (id: string, state: ItemState, extra: Partial<QueueItem> = {}) =>
    item({ id, groupId: "g1", state, ...extra });

  it("counts a group's items by what they are doing", () => {
    const stats = groupStats("g1", [
      inGroup("a", "done", { file: "/out/PL/001 - a.mkv" }),
      inGroup("b", "downloading"),
      inGroup("c", "failed"),
      inGroup("d", "paused"),
      inGroup("e", "queued"),
      item({ id: "x", groupId: "other", state: "done" }),
    ]);
    expect(stats).toEqual({
      total: 5,
      done: 1,
      failed: 1,
      active: 2,
      paused: 1,
      firstFile: "/out/PL/001 - a.mkv",
    });
    expect(groupPercent(stats)).toBe(20);
    expect(groupPercent(groupStats("none", []))).toBe(0);
  });

  it("lists only the groups that still have items, in creation order", () => {
    const groups = [
      { id: "g2", title: "Second", folder: null, index: 1 },
      { id: "g1", title: "First", folder: null, index: 0 },
      { id: "gone", title: "Empty", folder: null, index: 2 },
    ];
    const items = [inGroup("a", "queued"), item({ id: "b", groupId: "g2" })];
    expect(nonEmptyGroups(groups, items).map((g) => g.id)).toEqual(["g1", "g2"]);
  });

  it("hides the items of a collapsed group and keeps the order of the rest", () => {
    const items = [inGroup("a", "queued"), item({ id: "b" }), item({ id: "c", groupId: "g2" })];
    expect(itemsOutsideCollapsed(items, new Set()).map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(itemsOutsideCollapsed(items, new Set(["g1"])).map((i) => i.id)).toEqual(["b", "c"]);
  });

  it("recognises an item the archive skipped", () => {
    const skipped = inGroup("a", "done", {
      progress: { ...item().progress, label: "skippedArchive", percent: 100 },
    });
    expect(wasSkippedByArchive(skipped)).toBe(true);
    expect(wasSkippedByArchive(inGroup("b", "done"))).toBe(false);
    expect(wasSkippedByArchive({ ...skipped, state: "failed" })).toBe(false);
  });
});
