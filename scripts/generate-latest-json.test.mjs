// Tests for the updater manifest generator (spec §13.4).
//
// The release job trusts this file to decide whether an update can be
// installed. A wrong URL, a missing signature or a platform silently left out
// is not a cosmetic bug: it is a release that some users cannot update from.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  PLATFORMS,
  buildLatestJson,
  releaseAssetUrl,
  selectReleaseArtifacts,
} from "./generate-latest-json.mjs";

const APPIMAGE = "Scaricatore Di Porto_0.1.0_amd64.AppImage";
const APPIMAGE_ARCHIVE = `${APPIMAGE}.tar.gz`;
const WINDOWS = "Scaricatore Di Porto_0.1.0_x64-setup.exe";
const MACOS = "Scaricatore Di Porto.app.tar.gz";

const TYPICAL = [
  "Scaricatore Di Porto_0.1.0_amd64.deb",
  "Scaricatore Di Porto-0.1.0-1.x86_64.rpm",
  APPIMAGE,
  APPIMAGE_ARCHIVE,
  `${APPIMAGE_ARCHIVE}.sig`,
  WINDOWS,
  `${WINDOWS}.sig`,
  MACOS,
  `${MACOS}.sig`,
  "latest.json",
];

let dir;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "sdp-latest-json-"));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

/** Write the asset list; `.sig` files get a signature-looking content. */
function assets(names = TYPICAL) {
  for (const name of names) {
    writeFileSync(join(dir, name), name.endsWith(".sig") ? `signature:${name}\n` : "artifact");
  }
  return dir;
}

function build(overrides = {}) {
  return buildLatestJson({
    assetsDir: dir,
    version: "0.1.0",
    tag: "v0.1.0",
    repo: "EmanueleFusco/ScaricatoreDiPorto",
    now: new Date("2026-10-23T12:00:00Z"),
    ...overrides,
  });
}

describe("the updater manifest", () => {
  it("lists every platform with the signature content, not its path", async () => {
    assets();
    const manifest = await build();

    expect(manifest.version).toBe("0.1.0");
    expect(manifest.pub_date).toBe("2026-10-23T12:00:00.000Z");
    expect(Object.keys(manifest.platforms)).toEqual(PLATFORMS);

    expect(manifest.platforms["linux-x86_64"].signature).toBe(`signature:${APPIMAGE_ARCHIVE}.sig`);
    expect(manifest.platforms["windows-x86_64"].signature).toBe(`signature:${WINDOWS}.sig`);
    expect(manifest.platforms["darwin-aarch64"].signature).toBe(`signature:${MACOS}.sig`);
    expect(manifest.platforms["darwin-x86_64"]).toEqual(
      manifest.platforms["darwin-aarch64"],
    );
  });

  it("prefers the tar.gz the bundler creates for the updater", async () => {
    assets();
    const manifest = await build();
    expect(manifest.platforms["linux-x86_64"].url).toContain("AppImage.tar.gz");
    expect(manifest.platforms["linux-x86_64"].url).not.toContain("AppImage.sig");
  });

  it("falls back to the raw AppImage when no archive is present", async () => {
    assets([
      APPIMAGE,
      `${APPIMAGE}.sig`,
      WINDOWS,
      `${WINDOWS}.sig`,
      MACOS,
      `${MACOS}.sig`,
    ]);
    const manifest = await build();
    expect(manifest.platforms["linux-x86_64"].url).toContain("AppImage");
    expect(manifest.platforms["linux-x86_64"].url).not.toContain("AppImage.tar.gz");
  });

  it("escapes spaces in the release asset URL", async () => {
    assets();
    const manifest = await build();
    expect(manifest.platforms["linux-x86_64"].url).toBe(
      "https://github.com/EmanueleFusco/ScaricatoreDiPorto/releases/download/v0.1.0/" +
        "Scaricatore%20Di%20Porto_0.1.0_amd64.AppImage.tar.gz",
    );
  });

  it("uses the universal macOS archive for both architectures", async () => {
    const universal = "Scaricatore Di Porto_universal.app.tar.gz";
    assets([APPIMAGE_ARCHIVE, `${APPIMAGE_ARCHIVE}.sig`, WINDOWS, `${WINDOWS}.sig`, universal, `${universal}.sig`]);
    const manifest = await build();
    expect(manifest.platforms["darwin-aarch64"].url).toContain("universal");
    expect(manifest.platforms["darwin-x86_64"].url).toContain("universal");
  });

  it("refuses an ambiguous set of macOS archives", async () => {
    assets([
      APPIMAGE_ARCHIVE,
      `${APPIMAGE_ARCHIVE}.sig`,
      WINDOWS,
      `${WINDOWS}.sig`,
      MACOS,
      `${MACOS}.sig`,
      "Scaricatore Di Porto_native.app.tar.gz",
      "Scaricatore Di Porto_native.app.tar.gz.sig",
    ]);
    await expect(build()).rejects.toThrow(/more than one macOS updater archive/);
  });

  it("refuses a platform with no artifact", async () => {
    assets([APPIMAGE_ARCHIVE, `${APPIMAGE_ARCHIVE}.sig`, WINDOWS, `${WINDOWS}.sig`]);
    await expect(build()).rejects.toThrow(/macOS updater archive/);
  });

  it("refuses an artifact whose signature is missing", async () => {
    assets([
      APPIMAGE_ARCHIVE,
      `${APPIMAGE_ARCHIVE}.sig`,
      WINDOWS,
      MACOS,
      `${MACOS}.sig`,
    ]);
    await expect(build()).rejects.toThrow(new RegExp(`missing signature for ${WINDOWS}`));
  });

  it("refuses an empty signature", async () => {
    assets();
    writeFileSync(join(dir, `${WINDOWS}.sig`), "  \n");
    await expect(build()).rejects.toThrow(/signature file .* is empty/);
  });

  it("reports exactly which platform is missing", () => {
    expect(selectReleaseArtifacts([])).toEqual({ linux: null, windows: null, macos: null });
    expect(selectReleaseArtifacts([WINDOWS]).windows).toBe(WINDOWS);
    expect(selectReleaseArtifacts([MACOS]).macos).toBe(MACOS);
  });

  it("builds a GitHub release URL without double-escaping the tag", () => {
    expect(releaseAssetUrl("owner/repo", "v1.0.0", "a b.AppImage")).toBe(
      "https://github.com/owner/repo/releases/download/v1.0.0/a%20b.AppImage",
    );
  });
});

describe("the updater manifest command", () => {
  it("writes latest.json next to the artifacts", async () => {
    assets();
    const output = join(dir, "latest.json");
    const script = join(import.meta.dirname, "generate-latest-json.mjs");
    const result = spawnSync(
      process.execPath,
      [
        script,
        "--assets",
        dir,
        "--version",
        "0.1.0",
        "--tag",
        "v0.1.0",
        "--repo",
        "owner/repo",
        "--output",
        output,
      ],
      { encoding: "utf8" },
    );
    expect(result.status, result.stderr).toBe(0);
    const written = JSON.parse(readFileSync(output, "utf8"));
    expect(written.platforms["linux-x86_64"].url).toContain("owner/repo");
  });
});
