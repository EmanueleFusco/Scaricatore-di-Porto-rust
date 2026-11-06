# PROGRESS

Running log, one block per milestone/step, newest last (format defined in `AGENTS.md` §9; historical M0–M6 micro-steps consolidated and unified into English in D143).

---

## 2026-10-12 — M0-A / Project skeleton, settings, i18n, theme, shell UI, and initial CI (spec §1, §2.8, §3, §11)

- Done:
  - Repository layout prepared per `AGENTS.md` §3 (`docs/`, `src/`, `src-tauri/`, `scripts/`, `.gitignore`).
  - Backend (`src-tauri/src/`):
    - `error.rs`: `AppError` (`thiserror`) + camelCase `ErrorInfo { code, messageKey, detail }`.
    - `state.rs`: `Settings` with `serde(default)`, lenient enum deserialisation (D4), corrupt-file recovery to `.bak`, and `AppState` behind `RwLock`.
    - `services/paths.rs`: OS config/data/cache/download dirs, `private_temp_dir` (mode `0700` on Unix), `atomic_write` (temp + `fsync` + `rename` with per-write atomic counter, D92), `collision_free_stem`, and Unicode-preserving `sanitize_component()` (spec §12.5).
    - `commands/settings.rs`: `settings_get` and `settings_save`.
    - `lib.rs`: registered `single-instance` (first), `dialog`, and `opener` plugins (no `tauri-plugin-shell`, D2); release profile configured with `opt-level="s"`, `lto`, `codegen-units=1`, `panic="abort"`, `strip=true`.
  - Frontend (plain Vite + Svelte 5, D1):
    - `src/lib/i18n/`: `it.json` / `en.json`, `t()`, `tCount()`, OS-locale detection, and parity/usage tests.
    - `src/lib/styles/app.css`, `src/lib/theme.ts`, `src/lib/stores/{theme,settings}.svelte.ts`, `src/lib/router.svelte.ts`, `Sidebar.svelte`, `FooterStatus.svelte`, `Toast.svelte`, and Lucide inline SVG icons (`lucide-svelte`, D11).
  - Strict CSP (`script-src 'self'`) verified against production bundle via `scripts/serve-dist-with-csp.mjs`.
