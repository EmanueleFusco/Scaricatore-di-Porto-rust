/**
 * Types and pure helpers for the About page (spec §11.8, §13.4).
 *
 * The page shows three things support needs: the exact build, the update
 * state, and a diagnostics report that is safe to paste into an issue.
 */

export interface AppInfo {
  name: string;
  version: string;
  os: string;
  arch: string;
  triple: string;
  settingsPath: string;
  toolsDir: string;
  bundledDir: string;
  tempDir: string | null;
  modelsDir: string;
  logPath: string;
}

export interface UpdateCheck {
  available: boolean;
  currentVersion: string;
  version: string | null;
  notes: string | null;
  date: string | null;
}

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
  percent: number | null;
}

/** One third-party component, as shown in the licences list. */
export interface LicenseEntry {
  name: string;
  license: string;
  url: string;
  /** What the component does, in one i18n key. */
  roleKey: string;
}

/**
 * The components a user actually receives or downloads.
 *
 * Kept next to the code rather than parsed from `tools.manifest.json`: the
 * manifest is a build-time document and does not travel in the webview, and
 * this list also has to name the frontend libraries.
 */
export const LICENSES: readonly LicenseEntry[] = [
  {
    name: "Scaricatore Di Porto",
    license: "MIT",
    url: "https://github.com/EmanueleFusco/Scaricatore-di-Porto-rust/blob/main/LICENSE",
    roleKey: "about.licenses.role.app",
  },
  {
    name: "Tauri",
    license: "MIT OR Apache-2.0",
    url: "https://github.com/tauri-apps/tauri/blob/dev/LICENSE_APACHE",
    roleKey: "about.licenses.role.tauri",
  },
  {
    name: "Svelte",
    license: "MIT",
    url: "https://github.com/sveltejs/svelte/blob/main/LICENSE.md",
    roleKey: "about.licenses.role.svelte",
  },
  {
    name: "Lucide",
    license: "ISC",
    url: "https://github.com/lucide-icons/lucide/blob/main/LICENSE",
    roleKey: "about.licenses.role.lucide",
  },
  {
    name: "yt-dlp",
    license: "Unlicense",
    url: "https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE",
    roleKey: "about.licenses.role.ytdlp",
  },
  {
    name: "QuickJS-ng",
    license: "MIT",
    url: "https://github.com/quickjs-ng/quickjs/blob/master/LICENSE",
    roleKey: "about.licenses.role.qjs",
  },
  {
    name: "ffmpeg / ffprobe",
    license: "LGPL-2.1-or-later / GPL-2.0-or-later",
    url: "https://ffmpeg.org/legal.html",
    roleKey: "about.licenses.role.ffmpeg",
  },
  {
    name: "Deno",
    license: "MIT",
    url: "https://github.com/denoland/deno/blob/main/LICENSE.md",
    roleKey: "about.licenses.role.deno",
  },
  {
    name: "whisper.cpp",
    license: "MIT",
    url: "https://github.com/ggml-org/whisper.cpp/blob/master/LICENSE",
    roleKey: "about.licenses.role.whisper",
  },
];

export function normaliseAppInfo(raw: Partial<AppInfo> | null | undefined): AppInfo {
  return {
    name: typeof raw?.name === "string" && raw.name ? raw.name : "Scaricatore Di Porto",
    version: typeof raw?.version === "string" ? raw.version : "",
    os: typeof raw?.os === "string" ? raw.os : "",
    arch: typeof raw?.arch === "string" ? raw.arch : "",
    triple: typeof raw?.triple === "string" ? raw.triple : "",
    settingsPath: typeof raw?.settingsPath === "string" ? raw.settingsPath : "",
    toolsDir: typeof raw?.toolsDir === "string" ? raw.toolsDir : "",
    bundledDir: typeof raw?.bundledDir === "string" ? raw.bundledDir : "",
    tempDir: typeof raw?.tempDir === "string" && raw.tempDir ? raw.tempDir : null,
    modelsDir: typeof raw?.modelsDir === "string" ? raw.modelsDir : "",
    logPath: typeof raw?.logPath === "string" ? raw.logPath : "",
  };
}

export function normaliseUpdateCheck(raw: Partial<UpdateCheck> | null | undefined): UpdateCheck {
  return {
    available: raw?.available === true,
    currentVersion: typeof raw?.currentVersion === "string" ? raw.currentVersion : "",
    version: typeof raw?.version === "string" && raw.version ? raw.version : null,
    notes: typeof raw?.notes === "string" && raw.notes ? raw.notes : null,
    date: typeof raw?.date === "string" && raw.date ? raw.date : null,
  };
}

/** Percentage for a progress bar, clamped and rounded. */
export function updatePercent(progress: UpdateProgress): number | null {
  if (progress.percent !== null && Number.isFinite(progress.percent)) {
    return Math.max(0, Math.min(100, Math.round(progress.percent)));
  }
  if (progress.total !== null && progress.total > 0) {
    return Math.max(0, Math.min(100, Math.round((progress.downloaded / progress.total) * 100)));
  }
  return null;
}

/** Does this bundle type support replacing itself? Only AppImage on Linux. */
export function canSelfUpdate(bundleType: string | null): boolean {
  if (bundleType === null || bundleType === "") return true;
  return bundleType !== "deb" && bundleType !== "rpm";
}
