/**
 * Tests for the pure parts of `scripts/fetch_bundled_tools.mjs` (spec §5.2.1).
 *
 * Everything that decides *what* to download, *where* to put it and *whether*
 * the manifest is self-consistent lives here, so it is testable without a
 * network. The download itself is exercised for real by running the script in
 * CI before `tauri build`.
 */
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  UNIVERSAL_SLICES,
  destinationPath,
  exeSuffix,
  externalBinProblems,
  externalBinProblemsWith,
  install,
  isUniversalTriple,
  lipoArchName,
  parseSumFile,
  releasesPageUrl,
  tagFromLocation,
  validateManifest,
} from "./fetch_bundled_tools.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = JSON.parse(readFileSync(join(REPO_ROOT, "tools.manifest.json"), "utf8"));

describe("the shipped manifest", () => {
  it("has no structural problems", () => {
    expect(validateManifest(MANIFEST)).toEqual([]);
  });

  it("bundles yt-dlp and qjs and nothing else (spec §0.7)", () => {
    const bundled = MANIFEST.tools.filter((tool) => tool.bundled).map((tool) => tool.name);
    expect(bundled).toEqual(["yt-dlp", "qjs"]);
  });

  it("declares both bundled tools for every Tier 1 target", () => {
    for (const triple of [
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "aarch64-apple-darwin",
      "universal-apple-darwin",
    ]) {
      for (const tool of MANIFEST.tools.filter((candidate) => candidate.bundled)) {
        expect(tool.platforms[triple], `${tool.name} on ${triple}`).toBeDefined();
      }
    }
  });
});

describe("validateManifest", () => {
  const base = (overrides = {}) => ({
    schemaVersion: 1,
    tools: [
      {
        name: "t",
        role: "r",
        bundled: true,
        license: "MIT",
        versioning: "pinned",
        version: "1.0.0",
        minVersion: "1.0",
        platforms: {
          "x86_64-unknown-linux-gnu": {
            asset: "t",
            executableName: "t",
            archiveKind: "none",
            url: "https://example.invalid/t",
            sha256: "0".repeat(64),
          },
        },
        ...overrides,
      },
    ],
  });

  it("accepts a well-formed manifest", () => {
    expect(validateManifest(base())).toEqual([]);
  });

  it("rejects a pinned tool with no version", () => {
    const problems = validateManifest(base({ version: null }));
    expect(problems.join()).toContain('versioning is "pinned" so version must be filled in');
  });

  it("rejects a url with no checksum and a checksum with no url", () => {
    const noChecksum = base();
    delete noChecksum.tools[0].platforms["x86_64-unknown-linux-gnu"].sha256;
    expect(validateManifest(noChecksum).join()).toContain("both be set or both be absent");

    const noUrl = base();
    delete noUrl.tools[0].platforms["x86_64-unknown-linux-gnu"].url;
    expect(validateManifest(noUrl).join()).toContain("both be set or both be absent");
  });

  it("rejects a plain http url and an asset name with a path in it", () => {
    const problems = validateManifest(
      base({
        platforms: {
          "x86_64-unknown-linux-gnu": {
            asset: "../escape",
            executableName: "t",
            archiveKind: "none",
            url: "http://insecure.example/t",
            sha256: "0".repeat(64),
          },
        },
      }),
    );
    const joined = problems.join("\n");
    expect(joined).toContain("url must be https");
    expect(joined).toContain("is not a plain file name");
  });

  it("rejects a checksum that is not 64 lowercase hex characters", () => {
    const problems = validateManifest(
      base({
        platforms: {
          "x86_64-unknown-linux-gnu": {
            asset: "t",
            executableName: "t",
            archiveKind: "none",
            url: "https://example.invalid/t",
            sha256: "ABC",
          },
        },
      }),
    );
    expect(problems.join()).toContain("sha256 must be 64 lowercase hex characters");
  });

  it("rejects a duplicated tool name", () => {
    const manifest = base();
    manifest.tools.push(structuredClone(manifest.tools[0]));
    expect(validateManifest(manifest).join()).toContain("is declared twice");
  });

  it("wants https for releaseUrl, checksumUrl and releaseApi in latest mode", () => {
    const problems = validateManifest(
      base({
        versioning: "latest",
        version: null,
        releaseUrl: "http://insecure/{asset}",
        checksumUrl: "http://insecure/SHA2-256SUMS",
        releaseApi: "http://insecure/api",
      }),
    );
    const joined = problems.join("\n");
    expect(joined).toContain("releaseUrl must be an https template");
    expect(joined).toContain("checksumUrl must be an https URL");
    expect(joined).toContain("releaseApi must be an https URL");
  });
});