- Verified by running:
  - `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (37 passed), `npm run check` (0 errors), `npm test` (28 passed), `npm run build`, `npx tauri build --no-bundle` (4.58 MiB binary).
- Not verified / stubbed:
  - Manual Windows/macOS desktop checks (Linux host only).
- Spec items marked "verify" that I checked:
  - `tauri.conf.json` v2 schema (`minWidth`/`minHeight`, `csp`/`devCsp`, `bundle.externalBin`, `devtools` cargo feature).
  - `@tauri-apps/api` 2.12.3 `Window.theme()`, `onThemeChanged()`, `setTheme()` and permissions (`core:window:allow-theme`, `core:window:allow-set-theme`).
  - `std::fs::rename` atomic replacement on Windows (`MoveFileExW` + `MOVEFILE_REPLACE_EXISTING`).
- Decisions/deviations (also in DECISIONS.md): D1–D18.
- Next step: M0-B (process layer).

## 2026-10-13 — M0-B / Process layer and cross-OS tree termination (`services/process.rs`, spec §4)

- Done:
  - Implemented `src-tauri/src/services/process.rs`:
    - `ToolCommand` builder enforcing absolute executable paths, argv arrays only (no shell strings), `--` separator before URLs, `--ignore-config` for `yt-dlp`, isolated child temp directory (`TMP`/`TEMP`/`TMPDIR`), and `CREATE_NO_WINDOW` on Windows.
    - Process-tree lifecycle: Windows Job Object (`CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject` + `TerminateJobObject`, checking `ActiveProcesses == 0` first, D20/D22/D25); Unix process group (`process_group(0)` + `killpg(SIGTERM)` → 2 s grace → `killpg(SIGKILL)` with reaped-PID guard `forget_group()`, D21).
    - Concurrent stdout and stderr readers (`LineSinks`, D58) with 200-line ring buffer and `tokio::select!` cancellation via `CancellationToken` (never holding a mutex across `.await`).
    - Global running-process registry and app-exit hook killing all active trees on exit.
  - Added `src-tauri/windows-app-manifest.xml` and `cargo:rustc-link-arg-tests` in `src-tauri/build.rs` so Windows integration tests using `tauri::test::mock_builder()` link Common Controls v6 cleanly (D28, D124).
  - Added multi-call Rust test helper `fake_tool` and 16 integration tests in `src-tauri/tests/process_tree.rs`.
  - Isolated Tier 2 targets into `.github/workflows/tier2.yml` (D26) and added cross-OS `process-tests.yml`.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (80/80 green on Linux x64, Windows x64, and macOS arm64 CI runners).
- Not verified / stubbed:
  - Manual cancellation check against a live `yt-dlp` + `ffmpeg` tree on Windows/macOS (tracked in `docs/SPIKES.md` S5).
- Spec items marked "verify" that I checked:
  - `windows` 0.62 Job Object APIs (`CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`, `QueryInformationJobObject`, `TerminateJobObject`).
  - `tauri-build` / `embed-resource` `cargo:rustc-link-arg-bins` vs `cargo:rustc-link-arg-tests` behaviour on Windows MSVC.
- Decisions/deviations (also in DECISIONS.md): D19–D30.
- Next step: M0-C (tools manifest, bundled tools, provisioning, Tools UI).

## 2026-10-13 — M0-C / Tools manifest, bundled `yt-dlp` + `qjs`, provisioning, Tools UI, and CI guards (spec §5)

- Done:
  - Created `tools.manifest.json` (embedded via `include_str!`, D31) with pinned `yt-dlp` (`2026.08.19`) and `qjs` (`v0.17.0` QuickJS-NG) assets and SHA-256 checksums across Linux, Windows, and macOS (including universal macOS `builtFrom` slice assembly via `lipo`, D32/D50).
  - Implemented `scripts/fetch_bundled_tools.mjs` (`npm run fetch:tools` and `npm run check:tools`) with GitHub API + `GITHUB_TOKEN` + `/releases/latest` HTTP 302 redirect tag fallback (D33, D54) and Mach-O / `lipo -info` verification.
  - Implemented `src-tauri/src/services/manifest.rs`, `services/download.rs` (`reqwest 0.12` + `rustls-tls`, HTTPS-only in production, `Transport::LoopbackHttp` for tests, SHA-256 verification, path-traversal-safe `.zip`/`.tar.gz`/`.tar.xz` extraction, D35/D37/D38), and `services/tools.rs` (lookup order `Override > AppData > Bundled > PathEnv > WellKnown`, first-run copy of `yt-dlp` to app data with `fsync`, `ETXTBSY`/Windows sharing-violation spawn retry, and `yt-dlp -U` self-updater on the app-data copy, D36/D39/D43/D44/D47/D53/D116/D117).
  - Built `ToolsPage.svelte` and `SetupWizard.svelte` shell, plus CI syntax and runner allowlist guards (`scripts/check-ci-syntax.mjs`, `scripts/check-ci-runners.mjs`, `scripts/report-bundle-sizes.mjs`, D55).
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (186 passed), `cargo xwin test` under Wine (D51), `npm run check`, `npm test` (127 passed), `npm run check:ci`, `npm run check:runners`, `npm run check:tools`, and green 3-OS CI bundle run.
- Not verified / stubbed:
  - macOS notarization and Windows SmartScreen code signing (tracked in `docs/SPIKES.md` S1–S2).
- Spec items marked "verify" that I checked:
  - `tauri-utils` `external_binaries` naming (`<tool>-<triple>.exe` on Windows, 3 files required for `universal-apple-darwin`).
  - `ffmpeg -version` single-dash flag and `n`-prefixed version token (`n9.0.2`).
- Decisions/deviations (also in DECISIONS.md): D31–D55.
- Next step: M1 (single-video inspection, download, JS runtime fallback, cancellation).

## 2026-10-14 — M1 / Single-video inspection, download, QuickJS→Deno fallback, and cancellation (spec §5.3, §6, §8)

- Done:
  - `services/jsruntime.rs`: Auto/QuickJS/Deno modes (`--no-js-runtimes` before `--js-runtimes`), YouTube host matcher, and atomic `runtime-memory.json` store with 24 h TTL keyed by `(yt-dlp version, qjs version)` (D61).
  - `services/ytdlp.rs`: format selectors (exact height with fallback, max height cap, best, audio-only `.opus`, explicit format ID), merge output format (`mp4`/`mkv`), `--encoding utf-8` (D114), `--windows-filenames`, progress/print templates (`SDP_DL|`, `SDP_PP|`, `SDP_FORMAT|`, `SDP_FILE|`), 10/s progress throttle, stderr error classifier (spec §8.5 + `DENO_MISSING`), and OS-aware browser cookie args (D69).
  - `services/inspect.rs`: `-J --flat-playlist`, `Inspected` model (`Video`, `Playlist`, `Error`), resolution ladder filter, 60 s / 120 s timeout heuristic (D64), degraded-inspection warning detection and automatic retry (D73), and YouTube extraction self-test against `jNQXAC9IVRw` ("Me at the zoo", D63).
  - `services/job.rs`: single-video execution with automatic QuickJS→Deno fallback (on `YT_CHALLENGE`, degraded muxed `SDP_FORMAT` fallback below requested height, or 90 s download stall, D71), one-shot retry without `-f` on `FORMAT` error, cancellation killing the process tree and cleaning `.sdp-tmp-<job>`, and startup sweep of orphaned `.sdp-tmp-*` dirs.
  - Pinned `deno` `v2.9.7` in `tools.manifest.json` (6 platforms, SHA-256 verified, D59) and wired on-demand `tools_install`.
  - Session tool-resolution cache in `AppState` eliminating the ~0.46 s `yt-dlp --version` PyInstaller unpack overhead on every paste (D75), and 0 ms paste inspection vs 500 ms typing debounce (D76).
  - Removed forced `youtube:player_client=web_embedded,web,tv` default so `yt-dlp` uses its native `visionos,web` clients without PO-token degradation, exposing the override in Settings → Network & Accounts (D74).
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (285 passed + 3 ignored live tests in `tests/real_selftest.rs`), `npm run check`, `npm test` (162 passed).
  - Live `real_selftest` run: QuickJS self-test (1.95 s, 24 formats), "Me at the zoo" 144p `.mkv` + `.opus` download, and real Deno 2.9.7 download/checksum/extraction test.
- Not verified / stubbed:
  - Browser cookie extraction against a live logged-in browser profile.
- Spec items marked "verify" that I checked:
  - Verified all `yt-dlp 2026.08.19` flags against `--help` and upstream `options.py`: `--js-runtimes` / `--no-js-runtimes` (introduced in `2025.11.12`, D56), `-I` (`--playlist-items`, replacing removed `--playlist-end`, D60), `--progress-template`, `--print before_dl:` / `after_move:`, `--encoding utf-8`, `--paths home:/temp:`, `--windows-filenames`, `--merge-output-format`, `--impersonate`, `-x --audio-format`.
- Decisions/deviations (also in DECISIONS.md): D56–D78.
- Next step: M2 (persistent queue, scheduler, and queue UI).

## 2026-10-15 & 2026-10-16 — M2 / Persistent queue, resource scheduler, Add bar, virtualised list, and CI hardening (spec §7, §11.2, §11.5, §11.7)

- Done:
  - Implemented `src-tauri/src/services/queue/` (`model.rs`, `store.rs`, `scheduler.rs`, `events.rs`, `engine.rs`):
    - Full 12-state item machine, immutable `ItemOptions` snapshot, duplicate detection, 500-item finished history cap, and transient network auto-retry.
    - Atomic `queue.json` persistence with `.bak` corruption recovery; items are persisted as running *before* spawning child tools so crashes always recover them as `Interrupted` with their `.sdp-tmp-*` directory intact (D91).
    - Resource-aware scheduler with configurable parallel download lanes (1–4), per-host concurrency cap (`host_key` stripping `www.`, D85), encode lane, and transcription lane (D82).
    - Coalesced `queue:update` event stream (≤ 10 updates/s per item, D81) and optional background idle notification via `tauri-plugin-notification` 2.4 (`services/notify.rs`, D79).
  - Added IPC commands `queue_list`, `queue_add`, `queue_cancel`, `queue_pause`, `queue_resume`, `queue_retry`, `queue_remove`, `queue_pause_all`, `queue_resume_all`, `queue_retry_failed`, `queue_reorder`, `queue_move_to_top`, `queue_clear_finished`, `queue_lanes`, `read_links_file`, `app_set_language`, and parallel `inspect_urls` (3 concurrent inspections).
  - Built frontend queue store (`src/lib/stores/queue.svelte.ts` with `seq` guard preventing stale snapshots from rolling rows backward, D97), `AddBar.svelte` (session `MetadataCache` cap 50, per-row quality selector, primary **Download** priority action + **Add to queue** FIFO action, D98/D100/D106), `QueueList.svelte` (virtualised list with dynamically measured row pitch, scroll-to-added, and 2.5 s highlight pulse, D103/D104), and fixed-height `QueueItem.svelte` cards (inline Deno offer, stream detail `Video 1/2` / `Merging…`, file size, expandable details, D103/D105).
  - Added automated i18n dictionary and usage guards in both TypeScript (`src/lib/i18n/usage.test.ts`) and Rust (`message_key_in_dictionaries`, D99).
  - Split Linux CI artifacts per format (`.deb`, `.rpm`, `.AppImage`, D109), restricted artifact globs to installable packages with a release-job guard (D111), and made all CI workflows manual (`workflow_dispatch`, D118).
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (417 passed across Linux, Windows, and macOS CI runners, 3 ignored), `npm run check` (0 errors), `npm test` (207 passed), `npm run build`, `npm run check:ci`, `npm run check:runners`.
- Not verified / stubbed:
  - Optional "Keep computer awake" OS power assertion (spec §7.7).
- Spec items marked "verify" that I checked:
  - Confirmed Tauri v2 `Manager` / `Window` / `Webview` expose no OS locale getter to Rust, motivating `app_set_language` + `LANG`/`LC_ALL` fallback.
- Decisions/deviations (also in DECISIONS.md): D79–D118.
- Next step: M3 (playlists and non-YouTube sites).

## 2026-10-16 — M3 / Playlists, channels, URL cases, anti-bot retry, download archive, and non-YouTube sites (spec §6.2, §6.4, §8.2, §8.5, §11.3–§11.8)

- Done:
  - Extended `services/inspect.rs` and `src/lib/inspect.ts` with `PlaylistInfo` (`webpage_url`, `entries: Vec<EntryLite>`), pagination (`PLAYLIST_PAGE_SIZE = 1000` via `-I`, `MIX_DEFAULT_CAP = 50`), URL-case classifiers (`is_video_with_list_url`, `is_mix_url`, `is_search_or_feed_url`), `reconstruct_entry_url`, and `is_entry_available`.
  - Extended `services/ytdlp.rs` and `services/job.rs` with playlist numbering prefix (`-o` with `%` → `%%` escaping), video ID in filename toggle, H.264 preference (`-S vcodec:h264,res,acodec:m4a`), `--download-archive <app_data>/archive.txt` (scoped by default to playlist items so single videos can always be re-downloaded, with `--no-quiet` so archive-skip lines are never silenced by `--print`), playlist pacing (`--sleep-interval`), `--live-from-start`, `--proxy`, `--limit-rate`, custom `--referer` / `--user-agent` / `--add-headers`, and one-shot anti-bot retry with `--impersonate chrome` (probed via cached `--list-impersonate-targets`).
  - Added archive IPC commands (`archive_status`, `archive_clear`) and queue group commands (`queue_pause_group`, `queue_resume_group`, `queue_retry_failed_group`, `queue_remove_group`).
  - Built virtualised `PlaylistPanel.svelte` (search filter, range parser `1-10, 15, 20-`, Shift-click range selection, reverse order, folder-per-playlist via `sanitize_component`, numbering toggle, archive skip toggle, "Load more" pagination), `CookieBanner.svelte`, collapsible `QueueGroups.svelte`, and M3 settings in `GeneralSettings.svelte` and `NetworkSettings.svelte`.
  - Fixed `AddItemRequest.custom_headers` null deserialisation (`deserialize_null_as_default`) and prevented unintended native browser drag on UI buttons/links/icons.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (424 passed, 3 ignored), `npm run check` (0 errors), `npm test` (238 passed), `npm run build`.
- Not verified / stubbed:
  - Live 100-item YouTube playlist and live `--impersonate chrome` network runs (covered by offline fake-tool tests).
- Spec items marked "verify" that I checked:
  - Verified `-I, --playlist-items`, `--no-playlist`, `--yes-playlist`, `--download-archive`, `--no-quiet` (required alongside `--print`), `--sleep-interval`, `--live-from-start`, `--impersonate`, `--list-impersonate-targets`, `--proxy`, `--referer`, `--user-agent`, `--add-headers` against bundled `yt-dlp 2026.08.19 --help`.
- Decisions/deviations (also in DECISIONS.md): none (follows spec §6.4, §8.2, §11.3–§11.8).
- Next step: M4 (media processing with FFmpeg).

## 2026-10-17 — M4 / Media processing: FFmpeg encode, hardware encoder probing, trim, audio presets, embeds, and SponsorBlock (spec §8.4, §9, §14 row M4, §15 item 8)

- Done:
  - `services/encoders.rs`: OS-specific hardware/CPU encoder catalogue (`libx264`, `libx265`, `*_nvenc`, `*_amf`, `*_qsv`, `*_vaapi`, `*_videotoolbox` with `-tag:v hvc1` for HEVC), bounded concurrency (`Semaphore::new(2)`) 256×256 black-frame probe (`color=c=black:s=256x256:d=0.2`, 10 s timeout, VideoToolbox `-b:v 2M` retry), persistent cache keyed by `(ffmpeg_path, mtime, ffmpeg_version)`, and hardware-to-CPU fallback mapping (`h264_*` → `libx264`, `hevc_*` → `libx265`) with toast + log notification.
  - `services/ffmpeg.rs`: `TrimSpec` parser (`SS`, `MM:SS`, `HH:MM:SS`) and validator (`start < end <= duration`, validated at `queue_add` and pre-encode, probing duration via `ffprobe` when unknown), `FfmpegProgressTracker` (`-nostdin -hide_banner -progress pipe:1 -nostats` parsing `out_time_us`), audio presets (`aac` → `.m4a`, `mp3` → `.mp3`, `opus` → `.opus`), attached-thumbnail preservation (`embed_thumbnail`), and safe `.sdp-part` output lifecycle (original file deleted *only* after a non-empty encode succeeds and renames cleanly, or kept when `keep_original = true`; resume after crash in `Encoding` skips re-downloading).
  - `services/ytdlp.rs` & `services/job.rs`: added `--embed-metadata` (with `--no-embed-info-json` and conditional `--no-embed-chapters`), `--embed-thumbnail --convert-thumbnails jpg`, `--embed-chapters`, and YouTube-only `--sponsorblock-mark` / `--sponsorblock-remove`; treated postprocessor-only embed failures as warnings when the media file exists.
  - Queue engine & UI: dynamic `EncodeSlot` scheduling with `"waitingEncode"` / `"encoding"` labels, `ItemOptions::signature` separation for M4 options, `encoders_probe` / `open_path` / `reveal_path` commands, and M4 controls in `AddBar.svelte` and `GeneralSettings.svelte`.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (335 unit + 136 integration = 471 passed, 3 ignored), `npm run check` (0 errors), `npm test` (262 passed), `npm run build`.
- Not verified / stubbed:
  - Proprietary hardware encoders on physical Windows/macOS GPUs (verified via fake-tool suite and Linux host).
- Spec items marked "verify" that I checked:
  - `ffmpeg -progress pipe:1` (`out_time_us`), `-movflags +faststart`, `--convert-thumbnails jpg`, `--embed-metadata` implied flags (`--no-embed-info-json`, `--no-embed-chapters`), and `--sponsorblock-mark` / `--sponsorblock-remove`.
- Decisions/deviations (also in DECISIONS.md): none.
- Next step: M5 (Whisper sidecar CI, model catalogue, and offline transcription).

## 2026-10-18 & 2026-10-19 — M5 / Whisper sidecar CI, model download manager, and offline transcription pipeline (spec §5.4, §10.2, §11.5, §11.6, §15 item 9)

- Done:
  - Created `.github/workflows/whisper-sidecar.yml` building static `sdp-whisper` binaries from `ggml-org/whisper.cpp` `v1.9.5` across Linux (`x86_64`, `aarch64`), Windows (`x64`, `arm64` with `GGML_NATIVE=ON`, D123), and macOS (`arm64`, `x86_64`, `universal` with Metal embedded and duplicate-artifact guard, D122).
  - Pinned `whisper.models.json` (11 GGML Whisper models + `ggml-silero-v5.1.2.bin` VAD from `ggml-org/whisper-vad`, 885,098 bytes, SHA-256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`, D119/D121) and added `scripts/fetch_models_manifest.mjs` (`npm run check:models`).
  - Implemented `ModelDownloadManager` (`src-tauri/src/services/models.rs`) with path-traversal validation (`is_safe_asset_name`), HTTP `Range` resume into `.part`, pause/resume/cancel (`ModelInstallState::Paused`), cancel-on-delete, pre-flight disk-space check (accounting for existing `.part` bytes), automatic Silero VAD co-download, local `.bin` import, and 10/s progress throttling (`ModelProgressThrottle`).
  - Implemented offline Whisper pipeline (`src-tauri/src/services/transcribe/whisper.rs`, D120):
    - 16 kHz mono WAV extraction via `ffmpeg` in the item's isolated job temp dir with RAII cleanup guards (`CleanupFile`, `ChunkGuard`) and `ffprobe` duration probing.
    - Incremental stderr streaming of `ffmpeg -af silencedetect=noise=-30dB:d=0.5` for audio > 2 h (7200 s), splitting at silence within ±20 s of 30-min boundaries (or hard cut with 2.0 s overlap) and merging `.txt`, `.srt`, and `.vtt` with cue boundary deduplication and timestamp offsetting.
    - Pinned `sdp-whisper` execution (`-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`, `-of`, `--vad`, `-vm`, `-t`, `-ng`), GPU-to-CPU `-ng` fallback retry on GPU builds, hallucination loop warning toast (spec §10.2 item 7, without mutating transcript text, D121), and collision-free atomic output writing (`<stem>.{txt,srt,vtt}`).
  - Integrated transcription lane scheduling (`max_parallel_transcriptions`), video-language auto-defaulting from inspection, inline missing-sidecar / missing-model install offers on queue cards, `queue_transcribe_item` and `queue_add_local_file` commands, and UI controls in `AddBar.svelte`, `QueueItem.svelte`, and `TranscriptionSettings.svelte`.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (381 unit + 169 integration = 550 passed, 3 ignored), `npm run check` (0 errors), `npm test` (285 passed), `npm run check:models`.
