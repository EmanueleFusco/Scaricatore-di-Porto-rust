#!/usr/bin/env node
/**
 * Fetch the bundled tools (spec §5.2.1).
 *
 * Downloads the target-specific `yt-dlp` and `qjs` binaries, verifies their
 * checksums, renames `qjs` to the name yt-dlp requires, and drops both in
 * `src-tauri/binaries/<name>-<target-triple>[.exe]` — exactly where
 * `bundle.externalBin` looks for them.
 *
 * The binaries are **never committed** (spec §5.2.1); this script is what CI and
 * every developer run before `tauri build`.
 *
 * Everything it needs comes from `tools.manifest.json`, the same file the
 * backend compiles in, so the script and the app can never disagree about an
 * asset name.
 *
 * Usage
 * -----
 *   node scripts/fetch_bundled_tools.mjs                     # host target
 *   node scripts/fetch_bundled_tools.mjs --target aarch64-apple-darwin
 *   node scripts/fetch_bundled_tools.mjs --check             # offline validation
 *   node scripts/fetch_bundled_tools.mjs --verify-pins       # re-check pinned sha256
 *   node scripts/fetch_bundled_tools.mjs --print-sha256      # values for the manifest
 *
 * Options
 * -------
 *   --target <triple>   Rust target triple. Defaults to $SDP_TARGET_TRIPLE, then
 *                       `rustc --print host-tuple`, then a guess from the host.
 *   --output-dir <dir>  Defaults to `src-tauri/binaries`.
 *   --force             Re-download even if a matching binary is already there.
 *   --only <name,...>   Restrict to some tools (`--only qjs`).
 *
 * Exit code is 0 on success, 1 on any failure: a build that quietly bundles a
 * missing or unverified tool is worse than a failed build.
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, mkdir, mkdtemp, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..");
const MANIFEST_PATH = join(REPO_ROOT, "tools.manifest.json");
const TAURI_CONF_PATH = join(REPO_ROOT, "src-tauri", "tauri.conf.json");
const DEFAULT_OUTPUT_DIR = join(REPO_ROOT, "src-tauri", "binaries");

const UA = "scaricatore-di-porto-build";

/** Timeouts that are generous enough for a slow CI link and short enough to fail fast. */
const HTTP_TIMEOUT_MS = 120_000;

class Fail extends Error {}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const options = {
    target: null,
    outputDir: DEFAULT_OUTPUT_DIR,
    force: false,
    only: null,
    mode: "fetch",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case "--target":
        options.target = argv[++i] ?? fail("--target needs a value");
        break;
      case "--output-dir":
        options.outputDir = resolve(argv[++i] ?? fail("--output-dir needs a value"));
        break;
      case "--only":
        options.only = (argv[++i] ?? fail("--only needs a value"))
          .split(",")
          .map((name) => name.trim())
          .filter(Boolean);
        break;
      case "--force":
        options.force = true;
        break;
      case "--check":
        options.mode = "check";
        break;
      case "--verify-pins":
        options.mode = "verify-pins";
        break;
      case "--print-sha256":
        options.mode = "print-sha256";
        break;
      case "-h":
      case "--help":
        options.mode = "help";
        break;
      default:
        fail(`unknown option ${arg}. Try --help.`);
    }
  }
  return options;
}

function fail(message) {
  throw new Fail(message);
}

function usage() {
  return [
    "fetch_bundled_tools.mjs — download and verify the bundled yt-dlp and qjs.",
    "",
    "  node scripts/fetch_bundled_tools.mjs [--target <triple>] [--output-dir <dir>]",
    "                             [--force] [--only <name,...>]",
    "  node scripts/fetch_bundled_tools.mjs --check",
    "  node scripts/fetch_bundled_tools.mjs --verify-pins",
    "  node scripts/fetch_bundled_tools.mjs --print-sha256",
    "",
    `manifest: ${MANIFEST_PATH}`,
  ].join("\n");
}

// ---------------------------------------------------------------------------
// Target triple
// ---------------------------------------------------------------------------

/**
 * Resolve the Rust target triple.
 *
 * `rustc --print host-tuple` is the source of truth (it is what the Tauri docs
 * recommend for naming sidecars), but the script must also work where rustc is
 * not on PATH, so the host platform/arch is mapped explicitly.
 */