describe("parseSumFile", () => {
  // Real shape of yt-dlp's SHA2-256SUMS (downloaded 2026-10-13).
  const sums = [
    "1fa6733c37ea6fb51c99ad8fe785e7b7e5f3246c9b980230329d4fb72ed8d4d6  yt-dlp",
    "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a  yt-dlp.exe",
    "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a  yt-dlp_linux",
    "",
  ].join("\n");

  it("reads the entries of a GNU sha256sum file", () => {
    expect(parseSumFile(sums, "yt-dlp_linux")).toBe(
      "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a",
    );
    expect(parseSumFile(sums).size).toBe(3);
  });

  it("accepts the binary-mode asterisk marker", () => {
    const binary = `${"0".repeat(64)} *yt-dlp`;
    expect(parseSumFile(binary, "yt-dlp")).toBe("0".repeat(64));
  });

  it("ignores lines that are not checksums", () => {
    expect(parseSumFile("# a comment\nnot a checksum\n").size).toBe(0);
  });

  it("fails loudly when the wanted asset is absent", () => {
    expect(() => parseSumFile(sums, "yt-dlp_macos")).toThrowError(/does not list yt-dlp_macos/);
  });

  it("does not accept an uppercase checksum, so the entry simply is not listed", () => {
    // Silently returning a wrong-but-plausible checksum would be worse than
    // failing, so a missing entry raises instead.
    const upper = `${"A".repeat(64)}  yt-dlp_linux`;
    expect(() => parseSumFile(upper, "yt-dlp_linux")).toThrowError(/does not list yt-dlp_linux/);
  });
});

describe("externalBin and the manifest agree", () => {
  // The bug this exists for: declaring `bundle.externalBin` makes
  // `tauri_build::build()` — in build.rs, so it runs on EVERY cargo — fail with
  // "resource path doesn't exist". A tool in one file and not the other breaks
  // the build with a message that does not say which half is wrong.
  it("accepts the configuration the repository actually ships", () => {
    expect(externalBinProblems(MANIFEST)).toEqual([]);
  });

  it("names the tool that is in the manifest but missing from externalBin", () => {
    // Simulated by asking about a manifest that bundles one tool more than
    // tauri.conf.json lists.
    const extra = structuredClone(MANIFEST);
    extra.tools.push({
      name: "sdp-whisper",
      role: "Offline transcription",
      bundled: true,
      license: "MIT",
      versioning: "pinned",
      version: "1.0.0",
      minVersion: "1.0",
      platforms: {
        "x86_64-unknown-linux-gnu": {
          asset: "sdp-whisper",
          executableName: "sdp-whisper",
          archiveKind: "none",
          url: "https://example.invalid/sdp-whisper",
          sha256: "0".repeat(64),
        },
      },
    });
    const problems = externalBinProblems(extra);
    expect(problems.join("\n")).toContain("sdp-whisper is marked bundled:true");
    expect(problems.join("\n")).toContain("never packaged");
  });

  it("refuses an externalBin entry carrying a system extension", () => {
    // `tauri_utils::external_binaries` builds `<entry>-<triple>.exe` on Windows.
    // So `qjs.exe` would ask for `qjs.exe-x86_64-pc-windows-msvc.exe`, which
    // nothing produces. A directory prefix is fine and expected; an extension
    // is not. (This rule was wrong the first time it was written — it rejected
    // the documented `binaries/yt-dlp` form.)
    const problems = externalBinProblemsWith(MANIFEST, {
      bundle: { externalBin: ["binaries/yt-dlp", "binaries/qjs.exe"] },
    });
    expect(problems.join("\n")).toContain('"binaries/qjs.exe" must not carry a system extension');
  });

  it("accepts the documented directory form", () => {
    expect(
      externalBinProblemsWith(MANIFEST, {
        bundle: { externalBin: ["binaries/yt-dlp", "binaries/qjs"] },
      }),
    ).toEqual([]);
  });

  it("refuses an externalBin entry that is not a bundled tool", () => {
    // Spec §0.7: only yt-dlp and qjs may ever be in the installer.
    const problems = externalBinProblemsWith(MANIFEST, {
      bundle: { externalBin: ["binaries/yt-dlp", "binaries/qjs", "binaries/ffmpeg"] },
    });
    expect(problems.join("\n")).toContain("spec §0.7 allows only yt-dlp and qjs");
  });
});