- Not verified / stubbed:
  - End-to-end > 2 h real audio transcription with every catalogue model size (covered by fake-tool integration tests).
- Spec items marked "verify" that I checked:
  - `whisper.cpp` `v1.9.5` CLI flags (`-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`, `-of`, `--vad`, `-vm`, `-t`, `-ng`), Hugging Face LFS SHA-256 hashes, and `ffmpeg` `silencedetect` output format.
- Decisions/deviations (also in DECISIONS.md): D119–D124.
- Next step: M6 (subtitles-first pipeline, Gemini cloud transcription, summaries, and secret storage).

## 2026-10-20 & 2026-10-21 — M6 / Subtitles-first pipeline, Gemini cloud transcription engine, map-reduce summaries, and secret storage (spec §10.1, §10.3, §10.4, §10.5)

- Done:
  - Subtitle downloader & cleaner (`src-tauri/src/services/transcribe/subtitles.rs`): extracts manual or auto subtitles via `yt-dlp --write-subs --write-auto-subs --skip-download`, strips WebVTT headers, inline word timestamps (`<00:00:01.520><c>`), and HTML tags, deduplicates rolling auto-caption overlaps, groups paragraphs on pauses >= 2.0 s, and writes collision-free `<name>.txt` and `<name>.<lang>[.auto].srt`.
  - Secrets storage (`src-tauri/src/services/secrets.rs`): integrated `keyring` v3.6 with zero OpenSSL (`apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`, `linux-native`), resolving `GEMINI_API_KEY` env var → OS Keyring → `<config_dir>/gemini_api_key` fallback (mode `0600` on Unix). Key is redacted in `Debug` impls, marked sensitive on `reqwest` headers, and never exposed to the frontend or logs.
  - Gemini Cloud Engine (`src-tauri/src/services/transcribe/gemini.rs`): fails fast with `GeminiKeyMissing` on blank key; transcodes audio to 16 kHz mono Opus in Ogg (with MP3/WAV fallback); uploads from disk via Google Files API Resumable Upload (`POST /upload/v1beta/files` + streaming `POST`); polls until `ACTIVE` (10-min timeout, cancellable); transcribes verbatim via `generateContent` with HTTP 429 backoff retry, `blockReason` error reporting, and `MAX_TOKENS` truncation warning; discovers models dynamically from `GET /v1beta/models` (`-latest` first); and deletes remote cloud files via RAII `GeminiFileGuard` (`DELETE /v1beta/files/{name}`) on completion, error, or cancellation.
  - Summarization (`src-tauri/src/services/transcribe/summary.rs`): map-reduce summarization for transcripts exceeding `MAX_SUMMARY_CHUNK_CHARS` (`50_000` chars) on paragraph boundaries, writing collision-free `<name>.summary.md`.
  - Queue engine & UI: wired engine modes (`subtitles_whisper_gemini`, `whisper_only`, `gemini_only`, `subtitles_only`), `transcript_origin` badges (`subtitles_manual`, `subtitles_auto`, `whisper`, `gemini`), Whisper error preservation when no Gemini key is set, `commands/gemini.rs` IPC commands, and localized UI controls in `AddBar.svelte`, `QueueItem.svelte`, and `TranscriptionSettings.svelte`.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (425 unit + 193 integration = 618 passed, 3 ignored), `npm run check` (0 errors), `npm test` (292 passed), `npm run build`.
