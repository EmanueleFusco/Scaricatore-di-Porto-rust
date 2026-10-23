// Tests for the shell inside the CI steps.
//
// Why this file exists: on 2026-10-13 the `bundle (macos universal)` job failed
// with
//
//     resource path `binaries/yt-dlp-aarch64-apple-darwin` doesn't exist
//
// because the build script of a universal pass asks for the per-architecture
// names, and only the universal one was being fetched. The step written to
// catch that class of mistake (`lipo -info` on one file) had never been run on
// the machine that wrote it, and when it finally ran it turned out to contain
// a bug of its own — a substring trap that is invisible to the eye.
//
// A shell step nobody executes is not a check. These tests execute it, on
// Linux, on every push, against a fake `lipo` that reports what the *contents*
// of a file are rather than what its name promises — which is the only way to
// test the thing that matters: a file lying about its own name.
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const STEP_NAME = "Every bundled file is the architecture its name promises";

/** The shell of one named step, taken from workflow file exactly as the runner gets it. */
function stepScript(name, file = "ci.yml") {
  const yaml = readFileSync(join(REPO_ROOT, ".github", "workflows", file), "utf8");
  const at = yaml.indexOf(`- name: ${name}`);
  if (at === -1) throw new Error(`${file} has no step called "${name}"`);
  const runAt = yaml.indexOf("run: |", at);
  if (runAt === -1 || runAt > yaml.indexOf("- name:", at + 1)) {
    throw new Error(`step "${name}" does not use a run: | block`);
  }
  const lines = yaml.slice(runAt + "run: |".length).split("\n").slice(1);
  const indent = lines.find((l) => l.trim().length > 0).match(/^ */)[0].length;
  const body = [];
  for (const line of lines) {
    if (line.trim().length === 0) continue;
    if (line.match(/^ */)[0].length < indent) break;
    body.push(line.slice(indent));
  }
  return body.join("\n");
}

/**
 * A `lipo` whose answer comes from the first line of the file it is asked about,
 * so a test can make a file whose name and contents disagree.
 */
const FAKE_LIPO = `#!/usr/bin/env bash
first="$(head -1 "$2" 2>/dev/null)"
case "$first" in
  FAT\\ *)    echo "Architectures in the fat file: $2 are: \${first#FAT }" ;;
  SINGLE\\ *) echo "Non-fat file: $2 is architecture: \${first#SINGLE }" ;;
  *)         echo "lipo: $2: unhandled" >&2; exit 1 ;;
esac
`;

let root;

beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), "sdp-ci-step-"));
  const bin = join(root, "bin");
  const work = join(root, "work", "src-tauri", "binaries");
  mkdirSync(bin, { recursive: true });
  mkdirSync(work, { recursive: true });
  writeFileSync(join(bin, "lipo"), FAKE_LIPO);
  chmodSync(join(bin, "lipo"), 0o755);
  writeFileSync(join(root, "step.sh"), stepScript(STEP_NAME));
});

afterAll(() => {
  if (root) rmSync(root, { recursive: true, force: true });
});

/** Rebuild the six files a universal fetch produces, all telling the truth. */
function truth() {
  const dir = join(root, "work", "src-tauri", "binaries");
  for (const tool of ["yt-dlp", "qjs"]) {
    writeFileSync(join(dir, `${tool}-universal-apple-darwin`), "FAT x86_64 arm64\n");
    writeFileSync(join(dir, `${tool}-aarch64-apple-darwin`), "SINGLE arm64\n");
    writeFileSync(join(dir, `${tool}-x86_64-apple-darwin`), "SINGLE x86_64\n");
  }
  return dir;
}

/** Make one file say something else, or stop existing. */
function fake(dir, file, content) {
  if (content === null) rmSync(join(dir, file));
  else writeFileSync(join(dir, file), `${content}\n`);
}

function run() {
  const result = spawnSync("bash", [join(root, "step.sh")], {
    cwd: join(root, "work"),
    encoding: "utf8",
    env: { ...process.env, PATH: `${join(root, "bin")}:${process.env.PATH}` },
  });
  return { code: result.status, output: `${result.stdout}${result.stderr}` };
}