describe("install refuses a path where the contents belong", () => {
  // The bug the universal fetch shipped on 2026-10-13: `downloadSlice` returned
  // a path, `perArchBytes` passed it straight on, and `install` wrote that path
  // *as the contents* of binaries/qjs-aarch64-apple-darwin. Thirty bytes of
  // text, reported as a successful fetch. `lipo` then said "can't figure out
  // the architecture type" two steps later, which is a sentence about the wrong
  // thing entirely.
  //
  // JavaScript does not check types, and the symptom arrives far from the
  // mistake, so the check lives where the mistake is made.

  it("refuses a string payload and says what it is", async () => {
    const dir = mkdtempSync(join(tmpdir(), "sdp-install-"));
    const destination = join(dir, "qjs-aarch64-apple-darwin");
    await expect(install("/tmp/sdp-tools-abc/qjs-aarch64-apple-darwin", destination)).rejects.toThrow(
      /asked to write a string/,
    );
    // And nothing was written: refusing must not leave a half file behind.
    expect(existsSync(destination)).toBe(false);
    rmSync(dir, { recursive: true, force: true });
  });

  it("names the real cause rather than blaming the manifest", async () => {
    const dir = mkdtempSync(join(tmpdir(), "sdp-install-"));
    const destination = join(dir, "qjs");
    await expect(install("/tmp/some/path", destination)).rejects.toThrow(
      /bug in\s+fetch_bundled_tools\.mjs, not in the manifest or the network/,
    );
    rmSync(dir, { recursive: true, force: true });
  });

  it("writes bytes, and is not a veto on the ordinary case", async () => {
    const dir = mkdtempSync(join(tmpdir(), "sdp-install-"));
    const destination = join(dir, "qjs-aarch64-apple-darwin");
    // The first four bytes of every Mach-O, fat or not.
    const payload = Buffer.from([0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]);
    await install(payload, destination);
    expect(readFileSync(destination)).toEqual(payload);
    rmSync(dir, { recursive: true, force: true });
  });

  it("leaves no .tmp file behind on success", async () => {
    const dir = mkdtempSync(join(tmpdir(), "sdp-install-"));
    await install(Buffer.from([0xcf, 0xfa, 0xed, 0xfe]), join(dir, "qjs"));
    expect(readdirSync(dir)).toEqual(["qjs"]);
    rmSync(dir, { recursive: true, force: true });
  });
});

