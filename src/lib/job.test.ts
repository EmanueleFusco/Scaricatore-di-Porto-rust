import { describe, expect, it } from "vitest";
import {
  fileName,
  footerStateKey,
  heightWarning,
  labelKey,
  normaliseDone,
  normaliseProgress,
  offersDenoInstall,
  progressLabel,
  stageKey,
  type JobProgressEvent,
} from "$lib/job";

function progress(partial: Partial<JobProgressEvent> = {}): JobProgressEvent {
  return {
    jobId: "job-1",
    stage: "downloading",
    percent: 42,
    speedBps: 131072,
    etaSec: 10,
    label: "video",
    subIndex: 1,
    subTotal: 2,
    ...partial,
  };
}

describe("normaliseProgress", () => {
  it("keeps a complete payload", () => {
    expect(normaliseProgress(progress())).toEqual(progress());
  });

  it("degrades an unknown stage and non-numeric fields", () => {
    const event = normaliseProgress({ stage: "exploding", percent: "42" });
    expect(event.stage).toBe("preparing");
    expect(event.percent).toBeNull();
    expect(event.speedBps).toBeNull();
    expect(event.subIndex).toBeNull();
  });
});

describe("normaliseDone", () => {
  it("keeps a successful completion", () => {
    const done = normaliseDone({
      jobId: "job-1",
      status: "done",
      file: "/out/a.mkv",
      runtime: "quickjs",
      elapsedMs: 1234,
      error: null,
    });
    expect(done.status).toBe("done");
    expect(done.file).toBe("/out/a.mkv");
    expect(done.runtime).toBe("quickjs");
  });

  it("keeps the error of a failure", () => {
    const done = normaliseDone({
      jobId: "job-2",
      status: "failed",
      error: { code: "DENO_MISSING", messageKey: "error.ytdlp.denoMissing", detail: "d" },
    });
    expect(done.status).toBe("failed");
    expect(done.error?.code).toBe("DENO_MISSING");
    expect(offersDenoInstall(done, false)).toBe(true);
    expect(offersDenoInstall(done, true)).toBe(false);
  });

  it("an unknown status is a failure, not a silent success", () => {
    expect(normaliseDone({ status: "weird" }).status).toBe("failed");
    expect(offersDenoInstall(normaliseDone({ status: "weird" }), false)).toBe(false);
  });
});

describe("labels (spec §8.3, §11.9)", () => {
  it("maps stages and labels to i18n keys", () => {
    expect(stageKey("postProcessing")).toBe("job.stage.postProcessing");
    expect(labelKey("video")).toBe("job.label.video");
    expect(labelKey("audio")).toBe("job.label.audio");
    expect(labelKey("merging")).toBe("job.label.merging");
    expect(labelKey("extractingAudio")).toBe("job.label.extractingAudio");
    expect(labelKey("something-new")).toBe("job.label.postProcessing");
  });

  it("renders 'Video 1/2' and 'Audio 2/2' in both languages", () => {
    expect(progressLabel("en", progress({ label: "video", subIndex: 1, subTotal: 2 }))).toBe(
      "Video 1/2",
    );
    expect(progressLabel("en", progress({ label: "audio", subIndex: 2, subTotal: 2 }))).toBe(
      "Audio 2/2",
    );
    expect(progressLabel("it", progress({ label: "audio", subIndex: 2, subTotal: 2 }))).toBe(
      "Audio 2/2",
    );
  });

  it("renders the merge line without stream placeholders", () => {
    const merging = progress({ label: "merging", subIndex: null, subTotal: null });
    expect(progressLabel("en", merging)).toBe("Merging…");
    expect(progressLabel("it", merging)).toBe("Unione in corso…");
  });
});

describe("the status footer helpers (spec §11.2)", () => {
  it("names the stage while running and the outcome afterwards", () => {
    expect(footerStateKey("running", "downloading")).toBe("job.stage.downloading");
    expect(footerStateKey("running", "postProcessing")).toBe("job.stage.postProcessing");
    expect(footerStateKey("running", null)).toBe("job.stage.preparing");
    expect(footerStateKey("done", null)).toBe("job.stage.done");
    expect(footerStateKey("failed", null)).toBe("job.stage.failed");
    expect(footerStateKey("cancelled", null)).toBe("job.stage.cancelled");
    expect(footerStateKey("idle", null)).toBe("footer.idle");
  });

  it("takes the last segment of a path on either separator", () => {
    expect(fileName("/out/Title [id].mkv")).toBe("Title [id].mkv");
    expect(fileName("C:\\Users\\u\\Videos\\Title [id].mkv")).toBe("Title [id].mkv");
    expect(fileName("  /out/Title.mkv  ")).toBe("Title.mkv");
    expect(fileName("Title.mkv")).toBe("Title.mkv");
    expect(fileName("")).toBe("");
  });
});

describe("the degraded-resolution warning", () => {
  function done(partial: Partial<Parameters<typeof normaliseDone>[0]> = {}) {
    return normaliseDone({
      jobId: "job-1",
      status: "done",
      file: "/out/a.mkv",
      requestedHeight: 1080,
      actualHeight: 1080,
      ...partial,
    });
  }

  it("stays quiet when the height is the one that was asked for", () => {
    expect(heightWarning(done())).toBeNull();
  });

  it("flags a lower result, which is what a degraded run looks like", () => {
    expect(heightWarning(done({ actualHeight: 360 }))).toEqual({
      requested: 1080,
      actual: 360,
    });
  });

  it("says nothing when a side is unknown or the job did not succeed", () => {
    expect(heightWarning(done({ actualHeight: null }))).toBeNull();
    expect(heightWarning(done({ requestedHeight: null }))).toBeNull();
    expect(heightWarning(done({ status: "failed" }))).toBeNull();
    expect(heightWarning(null)).toBeNull();
  });
});
