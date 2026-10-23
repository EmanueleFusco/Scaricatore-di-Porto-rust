/**
 * Pure types and helpers for the Tools page and the Setup wizard
 * (spec §5.2, §11.1, §11.8).
 *
 * No Svelte runes and no `@tauri-apps` import, so every rule here is unit
 * tested (AGENTS.md §6). The store in `stores/tools.svelte.ts` owns the IPC.
 */

import type { BackendErrorInfo } from "$lib/settings";

/** Mirror of the Rust `services::tools::ToolSource` (spec §5.2 order). */
export type ToolSource = "override" | "appData" | "bundled" | "path" | "wellKnown";

export const TOOL_SOURCES: readonly ToolSource[] = [
  "override",
  "appData",
  "bundled",
  "path",
  "wellKnown",
];

export function isToolSource(value: unknown): value is ToolSource {
  return TOOL_SOURCES.includes(value as ToolSource);
}

/** Mirror of the Rust `services::tools::ToolStatus`. */
export interface ToolStatus {
  name: string;
  /** Human-readable role from the manifest; empty for a tool we only detect. */
  role: string;
  bundled: boolean;
  availableOnPlatform: boolean;
  /** The manifest pins a release this app could download. */
  installable: boolean;
  source: ToolSource | null;
  path: string | null;
  version: string | null;
  /** The raw `--version` output, for "Copy details". */
  banner: string | null;
  meetsMinVersion: boolean;
  /** Every path the lookup tried, so a red row can explain itself. */
  searched: string[];
}

/** Mirror of the Rust `commands::tools::ToolsView`. */
export interface ToolsView {
  tools: ToolStatus[];
  triple: string;
  toolsDir: string;
  bundledDir: string;
  tempDir: string | null;
  setupWizardCompleted: boolean;
}

/** Mirror of the Rust `services::tools::UpdateOutcome` (spec §5.5). */
export interface UpdateOutcome {
  updated: boolean;
  previousVersion: string | null;
  version: string | null;
  message: string;
}

/** Mirror of `commands::tools::ToolProgressEvent` (spec §3). */
export interface ToolProgressEvent {
  tool: string;
  received: number;
  total: number | null;
}

/** Mirror of `services::inspect::SelfTestReport` (spec §5.3.6). */
export interface SelfTestReport {
  ok: boolean;
  /** `quickjs` | `deno`, when a run happened at all. */
  runtime: string | null;
  elapsedMs: number;
  formats: number;
  heights: number[];
  error: BackendErrorInfo | null;
}

/** Coerce anything the backend sends into a complete self-test report. */
export function normaliseSelfTest(raw: unknown): SelfTestReport {
  const value = (raw ?? {}) as Record<string, unknown>;
  const error = value.error as Partial<BackendErrorInfo> | null | undefined;
  return {
    ok: value.ok === true,
    runtime: typeof value.runtime === "string" && value.runtime !== "" ? value.runtime : null,
    elapsedMs: typeof value.elapsedMs === "number" ? value.elapsedMs : 0,
    formats: typeof value.formats === "number" ? value.formats : 0,
    heights: Array.isArray(value.heights)
      ? value.heights.filter((h): h is number => typeof h === "number")
      : [],
    error:
      error && typeof error === "object"
        ? {
            code: typeof error.code === "string" ? error.code : "UNKNOWN",
            messageKey:
              typeof error.messageKey === "string" ? error.messageKey : "error.ytdlp.unknown",
            detail: typeof error.detail === "string" ? error.detail : null,
          }
        : null,
  };
}

/** Release channel for the yt-dlp updater (spec §5.5, §11.8). */
export type YtdlpChannel = "stable" | "nightly";

export function isYtdlpChannel(value: unknown): value is YtdlpChannel {
  return value === "stable" || value === "nightly";
}

/**
 * What one row should say about itself.
 *
 * Split out from the component so the wording can be tested: the rules are
 * subtle (`belowMinVersion` beats `ok`, a missing tool beats both) and a
 * screenshot is a poor way to regress them.
 */
export type ToolState =
  | "ok"
  | "belowMinVersion"
  | "notInstalled"
  | "notAvailable"
  | "notYetAvailable";

export interface ToolStateInfo {
  state: ToolState;
  /** i18n key for the status cell. */
  key: string;
  /** Is this a row the user has to act on? Drives the red dot in the footer. */
  problem: boolean;
}

/**
 * Tools that are **optional by design**: not being installed is a normal state,
 * not a problem. Deno is the automatic fallback runtime (spec §5.3) and
 * sdp-whisper only matters when the user chooses local Whisper transcription.
 * Neither is bundled, so a red "needs attention" dot for their absence would
 * light up the footer on every fresh install.
 */
export const OPTIONAL_TOOLS: readonly string[] = ["deno", "sdp-whisper"];

/**
 * Classify a tool for the status cell.
 *
 * Order matters and is deliberate:
 * 1. not available on this platform — nothing the user can do (spec §1.1: say
 *    so plainly instead of failing silently);
 * 2. declared but with no published release yet — the row exists so the tool
 *    is visible, but there is no button that would download a 404;
 * 3. below `minVersion` — found and running, but too old for the flags we need;
 * 4. not installed — nothing was found anywhere;
 * 5. ok.
 */
