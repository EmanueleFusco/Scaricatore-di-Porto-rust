/**
 * Types and pure helpers for Whisper models and download management (spec §10.2, §11.6).
 */

export type ModelInstallState =
  | { kind: "notInstalled" }
  | { kind: "downloading"; receivedBytes: number; totalBytes: number | null }
  | { kind: "paused"; receivedBytes: number; totalBytes: number | null }
  | { kind: "installed"; sizeBytes: number };

export interface ModelViewItem {
  id: string;
  label: string;
  file: string;
  sizeBytes: number;
  ramEstimate: string;
  tier: string;
  indicativeWer: number | null;
  multilingual: boolean;
  recommended: boolean;
  advanced: boolean;
  notes: string | null;
  state: ModelInstallState;
}

export interface VadStatusView {
  id: string;
  installed: boolean;
  file: string;
  sizeBytes: number;
}

export interface ModelsView {
  models: ModelViewItem[];
  activeModel: string | null;
  recommendedModel: string;
  lowCoreSystem: boolean;
  freeSpaceBytes: number;
  vad: VadStatusView;
  modelsDir: string;
}

export interface ModelProgressEvent {
  id: string;
  receivedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  phase: "vad" | "model";
}

/** One gigabyte in bytes (spec §11.6: warn before large downloads > 1 GiB). */
export const ONE_GIB = 1024 * 1024 * 1024;

/**
 * Checks whether downloading this model triggers the large download warning (> 1 GiB).
 */
export function isLargeDownload(sizeBytes: number): boolean {
  return sizeBytes > ONE_GIB;
}

/**
 * Format bytes into human-readable string (e.g. 547 MB, 1.1 GB).
 */
export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const unitIndex = Math.min(i, units.length - 1);
  const value = bytes / Math.pow(1024, unitIndex);
  if (unitIndex === 0 || value % 1 === 0) return `${Math.round(value)} ${units[unitIndex]}`;
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

/**
 * Format indicative Word Error Rate percentage.
 */
export function formatWer(wer: number | null): string {
  if (wer === null || wer === undefined) return "—";
  return `~${wer.toFixed(1)}%`;
}

/**
 * Filter models for display based on full precision toggle under Advanced.
 */
export function filterModels(
  models: ModelViewItem[],
  showFullPrecision: boolean,
): ModelViewItem[] {
  if (showFullPrecision) return models;
  return models.filter((m) => !m.advanced);
}
