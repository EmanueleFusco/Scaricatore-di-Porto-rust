import { describe, it, expect } from "vitest";
import {
  formatBytes,
  formatWer,
  isLargeDownload,
  filterModels,
  ONE_GIB,
  type ModelViewItem,
} from "$lib/models";

const MOCK_MODELS: ModelViewItem[] = [
  {
    id: "tiny-q5_1",
    label: "Tiny",
    file: "ggml-tiny-q5_1.bin",
    sizeBytes: 32560352,
    ramEstimate: "~390 MB",
    tier: "q5_1",
    indicativeWer: null,
    multilingual: true,
    recommended: false,
    advanced: false,
    notes: "drafts only",
    state: { kind: "notInstalled" },
  },
  {
    id: "large-v3-turbo-q5_0",
    label: "Turbo",
    file: "ggml-large-v3-turbo-q5_0.bin",
    sizeBytes: 574041195,
    ramEstimate: "~1 GB",
    tier: "q5_0",
    indicativeWer: 2.8,
    multilingual: true,
    recommended: true,
    advanced: false,
    notes: "best trade-off",
    state: { kind: "installed", sizeBytes: 574041195 },
  },
  {
    id: "large-v3",
    label: "Large v3 (FP16)",
    file: "ggml-large-v3.bin",
    sizeBytes: 3095033488,
    ramEstimate: "~4.5 GB",
    tier: "fp16",
    indicativeWer: 2.5,
    multilingual: true,
    recommended: false,
    advanced: true,
    notes: "full precision",
    state: { kind: "notInstalled" },
  },
];

describe("models helpers", () => {
  it("formatBytes formats B, MB, GB properly", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1 KB");
    expect(formatBytes(547 * 1024 * 1024)).toBe("547 MB");
    expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.5 GB");
  });

  it("formatWer formats percentages and handles null", () => {
    expect(formatWer(null)).toBe("—");
    expect(formatWer(2.8)).toBe("~2.8%");
    expect(formatWer(8.0)).toBe("~8.0%");
  });

  it("isLargeDownload flags models larger than 1 GiB", () => {
    expect(isLargeDownload(500 * 1024 * 1024)).toBe(false);
    expect(isLargeDownload(ONE_GIB)).toBe(false);
    expect(isLargeDownload(ONE_GIB + 1)).toBe(true);
    expect(isLargeDownload(1.5 * ONE_GIB)).toBe(true);
  });

  it("filterModels respects showFullPrecision toggle", () => {
    const regular = filterModels(MOCK_MODELS, false);
    expect(regular.map((m) => m.id)).toEqual(["tiny-q5_1", "large-v3-turbo-q5_0"]);

    const all = filterModels(MOCK_MODELS, true);
    expect(all.map((m) => m.id)).toEqual(["tiny-q5_1", "large-v3-turbo-q5_0", "large-v3"]);
  });

  it("supports paused ModelInstallState variant", () => {
    const pausedModel: ModelViewItem = {
      ...MOCK_MODELS[0],
      state: { kind: "paused", receivedBytes: 1024, totalBytes: 32560352 },
    };
    expect(pausedModel.state.kind).toBe("paused");
    if (pausedModel.state.kind === "paused") {
      expect(pausedModel.state.receivedBytes).toBe(1024);
      expect(pausedModel.state.totalBytes).toBe(32560352);
    }
  });
});