- Not verified / stubbed:
  - Live Gemini API requests with a real Google account key (verified via local `MockGeminiServer` per AGENTS.md §6).
- Spec items marked "verify" that I checked:
  - `cargo tree -i openssl-sys` (0 matches), Google Files API resumable upload (`POST /upload/v1beta/files` with `X-Goog-Upload-Protocol: resumable`), and `GET /v1beta/models`.
- Decisions/deviations (also in DECISIONS.md): D125.
- Next step: M7 (polish, on-demand tools, updater, docs, and release).

## 2026-10-23 — M7 / Polish, on-demand tools, Setup Wizard, updater, diagnostics, and release preparation (spec §5.1, §5.2, §11.5, §11.8, §13.4, §15)

- Done:
  - **On-demand tools & sidecar** (D126, D134): added `ArchiveKind::Gz` and generic `tools::install_tool` for plain, `.gz`, `.zip`, `.tar.gz`, and `.tar.xz` assets; pinned `ffmpeg`/`ffprobe` and all seven published `whisper-v1.9.5` `sdp-whisper` release assets under the corrected `EmanueleFusco/Scaricatore-di-Porto-rust` slug in `tools.manifest.json`.
  - **Setup Wizard & Queue UX** (D130): real single/batch tool install with retry/cancel, recommended Whisper model download/cancel/resume, engine order, output-folder picker, and focus trap; queue filter (`Ctrl/Cmd+F`), checkboxes, Shift-click range selection, bulk actions bar, and `Ctrl/Cmd+,` shortcut.
  - **About, diagnostics & updater** (D127, D128): `commands/app.rs`, `services/logging.rs` (`scaricatore-di-porto.log`), `services/diagnostics.rs` (privacy-tested **Copy diagnostics**), `tauri-plugin-updater` 2.14.0 + `tauri-plugin-process` 2.4.0, `scripts/generate-latest-json.mjs`, and non-modal update banner.
  - **Docs, licence & acceptance** (D131, D132, D133, D138): MIT `LICENSE`, English `README.md`, `docs/SUPPORTED_SITES.md`, `docs/TROUBLESHOOTING.md`, and `docs/ACCEPTANCE.md` with spec §15 checklist and Linux size/memory measurements (binary **8.5 MiB**, `.deb`/`.rpm` **43.2 MiB**, AppImage **148.9 MiB**; idle main process **191,920 kB RSS / 87,832 kB PSS / 44,628 kB RssAnon**).
  - **Linux Wayland DMA-BUF fallback** (D135, D136): added `is_wayland_session()` and `apply_webkit_dma_buf_fallback()` in `src-tauri/src/lib.rs` to set `WEBKIT_DISABLE_DMABUF_RENDERER=1` automatically on Linux Wayland + NVIDIA proprietary driver sessions before webview creation.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (441 unit + 193 integration = 634 passed, 5 ignored), `npm run check` (0 errors), `npm test` (16 files, 333 passed), `npm run build`, `npm run check:ci` (24 blocks), `npm run check:runners`, `npm run check:tools`, `npm run check:release -- v0.1.0`.
  - Live ignored self-tests (`real_selftest`): QuickJS self-test passed (2.1 s), "Me at the zoo" downloaded (`.mkv` + `.opus`), and `sdp-whisper` Linux x64 sidecar downloaded, SHA-256 verified, and ran `-h`.
