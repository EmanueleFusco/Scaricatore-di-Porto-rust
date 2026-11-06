# SPIKES

Manual checks that **only the owner** can perform because they require signing keys, accounts,
target hardware, or human UX judgement. Empty result cells mean "not checked yet" (never "assumed
OK").

References: spec §5.2.1 (notarization and antivirus), §1 (size budget), §7.4 (crash recovery), §14
row M0 (exit criteria).

---

## S1 — macOS: sign and notarize the app **with** bundled `yt-dlp` and `qjs`

**Why.** `yt-dlp_macos` is a PyInstaller binary; spec §5.2.1 requires verifying that it notarizes
cleanly inside the `.app` bundle. If notarization fails and cannot be fixed with entitlements, the
spec fallback is: **do not bundle `yt-dlp` on macOS and download it on first launch**.

**How.**

1. Fetch bundled tools: `node scripts/fetch_bundled_tools.mjs --target universal-apple-darwin`
2. Build: `npx tauri build --target universal-apple-darwin`
3. Sign with `codesign --options runtime --timestamp` using the Developer ID identity, then
   `xcrun notarytool submit ... --wait` and `stapler staple`.
4. Verify that `Contents/MacOS/yt-dlp` and `Contents/MacOS/qjs` are signed alongside the main
   binary (`tauri-bundler` only signs external binaries when signing is active).

| Question | Result |
| :-- | :-- |
| Does `yt-dlp_macos` sign with hardened runtime? | |
| Are extra entitlements required for `yt-dlp_macos`? Which ones? | |
| Does `notarytool` accept the `.app` **containing** both bundled binaries? | |
| Does universal `qjs` (`lipo` of `arm64` + `x86_64`) sign and pass? | |
| Signed `.dmg` / `.app.zip` size? | |
| **If it fails:** adopt the "download on first launch on macOS" fallback? | |

**Candidate entitlements if notarization fails** (spec §5.2.1):

- `com.apple.security.cs.allow-unsigned-executable-memory` — QuickJS JIT / runtime codegen
- `com.apple.security.cs.disable-library-validation` — PyInstaller loading extracted libraries

---

## S2 — Windows: signed NSIS installer on a clean machine with Defender & SmartScreen enabled

**Why.** Spec §5.2.1 notes that PyInstaller executables can trigger antivirus / SmartScreen
heuristics.

**How.**

1. Download the `windows-x64` NSIS `.exe` installer from CI (`bundle (windows-x64)`).
2. Sign it with a code-signing certificate (`bundle.windows.certificateThumbprint` or CI secret).
3. Copy to a clean Windows machine with Defender and SmartScreen enabled, install, and launch.

| Question | Result |
| :-- | :-- |
| Does the installer open without "Windows protected your PC"? | |
| Does Defender flag anything during installation? | |
| How long does the first launch / inspection take (PyInstaller unpack)? | |
| Does the app find the bundled `yt-dlp.exe` and `qjs.exe`? | |
| Is `yt-dlp.exe` quarantined by Defender? | |

---

## S3 — `yt-dlp` startup cost: one-file PyInstaller bundle vs unpacked directory variant

**Why.** Spec §5.2.1 notes that official one-file PyInstaller builds unpack on every run. Because we
cache tool resolution per session (D75), `yt-dlp` is only spawned once per inspection/download. If
the unpack cost is still noticeable (especially on Windows with Defender), evaluate the unpacked
zip variant (`yt-dlp_linux.zip` / `yt-dlp_win.zip`, which lacks `-U` self-update and would require
folder replacement in `tools_update_ytdlp`).

```bash
time ./src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu --version
for i in 1 2 3 4 5; do
  /usr/bin/time -f "%e s" ./src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu --version >/dev/null
done
```

| Platform | One-file, 1st run | One-file, avg of 5 | Unpacked zip, avg of 5 |
| :-- | :-- | :-- | :-- |
| Linux x64 | | 0.46 s (D75) | |
| Windows x64 | | | |
| macOS arm64 | | | |

---

## S4 — Installer size budget (spec §1, D132)

**Status (closed in M7 / D132).** Spec §1 fixes the **application binary** at **≤ 15 MB** and
excludes the Linux AppImage (which embeds WebKitGTK). Linux release measurements with the M7
updater enabled (2026-10-23, `ubuntu-24.04` / local build):

| Artifact | Size | Budget status |
| :-- | --: | :-- |
| Application binary (`scaricatore-di-porto`) | **8.5 MiB** (4.70 MiB pre-M7) | ✅ Under spec §1 target (≤ 15 MB) |
| `.deb` / `.rpm` (includes `yt-dlp` 38.6 MiB + `qjs` 2.4 MiB) | **43.2 MiB** | Reported by CI; no hard failure gate (D132) |
| `.AppImage` | **148.9 MiB** | Excluded by spec §1 (embeds WebKitGTK) |
| Windows NSIS `*-setup.exe` | | Reported by CI `bundle (windows-x64)` |
| macOS universal `.dmg` / `.app.zip` | | Reported by CI `bundle-macos-universal` |

---

## S5 — Manual cancellation check against a live `yt-dlp` process tree

**Why.** `tests/process_tree.rs` verifies tree termination using `fake_tool`. This check confirms
that cancelling a real in-flight download kills `yt-dlp` and any child `ffmpeg` / `qjs` / `deno`
processes within ~2 s and leaves no orphans when closing the app mid-download.

| System | `yt-dlp` left? | `ffmpeg` left? | `qjs`/`deno` left? | Killed within ~2 s? | Orphans on app quit? |
| :-- | :-- | :-- | :-- | :-- | :-- |
| Windows x64 | | | | | |
| Linux x64 | | | | | |
| macOS arm64 | | | | | |

---

## S6 — Tier 2 targets (`.github/workflows/tier2.yml`, spec §1.1)

Experimental CI-only targets (`workflow_dispatch`):

- [ ] macOS Intel (`macos-15-intel`): build + fake-tool tests green?
- [ ] Windows ARM64 (`windows-11-arm`): build + fake-tool tests green?
- [ ] Linux arm64 (`ubuntu-24.04-arm`): build + fake-tool tests green?
- [ ] Missing platform tool assets cleanly reported as "not available on this platform" in UI (D126)?

---

## S7 — Mid-download crash recovery: item comes back `Interrupted` (spec §7.4, D91)

**Why.** `start_item` persists `Downloading` to `queue.json` before spawning the child process (D91).
Start a real download, wait until progress/speed is updating, then force-kill the app (`kill -9` on
Linux/macOS or **End task** in Windows Task Manager) and relaunch.

| System | `queue.json` state after kill | Row state on relaunch | Auto-started on its own? (must be No) | `.part` files preserved & resumed on **Resume**? |
| :-- | :-- | :-- | :-- | :-- |
| Linux x64 | | | | |
| Windows x64 | | | | |
| macOS arm64 | | | | |
