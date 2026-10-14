/**
 * Pure helpers for the temporary single-job UI (M1).
 *
 * Mirrors the Rust event payloads of spec §3 (`job:progress`, `job:done`). All
 * user-visible wording comes from the i18n dictionaries through these helpers,
 * so there is no concatenated sentence in a component (spec §11.9).
 */

import { t, type Lang } from "$lib/i18n";
import type { BackendErrorInfo } from "$lib/settings";

/** Mirror of the Rust `commands::download::QualityRequest`. */
export type QualityRequest =
  | { kind: "best" }
  | { kind: "exact"; height: number }
  | { kind: "audioOnly" };

export type JobStage =
  | "preparing"
  | "downloading"
  | "postProcessing"
  | "done"
  | "failed"
  | "cancelled";

/** The state of the temporary single-job flow (M1). */
export type JobStatus = "idle" | "running" | "done" | "failed" | "cancelled";

/** Mirror of `commands::download::JobProgressEvent`. */
export interface JobProgressEvent {
  jobId: string;
  stage: JobStage;
  percent: number | null;
  speedBps: number | null;
  etaSec: number | null;
  label: string;
  subIndex: number | null;
  subTotal: number | null;
}

/** Mirror of `commands::download::JobDoneEvent`. */
export interface JobDoneEvent {
  jobId: string;
  status: "done" | "failed" | "cancelled";
  file: string | null;
  runtime: string | null;
  elapsedMs: number | null;
  error: BackendErrorInfo | null;
}

/** Mirror of `commands::download::DownloadStarted`. */
export interface DownloadStarted {
  jobId: string;
  outputDir: string;
}

const STAGES: readonly JobStage[] = [
  "preparing",
  "downloading",
  "postProcessing",
  "done",
  "failed",
  "cancelled",
];

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function asNullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function normaliseProgress(raw: unknown): JobProgressEvent {
  const value = (raw ?? {}) as Record<string, unknown>;
  return {
    jobId: asString(value.jobId),
    stage: STAGES.includes(value.stage as JobStage) ? (value.stage as JobStage) : "preparing",
    percent: asNullableNumber(value.percent),
    speedBps: asNullableNumber(value.speedBps),
    etaSec: asNullableNumber(value.etaSec),
    label: asString(value.label),
    subIndex: asNullableNumber(value.subIndex),
    subTotal: asNullableNumber(value.subTotal),
  };
}

export function normaliseDone(raw: unknown): JobDoneEvent {
  const value = (raw ?? {}) as Record<string, unknown>;
  const status = value.status;
  const error = value.error as Partial<BackendErrorInfo> | null | undefined;
  return {
    jobId: asString(value.jobId),
    status: status === "done" || status === "failed" || status === "cancelled" ? status : "failed",
    file: typeof value.file === "string" && value.file !== "" ? value.file : null,
    runtime: typeof value.runtime === "string" && value.runtime !== "" ? value.runtime : null,
    elapsedMs: asNullableNumber(value.elapsedMs),
    error:
      error && typeof error === "object"
        ? {
            code: asString(error.code) || "UNKNOWN",
            messageKey: asString(error.messageKey) || "error.ytdlp.unknown",
            detail: typeof error.detail === "string" ? error.detail : null,
          }
        : null,
  };
}

/** i18n key for a stage chip. */
export function stageKey(stage: JobStage): string {
  return `job.stage.${stage}`;
}

/** i18n key for a stream/step label token coming from the backend. */
export function labelKey(label: string): string {
  switch (label) {
    case "preparing":
    case "verifying":
      return "job.label.preparing";
    case "video":
      return "job.label.video";
    case "audio":
      return "job.label.audio";
    case "merging":
      return "job.label.merging";
    case "moving":
      return "job.label.moving";
    case "extractingAudio":
      return "job.label.extractingAudio";
    case "embedding":
      return "job.label.embedding";
    case "postProcessing":
      return "job.label.postProcessing";
    default:
      return "job.label.postProcessing";
  }
}

/**
 * The human line under the progress bar: "Video 1/2", "Merging…", "Preparing…".
 *
 * The backend sends stable tokens, never sentences (spec §11.9), so plural and
 * word order are decided here, per language.
 */
export function progressLabel(lang: Lang, event: JobProgressEvent): string {
  const key = labelKey(event.label);
  if (event.label === "video" || event.label === "audio") {
    return t(lang, key, { index: event.subIndex ?? 1, total: event.subTotal ?? 1 });
  }
  return t(lang, key);
}

/** What the footer under the bar shows: speed and ETA, both optional. */
export function progressDetail(
  lang: Lang,
  event: JobProgressEvent,
  speed: (bps: number) => string | null,
  eta: (sec: number) => string | null,
): string[] {
  const parts: string[] = [];
  if (event.speedBps !== null) {
    const value = speed(event.speedBps);
    if (value) parts.push(t(lang, "queue.speed", { value }));
  }
  if (event.etaSec !== null) {
    const value = eta(event.etaSec);
    if (value) parts.push(t(lang, "queue.eta", { value }));
  }
  return parts;
}

/** Does a failed job offer the Deno install (spec §5.3.2 "ask once")? */
export function offersDenoInstall(
  done: JobDoneEvent | null,
  dismissed: boolean,
): boolean {
  return !dismissed && done?.status === "failed" && done.error?.code === "DENO_MISSING";
}

/**
 * i18n key for the state word of the status footer (spec §11.2).
 *
 * While a job runs the footer names the *stage* ("Download", "Elaborazione");
 * afterwards it names the outcome. `idle` keeps the M0 wording.
 */
export function footerStateKey(status: JobStatus, stage: JobStage | null): string {
  switch (status) {
    case "running":
      return stage ? stageKey(stage) : "job.stage.preparing";
    case "done":
      return "job.stage.done";
    case "failed":
      return "job.stage.failed";
    case "cancelled":
      return "job.stage.cancelled";
    default:
      return "footer.idle";
  }
}

/**
 * The last segment of a path, for a compact footer line.
 *
 * Both separators, on every OS: a Windows path shown on Linux (a log line, a
 * pasted path) must not come out whole.
 */
export function fileName(path: string): string {
  const trimmed = path.trim();
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
}