- Not verified / stubbed:
  - Windows 10/11 x64 and macOS 13+ manual hardware acceptance, macOS notarization, Windows code signing, live Gemini API run, and live self-update from a published signed release (see `docs/ACCEPTANCE.md`).
- Spec items marked "verify" that I checked:
  - Tauri v2 `createUpdaterArtifacts: true` output formats (`.AppImage` + `.sig`, `*.app.tar.gz` + `.sig`, `*-setup.exe` + `.sig`), `whisper-v1.9.5` release assets, and `tauri-plugin-updater 2.14.0` / `tauri-plugin-process 2.4.0`.
- Decisions/deviations (also in DECISIONS.md): D126–D136.
- Next step: rotate lost local updater key, polish UI/icons, and fix remaining cross-platform integration issues.

## 2026-10-24 — M7 / Updater key rotation, English-only docs, UI polish, app icons, and cross-platform fixes

- Done:
  - **Updater key rotation** (D137): generated new minisign key pair at `~/.config/scaricatore-di-porto/updater.key` (mode `0600`, outside repo), updated `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`, and verified `.AppImage.sig` Ed25519 signature.
  - **Docs cleanup** (D138): rewrote `README.md`, removed `.it.md` duplicates, and verified `docs/TROUBLESHOOTING.md` and `docs/SUPPORTED_SITES.md` against UI strings and backend behaviour.
  - **UI polish & localization**: added theme surface tokens and shared control classes in `src/lib/styles/app.css`, styled `<select>` controls consistently across OSes, localized tool roles (`tools.role.<name>`) and Whisper model labels/notes (`models.label.<id>`, `models.notes.<id>`), collapsed multi-line `ffmpeg`/`ffprobe` version banners via `splitToolBanner()`, removed `(spec §...)` jargon from i18n strings, and wired the top non-modal update banner in `App.svelte`.
  - **App icons & Linux desktop integration**: regenerated `src-tauri/icons/` from the 1024 px master (`icona_1024.png` with 140/1024 rounded mask on Windows/Linux and Apple grid 824/1024 on macOS `icon.icns`), reordered `bundle.icon` in `tauri.conf.json`, set `window.set_icon(...)`, and added `ensure_linux_desktop_integration()` in `src-tauri/src/lib.rs`.
  - **Cross-platform `gemini_pipeline` test fixes**: removed `#[cfg(any(target_os = "linux", test))]` gate on `get_gemini_api_key_with()` read path in `secrets.rs` so macOS/Windows integration tests read temporary `<config_dir>/gemini_api_key` files; made `MockGeminiServer` handle connections concurrently and synchronised cancellation deterministically on Windows.
  - **Fixed "Open file" (`open_path`) & focus ring clipping**: switched `QueueItem.svelte` and `QueueGroups.svelte` from JS `plugin:opener|open_path` (blocked by ACL `FsScope`) to backend IPC commands `open_path` and `reveal_path` in `src-tauri/src/commands/system.rs` with parent-dir fallback; updated `.input`/`.select` focus rings in `app.css` to avoid overflow clipping.
