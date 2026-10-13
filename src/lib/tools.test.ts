import { describe, expect, it } from "vitest";
import {
  canUpdate,
  canUseOwnPath,
  hasToolProblem,
  isToolSource,
  isWizardStep,
  normaliseToolStatus,
  normaliseToolsView,
  readyToolCount,
  toolState,
  WIZARD_STEPS,
  wizardIndex,
  wizardNext,
  wizardPrevious,
  wizardToolNames,
  type ToolStatus,
  type ToolsView,
} from "$lib/tools";
import { hasKey, keysOf } from "$lib/i18n";

function tool(overrides: Partial<ToolStatus> = {}): ToolStatus {
  return {
    name: "yt-dlp",
    role: "Downloader",
    bundled: true,
    availableOnPlatform: true,
    installable: false,
    source: "appData",
    path: "/data/tools/yt-dlp/yt-dlp",
    version: "2026.08.19",
    banner: "2026.08.19",
    meetsMinVersion: true,
    searched: [],
    ...overrides,
  };
}

describe("toolState()", () => {
  it("says ok for a found tool that is recent enough", () => {
    expect(toolState(tool())).toEqual({ state: "ok", key: "tools.state.ok", problem: false });
  });

  it("says notInstalled and flags a problem when nothing was found", () => {
    const info = toolState(tool({ path: null, source: null, version: null }));
    expect(info.state).toBe("notInstalled");
    expect(info.problem).toBe(true);
  });

  it("prefers belowMinVersion over ok, but not over notInstalled", () => {
    // Found and running but too old: a different problem from "not there", and
    // one with a different fix.
    const old = toolState(tool({ version: "2020.01.01", meetsMinVersion: false }));
    expect(old.state).toBe("belowMinVersion");
    expect(old.problem).toBe(true);

    // Nothing found *and* the flag is false: still "not installed".
    const missing = toolState(
      tool({ path: null, version: null, source: null, meetsMinVersion: false }),
    );
    expect(missing.state).toBe("notInstalled");
  });

  it("says notAvailable first, and does not blame the user for it", () => {
    const info = toolState(
      tool({ availableOnPlatform: false, path: null, source: null, version: null }),
    );
    expect(info.state).toBe("notAvailable");
    // Spec §1.1: "not available on this platform" is information, not a defect.
    expect(info.problem).toBe(false);
  });

  it("names an i18n key that exists in both dictionaries", () => {
    for (const candidate of [
      tool(),
      tool({ path: null }),
      tool({ meetsMinVersion: false }),
      tool({ availableOnPlatform: false }),
    ]) {
      const key = toolState(candidate).key;
      expect(hasKey("it", key), `it is missing ${key}`).toBe(true);
      expect(hasKey("en", key), `en is missing ${key}`).toBe(true);
    }
  });
});

describe("action availability", () => {
  it("offers Update only for yt-dlp, and only when it was found", () => {
    expect(canUpdate(tool({ name: "yt-dlp" }))).toBe(true);
    expect(canUpdate(tool({ name: "qjs" }))).toBe(false);
    expect(canUpdate(tool({ name: "yt-dlp", path: null }))).toBe(false);
  });

  it("offers 'use my own path' for anything that could exist here", () => {
    expect(canUseOwnPath(tool({ name: "ffmpeg" }))).toBe(true);
    expect(canUseOwnPath(tool({ availableOnPlatform: false }))).toBe(false);
  });
});

describe("hasToolProblem()", () => {
  it("is the red dot in the status footer (spec §11.2)", () => {
    expect(hasToolProblem([tool(), tool({ name: "qjs" })])).toBe(false);
    expect(
      hasToolProblem([tool(), tool({ name: "qjs", path: null, source: null, version: null })]),
    ).toBe(true);
    // A tool that does not exist on this platform must not light the dot up.
    expect(
      hasToolProblem([tool({ availableOnPlatform: false, path: null, source: null })]),
    ).toBe(false);
    expect(hasToolProblem([])).toBe(false);
  });
});

describe("normaliseToolStatus()", () => {
  it("fills in every field for an empty object", () => {
    expect(normaliseToolStatus(null)).toEqual({
      name: "",
      role: "",
      bundled: false,
      availableOnPlatform: true,
      installable: false,
      source: null,
      path: null,
      version: null,
      banner: null,
      meetsMinVersion: true,
      searched: [],
    });
  });

  it("drops an unknown source instead of trusting it", () => {
    expect(normaliseToolStatus({ source: "somewhere-else" as never }).source).toBeNull();
    expect(normaliseToolStatus({ source: "bundled" }).source).toBe("bundled");
  });

  it("treats an empty string as absent", () => {
    const out = normaliseToolStatus({ path: "", version: "", banner: "" });
    expect(out.path).toBeNull();
    expect(out.version).toBeNull();
    expect(out.banner).toBeNull();
  });

  it("keeps only string entries in `searched`", () => {
    const out = normaliseToolStatus({ searched: ["/a", 3, null, "/b"] as never });
    expect(out.searched).toEqual(["/a", "/b"]);
  });
});

