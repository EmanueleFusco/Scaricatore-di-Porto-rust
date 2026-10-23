# Acceptance checklist — spec §15

This file records the state of the 16 acceptance items in
`docs/RUST_PORT_GUIDELINES.md` §15 at the end of M7 (2026-10-23, branch
`m7-polish`). It is deliberately a status report, not a release certificate:
the spec requires every item to pass on every Tier 1 target, and only Linux x64
was available for manual testing here.

## Legend

- **Verified here** — a command or manual check was actually run on this
  machine, Linux x64 / CachyOS / KDE Wayland.
- **Automated only** — the offline unit/integration suite passes, but the real
  service, live site, account or hardware was not exercised.
- **Open** — not run here; needs Windows/macOS hardware, a live account, or a
  published release.
- **Blocked** — an external prerequisite is missing (the `sdp-whisper` release,
  a signed M7 release, or a Gemini API key).

## Checklist

| # | Spec item | Status on this machine | Evidence and remaining gap |
| --: | :-- | :-- | :-- |
| 1 | Release build, clippy/fmt clean; app binary ≤ 15 MB; installer budget; idle Rust RSS ≲ 30 MB | **Partial — RSS target missed** | Linux bundle built and signed; app binary **8.5 MiB**; `.deb`/`.rpm` **43.2 MiB**, AppImage **148.9 MiB** reported, no hard installer gate (D132). Main process idle: **191,920 kB RSS / 87,832 kB PSS / 44,628 kB RssAnon**; the 30 MB target is not met (D133). |
| 2 | Fresh machine with no tools: bundled yt-dlp + qjs work; wizard installs ffmpeg with checksums; Deno/sidecar/models on demand; public 1080p download | **Partial** | `bundled_tools` integration tests pass; the ignored real ffmpeg install test downloaded, verified and ran **7.0.2**; the wizard install/retry/cancel flow was checked manually. Live YouTube inspection/download passed on the zoo video (144p). A public **1080p** download and a truly fresh machine were not tested. |
| 3 | JS runtime self-test with QuickJS; automatic Deno fallback and remembered result | **Partial — QuickJS verified live** | `the_self_test_passes_with_the_bundled_tools` passed: QuickJS, 24 formats, `[240, 144]`, 2.1 s. The Deno fallback path is covered by fake-tool tests; a real Deno fallback was not triggered live in M7. |
| 4 | 4K/Shorts resolution listing and merge; audio-only `.opus` | **Automated only** | Format-selector and merge tests pass. The live download produced an `.mkv` and an `.opus` at the test video's available heights; no 4K or Shorts source was tested. |
| 5 | 100-item playlist: selection, folder/numbering, archive, deleted entry | **Automated only** | Playlist parsing, selection and archive tests pass; no live 100-item playlist was run in M7. |
| 6 | Non-YouTube: generic page, HLS/direct, audio-only, cookies, live/DRM, impersonation retry | **Automated only** | Offline fake-tool tests cover the selectors, fallbacks and messages; no live non-YouTube source or cookies-protected page was tested in M7. |
| 7 | Queue: parallel, per-host cap, pause/resume, retry, reorder, restart recovery | **Verified on Linux** | Integration tests pass and the queue, filter, selection, bulk actions and shortcuts were checked manually on Linux/KDE. Windows and macOS were not run. |
| 8 | Encode/trim: audio formats, CPU/hardware encoders, fallback, trim tolerance, originals kept | **Automated only** | ffmpeg integration tests pass, including forced-failure fallback and trim validation. Hardware encoders were not available/tested; no real CPU encode was run manually in M7. |
| 9 | Whisper: every model download/resume/verify/select/delete, IT/EN quality, SRT/VTT, >2 h chunking, import, cancel | **Partial / blocked** | Model download, resume, checksum and chunking tests pass; the wizard downloaded the recommended model and a local file was transcribed manually on Linux. The `sdp-whisper` release assets are still 404, so the sidecar install cannot be tested end-to-end, and not every model size or a >2 h file was exercised. |
| 10 | Subtitles-first: manual, auto captions, duplicate cleaning, Whisper fallback | **Automated only** | Subtitle cleaner and pipeline tests pass; no live YouTube subtitle download was run in M7. |
| 11 | Gemini: keyring/0600 storage, dynamic models, retired model, cloud-file cleanup, summary | **Partial** | Storage, model filtering, cleanup-guard and summary tests pass; the invalid-key banner was checked manually. No live Gemini request was made (no API key available). |
| 12 | Cancel kills the process tree within ~2 s; closing the app leaves no orphans | **Verified on Linux (automated)** | Process-tree, cancellation and orphan tests pass on Linux. The manual cancel path was not re-run in M7, and Windows Job Objects/macOS process groups were not exercised on hardware. |
| 13 | Localized error messages for the listed failure cases | **Partial** | Dictionary/error-classifier tests pass and the Gemini error banner was checked manually. The other listed cases (no ffmpeg, no JS runtime, disk full, age-restricted, etc.) were not each reproduced in the UI. |
| 14 | yt-dlp update without restart; app updater from a signed `latest.json` | **Partial / blocked** | yt-dlp update tests pass. A local signed Linux bundle produced `.deb.sig`, `.rpm.sig`, `.AppImage.sig`; `latest.json` was generated from the real `.AppImage.sig` with four platform entries. No signed M7 release exists yet, so a real update cannot be checked. |
| 15 | Theme follows the OS on Windows/macOS/GNOME/KDE; manual override; macOS clipboard shortcuts; full keyboard use | **Partial** | System/light/dark and keyboard navigation were checked manually on KDE Wayland. GNOME, Windows, macOS and the macOS clipboard shortcuts were not tested. |
| 16 | Accented/non-Latin filenames; no Windows console flashes; no secrets in logs | **Partial** | Filename sanitising tests pass, including accented text; diagnostics is tested to contain no API key/cookie. No Windows console-window check was possible. |