// bash is the runner's shell on Linux and macOS; there is nothing to test on
// Windows, and pretending otherwise would be the kind of green that means
// nothing.
describe.skipIf(process.platform === "win32")(
  `the CI step "${STEP_NAME}"`,
  () => {
    it("passes when every file is what its name says", () => {
      truth();
      const { code, output } = run();
      expect(output).not.toContain("::error::");
      expect(code).toBe(0);
    });

    it("fails when a per-architecture file is missing", () => {
      const dir = truth();
      // The exact CI failure: the build script asked for this and it was not
      // there. Without it in place, the job fails three steps later with a
      // tauri-build error that says nothing about which of six files is wrong.
      fake(dir, "yt-dlp-aarch64-apple-darwin", null);
      const { code, output } = run();
      expect(code).toBe(1);
      expect(output).toContain("yt-dlp-aarch64-apple-darwin non esiste");
    });

    it("fails when the universal file is not universal", () => {
      const dir = truth();
      fake(dir, "qjs-universal-apple-darwin", "SINGLE arm64");
      const { code, output } = run();
      expect(code).toBe(1);
      expect(output).toContain("qjs-universal-apple-darwin");
    });

    it("fails when the universal file lacks one of the two architectures", () => {
      const dir = truth();
      fake(dir, "yt-dlp-universal-apple-darwin", "FAT x86_64");
      const { code, output } = run();
      expect(code).toBe(1);
      expect(output).toContain("non contiene arm64");
    });

    it("fails when a slice is really a fat binary", () => {
      const dir = truth();
      fake(dir, "yt-dlp-aarch64-apple-darwin", "FAT x86_64 arm64");
      const { code, output } = run();
      expect(code).toBe(1);
      // Rejected by the `is architecture:` match, not by a second "is it fat?"
      // check: `lipo -info` on a fat binary never prints `is architecture:`.
      // That is why the step has no such second check — an unreachable branch
      // is an assertion nobody can exercise.
      expect(output).toContain("doveva essere solo arm64");
    });

    it("fails when a slice is the wrong architecture", () => {
      const dir = truth();
      fake(dir, "qjs-aarch64-apple-darwin", "SINGLE x86_64");
      const { code, output } = run();
      expect(code).toBe(1);
      expect(output).toContain("doveva essere solo arm64");
    });

    it("does not mistake `Non-fat file` for a fat one", () => {
      // The bug this whole file was written for, kept as a test: `Non-fat file:`
      // contains the substring `fat file`, so the first version of the step
      // rejected the very files it was supposed to accept, and it did so on the
      // only run it ever had.
      const dir = truth();
      fake(dir, "qjs-x86_64-apple-darwin", "SINGLE x86_64");
      const { code, output } = run();
      expect(output).not.toContain("::error::");
      expect(code).toBe(0);
    });
  },
);
// The release must carry installers and nothing else (D111).
//
// The bundlers leave the *unpacked* package next to the installer
// (`bundle/deb/<name>/data/usr/…`, `bundle/appimage/<name>.AppDir/`,
// `bundle/squashfs-root/`), and the upload globs used to be `**`: a local build
// shows the 42 MB `.deb` arriving as a 127 MB artifact and the 148 MB AppImage
// as a 251 MB one, and the flatten step would have moved those trees into a
// draft release as well. The globs name files now; this executes the guard that
// keeps a release that way.
describe.skipIf(process.platform === "win32")(
  'the CI step "Only installers reached the release"',
  () => {
    const STEP = "Only installers reached the release";

    /** What the four upload steps produce, plus the updater's files of M7. */
    const INSTALLERS = [
      "Scaricatore Di Porto_0.1.0_amd64.deb",
      "Scaricatore Di Porto-0.1.0-1.x86_64.rpm",
      "Scaricatore Di Porto_0.1.0_amd64.AppImage",
      "Scaricatore Di Porto_0.1.0_amd64.AppImage.tar.gz",
      "Scaricatore Di Porto_0.1.0_amd64.AppImage.tar.gz.sig",
      "macos-universal-Scaricatore Di Porto.app.zip",
      "Scaricatore Di Porto.app.tar.gz",
      "Scaricatore Di Porto.app.tar.gz.sig",
      "Scaricatore Di Porto_0.1.0_aarch64.dmg",
      "Scaricatore Di Porto_0.1.0_x64-setup.exe",
      "SHA256SUMS",
      "latest.json",
    ];

    let root;
    let assets;

    beforeAll(() => {
      root = mkdtempSync(join(tmpdir(), "sdp-release-step-"));
      assets = join(root, "assets");
      writeFileSync(join(root, "step.sh"), stepScript(STEP));
    });

    afterAll(() => {
      if (root) rmSync(root, { recursive: true, force: true });
    });

    /** `assets/` as the upload steps leave it, before the flatten step runs. */
    function release(files = INSTALLERS) {
      rmSync(assets, { recursive: true, force: true });
      for (const name of files) {
        mkdirSync(dirname(join(assets, name)), { recursive: true });
        writeFileSync(join(assets, name), "x");
      }
      return assets;
    }

    function runGuard() {
      const result = spawnSync("bash", [join(root, "step.sh")], {
        cwd: root,
        encoding: "utf8",
      });
      return { code: result.status, output: `${result.stdout}${result.stderr}` };
    }

    it("passes when assets/ holds installers and nothing else", () => {
      release();
      const { code, output } = runGuard();
      expect(output).not.toContain("::error::");
      expect(code).toBe(0);
    });

    it("fails on the unpacked application binary a bundler leaves behind", () => {
      // Exactly what the flatten step does to `deb/<name>/data/usr/bin/…` when
      // the artifact carried the staging tree: the binary ends up beside the
      // installers, with the name of the application.
      release([...INSTALLERS, "scaricatore-di-porto"]);
      const { code, output } = runGuard();
      expect(code).toBe(1);
      expect(output).toContain("not installers");
      expect(output).toContain("scaricatore-di-porto");
    });

    it("fails on a nested staging file too", () => {
      release([...INSTALLERS, "usr/share/icons/hicolor/256x256/apps/scaricatore.png"]);
      const { code, output } = runGuard();
      expect(code).toBe(1);
      expect(output).toContain("usr/share/icons/hicolor/256x256/apps/scaricatore.png");
    });
  },
);