export function toolState(tool: ToolStatus): ToolStateInfo {
  if (!tool.availableOnPlatform) {
    return { state: "notAvailable", key: "tools.state.notAvailable", problem: false };
  }
  if (!tool.path && !tool.installable && !tool.bundled) {
    return {
      state: "notYetAvailable",
      key: "tools.state.notYetAvailable",
      problem: false,
    };
  }
  if (!tool.path) {
    return {
      state: "notInstalled",
      key: "tools.state.notInstalled",
      problem: !OPTIONAL_TOOLS.includes(tool.name),
    };
  }
  if (!tool.meetsMinVersion) {
    return { state: "belowMinVersion", key: "tools.state.belowMinVersion", problem: true };
  }
  return { state: "ok", key: "tools.state.ok", problem: false };
}

/** Does the row offer an Update button? Only the tool that can self-update. */
export function canUpdate(tool: ToolStatus): boolean {
  return tool.name === "yt-dlp" && tool.path !== null;
}

/** Does the row offer "Use my own path"? Every resolved tool can have one. */
export function canUseOwnPath(tool: ToolStatus): boolean {
  return tool.availableOnPlatform;
}

/** Should the footer show the red "tools need attention" dot (spec §11.2)? */
export function hasToolProblem(tools: readonly ToolStatus[]): boolean {
  return tools.some((tool) => toolState(tool).problem);
}

/** Coerce anything the backend sends into a complete, known-shaped row. */
export function normaliseToolStatus(raw: Partial<ToolStatus> | null | undefined): ToolStatus {
  return {
    name: typeof raw?.name === "string" ? raw.name : "",
    role: typeof raw?.role === "string" ? raw.role : "",
    bundled: raw?.bundled === true,
    availableOnPlatform: raw?.availableOnPlatform !== false,
    installable: raw?.installable === true,
    source: isToolSource(raw?.source) ? raw.source : null,
    path: typeof raw?.path === "string" && raw.path ? raw.path : null,
    version: typeof raw?.version === "string" && raw.version ? raw.version : null,
    banner: typeof raw?.banner === "string" && raw.banner ? raw.banner : null,
    meetsMinVersion: raw?.meetsMinVersion !== false,
    searched: Array.isArray(raw?.searched)
      ? raw.searched.filter((entry): entry is string => typeof entry === "string")
      : [],
  };
}

/** Coerce the whole payload, dropping rows without a name. */
export function normaliseToolsView(raw: Partial<ToolsView> | null | undefined): ToolsView {
  return {
    tools: Array.isArray(raw?.tools)
      ? raw.tools.map(normaliseToolStatus).filter((tool) => tool.name !== "")
      : [],
    triple: typeof raw?.triple === "string" ? raw.triple : "",
    toolsDir: typeof raw?.toolsDir === "string" ? raw.toolsDir : "",
    bundledDir: typeof raw?.bundledDir === "string" ? raw.bundledDir : "",
    tempDir: typeof raw?.tempDir === "string" && raw.tempDir ? raw.tempDir : null,
    setupWizardCompleted: raw?.setupWizardCompleted === true,
  };
}

/**
 * The steps of the first-run wizard that exist in M0 (spec §11.1).
 *
 * Step 3 is Transcription, which belongs to M5: it is present as a skippable
 * placeholder rather than missing, because a wizard whose step list changes
 * shape between versions is worse than one that says "later".
 */
export const WIZARD_STEPS = ["welcome", "tools", "transcription", "output"] as const;
export type WizardStep = (typeof WIZARD_STEPS)[number];

export function isWizardStep(value: unknown): value is WizardStep {
  return WIZARD_STEPS.includes(value as WizardStep);
}

/** Zero-based index of `step`, or -1. */
export function wizardIndex(step: WizardStep): number {
  return WIZARD_STEPS.indexOf(step);
}

export function wizardNext(step: WizardStep): WizardStep | null {
  const next = wizardIndex(step) + 1;
  return next < WIZARD_STEPS.length ? WIZARD_STEPS[next] : null;
}

export function wizardPrevious(step: WizardStep): WizardStep | null {
  const index = wizardIndex(step) - 1;
  return index >= 0 ? WIZARD_STEPS[index] : null;
}

/** How many tools are ready, for the wizard's "all set" line (spec §11.1). */
export function readyToolCount(tools: readonly ToolStatus[]): number {
  return tools.filter((tool) => toolState(tool).state === "ok").length;
}

/** The tools the wizard step 2 must report on, in a stable order. */
export function wizardToolNames(tools: readonly ToolStatus[]): string[] {
  const preferred = ["yt-dlp", "qjs", "ffmpeg"];
  const present = preferred.filter((name) => tools.some((tool) => tool.name === name));
  const rest = tools
    .map((tool) => tool.name)
    .filter((name) => !present.includes(name));
  return [...present, ...rest];
}

/**
 * In-flight promise deduplicator for background probing operations (spec §9.3).
 * Concurrent calls without `force` share the same promise; `force` initiates a new run.
 */
export class InFlightDeduplicator<T> {
  #inFlight: Promise<T> | null = null;

  run(fn: () => Promise<T>, force = false): Promise<T> {
    if (!force && this.#inFlight) {
      return this.#inFlight;
    }
    const promise = fn();
    this.#inFlight = promise;
    void promise.finally(() => {
      if (this.#inFlight === promise) {
        this.#inFlight = null;
      }
    });
    return promise;
  }

  get inFlight(): Promise<T> | null {
    return this.#inFlight;
  }
}