function resolveTarget(explicit) {
  if (explicit) return explicit;
  if (process.env.SDP_TARGET_TRIPLE) return process.env.SDP_TARGET_TRIPLE.trim();

  try {
    const printed = execFileSync("rustc", ["--print", "host-tuple"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (printed) return printed;
  } catch {
    // No rustc: fall through to the manual mapping below.
  }

  const arch = { x64: "x86_64", arm64: "aarch64", ia32: "i686", arm: "armv7" }[process.arch];
  if (!arch) fail(`cannot guess the target triple for arch ${process.arch}; pass --target`);
  if (process.platform === "darwin") return `${arch}-apple-darwin`;
  if (process.platform === "win32") return `${arch}-pc-windows-msvc`;
  if (process.platform === "linux") return `${arch}-unknown-linux-gnu`;
  fail(`unsupported host platform ${process.platform}; pass --target explicitly`);
  return "";
}

/** `.exe` only where Windows needs it. Matches Tauri, not Node. */
export function exeSuffix(triple) {
  return triple.includes("windows") ? ".exe" : "";
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

async function loadManifest() {
  let raw;
  try {
    raw = await readFile(MANIFEST_PATH, "utf8");
  } catch (err) {
    fail(`cannot read ${MANIFEST_PATH}: ${err.message}`);
  }
  let manifest;
  try {
    manifest = JSON.parse(raw);
  } catch (err) {
    fail(`${MANIFEST_PATH} is not valid JSON: ${err.message}`);
  }
  if (manifest.schemaVersion !== 1) {
    fail(`${MANIFEST_PATH}: schemaVersion is ${manifest.schemaVersion}, this script understands 1`);
  }
  return manifest;
}

function platformEntry(manifest, toolName, triple) {
  const tool = manifest.tools.find((candidate) => candidate.name === toolName);
  if (!tool) fail(`${MANIFEST_PATH} has no tool called ${toolName}`);
  const entry = tool.platforms?.[triple];
  if (!entry) {
    fail(
      `${toolName} is not declared for ${triple} in ${MANIFEST_PATH}.\n` +
        `  Declared targets: ${Object.keys(tool.platforms ?? {}).join(", ") || "(none)"}`,
    );
  }
  return { tool, entry };
}

/** Offline checks that need no network and no filesystem writes. */
export function validateManifest(manifest) {
  const problems = [];
  const names = manifest.tools.map((tool) => tool.name);
  for (const name of new Set(names)) {
    if (names.filter((candidate) => candidate === name).length > 1) {
      problems.push(`the tool ${name} is declared twice`);
    }
  }
  for (const tool of manifest.tools) {
    if (!tool.bundled) continue;
    if (!tool.license) problems.push(`${tool.name}: no license (spec §5.2.1 ships notices)`);
    if (!/^\d+(\.\d+)*$/.test(tool.minVersion ?? "")) {
      problems.push(`${tool.name}: minVersion ${JSON.stringify(tool.minVersion)} is not dotted numeric`);
    }
    if (tool.versioning === "latest") {
      if (!/^https:\/\//.test(tool.releaseUrl ?? "")) {
        problems.push(`${tool.name}: releaseUrl must be an https template`);
      }
      if (!/^https:\/\//.test(tool.checksumUrl ?? "")) {
        problems.push(`${tool.name}: checksumUrl must be an https URL`);
      }
      if (tool.releaseApi && !/^https:\/\//.test(tool.releaseApi)) {
        problems.push(`${tool.name}: releaseApi must be an https URL`);
      }
    } else if (!tool.version) {
      problems.push(`${tool.name}: versioning is "pinned" so version must be filled in`);
    }
    for (const [triple, entry] of Object.entries(tool.platforms ?? {})) {
      if (!entry.executableName) problems.push(`${tool.name}/${triple}: executableName is empty`);
      if (!entry.asset && !(entry.builtFrom ?? []).length) {
        problems.push(`${tool.name}/${triple}: neither asset nor builtFrom`);
      }
      if (entry.asset && /[/\\]|\.\.|^\./.test(entry.asset)) {
        problems.push(`${tool.name}/${triple}: asset ${JSON.stringify(entry.asset)} is not a plain file name`);
      }
      if (Boolean(entry.url) !== Boolean(entry.sha256)) {
        problems.push(`${tool.name}/${triple}: url and sha256 must both be set or both be absent`);
      }
      if (entry.url && !entry.url.startsWith("https://")) {
        problems.push(`${tool.name}/${triple}: url must be https`);
      }
      if (entry.sha256 && !/^[0-9a-f]{64}$/.test(entry.sha256)) {
        problems.push(`${tool.name}/${triple}: sha256 must be 64 lowercase hex characters`);
      }
      // `builtFrom` exists to say which slices Tauri will assemble a universal
      // target from, and Tauri assembles `universal-apple-darwin` from exactly
      // two. A third slice would be downloaded and thrown away; a single one
      // would produce a bundle that is not universal while claiming to be.
      const builtFrom = entry.builtFrom ?? [];
      if (builtFrom.length > 0 && isUniversalTriple(triple)) {
        const same = builtFrom.length === UNIVERSAL_SLICES.length &&
          UNIVERSAL_SLICES.every((slice) => builtFrom.includes(slice));
        if (!same) {
          problems.push(
            `${tool.name}/${triple}: builtFrom is [${builtFrom.join(", ")}] but ` +
              `${UNIVERSAL_TRIPLE} is assembled from [${UNIVERSAL_SLICES.join(", ")}]`,
          );
        }
      }
    }
  }
  return problems;
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/**
 * Transient failures are real and observed: `releases/latest/download/<asset>`
 * answered 404 several times in a row while the identical pinned-tag URL
 * returned 200, minutes apart, on the same machine. A build script that gives
 * up on the first such blip fails for no reason the developer can act on, so
 * transient statuses are retried with a backoff and only a persistent failure
 * is an error.
 */
const TRANSIENT_STATUSES = new Set([404, 408, 425, 429, 500, 502, 503, 504]);
const RETRY_DELAYS_MS = [1_000, 3_000, 8_000, 15_000, 30_000];

function isRetryable(status) {
  return status === undefined || TRANSIENT_STATUSES.has(status);
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function fetchWithRetry(url, accept, what) {
  let lastError = "";
  for (let attempt = 0; attempt <= RETRY_DELAYS_MS.length; attempt += 1) {
    if (attempt > 0) {
      const delay = RETRY_DELAYS_MS[attempt - 1];
      process.stdout.write(`  retrying in ${delay / 1000}s (attempt ${attempt + 1})\n`);
      await sleep(delay);
    }
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), HTTP_TIMEOUT_MS);
    try {
      const response = await fetch(url, {
        redirect: "follow",
        signal: controller.signal,
        headers: { "user-agent": UA, accept },
      });
      if (response.ok) return response;
      lastError = `HTTP ${response.status} ${response.statusText}`;
      if (!isRetryable(response.status)) break;
    } catch (err) {
      lastError = err.name === "AbortError" ? "timed out" : `${err.name}: ${err.message}`;
      // A name resolution or connection error is retryable by nature.
    } finally {
      clearTimeout(timer);
    }
  }
  fail(`${what} failed: ${url}\n  ${lastError}`);
  return undefined;
}

async function download(url, destination) {
  const response = await fetchWithRetry(url, "application/octet-stream", "download");
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length === 0) fail(`download produced an empty file: ${url}`);
  await writeFile(destination, bytes);
  return bytes;
}

async function downloadText(url) {
  const response = await fetchWithRetry(url, "text/plain", "download");
  return response.text();
}

/**
 * Resolve the newest release of an "always latest" tool.
 *
 * Why not just use the `/releases/latest/download/<asset>` URL: measured on
 * 2026-10-13, that alias answered 404 for minutes at a time — for curl and for
 * Node, over HTTP/1.1 and HTTP/2, while the *pinned* tag URL returned 200 the
 * whole time. A build that depends on a redirect target being warm is a build
 * that fails for no actionable reason.
 *
 * So the concrete tag is resolved through the Releases API (different
 * infrastructure, and it tells us which version we shipped — spec §5.2.1
 * "recording its version"), and the `latest` URL stays only as a fallback when
 * the API itself is unreachable or rate-limited.
 *
 * Returns `{tag, assets}` or `null` when the API could not be reached.
 */
/**
 * The web page for the same repository's latest release, derived from the API
 * URL. The two differ only in host and path prefix, and deriving it keeps the
 * manifest to one URL per tool.
 */
export function releasesPageUrl(apiUrl) {
  const page = apiUrl.replace(
    /^https:\/\/api\.github\.com\/repos\//,
    "https://github.com/",
  );
  if (page === apiUrl) {
    fail(
      `${apiUrl} is not a GitHub releases API URL, so the redirect fallback\n` +
        `  cannot be derived from it. Fix releaseApi in the manifest.`,
    );
  }
  return page;
}

/**
 * The tag out of a `Location:` header pointing at `/releases/tag/<tag>`.
 *
 * Pure, and tested, because it is the one piece of the fallback that runs only
 * when the API has already failed — the worst possible moment to discover it is
 * wrong.
 */
export function tagFromLocation(location) {
  const match = /\/releases\/tag\/([^/?#]+)/.exec(location ?? "");
  if (!match) return null;
  const tag = decodeURIComponent(match[1]);
  // A tag is not allowed to contain a slash or a space, and `.` and `..` pass
  // every character test while still walking the URL back up a directory. Both
  // would end up in `/releases/download/<tag>/<asset>`, so both are refused.
  if (tag === "." || tag === "..") return null;
  return /^[\w.\-+]+$/.test(tag) ? tag : null;
}

/**
 * Ask the Releases API for the latest stable release.
 *
 * Three ways to get the tag, tried in this order, because on 2026-10-13 the
 * first one failed on GitHub-hosted runners and the second (the one we had at
 * the time) was broken outright:
 *
 * 1. **the API with `GITHUB_TOKEN`**, when the environment has one. Un-
 *    authenticated API calls are limited to 60/hour per IP, and GitHub-hosted
 *    runners share IP ranges, so a shared range gets `HTTP 403` — which is
 *    exactly what `bundle (macos-arm64)` got;
 * 2. **the API without a token**, for a developer machine;
 * 3. **the `Location:` header of `https://github.com/<owner>/<repo>/releases/
 *    latest`**, which GitHub answers with a 302 to `/releases/tag/<tag>`. No
 *    API, no quota, no rate limit. Measured on 2026-10-13:
 *
 *        $ curl -sI https://github.com/yt-dlp/yt-dlp/releases/latest
 *        HTTP/2 302
 *        location: https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19
 *
 * The `latest/download/<asset>` alias is **not** used any more. It answered
 * `404` three times out of three on the same day, while the explicit-tag URL
 * returned 206 for the same asset, so falling back to it meant: API rate-limited
 * -> 404 -> six retries over 57 seconds -> failure. A fallback that is known to
 * be broken is worse than no fallback, because it costs a minute before it gives
 * up.
 *
 * Returns `{tag, assets}` — with `assets: null` when only the tag is known, and
 * the caller builds the download URLs from the manifest templates.
 */
async function resolveLatestRelease(tool) {
  if (!tool.releaseApi) return null;

  const headers = {
    "user-agent": UA,
    accept: "application/vnd.github+json",
    "x-github-api-version": "2022-11-28",
  };
  // First-party GitHub only, and never logged: it just lifts the rate limit.
  const token = process.env.GITHUB_TOKEN;
  if (token) headers.authorization = `Bearer ${token}`;

  let payload;
  try {
    const response = await fetch(tool.releaseApi, { redirect: "follow", headers });
    if (response.ok) {
      payload = await response.json();
    } else {
      const limit = response.headers.get("x-ratelimit-remaining");
      process.stdout.write(
        `  the releases API answered HTTP ${response.status}` +
          `${limit === "0" ? " (rate limit exhausted)" : ""}; ` +
          `asking the releases page for the tag instead\n`,
      );
    }
  } catch (err) {
    process.stdout.write(
      `  the releases API is unreachable (${err.message}); asking the releases page for the tag\n`,
    );
  }

  if (payload) {
    if (payload.draft === true || payload.prerelease === true) {
      fail(
        `the releases API returned ${payload.tag_name}, which is marked draft/prerelease.\n` +
          `  "latest stable" must not mean "whatever is newest".`,
      );
    }
    const assets = new Map(
      (payload.assets ?? []).map((asset) => [asset.name, asset.browser_download_url]),
    );
    return { tag: payload.tag_name, assets };
  }

  const tag = await tagFromLatestRedirect(releasesPageUrl(tool.releaseApi));
  if (tag) {
    process.stdout.write(`  the releases page points at ${tag}\n`);
    return { tag, assets: null };
  }
  return null;
}

/** The tag in the `Location:` header of the repository's "latest release" page. */
async function tagFromLatestRedirect(pageUrl) {
  try {
    const response = await fetch(pageUrl, {
      redirect: "manual",
      headers: { "user-agent": UA },
    });
    const location = response.headers.get("location");
    return location ? tagFromLocation(location) : null;
  } catch {
    return null;
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/**
 * Parse a GNU `sha256sum` file: `<64 hex>  <name>`, two spaces.
 *
 * Only the entries we were asked about are read; the format allows a `*` marker
 * for binary mode, which is accepted and ignored.
 */
export function parseSumFile(text, wanted) {
  const sums = new Map();
  for (const line of text.split("\n")) {
    const match = /^([0-9a-f]{64}) [ *](.+)$/.exec(line.trim());
    if (!match) continue;
    sums.set(match[2], match[1]);
  }
  if (wanted && !sums.has(wanted)) {
    fail(
      `the release's SHA2-256SUMS does not list ${wanted}.\n` +
        `  Entries: ${[...sums.keys()].slice(0, 12).join(", ")}`,
    );
  }
  return wanted ? sums.get(wanted) : sums;
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/**
 * `binaries/<name>-<triple>[.exe]` — exactly what `bundle.externalBin` resolves.
 *
 * `name` is the tool name (`qjs`), **not** the manifest's `executableName`
 * (`qjs.exe` on Windows). Tauri appends the system extension itself when it
 * strips the triple suffix, so passing the already-suffixed name would produce
 * `qjs.exe-x86_64-pc-windows-msvc.exe`, which nothing looks for.
 */
export function destinationPath(outputDir, name, triple) {
  return join(outputDir, `${name}-${triple}${exeSuffix(triple)}`);
}

export async function install(bytes, destination) {
  // Guard, not decoration. `install` has no other way to notice that it was
  // handed a string instead of bytes: `writeFile` accepts a string happily and
  // writes it, so a path that reached here would become a small text file and
  // the fetch would report success. That happened once (see `downloadSlice`),
  // and it was only noticed two steps later by `lipo`, which reported an
  // architecture problem about a file that was never an executable. One cheap
  // type check turns a mystery into a sentence at the point of the mistake.
  if (typeof bytes === "string") {
    fail(
      `internal error: asked to write a string to ${destination}\n` +
        `  A path was passed where the contents were expected. This is a bug in\n` +
        `  fetch_bundled_tools.mjs, not in the manifest or the network.`,
    );
  }
  await mkdir(dirname(destination), { recursive: true });
  // Write beside the destination and rename, so a killed build never leaves a
  // half-written executable that the bundler would happily copy into the app.
  const temporary = `${destination}.tmp-${process.pid}`;
  await writeFile(temporary, bytes);
  if (!exeSuffix(destination)) {
    await chmod(temporary, 0o755);
  }
  await rename(temporary, destination);
  return destination;
}

/**
 * Run `--version` on a freshly installed binary of *this* machine's target and
 * return the first line, or `null` when it is for another target or refuses to
 * run.
 *
 * The checksum has already been verified before this point, so running the file
 * is not a new trust decision — it is the same one the app makes on first
 * launch, and catching "the download is fine but the binary is broken" here is
 * far cheaper than catching it on a user's machine.
 */
function recordVersion(destination, triple) {
  const hostTriple = resolveTarget();
  if (hostTriple !== triple) return null;
  try {
    const output = execFileSync(destination, ["--version"], {
      encoding: "utf8",
      timeout: 30_000,
      stdio: ["ignore", "pipe", "pipe"],
    });
    return output.split("\n")[0].trim() || null;
  } catch (err) {
    if (err.status === null || err.status === undefined) {
      process.stdout.write(
        `  warning: could not run the freshly installed binary (${err.message}).\n` +
          `  It is checksum-verified; if this is macOS an unsigned ad-hoc binary may\n` +
          `  need "xattr -d com.apple.quarantine" (spec §5.2).\n`,
      );
      return null;
    }
    fail(`the freshly installed binary exited with ${err.status}\n  ${err.stderr?.toString().trim() ?? ""}`);
    return null;
  }
}

// ---------------------------------------------------------------------------
// The two bundled tools
// ---------------------------------------------------------------------------

/**
 * The universal macOS target Tauri builds, and the two slices it builds it
 * from.
 *
 * Not a guess about Rust triples: `tauri build --target universal-apple-darwin`
 * runs cargo **once per architecture**, and `tauri-build` then asks for the
 * external binaries by `TARGET` (read from the environment, so per-arch):
 *
 *     let target_triple = env::var("TARGET").unwrap();            // aarch64-apple-darwin
 *     copy_binaries(ResourcePaths::new(
 *       &external_binaries(paths, &target_triple, &target), true), ...)
 *
 * and only at the end `tauri-bundler` asks once, with `self.target`:
 *
 *     external_bin: ... external_binaries(bins, &target, &target_platform)
 *
 * So a universal build needs **all three** names on disk, per tool:
 *
 *     binaries/yt-dlp-aarch64-apple-darwin        <- build script, pass 1
 *     binaries/yt-dlp-x86_64-apple-darwin        <- build script, pass 2
 *     binaries/yt-dlp-universal-apple-darwin     <- bundler, packaging
 *
 * Missing the first two is the `resource path 'binaries/yt-dlp-aarch64-apple-
 * darwin' doesn't exist` failure the CI hit on 2026-10-13, in the build script
 * of the very first per-arch pass.
 */
export const UNIVERSAL_TRIPLE = "universal-apple-darwin";
export const UNIVERSAL_SLICES = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/** True for the one target Tauri assembles from several slices. */
export function isUniversalTriple(triple) {
  return triple === UNIVERSAL_TRIPLE;
}

/**
 * The name `lipo -thin` wants for a slice, which is not the Rust triple.
 *
 * Rust says `aarch64-apple-darwin`; `lipo` and the Mach-O `fat_arch` headers say
 * `arm64`. Read off a real asset on 2026-10-13 (`yt-dlp_macos`, magic
 * `0xcafebabe`, nfat_arch 2): `x86_64` and `arm64`.
 */
export function lipoArchName(triple) {
  const [arch] = triple.split("-");
  if (!arch) fail(`cannot read an architecture out of the triple "${triple}"`);
  return arch === "aarch64" ? "arm64" : arch;
}

/**
 * A universal macOS build cannot be assembled without `lipo`.
 *
 * Checked once, before any download, and with one message used everywhere: the
 * first version of this guard lived inside `buildUniversalQjs` only, and the
 * tools are fetched in manifest order — `yt-dlp` comes before `qjs`, so on a
 * non-macOS machine yt-dlp reached `lipo` first and failed with 37 MB already
 * downloaded and whatever `lipo` says when it is not installed. Failing before
 * the first byte is the difference between a sentence and a mystery.
 */
function requireMacOsForUniversal(triple) {
  if (!isUniversalTriple(triple) || process.platform === "darwin") return;
  fail(
    `the universal macOS build needs lipo, which only exists on macOS.\n` +
      `  You asked for ${triple} on ${process.platform}, and nothing was downloaded.\n` +
      `  Use --target aarch64-apple-darwin (or x86_64-apple-darwin) for a native build.`,
  );
}

/**
 * macOS universal qjs: `lipo` the two slices (spec §5.2.1, CI runs this on macOS). */
async function buildUniversalQjs(manifest, triple, scratch) {
  requireMacOsForUniversal(triple);
  const { entry } = platformEntry(manifest, "qjs", triple);
  const slices = entry.builtFrom ?? [];
  if (slices.length < 2) fail(`${triple}: builtFrom must name at least two slices, got ${slices.length}`);

  const paths = [];
  for (const slice of slices) {
    // `.path`: lipo needs a file, not bytes.
    paths.push((await downloadSlice(manifest, "qjs", slice, scratch)).path);
  }

  const output = join(scratch, "qjs-universal");
  try {
    execFileSync("lipo", ["-create", ...paths, "-output", output], { stdio: "pipe" });
  } catch (err) {
    fail(`lipo failed: ${err.stderr?.toString().trim() || err.message}`);
  }
  const bytes = await readFile(output);
  return bytes;
}

/**
 * Download one slice and verify it against the manifest checksum.
 *
 * Returns `{ bytes, path }` and *not* one of the two, because the two callers
 * need different ones and picking wrong is silent: `buildUniversalQjs` needs a
 * file to hand to `lipo -create`, `perArchBytes` needs the content to write.
 *
 * This is not a hypothetical. The first version returned only `path`, and the
 * universal fetch wrote that path string *as the contents* of
 * `binaries/qjs-aarch64-apple-darwin` — a thirty-byte text file. Nothing failed
 * at that point; the build went on and the damage surfaced two steps later as
 * `lipo: can't figure out the architecture type`, which says nothing about the
 * real cause. Making the caller choose is the cheapest way to stop that.
 */
async function downloadSlice(manifest, name, slice, scratch) {
  const source = platformEntry(manifest, name, slice);
  if (!source.entry.url || !source.entry.sha256) fail(`${slice} is not pinned`);
  const path = join(scratch, `${name}-${slice}`);
  const bytes = await download(source.entry.url, path);
  const actual = sha256(bytes);
  if (actual !== source.entry.sha256) {
    fail(
      `checksum mismatch for ${source.entry.asset}\n  expected ${source.entry.sha256}\n  actual   ${actual}`,
    );
  }
  return { bytes, path };
}

/**
 * One per-architecture file, for the build script of a universal pass.
 *
 * Two ways to get it, and which one applies is a fact about the tool, taken
 * from the manifest rather than guessed:
 *
 * * `builtFrom` present (qjs) — the slice is a separate release asset, already
 *   checksummed, downloaded like any other;
 * * `builtFrom` absent (yt-dlp) — the universal asset is *already* a fat
 *   binary, so the slice is carved out of it. `yt-dlp_macos` verified on
 *   2026-10-13 by reading its Mach-O header on Linux, no macOS needed: magic
 *   `0xcafebabe`, `nfat_arch` 2, `x86_64` (73 808 B) + `arm64` (37 047 744 B).
 *
 * If a future asset were single-architecture, `lipo -thin` fails loudly here
 * rather than producing a bundle that boots on one CPU and not the other.
 */
async function perArchBytes(manifest, name, slice, universalPath, scratch) {
  const { entry } = platformEntry(manifest, name, UNIVERSAL_TRIPLE);
  if ((entry.builtFrom ?? []).length > 0) {
    // `.bytes`: the caller writes this to disk, and what goes on disk must be
    // the slice and not the place the slice was downloaded to.
    return (await downloadSlice(manifest, name, slice, scratch)).bytes;
  }
  const output = join(scratch, `${name}-${slice}-thin`);
  const arch = lipoArchName(slice);
  try {
    execFileSync("lipo", ["-thin", arch, universalPath, "-output", output], { stdio: "pipe" });
  } catch (err) {
    fail(
      `lipo -thin ${arch} failed for ${name}/${slice}.\n` +
        `  The universal asset must contain that architecture.\n` +
        `  ${err.stderr?.toString().trim() || err.message}`,
    );
  }
  return await readFile(output);
}

/**
 * Write the two per-architecture files that a universal build's build script
 * asks for, reading the universal file already on disk.
 *
 * The universal file is read from its destination rather than passed in, so
 * this behaves the same whether the universal file was just fetched or was
 * already there and skipped.
 */
async function installPerArchSiblings(manifest, name, outputDir, scratch, options) {
  for (const slice of UNIVERSAL_SLICES) {
    const destination = destinationPath(outputDir, name, slice);
    if (!options.force && existsSync(destination)) {
      const { size } = await stat(destination);
      process.stdout.write(
        `${name}: ${slice} already present (${destination}, ${size} bytes) — use --force\n`,
      );
      continue;
    }
    const bytes = await perArchBytes(
      manifest,
      name,
      slice,
      destinationPath(outputDir, name, UNIVERSAL_TRIPLE),
      scratch,
    );
    await install(bytes, destination);
    process.stdout.write(
      `${name}: installed ${slice} -> ${destination} (${bytes.length} bytes)\n`,
    );
  }
}

async function fetchTool(manifest, name, triple, scratch) {
  const { tool, entry } = platformEntry(manifest, name, triple);

  // Locally assembled (macOS universal qjs): no checksum by construction.
  if ((entry.builtFrom ?? []).length > 0) {
    process.stdout.write(`${name}: assembling ${triple} from ${entry.builtFrom.join(" + ")}\n`);
    return buildUniversalQjs(manifest, triple, scratch);
  }

  if (!entry.asset) fail(`${name}/${triple}: no asset name in the manifest`);
  if (!entry.executableName) fail(`${name}/${triple}: no executableName in the manifest`);

  // `latest` tools resolve the concrete release first; pinned ones already carry
  // the exact release URL and must not be second-guessed.
  let release = null;
  if (tool.versioning === "latest") {
    if (!/^https:\/\//.test(tool.releaseUrl ?? "")) {
      fail(`${name}/${triple}: releaseUrl must be an https template`);
    }
    release = await resolveLatestRelease(tool);
    if (release) process.stdout.write(`${name}: latest release is ${release.tag}\n`);
  }

  const fromRelease = release?.assets?.get(entry.asset) ?? null;
  if (release?.assets && !fromRelease) {
    fail(
      `${name}: release ${release.tag} has no asset called ${entry.asset}.\n` +
        `  This is the "names changed between tags" case from spec §5.2.1:\n` +
        `  fix tools.manifest.json.`,
    );
  }
  // The tag is substituted once it is known, and `latest` only while it is not.
  // Building the URL from the template rather than from the asset list is what
  // lets the redirect fallback work at all: it gives us the tag but no list.
  const tag = release?.tag ?? "latest";
  const url =
    entry.url ?? fromRelease ?? tool.releaseUrl.replace("{tag}", tag).replace("{asset}", entry.asset);
  if (!/^https:\/\//.test(url)) fail(`${name}/${triple}: refusing to fetch a non-https URL: ${url}`);

  process.stdout.write(`${name}: downloading ${url}\n`);
  const bytes = await download(url, join(scratch, entry.asset));
  const actual = sha256(bytes);

  if (tool.versioning === "latest") {
    // Spec §5.1: yt-dlp's checksum comes from the release's own SHA2-256SUMS,
    // not from us, because the version is always the newest one.
    const sumUrl =
      release?.assets?.get("SHA2-256SUMS") ?? tool.checksumUrl.replace("{tag}", tag);
    if (entry.sha256) {
      if (entry.sha256 !== actual) {
        fail(`checksum mismatch for ${entry.asset}\n  expected ${entry.sha256}\n  actual   ${actual}`);
      }
      process.stdout.write(`${name}: checksum matches the manifest (${actual.slice(0, 16)}…)\n`);
    } else {
      const expected = parseSumFile(await downloadText(sumUrl), entry.asset);
      if (expected !== actual) {
        fail(`checksum mismatch against ${sumUrl}\n  expected ${expected}\n  actual   ${actual}\n  asset    ${entry.asset}`);
      }
      process.stdout.write(`${name}: checksum verified against SHA2-256SUMS (${actual.slice(0, 16)}…)\n`);
    }
  } else {
    if (!entry.sha256) {
      fail(
        `${name}/${triple}: the manifest pins no sha256 and the tool is not "latest".\n` +
          `  Run with --print-sha256 to get the value, then fill it in.`,
      );
    }
    if (entry.sha256 !== actual) {
      fail(`checksum mismatch for ${entry.asset}\n  expected ${entry.sha256}\n  actual   ${actual}`);
    }
    process.stdout.write(`${name}: checksum verified against the manifest (${actual.slice(0, 16)}…)\n`);
  }

  if (entry.archiveKind && entry.archiveKind !== "none") {
    fail(
      `${name}/${triple} is an archive (${entry.archiveKind}).\n` +
        `  Only the two bundled tools may be in the installer (spec §0.7) and both are\n` +
        `  plain executables, so an archive here means the manifest and the spec disagree.`,
    );
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

async function modeCheck(manifest, triple) {
  const problems = validateManifest(manifest);
  problems.push(...externalBinProblems(manifest));
  if (problems.length > 0) {
    for (const problem of problems) process.stderr.write(`  - ${problem}\n`);
    fail(`${problems.length} problem(s) in ${MANIFEST_PATH}`);
  }
  const bundled = manifest.tools.filter((tool) => tool.bundled);
  process.stdout.write(`manifest OK: ${bundled.length} bundled tool(s), schema 1\n`);
  for (const tool of bundled) {
    const entry = tool.platforms[triple];
    const state = entry
      ? (entry.builtFrom ?? []).length > 0
        ? `lipo of ${entry.builtFrom.join(" + ")}`
        : entry.asset
      : "NOT AVAILABLE for this target";
    process.stdout.write(`  ${tool.name.padEnd(8)} ${triple.padEnd(28)} ${state}\n`);
  }
}

/**
 * `bundle.externalBin` in tauri.conf.json e `bundled: true` nel manifesto
 * devono dire le stesse cose.
 *
 * Il motivo è un bug vero, non una previsione. Dichiarando `externalBin`,
 * `tauri_build::build()` — che sta in `build.rs` e quindi gira a **ogni**
 * `cargo`, non solo a `tauri build` — si ferma con
 * `resource path 'binaries/yt-dlp-<triple>' doesn't exist` se il file non c'è.
 * Un tool presente nel manifesto ma assente da `externalBin` (o viceversa)
 * significa: uno dei due è sbagliato, e la CI lo scoprirebbe solo compilando,
 * con un messaggio che non dice quale dei due sia.
 *
 * Costa niente (è un controllo offline su due file JSON) e chiude il ciclo.
 */
export function externalBinProblems(manifest) {
  return externalBinProblemsWith(manifest, readTauriConf());
}

/** `src-tauri/tauri.conf.json`, parsed. Throws with the path in the message. */
function readTauriConf() {
  try {
    return JSON.parse(readFileSync(TAURI_CONF_PATH, "utf8"));
  } catch (err) {
    throw new Fail(`cannot read ${TAURI_CONF_PATH}: ${err.message}`);
  }
}

/** Same checks against a caller-supplied config; the tests use it directly. */
export function externalBinProblemsWith(manifest, conf) {
  const problems = [];
  const declared = conf.bundle?.externalBin ?? [];
  if (!Array.isArray(declared)) {
    return [`${TAURI_CONF_PATH}: bundle.externalBin must be an array`];
  }

  const wanted = declared.map((entry) => basename(entry));
  const bundled = manifest.tools.filter((tool) => tool.bundled).map((tool) => tool.name);

  for (const name of bundled) {
    if (!wanted.includes(name)) {
      problems.push(
        `${name} is marked bundled:true in the manifest but is NOT in ` +
          `bundle.externalBin — it would be fetched and never packaged.`,
      );
    }
  }
  for (const name of wanted) {
    if (!bundled.includes(name)) {
      problems.push(
        `bundle.externalBin lists ${name}, which is not a bundled tool in the manifest — ` +
          `spec §0.7 allows only yt-dlp and qjs in the installer.`,
      );
    }
  }
  // The entry may carry a directory (`binaries/yt-dlp`) — that is the
  // documented form and the one the bundler wants. What it must NOT carry is a
  // system extension, because `tauri_utils::external_binaries` builds
  // `<entry>-<triple>.exe` on Windows: writing `qjs.exe` here would ask for
  // `qjs.exe-x86_64-pc-windows-msvc.exe`, which nothing produces.
  // (Read from tauri-utils/src/resources.rs, `external_binaries`.)
  for (const entry of declared) {
    if (/\.(exe|dll|so|dylib|bin)$/i.test(entry)) {
      problems.push(
        `bundle.externalBin entry ${JSON.stringify(entry)} must not carry a system ` +
          `extension: the bundler appends -<triple> and .exe itself`,
      );
    }
  }
  return problems;
}

/** Last path segment, on both separators. */
function basename(entry) {
  const text = String(entry).replace(/\\/g, "/");
  return text.slice(text.lastIndexOf("/") + 1);
}

async function modeVerifyPins(manifest) {
  let checked = 0;
  for (const tool of manifest.tools) {
    if (tool.versioning !== "pinned") continue;
    const scratch = await mkdtemp(join(tmpdir(), "sdp-pins-"));
    try {
      for (const [triple, entry] of Object.entries(tool.platforms ?? {})) {
        if (!entry.url || !entry.sha256) continue;
        const bytes = await download(entry.url, join(scratch, entry.asset));
        const actual = sha256(bytes);
        const mark = actual === entry.sha256 ? "ok" : "MISMATCH";
        process.stdout.write(`${mark.padEnd(9)} ${tool.name}/${triple} ${actual}\n`);
        if (mark !== "ok") {
          fail(`${tool.name}/${triple}: expected ${entry.sha256}, downloaded ${actual}`);
        }
        checked += 1;
      }
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  }
  process.stdout.write(`${checked} pinned checksum(s) verified\n`);
}

async function modePrintSha256(manifest) {
  for (const tool of manifest.tools) {
    if (tool.versioning !== "pinned") continue;
    const scratch = await mkdtemp(join(tmpdir(), "sdp-sha-"));
    try {
      for (const [triple, entry] of Object.entries(tool.platforms ?? {})) {
        if (!entry.url) continue;
        const bytes = await download(entry.url, join(scratch, entry.asset));
        process.stdout.write(`${triple.padEnd(28)} ${sha256(bytes)}  ${entry.asset}\n`);
      }
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  }
}

async function modeFetch(manifest, triple, options) {
  // Before anything else, and before the first byte: asking for a universal
  // macOS build anywhere else cannot work, and there is no point discovering
  // it 37 MB later.
  requireMacOsForUniversal(triple);
  const names = manifest.tools.filter((tool) => tool.bundled).map((tool) => tool.name);
  const wanted = options.only ?? names;
  for (const name of wanted) {
    if (!names.includes(name)) {
      fail(`${name} is not a bundled tool. Bundled: ${names.join(", ")}`);
    }
  }

  await mkdir(options.outputDir, { recursive: true });
  const scratch = await mkdtemp(join(tmpdir(), "sdp-tools-"));
  try {
    for (const name of wanted) {
      const { entry } = platformEntry(manifest, name, triple);
      if (!entry.executableName) fail(`${name}/${triple}: no executableName in the manifest`);
      const destination = destinationPath(options.outputDir, name, triple);
      if (!options.force && existsSync(destination)) {
        const { size } = await stat(destination);
        process.stdout.write(`${name}: already present (${destination}, ${size} bytes) — use --force\n`);
        // The per-arch siblings are part of the same set. Without this, a
        // resumed run would find the universal file already there, skip it, and
        // never write the two files the build script is about to ask for — the
        // same failure as before, but only on the second run.
        if (isUniversalTriple(triple)) {
          await installPerArchSiblings(manifest, name, options.outputDir, scratch, options);
        }
        continue;
      }
      const bytes = await fetchTool(manifest, name, triple, scratch);
      await install(bytes, destination);
      process.stdout.write(`${name}: installed -> ${destination} (${bytes.length} bytes)\n`);
      if (isUniversalTriple(triple)) {
        await installPerArchSiblings(manifest, name, options.outputDir, scratch, options);
      }
      // Spec §5.2.1: "the latest stable yt-dlp at build time (recording its
      // version)". Only asked of the host's own triple — a binary fetched for
      // another target cannot be executed here, and executing a foreign binary
      // would be the wrong thing to try.
      const recorded = recordVersion(destination, triple);
      if (recorded) process.stdout.write(`${name}: version ${recorded}\n`);
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }

  process.stdout.write(`\nbundled tools ready for ${triple} in ${options.outputDir}\n`);
  process.stdout.write("reminder: they are git-ignored on purpose (spec §5.2.1).\n");
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.mode === "help") {
    process.stdout.write(`${usage()}\n`);
    return;
  }
  const manifest = await loadManifest();
  const triple = resolveTarget(options.target);

  switch (options.mode) {
    case "check":
      await modeCheck(manifest, triple);
      break;
    case "verify-pins":
      await modeVerifyPins(manifest);
      break;
    case "print-sha256":
      await modePrintSha256(manifest);
      break;
    default:
      await modeCheck(manifest, triple);
      process.stdout.write("\n");
      await modeFetch(manifest, triple, options);
  }
}

// Only run when invoked as a program: importing this module — which the unit
// tests do — must not start downloading anything.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    process.stderr.write(`\nfetch_bundled_tools: ${err instanceof Fail ? err.message : (err?.stack ?? err)}\n`);
    process.exit(1);
  });
}