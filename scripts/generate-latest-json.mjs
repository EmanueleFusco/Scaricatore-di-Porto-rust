// Generate the static updater manifest (spec §13.4).
//
// Tauri's updater reads `latest.json` from the release URL. The file lists
// every platform the release ships, with the artifact URL and the *content* of
// its `.sig` file: the plugin rejects a path or a URL in the `signature` field.
//
// This script runs in the release job after the artifacts are downloaded and
// flattened. It fails when a platform or a signature is missing, because a
// release with an incomplete manifest is an app that cannot update itself.
//
//   node scripts/generate-latest-json.mjs --assets assets
//
// The version defaults to `src-tauri/tauri.conf.json`, the tag to
// `GITHUB_REF_NAME`, the repository to `GITHUB_REPOSITORY` and the output to
// `<assets>/latest.json`.
import { readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

/** Platform keys understood by tauri-plugin-updater. */
export const PLATFORMS = [
  "linux-x86_64",
  "windows-x86_64",
  "darwin-aarch64",
  "darwin-x86_64",
];

const SIGNATURE = ".sig";

/** Stable order, so a diff of two manifests is readable. */
function sorted(names) {
  return [...names].sort((a, b) => a.localeCompare(b));
}

/**
 * Pick one updater artifact per platform from the flattened release assets.
 *
 * With `createUpdaterArtifacts: true` the Linux updater artifact is the raw
 * AppImage plus its `.sig`; a legacy v1-compatible build additionally wraps it
 * in `.AppImage.tar.gz`, so prefer that archive when it is present. macOS ships
 * one universal `.app.tar.gz`, used for both darwin keys because the app inside
 * is universal.
 */
export function selectReleaseArtifacts(files) {
  const names = sorted(files.filter((file) => !file.endsWith(SIGNATURE) && file !== "latest.json"));

  const appimages = names.filter((file) => /\.AppImage(?:\.tar\.gz)?$/.test(file));
  const linux =
    appimages.find((file) => file.endsWith(".AppImage.tar.gz")) ?? appimages[0] ?? null;

  const windows = names.find((file) => /-setup\.exe$/i.test(file)) ?? null;

  const macos = names.filter((file) => file.endsWith(".app.tar.gz"));
  let mac = null;
  if (macos.length === 1) {
    mac = macos[0];
  } else if (macos.length > 1) {
    // A future build may upload both the native and the universal archive.
    // Prefer the universal one, but only when the name says which is which:
    // guessing would attach a signature that does not match the binary.
    const universal = macos.filter((file) => /universal/i.test(file));
    if (universal.length !== 1) {
      throw new Error(
        `more than one macOS updater archive and no single universal one: ${macos.join(", ")}`,
      );
    }
    mac = universal[0];
  }

  return { linux, windows, macos: mac };
}

async function readSignature(assetsDir, artifact) {
  const path = join(assetsDir, `${artifact}${SIGNATURE}`);
  let text;
  try {
    text = await readFile(path, "utf8");
  } catch {
    throw new Error(`missing signature for ${artifact} (expected ${artifact}${SIGNATURE})`);
  }
  const signature = text.trim();
  if (!signature) {
    throw new Error(`signature file ${artifact}${SIGNATURE} is empty`);
  }
  return signature;
}

/** URL of one release asset. Spaces and other reserved characters are escaped. */
export function releaseAssetUrl(repo, tag, file) {
  return `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(file)}`;
}

/**
 * Build the manifest for the artifacts in `assetsDir`.
 *
 * `now` is injectable so the test can assert an exact `pub_date`.
 */
export async function buildLatestJson({ assetsDir, version, tag, repo, now = new Date() }) {
  const files = await readdir(assetsDir);
  const { linux, windows, macos } = selectReleaseArtifacts(files);

  const missing = [];
  if (!linux) missing.push("Linux AppImage (*.AppImage or *.AppImage.tar.gz)");
  if (!windows) missing.push("Windows installer (*-setup.exe)");
  if (!macos) missing.push("macOS updater archive (*.app.tar.gz)");
  if (missing.length > 0) {
    throw new Error(`release is missing an updater artifact: ${missing.join("; ")}`);
  }

  const [linuxSignature, windowsSignature, macSignature] = await Promise.all([
    readSignature(assetsDir, linux),
    readSignature(assetsDir, windows),
    readSignature(assetsDir, macos),
  ]);

  const url = (file) => releaseAssetUrl(repo, tag, file);

  return {
    version,
    notes: "",
    pub_date: now.toISOString(),
    platforms: {
      "linux-x86_64": { signature: linuxSignature, url: url(linux) },
      "windows-x86_64": { signature: windowsSignature, url: url(windows) },
      "darwin-aarch64": { signature: macSignature, url: url(macos) },
      "darwin-x86_64": { signature: macSignature, url: url(macos) },
    },
  };
}

function argument(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`--${name} needs a value`);
  }
  return value;
}

async function versionFromConfig() {
  const raw = await readFile("src-tauri/tauri.conf.json", "utf8");
  const version = JSON.parse(raw).version;
  if (typeof version !== "string" || version.length === 0) {
    throw new Error("src-tauri/tauri.conf.json has no version");
  }
  return version;
}

async function main() {
  const assetsDir = argument("assets", "assets");
  const version = argument("version", null) ?? (await versionFromConfig());
  const tag = argument("tag", process.env.GITHUB_REF_NAME);
  const repo = argument("repo", process.env.GITHUB_REPOSITORY);

  if (!tag) {
    throw new Error("missing tag: pass --tag v0.1.0 or set GITHUB_REF_NAME");
  }
  if (!repo) {
    throw new Error("missing repository: pass --repo owner/name or set GITHUB_REPOSITORY");
  }

  const output = argument("output", join(assetsDir, "latest.json"));
  const manifest = await buildLatestJson({ assetsDir, version, tag, repo });
  await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`[ci] wrote ${output} for ${version} (${Object.keys(manifest.platforms).length} platforms)`);
}

// `import.meta.url` comparison keeps the module importable from the tests
// without running `main`.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`[ci] ${error.message}`);
    process.exit(1);
  });
}