describe("normaliseToolsView()", () => {
  it("returns an empty view for nothing", () => {
    expect(normaliseToolsView(null)).toEqual({
      tools: [],
      triple: "",
      toolsDir: "",
      bundledDir: "",
      tempDir: null,
      setupWizardCompleted: false,
    });
  });

  it("keeps a complete payload untouched", () => {
    const raw: ToolsView = {
      tools: [tool()],
      triple: "x86_64-unknown-linux-gnu",
      toolsDir: "/data/tools",
      bundledDir: "/app",
      tempDir: "/cache/tmp",
      setupWizardCompleted: true,
    };
    expect(normaliseToolsView(raw)).toEqual(raw);
  });

  it("drops rows with no name, which would be unidentifiable in the table", () => {
    const out = normaliseToolsView({ tools: [tool(), { version: "1.0" }] as never });
    expect(out.tools).toHaveLength(1);
  });
});

describe("isToolSource()", () => {
  it("recognises exactly the five sources of spec §5.2, in order", () => {
    const sources = ["override", "appData", "bundled", "path", "wellKnown"];
    expect(sources.every(isToolSource)).toBe(true);
    expect(isToolSource("PATH")).toBe(false);
    expect(isToolSource(null)).toBe(false);
  });
});

describe("the wizard steps", () => {
  it("has the four steps of spec §11.1, transcription included", () => {
    // Step 3 is a skippable placeholder, not missing: a wizard whose list
    // changes shape between versions is worse than one that says "later".
    expect(WIZARD_STEPS).toEqual(["welcome", "tools", "transcription", "output"]);
  });

  it("walks forwards and backwards without wrapping", () => {
    expect(wizardIndex("welcome")).toBe(0);
    expect(wizardPrevious("welcome")).toBeNull();
    expect(wizardNext("welcome")).toBe("tools");
    expect(wizardNext("output")).toBeNull();
    expect(wizardPrevious("tools")).toBe("welcome");
  });

  it("isWizardStep rejects anything else", () => {
    expect(isWizardStep("output")).toBe(true);
    expect(isWizardStep("models")).toBe(false);
  });

  it("names every step in both dictionaries", () => {
    for (const step of WIZARD_STEPS) {
      const key = `wizard.${step === "transcription" ? "transcription" : step}`;
      // `welcome` and `tools` and `output` have a `.body` key; the wizard
      // itself has one per step.
      const specific =
        step === "welcome"
          ? "wizard.welcome.body"
          : step === "tools"
            ? "wizard.tools.body"
            : step === "transcription"
              ? "wizard.transcription.body"
              : "wizard.output.body";
      expect(key.length).toBeGreaterThan(0);
      expect(hasKey("it", specific), `it is missing ${specific}`).toBe(true);
      expect(hasKey("en", specific), `en is missing ${specific}`).toBe(true);
    }
  });
});

describe("readyToolCount() and wizardToolNames()", () => {
  it("counts only the tools that are ready", () => {
    const tools = [
      tool({ name: "yt-dlp" }),
      tool({ name: "qjs" }),
      tool({ name: "ffmpeg", path: null, source: null, version: null }),
    ];
    expect(readyToolCount(tools)).toBe(2);
  });

  it("orders the wizard rows the way spec §11.1 talks about them", () => {
    const tools = [tool({ name: "ffprobe" }), tool({ name: "qjs" }), tool({ name: "yt-dlp" })];
    expect(wizardToolNames(tools)).toEqual(["yt-dlp", "qjs", "ffprobe"]);
  });
});

describe("the dictionaries", () => {
  it("still carry the same keys in both languages", () => {
    expect(keysOf("it")).toEqual(keysOf("en"));
  });

  it("carry a message for every tool state and source", () => {
    const required = [
      "tools.state.ok",
      "tools.state.notInstalled",
      "tools.state.belowMinVersion",
      "tools.state.notAvailable",
      "tools.source.override",
      "tools.source.appData",
      "tools.source.bundled",
      "tools.source.path",
      "tools.source.wellKnown",
      "tools.action.update",
      "tools.action.test",
      "tools.action.ownPath",
      "tools.update.done",
      "tools.update.upToDate",
    ];
    for (const key of required) {
      expect(hasKey("it", key), `it is missing ${key}`).toBe(true);
      expect(hasKey("en", key), `en is missing ${key}`).toBe(true);
    }
  });
});