## Verification run

These are the checks from the end of M7. Exact totals are repeated in the
`docs/PROGRESS.md` M7 block.

```text
( cd src-tauri && cargo fmt --all -- --check )                  pass
( cd src-tauri && cargo clippy --all-targets -- -D warnings )   pass, 0 warnings
( cd src-tauri && cargo test )                                  436 unit + 193 integration = 629 passed, 0 failed, 4 ignored
npm run check                                                    0 errors, 0 warnings
npm test -- --run                                                16 files, 333 tests passed
npm run build                                                    pass
npm run check:ci                                                 24 run blocks valid
npm run check:runners                                            pass
npm run check:tools                                              pass
npm run check:release -- v0.1.0                                  pass
npx tauri build (local Linux, updater key)                       pass
node scripts/generate-latest-json.mjs ...                        4 platforms generated
```

Real, ignored checks run by hand on 2026-10-23:

```text
cargo test --test real_selftest the_self_test_passes_with_the_bundled_tools \
  -- --ignored --nocapture
  -> ok: QuickJS, 24 formats, heights [240, 144], 2137 ms

cargo test --test real_selftest a_real_inspection_and_download_end_to_end \
  -- --ignored --nocapture
  -> ok: "Me at the zoo" inspected in 2093 ms, .mkv downloaded in 2835 ms,
     .opus produced
```

## What is still needed for a full §15 sign-off

1. Run the manual checklist on Windows 10/11 x64 and macOS 13+ (Intel and
   Apple Silicon) on real hardware.
2. Test a public 1080p/4K video, a Shorts video, a 100-item playlist, at least
   five non-YouTube sources, a cookies-protected page and a live/DRM case.
3. Test with a real Gemini API key, including a retired model name and cloud
   file deletion after success, error and cancel.
4. Publish the `sdp-whisper` release assets, then install the sidecar from the
   app and transcribe with every catalogue model.
5. Publish a signed M7 release with `latest.json`, then exercise the updater
   from an installed AppImage (and confirm the `.deb`/`.rpm` releases-link
   behaviour).
6. Decide what to do about the Linux idle-RSS target (D133): repeat on a clean
   VM, reduce the WebKitGTK baseline, or revise the target with the owner.
7. Complete macOS notarization and Windows code signing; both need human
   credentials and were not attempted here.