- Verified by running:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (all unit + integration tests pass, including 20/20 `gemini_pipeline`), `npm run check` (0 errors), `npm test` (16 files, 338 passed), `npm run build`.
- Not verified / stubbed: none.
- Decisions/deviations (also in DECISIONS.md): D137, D138.
- Next step: co-locate `ffmpeg`/`ffprobe` and upgrade on-demand FFmpeg.

## 2026-10-24 — M7 / Co-locate `ffmpeg` and `ffprobe`, upgrade to FFmpeg 9.0.2, fix thumbnail stream mapping, and polish transcription UI (spec §5.1, §5.2, §8.2, §8.4, §9.2, §10, §11)

- Done:
  - **Co-located `ffmpeg` and `ffprobe` & system `PATH` switching** (D139):
    - Mapped both `ffmpeg` and `ffprobe` to `<tools_dir>/ffmpeg/` in `src-tauri/src/services/tools.rs` (matching `yt-dlp --ffmpeg-location`), added `migrate_legacy_ffprobe_dir()`, and added `sibling_companion_override`.
    - Mapped `"ffprobe not found"` / `"ffprobe is not installed"` to `YtdlpErrorCode::FfmpegMissing` in `services/ytdlp.rs`.
    - Added `systemPath` detection, live Windows Registry `PATH` refresh (`HKCU\Environment` + `HKLM\...\Session Manager\Environment`), and `tools_use_system` ("Use system PATH" in `ToolsPage.svelte`).
    - Made `ToolContext::in_dir` and `ToolContext::isolated` hermetic with respect to host `PATH`, and fixed `ToolsPage.svelte` table cell borders.
  - **FFmpeg 9.0.2 upgrade & thumbnail encoding fixes** (D140):
    - Upgraded `ffmpeg` and `ffprobe` in `tools.manifest.json` to FFmpeg `9.0.2` across Windows (`GyanD/codexffmpeg`), Linux x64/arm64 (`Tyrrrz/FFmpegBin`), and macOS arm64/x64 (`Tyrrrz/FFmpegBin`), with single-pass companion extraction in `tools::install_tool` and breadth-first recursive `find_executable` in `services/download.rs`.
    - Fixed FFmpeg `exit code Some(-22)` and `Some(-1094995529)` (`Tag hvc1 incompatible with output codec id '7' (mp4v)`) when encoding media with attached thumbnails (`embed_thumbnail`): mapped `-map 0:disp:attached_pic?`, indexed primary video encoder flags (`-c:v:0`, `-tag:v:0 hvc1`, `-b:v:0`, `-q:v:0`), and copied the attached picture stream (`-c:v:1 copy -disposition:v:1 attached_pic`).
    - Prevented status counter wrapping in `FooterStatus.svelte`.
  - **Transcription UI polish, custom scrollbars, and cross-platform clippy fix** (D141, D142):
    - Renamed download option labels from "Whisper transcription" to "Transcription" / "Transcribe" (`it.json`, `en.json`), moved the **Engine** dropdown to the top of `AddBar.svelte` (disabling Whisper controls when `gemini_only` or `subtitles_only` is selected), and emitted stage-specific labels (`transcribingSubtitles`, `transcribingWhisper`, `transcribingGemini`) from `services/queue/engine.rs`.
    - Added sleek theme-integrated scrollbars in `src/lib/styles/app.css` and updated FFmpeg/FFprobe license strings to `"LGPL-2.1-or-later / GPL-2.0-or-later"`.
    - Scoped `let mut dirs = dirs;` inside `#[cfg(windows)]` in `live_path_dirs()` (`services/tools.rs`) so Linux and macOS CI compile cleanly under `-D unused-mut`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → pass.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → pass, 0 warnings.
  - `( cd src-tauri && cargo test )` → all 504 unit and integration tests passed.
  - `npm.cmd run check` → 0 errors, 0 warnings.
  - `npm.cmd test -- --run` → 16 test files, 327 passed.
  - `npm.cmd run build` → pass.
  - `npm.cmd run check:ci` → 24 CI workflow blocks valid.