describe("the updater wiring in ci.yml (spec §13.4)", () => {
  const yaml = readFileSync(join(REPO_ROOT, ".github", "workflows", "ci.yml"), "utf8");

  /** The YAML of one step, from its `- name:` to the next step. */
  function step(name) {
    const at = yaml.indexOf(`- name: ${name}`);
    if (at === -1) throw new Error(`ci.yml has no step called "${name}"`);
    const next = yaml.indexOf("- name:", at + 1);
    return yaml.slice(at, next === -1 ? undefined : next);
  }

  it("signs both the native and the universal build", () => {
    for (const name of ["Build the bundle", "Build the universal bundle"]) {
      const block = step(name);
      expect(block, name).toContain("TAURI_SIGNING_PRIVATE_KEY");
      expect(block, name).toContain("TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
    }
  });

  it("fails early when the signing key is not configured", () => {
    expect(yaml.match(/- name: Check the updater signing key/g)).toHaveLength(2);
  });

  it("uploads the Windows and macOS signatures", () => {
    expect(step("Upload the bundle")).toContain("nsis/*.exe*");
    expect(step("Upload the universal bundle")).toContain("macos/*.app.tar.gz*");
  });

  it("generates latest.json before creating the draft release", () => {
    const generate = yaml.indexOf("- name: Generate the updater manifest");
    const release = yaml.indexOf("- name: Create the draft release");
    expect(generate).toBeGreaterThan(-1);
    expect(release).toBeGreaterThan(generate);
    expect(step("Generate the updater manifest")).toContain(
      "node scripts/generate-latest-json.mjs --assets assets",
    );
  });

  it("checks out the repository so the generator can run", () => {
    const releaseJob = yaml.slice(yaml.indexOf("  release:"));
    expect(releaseJob).toContain("actions/checkout@v7");
  });
});

describe("the Whisper sidecar CI workflow (.github/workflows/whisper-sidecar.yml)", () => {
  const WORKFLOW_PATH = join(REPO_ROOT, ".github", "workflows", "whisper-sidecar.yml");

  it("does not produce colliding binary artifact names across build jobs", () => {
    const yaml = readFileSync(WORKFLOW_PATH, "utf8");
    // Find all target binary names created into ../dist/ across build steps
    const matches = [
      ...yaml.matchAll(/(?:cp\s+\S+\s+\.\.\/dist\/|-output\s+\.\.\/dist\/)(sdp-whisper[^\s]+)/g),
    ].map((m) => m[1]);

    const counts = new Map();
    for (const name of matches) {
      counts.set(name, (counts.get(name) || 0) + 1);
    }
    const duplicates = [...counts.entries()]
      .filter(([_, c]) => c > 1)
      .map(([name, c]) => `${name} (${c} occurrences)`);

    expect(
      duplicates,
      `Colliding binary artifacts found in whisper-sidecar.yml: ${duplicates.join(", ")}`,
    ).toEqual([]);
  });

  it("specifies needs including build-tier1 and build-tier2 for packaging", () => {
    const yaml = readFileSync(WORKFLOW_PATH, "utf8");
    expect(yaml).toMatch(/needs:\s*\[\s*build-tier1\s*,\s*build-tier2\s*\]/);
  });

  it("has contents: write permission and publishes releases via gh release", () => {
    const yaml = readFileSync(WORKFLOW_PATH, "utf8");
    expect(yaml).toMatch(/permissions:\s*\n\s*contents:\s*write/);
    expect(yaml).toContain("gh release upload");
    expect(yaml).toContain("gh release create");
  });

  it("configures GGML_NATIVE=ON for Windows ARM64 MSVC build step", () => {
    const yaml = readFileSync(WORKFLOW_PATH, "utf8");
    const winArmMatch = yaml.match(
      /name:\s*Build whisper-cli \(Windows ARM64\)[\s\S]*?(?=\n\s*-\s*name:|\n\s{2,4}[a-zA-Z0-9_-]+:|$)/,
    );
    expect(winArmMatch).not.toBeNull();
    expect(winArmMatch[0]).toContain("-DGGML_NATIVE=ON");
  });

  describe.skipIf(process.platform === "win32")(
    'the CI step "Collect and compute SHA256SUMS"',
    () => {
      let root;
      let incoming;
      let scriptPath;

      beforeAll(() => {
        root = mkdtempSync(join(tmpdir(), "sdp-whisper-collect-"));
        incoming = join(root, "incoming");
        scriptPath = join(root, "step.sh");
        writeFileSync(
          scriptPath,
          stepScript("Collect and compute SHA256SUMS", "whisper-sidecar.yml"),
        );
      });

      afterAll(() => {
        if (root) rmSync(root, { recursive: true, force: true });
      });

      function runCollect() {
        const result = spawnSync("bash", [scriptPath], {
          cwd: root,
          encoding: "utf8",
        });
        return { code: result.status, output: `${result.stdout}${result.stderr}` };
      }

      it("fails if duplicate artifact names are found across downloaded job artifacts", () => {
        rmSync(join(root, "release-assets"), { recursive: true, force: true });
        rmSync(incoming, { recursive: true, force: true });
        mkdirSync(join(incoming, "job1"), { recursive: true });
        mkdirSync(join(incoming, "job2"), { recursive: true });
        writeFileSync(join(incoming, "job1", "sdp-whisper-x86_64-apple-darwin"), "metal-binary");
        writeFileSync(join(incoming, "job2", "sdp-whisper-x86_64-apple-darwin"), "cpu-binary");

        const { code, output } = runCollect();
        expect(code).not.toBe(0);
        expect(output).toContain("Duplicate artifact file found");
      });

      it("succeeds and produces SHA256SUMS when artifact names are unique", () => {
        rmSync(join(root, "release-assets"), { recursive: true, force: true });
        rmSync(incoming, { recursive: true, force: true });
        mkdirSync(join(incoming, "job1"), { recursive: true });
        mkdirSync(join(incoming, "job2"), { recursive: true });
        writeFileSync(join(incoming, "job1", "sdp-whisper-universal-apple-darwin"), "universal");
        writeFileSync(join(incoming, "job2", "sdp-whisper-x86_64-apple-darwin"), "x64");

        const { code, output } = runCollect();
        expect(code).toBe(0);
        const sums = readFileSync(join(root, "release-assets", "SHA256SUMS"), "utf8");
        expect(sums).toContain("sdp-whisper-universal-apple-darwin");
        expect(sums).toContain("sdp-whisper-x86_64-apple-darwin");
      });
    },
  );
});