describe("the universal macOS target needs three names per tool", () => {
  // The CI failure this exists for:
  //   resource path `binaries/yt-dlp-aarch64-apple-darwin` doesn't exist
  // `tauri build --target universal-apple-darwin` runs cargo once per
  // architecture, and tauri-build asks for the external binaries by `TARGET`,
  // so it wants the per-arch names; the bundler asks once at the end and wants
  // the universal one. Missing either half breaks the build.

  it("knows the two slices Tauri assembles universal-apple-darwin from", () => {
    expect(UNIVERSAL_SLICES).toEqual(["aarch64-apple-darwin", "x86_64-apple-darwin"]);
  });

  it("reads the name lipo wants out of a triple, not the Rust one", () => {
    // Rust says aarch64; lipo and the Mach-O fat headers say arm64.
    expect(lipoArchName("aarch64-apple-darwin")).toBe("arm64");
    expect(lipoArchName("x86_64-apple-darwin")).toBe("x86_64");
  });

  it("recognises the one target that is a bundle of slices", () => {
    expect(isUniversalTriple("universal-apple-darwin")).toBe(true);
    for (const native of [...UNIVERSAL_SLICES, "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]) {
      expect(isUniversalTriple(native)).toBe(false);
    }
  });

  it("names a file the way the build script will look for it", () => {
    // destinationPath is what both passes agree on: one suffix stripped, one
    // kept, depending on which of them is asking.
    expect(destinationPath("/out", "yt-dlp", "universal-apple-darwin")).toBe(
      "/out/yt-dlp-universal-apple-darwin",
    );
    for (const slice of UNIVERSAL_SLICES) {
      expect(destinationPath("/out", "yt-dlp", slice)).toBe(`/out/yt-dlp-${slice}`);
      expect(lipoArchName(slice)).toMatch(/^(arm64|x86_64)$/);
    }
  });

  it("the manifest's builtFrom lists exactly the slices Tauri assembles", () => {
    // qjs is a real example: it is lipo'd together, so its builtFrom IS the
    // slice list, and a drift between the two would ship a bundle that is not
    // universal while every name says it is.
    const problems = validateManifest(MANIFEST);
    expect(problems).toEqual([]);
    const qjs = MANIFEST.tools.find((t) => t.name === "qjs");
    expect([...qjs.platforms["universal-apple-darwin"].builtFrom].sort()).toEqual(
      [...UNIVERSAL_SLICES].sort(),
    );
  });

  it("catches a builtFrom that does not match the slices Tauri builds", () => {
    const broken = structuredClone(MANIFEST);
    const qjs = broken.tools.find((t) => t.name === "qjs");
    qjs.platforms["universal-apple-darwin"].builtFrom = ["aarch64-apple-darwin"];
    const problems = validateManifest(broken).join("\n");
    expect(problems).toContain("but universal-apple-darwin is assembled from");
  });

  it("catches a third slice that would be downloaded and thrown away", () => {
    const broken = structuredClone(MANIFEST);
    const qjs = broken.tools.find((t) => t.name === "qjs");
    qjs.platforms["universal-apple-darwin"].builtFrom = [
      "aarch64-apple-darwin",
      "x86_64-apple-darwin",
      "armv7-apple-darwin",
    ];
    expect(validateManifest(broken).join("\n")).toContain(
      "but universal-apple-darwin is assembled from",
    );
  });

  it("leaves a native target alone: it has no slices to reconcile", () => {
    const native = structuredClone(MANIFEST);
    const qjs = native.tools.find((t) => t.name === "qjs");
    expect(validateManifest(native)).toEqual([]);
    expect(qjs.platforms["aarch64-apple-darwin"].builtFrom ?? []).toEqual([]);
  });
});

describe("finding the latest tag without the API", () => {
  // On 2026-10-13 `bundle (macos-arm64)` got HTTP 403 from the releases API
  // (60/hour per IP, and GitHub-hosted runners share ranges), fell back to the
  // `latest/download/` alias, and that alias answered 404 — three times out of
  // three, while the explicit-tag URL returned 206 for the same asset. So the
  // fallback was not merely unlucky, it was pointing at something broken.
  //
  // The way out that is still measurable: the releases *page* answers a 302 whose
  // `Location:` carries the tag, and that needs no API and no quota.

  const API = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";

  it("derives the releases page from the API URL", () => {
    expect(releasesPageUrl(API)).toBe("https://github.com/yt-dlp/yt-dlp/releases/latest");
  });

  it("refuses an API URL it cannot derive a page from", () => {
    expect(() => releasesPageUrl("https://example.invalid/releases/latest")).toThrow(
      /not a GitHub releases API URL/,
    );
  });

  it("reads the tag out of the redirect Location", () => {
    // Verbatim `curl -sI` on 2026-10-13.
    expect(
      tagFromLocation("https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19"),
    ).toBe("2026.08.19");
  });

  it("ignores a Location that is not a release tag", () => {
    for (const bad of [
      null,
      "",
      "https://github.com/yt-dlp/yt-dlp",
      "https://github.com/login?return_to=%2Fyt-dlp%2Fyt-dlp",
      // `.` and `..` pass every character test while still walking the URL back
      // up a directory. Found by this test, not by reading the code.
      "https://github.com/yt-dlp/yt-dlp/releases/tag/../../etc",
      "https://github.com/yt-dlp/yt-dlp/releases/tag/..",
      "https://github.com/yt-dlp/yt-dlp/releases/tag/.",
    ]) {
      expect(tagFromLocation(bad)).toBeNull();
    }
  });

  it("takes the tag segment and drops whatever follows it", () => {
    // Not `null`, and that is the right answer: the capture stops at the first
    // `/`, so the tag comes back clean and the trailing junk is discarded
    // rather than pasted into the download URL. The first version of this test
    // asserted `null` here and was wrong — the property that matters is that
    // what comes back never contains a slash.
    const tag = tagFromLocation(
      "https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19/../../etc",
    );
    expect(tag).toBe("2026.08.19");
    expect(tag).not.toContain("/");
  });

  it("decodes a percent-encoded tag rather than pasting it into the URL", () => {
    expect(tagFromLocation("https://github.com/o/r/releases/tag/v1.0%2Bbuild")).toBe("v1.0+build");
    // …and refuses one that decodes into something that is not a tag.
    expect(tagFromLocation("https://github.com/o/r/releases/tag/2026.08.19%2F..%2Fevil")).toBeNull();
  });

  it("the manifest carries the tag as a placeholder, not as a literal 'latest'", () => {
    // The whole fallback depends on this: if the template said `latest` there
    // would be nowhere to put the tag the redirect gave us.
    for (const tool of MANIFEST.tools.filter((t) => t.versioning === "latest")) {
      expect(tool.releaseUrl).toContain("{tag}");
      expect(tool.checksumUrl).toContain("{tag}");
      expect(tool.releaseUrl).not.toContain("/download/latest/");
      expect(tool.checksumUrl).not.toContain("/download/latest/");
    }
  });

  it("substituting the tag yields a URL that is https and has no placeholder left", () => {
    const tool = MANIFEST.tools.find((t) => t.versioning === "latest");
    const url = tool.releaseUrl.replace("{tag}", "2026.08.19").replace("{asset}", "yt-dlp_macos");
    expect(url).toBe("https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos");
    expect(url).not.toMatch(/\{[a-z]+\}/);
    expect(url.startsWith("https://")).toBe(true);
  });
});

describe("placement", () => {
  it("adds .exe only on Windows", () => {
    expect(exeSuffix("x86_64-pc-windows-msvc")).toBe(".exe");
    expect(exeSuffix("x86_64-unknown-linux-gnu")).toBe("");
    expect(exeSuffix("universal-apple-darwin")).toBe("");
  });

  it("names the file the way bundle.externalBin expects: <name>-<triple>", () => {
    expect(destinationPath("/out", "qjs", "x86_64-unknown-linux-gnu")).toBe(
      join("/out", "qjs-x86_64-unknown-linux-gnu"),
    );
    expect(destinationPath("/out", "yt-dlp", "aarch64-apple-darwin")).toBe(
      join("/out", "yt-dlp-aarch64-apple-darwin"),
    );
    expect(destinationPath("/out", "qjs", "universal-apple-darwin")).toBe(
      join("/out", "qjs-universal-apple-darwin"),
    );
    expect(destinationPath("/out", "qjs", "x86_64-pc-windows-msvc")).toBe(
      join("/out", "qjs-x86_64-pc-windows-msvc.exe"),
    );
  });

  it("uses the tool name, not the asset name (spec §5.2.1)", () => {
    // yt-dlp's asset is `yt-dlp_linux` but the bundled file must be
    // `yt-dlp-<triple>`, because that is what `externalBin: ["binaries/yt-dlp"]`
    // resolves to. The manifest's `executableName` is the *installed* name used
    // for the app-data copy, and on Windows it already carries `.exe`.
    const ytdlp = MANIFEST.tools.find((tool) => tool.name === "yt-dlp");
    expect(destinationPath("bin", "yt-dlp", "x86_64-unknown-linux-gnu")).toBe(
      join("bin", "yt-dlp-x86_64-unknown-linux-gnu"),
    );
    expect(ytdlp.platforms["x86_64-unknown-linux-gnu"].asset).toBe("yt-dlp_linux");
    expect(ytdlp.platforms["x86_64-unknown-linux-gnu"].executableName).toBe("yt-dlp");

    // The Windows installed name keeps `.exe`, and must not double up.
    const qjs = MANIFEST.tools.find((tool) => tool.name === "qjs");
    expect(qjs.platforms["x86_64-pc-windows-msvc"].executableName).toBe("qjs.exe");
    expect(destinationPath("bin", "qjs", "x86_64-pc-windows-msvc")).toBe(
      join("bin", "qjs-x86_64-pc-windows-msvc.exe"),
    );
  });
});