- Not verified / stubbed: none.
- Spec items marked "verify" that I checked: `yt-dlp --ffmpeg-location` same-directory requirement; FFmpeg stream-indexed `-c:v:0` / `-tag:v:0` flags with `0:disp:attached_pic?`; upstream FFmpeg licensing at `ffmpeg.org/legal.html`.
- Decisions/deviations (also in DECISIONS.md): D139–D142.
- Next step: push to `origin/main`, update tag `v0.1.0`, and run CI release workflow.

## 2026-11-06 — Docs / Synthesize and unify `docs/PROGRESS.md`, `docs/DECISIONS.md`, and `docs/SPIKES.md` into concise English (D143)

- Done:
  - Audited all 9 Markdown files in the repository (`docs/PROGRESS.md`, `docs/DECISIONS.md`, `docs/RUST_PORT_GUIDELINES.md`, `docs/SPIKES.md`, `AGENTS.md`, `docs/TROUBLESHOOTING.md`, `docs/ACCEPTANCE.md`, `docs/SUPPORTED_SITES.md`, `README.md`).
  - Compacted `docs/DECISIONS.md` from 3,627 lines (227 KB) to ~435 lines (~32 KB), retaining every decision identifier (`D1`–`D143`), technical root cause, verified flag/hash/measurement, superseding note, and test-asserted string (`885,098 bytes`, `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`, `ggml-org/whisper-vad`), and translating D1–D76 from Italian to English.
  - Compacted `docs/PROGRESS.md` from 4,702 lines (353 KB) to ~245 lines (~22 KB) by consolidating M0-A through M6 micro-steps and review fixes into per-milestone blocks in English while keeping M7 and recent post-M7 blocks detailed at the bottom.
  - Translated and compacted `docs/SPIKES.md` (`S1`–`S6`) from 292 lines (12.5 KB) in Italian to ~115 lines (~5 KB) in English.
  - Left `docs/RUST_PORT_GUIDELINES.md` untouched (read-only per `AGENTS.md` §1/§8) and kept `README.md`, `AGENTS.md`, `docs/TROUBLESHOOTING.md`, `docs/ACCEPTANCE.md`, and `docs/SUPPORTED_SITES.md` as-is (already concise and in English).
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → pass.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → pass, 0 warnings.
  - `( cd src-tauri && cargo test )` → all unit and integration tests pass (including `docs_progress_and_decisions_reflect_whisper_vad_and_hallucination_reality` in `tests/whisper_pipeline.rs`).
  - `npm.cmd run check` → 0 errors, 0 warnings.
  - `npm.cmd test -- --run` → all frontend tests pass.
- Not verified / stubbed: none.
- Spec items marked "verify" that I checked: none (documentation synthesis only).
- Decisions/deviations (also in DECISIONS.md): D143.
- Next step: commit documentation synthesis.

## 2026-11-06 — M7 / Version 0.1.0 functional, UI and cross-platform audit

- Done:
  - Audited `ad1f67f` (application code identical to local tag `v0.1.0` / `89a4052`); added `docs/AUDIT_0.1.0.md` with prioritized findings, reproduction evidence, UI recommendations and explicit coverage limits.
  - Launched the current native Windows app, inspected/downloaded the public zoo video, checked unavailable/unsupported/mixed-scheme inputs and the video/playlist choice, and completed loopback direct-video, Opus audio, HLS and HTML5-page downloads.
  - Exercised parallel slow transfers, pause/resume, cancellation, queue filtering, compact/maximized layouts, the in-app QuickJS self-test, version display and the published update check. Closing during a transfer left no app or app-data tool processes in the subsequent host process inspection.
  - Reproduced two high-priority defects: encoded output replacement without collision protection, and pending inspection rows stranded by a newer batch. Diagnostic probes also confirmed playlist reverse/order mismatch, whole mixed-batch resubmission and FFmpeg compiler-version misidentification. Recorded additional source-confirmed/platform-inferred defects separately from live reproductions.
  - Ran real FFmpeg H.264/AAC trim (3.0 s output, original preserved), downloaded/verified the Windows Whisper sidecar and Tiny model, and generated nonempty TXT/SRT/VTT from short English speech. Tiny recognition errors remain; this is not an Italian/recommended-model quality acceptance.
  - Built the current production executable: 6,961,152 bytes. No installer or update was installed. No functional source, dependencies, tags or remote GitHub state changed.
  - Stopped the loopback servers, moved diagnostic scripts/fixtures/logs into ignored `src-tauri/target/audit-2026-11-06/`, removed only the nine audit-created queue entries, and verified that the two original history entries were unchanged. Generated media in Downloads remains available for inspection; model/sidecar probe installations were temporary.
- Verified by running:
  - `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` -> pass; repeated after removing temporary diagnostic test sources.
  - `cargo test` -> 637 passed, 5 ignored; `npm.cmd test -- --run` -> 327 passed, 12 skipped, 16 files.
  - `npm.cmd run check` -> 0 errors, 0 warnings; repeated at the end. `npm.cmd run build` -> pass.
  - `npm.cmd run check:ci` -> all 24 blocks valid; `npm.cmd run check:release -- v0.1.0` -> pass.
  - `npm.cmd run tauri build -- --no-bundle` -> pass, production executable compiled in 2m 22s; not independently tested as an installed package.
  - `real_selftest-d76210a72cb6d09f.exe --ignored --no-capture --test-threads 1 the_self_test_passes_with_the_bundled_tools` -> pass, QuickJS, 25 formats, 8,766 ms; equivalent direct invocation of `the_whisper_sidecar_install_downloads_verifies_and_runs` -> pass.
  - `node audit-output/store-repros.mjs` -> three undesired behaviors reproduced by the compiled stores/pure request builder before archival; `cargo test --test audit_repros -- --ignored --nocapture` -> three temporary diagnostic probes passed, including a real 32,152,673-byte Tiny model download/checksum/transcription. Diagnostic assertions deliberately observed current defects; they are not fixes or permanent regression expectations.
  - `npm.cmd run app:binary` -> failed to find an existing Windows release binary; recorded as F10. `git diff --check` -> pass.
- Not verified / stubbed:
  - macOS/Linux native acceptance, fresh-machine installer/signing/AV, 1080p/4K/Shorts, a live large playlist, authenticated/live/DRM public providers, hardware encoders, Italian/large-model/long-audio/VAD transcription, real Gemini/summary/cloud cleanup, actual signed self-update, screen readers and a 5,000-row performance run. The folder picker opened but its modal controls were not reliably targetable by the automation helper; this was not classified as an app bug.
  - Initial sandboxed Vitest/Rust/CI failures were temporary-file/named-object permission restrictions; unsandboxed reruns passed. A Cargo live-test launch hit the open Windows executable lock; direct invocation of the existing test executable passed. The first development instance was stopped to recover an automation-blocked native dialog; the later close-during-transfer check used normal window closure.
- Spec items marked "verify" that I checked: used the actual FFmpeg/ffprobe help, the sidecar help and manifest-provided model/tool URLs and checksums; confirmed AppImage temporary mounts and the persistent `APPIMAGE` path in official AppImage documentation.
- Decisions/deviations (also in DECISIONS.md): D144; findings only, awaiting owner triage before implementation. The read-only specification and reference app were not edited.
- Next step: discuss F01–F02 first, then queue/playlist corrections and the remaining platform/service acceptance matrix.

## 2026-11-06 — M7 / Keep local audit material out of commits

- Done: added `/audit-output/` and `*.private.json` to `.gitignore`; existing tool/model/build/log exclusions remain in force. Recorded the owner's preference in `AGENTS.md`: local diagnostic artifacts stay ignored; application changes, reusable regression tests and useful documentation are committed.
- Verified by running: `git check-ignore -v audit-output/probe.mjs sample.private.json audit-cargo-tests.log src-tauri/target/audit-2026-11-06/queue-before-cleanup.private.json` -> all ignored; `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm.cmd run check`, `npm.cmd test -- --run`, `npm.cmd run build` -> pass with the pending F01 implementation in the working tree.
- Not verified / stubbed: no runtime behavior changed by the ignore rules.
- Spec items marked "verify" that I checked: none.
- Decisions/deviations (also in DECISIONS.md): none; explicit owner instruction.
- Next step: commit the encoded-output collision fix separately.

## 2026-11-06 — M7 / Preserve encoded results and isolate concurrent staging files (F01)

- Done: kept the default encoded filename for the first result and selected a numbered name when occupied, using no-clobber publication instead of overwrite-on-rename. Each encode owns a unique same-filesystem staging file. Video/audio results, existing clips and source-retention behavior are covered by reusable fake-tool regression tests, including simultaneous encodes.
- Verified by running: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` -> pass; `cargo test` -> 642 passed, 5 ignored; `cargo test --test ffmpeg_encode` -> 23 passed; `npm.cmd run check` -> 0 errors/warnings; `npm.cmd test -- --run` -> 327 passed, 12 skipped; `npm.cmd run build` -> pass. One newly added unit assertion initially compared TempPath values directly; corrected it to compare owned paths before the successful full run.
- Not verified / stubbed: native macOS/Linux execution and removable-filesystem acceptance; optimized build/size comparison still running. No new real network/media acceptance was claimed.
- Spec items marked "verify" that I checked: cached tempfile 3.27.0 `Builder::tempfile_in`, `into_temp_path`, `TempPath::persist_noclobber`, and Windows/Unix no-replace implementation; `cargo tree -i tempfile --edges normal` confirms the updater already ships this runtime dependency.
- Decisions/deviations (also in DECISIONS.md): D145; suffix colliding outputs instead of replacing them. No new crate or feature added to the runtime dependency graph.
- Next step: fix per-row inspection lifecycle/cancellation (F02), and record the release build result when available.
- Release follow-up: `npm.cmd run tauri build -- --no-bundle` passed in 2m 13s; Windows x64 executable 6,963,200 bytes (+2,048 bytes against the audit baseline). Installer and native macOS/Linux acceptance remain unverified.

## 2026-11-06 — M7 / Isolate inspection requests and cancel removed rows (F02)

- Done: assigned inspection ownership per request/row instead of one global generation. Concurrent batches, retries and playlist pagination no longer invalidate unrelated rows; stale successes/errors are ignored and omitted answers become retryable errors. Removing/clearing rows sends targeted cancellation. The backend owns per-URL tokens, preserves cancel-before-registration requests, unregisters finished batches, and cancels inspections on app exit. Added nine actual rune-store regression cases, three state tests and three IPC tests, including cancellation of an already running fake tool and prevention of queued tool launch.
- Verified by running: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` -> pass; `cargo test` -> 648 passed, 5 ignored; `cargo test --test m1_ipc batch_ -- --nocapture` -> 2 passed; `npm.cmd test -- --run` -> 336 passed, 12 skipped, 17 files; `npm.cmd run check` -> 0 errors/warnings; `npm.cmd run build` -> pass; `git diff --check` -> pass. Rust/Vitest commands ran outside the Windows filesystem sandbox as in the audit; logs remain under ignored `src-tauri/target/`.
- Not verified / stubbed: no additional native GUI or real-provider acceptance for this change; no native macOS/Linux execution. The API still returns one complete batch, with up to three inspections per call. Incremental row results/global concurrency across overlapping calls remain follow-up improvements.
- Spec items marked "verify" that I checked: existing Svelte Vite compile-module support for `.svelte.ts` rune-store tests and installed Vitest mock APIs; no external tool flags or APIs added.
- Decisions/deviations (also in DECISIONS.md): D146; backward-compatible optional cancellation filters and bounded pending cancellations. No dependency added.
- Next step: mixed-duplicate queue handling (F03) and playlist ordering (F04), followed by remaining UI/platform findings and native acceptance.
