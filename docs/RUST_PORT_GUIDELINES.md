# Porting "Scaricatore Di Porto" (v2.5) to Rust + Tauri v2 — Agent Guidelines (rev. 3)

Audience: an autonomous coding agent (or developer) re-implementing the Python/CustomTkinter app **Scaricatore Di Porto v2.5** (`ScaricatoreDiPorto.py`, ~1000 lines, Windows-only, single-video downloader) as a **light, cross-platform** desktop app (Windows, macOS, Linux) that is also an **upgrade**: queue and playlists, first-class support for non-YouTube sites, a better offline transcription engine, and a redesigned UI.

This revision is based on a line-by-line reading of the Python source plus up-to-date research (Oct 2026). Section 12 lists behaviours of the Python app that are **bugs or smells and must NOT be ported**. Read it before coding.

**Convention.** "verify" means: the exact flag / asset name / URL / version may have changed upstream. Check the current official docs or `--help` output of the pinned version before hard-coding, and keep such values in ONE place (`tools.manifest.json` + a constants module, §5.1).

## 0. Product decisions (fixed — do not re-open)

1. **Offline transcription = Whisper (whisper.cpp)**, run as a separate **sidecar CLI built by our own CI** (§5.4, §10.2). The user chooses which model size to download (§10.2, §11.6). **VOSK is dropped**: no libvosk, no FFI.
2. **JavaScript runtime for YouTube: QuickJS (`qjs`) first, Deno as automatic fallback** (§5.3). Deno is downloaded lazily, only if needed.
3. **Gemini** stays as an optional cloud engine (§10.3), with a dynamic model list (the original hard-coded list is partly obsolete).
4. **New UI**, queue-centric, not a port of the CustomTkinter tabs (§11).
5. **New features to implement:** queue + playlists, versatile non-YouTube support, subtitles-first transcripts, SRT/VTT output, embedded metadata/thumbnail/chapters, SponsorBlock, app auto-update + yt-dlp updater, optional Gemini summary.
6. Everything must work on Windows, macOS and Linux without manual setup beyond installing the app.
7. **Bundled baseline:** the installer includes **yt-dlp and qjs**, so the core download feature works right after installation (§5.2.1). Everything else is a **composable on-demand download**: ffmpeg (if not found), Deno (fallback only), the Whisper sidecar, the VAD model and the Whisper models.

---

## 1. Goals and realistic targets

| Goal | Target |
| :-- | :-- |
| Platforms | Windows 10/11 x64, Linux x64, macOS 12+ (universal build). Support tiers in §1.1: the project owner cannot manually test every target. |
| Feature parity | Everything the Python app does (§2.9), plus the new features in §0.5. |
| Weight | App binary **≤ 15 MB**. Installer = app + **bundled yt-dlp + qjs** (the only bundled third-party tools); its real size is dominated by yt-dlp's standalone binary, so **measure it in M0 and fix the budget then**, report it in CI on every build. Deno, ffmpeg, the Whisper sidecar, the VAD model and the models are detected or downloaded on demand into the app data dir (§5). The AppImage is inherently larger (embeds WebKitGTK); size targets do not apply to it. |
| Memory | Rust process idle ≲ 30 MB. The OS WebView runs in separate process(es) (WebView2 is Chromium-based): measure the Rust process alone and the whole tree separately; do not promise < 45 MB for the tree. |
| Responsiveness | UI stays smooth with a 5,000-entry playlist and 4 parallel jobs (virtualized lists, throttled events, §7.6, §11.7). |
| No Python | **No Python installation required** on any OS. Note: the official yt-dlp executable is itself a PyInstaller bundle that embeds its own Python runtime; that is inside the binary and is not a dependency of our app. |
| Safety | No orphan processes after Cancel/exit; no secrets in logs; no TLS verification disabled anywhere. |

### 1.1 Support tiers (the owner cannot test every target)
Assumption to confirm with the owner: they can manually test **Windows x64, Linux x64 and macOS on Apple Silicon**; they **cannot** test macOS Intel (x64), Windows ARM64 or Linux arm64.

| Tier | Targets | Policy |
| :-- | :-- | :-- |
| **1 — manually tested** | Windows x64, Linux x64, macOS arm64 | The acceptance checklist (§15) must pass here before a release. |
| **2 — CI-only, "experimental"** | macOS Intel (the x64 slice of the universal binary), Windows ARM64, Linux arm64 | Built and smoke-tested in CI only; release notes label them **experimental / not manually tested**. Do not add target-specific code paths unless CI shows a failure. |

Rules:
* **Never publish a Tier 2 artifact that CI did not at least build and run the fake-tool integration tests on** (§15). Where a runner exists, also run the real-tool self-test (§5.3.6). Check which runners are currently available (verify: macOS Intel, Windows ARM64, Linux arm64 hosted runners); if a target has no way to run any test, **do not publish it**.
* macOS Intel: if the owner has an Apple Silicon Mac, the x64 slice can be exercised under Rosetta (`arch -x86_64`) as a partial manual check. GPU/hardware paths (Metal, VideoToolbox) are untestable there and must fail soft (CPU fallback, §9.3, §10.2).
* Tools for Tier 2 targets (yt-dlp, qjs, Whisper sidecar, ffmpeg source) must come from the manifest like everything else; if no asset exists for a target, show "not available on this platform" instead of failing silently (a silent failure or a hard crash is not acceptable).
* Keep Tier 2 cheap: no extra UI, no extra settings. A bug report template tells users to attach "Copy diagnostics" (§11.8).

Weight rules (apply everywhere):
* Release profile: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
* `reqwest` with `default-features = false, features = ["rustls-tls", "stream", "json"]` (**no OpenSSL**). Check root certificates work on all 3 OSes (`rustls-tls-native-roots` or `webpki-roots`).
* `tokio` with only needed features; use Tauri's async runtime, not a second one.
* No `wmi`, `winreg`, `sysinfo` or other heavy crates unless justified. `zip`/`tar`/`flate2`/`xz` only with the needed features. Deserialize JSON into structs with **only the needed fields**.
* Frontend: TypeScript + Vite + **Svelte** (compiles to a tiny runtime; fits a reactive queue UI better than vanilla). No CSS framework runtime, system font stack, no remote assets, inline SVG icons.
* **No `tauri-plugin-shell`.** The two bundled tools (yt-dlp, qjs) are declared as `bundle.externalBin` (verify) so the bundler handles per-target binaries and naming; they are still spawned **directly from Rust** (`tokio::process::Command` on the resolved path), never through the shell plugin. Downloaded tools live in the app data dir. Allowed plugins: `dialog`, `opener`, `single-instance`, `updater` (+ `process` for relaunch), optionally `notification` and `clipboard-manager`. Strict CSP, no remote content, devtools off in release, minimal `capabilities/*.json`.

---

## 2. Product scope

### 2.1 Screens
**Queue** (home), **Settings** (General, Downloads, Transcription & Models, Network & Accounts, Tools, About/Updates). First-run **Setup wizard** (§11.1). Details in §11.

### 2.2 Adding links
* One input accepts **one or many links** (paste several lines; drop text or a `.txt` file of URLs; dedupe; ignore non-http(s)).
* Each link is **inspected** (§6) and classified: *video*, *playlist/channel*, *audio-only*, *live*, *error*. The user sees title, site, duration and available qualities before adding to the queue; for playlists, a selection panel (§6.4).
* Mixed sites in one paste are allowed.

### 2.3 Job options (per item, with global defaults and presets)
* **Kind:** Video / Audio only.
* **Quality:** *Best*, *max height* presets (2160/1440/1080/720/480), *exact height* (as the original), or an explicit *format id* (advanced, from the format table).
* **Container:** MP4 preferred, MKV fallback (§8.2).
* **Subtitles/Transcript** options (§10), **Encode/Trim** (§9), **Embed** metadata/thumbnail/chapters, **SponsorBlock** (YouTube), **Auto-play**, output folder and file naming.

### 2.4 Queue (§7)
Persistent, parallel (configurable), pause/resume/cancel/retry/reorder, per-item log and error detail, overall progress for playlists.

### 2.5 Transcription (§10)
Source order: **existing subtitles → Whisper (offline) → Gemini (cloud)**, selectable. Outputs `.txt`, `.srt`, `.vtt` (Whisper/subtitles) and optional `.summary.md` (Gemini).

### 2.6 Settings
Theme (System/Dark/Light), language (IT/EN), default output folder, naming template, parallelism, cookies (from browser or cookies file), proxy, rate limit, tools status/install/update, Whisper models manager, Gemini key (keyring), updates.

### 2.7 Order of operations per item
`inspect → download (+ subtitles) → merge/post-process (embed) → encode/trim → auto-play → transcript (subtitles → Whisper → Gemini) → summary`. A failure in a later stage must **not** discard earlier outputs: show "Downloaded, transcription failed: <reason>" and keep the file.

### 2.8 Persisted config
`serde(default)` on every field, unknown keys ignored, schema version number, OS config dir (Tauri path API), atomic writes (temp + rename). Queue state in a separate file (§7.4). Secrets in the OS keyring (§10.5). Optional one-time import of the legacy `config.json` (keys: `appearance_mode`, `custom_ffmpeg_enabled`, `custom_ffmpeg_path`, `gemini_api_key`, `browser_cookies`).

### 2.9 Parity checklist with the Python app (must still work)
Resolution list incl. vertical / ultrawide / 2:1 heights; audio-only to Opus; encode with CPU/GPU codecs; trim with start/end; auto-play; browser cookies; custom ffmpeg path; theme override; Gemini transcription with output language; yt-dlp update; cancel (new); URL metadata cache; debounce 500 ms on URL input; trim↔encode coupling (turning Trim on forces Encode on).

---

## 3. Architecture

```
src-tauri/src/
├── main.rs / lib.rs        // Tauri builder, plugins, single-instance, state, exit hook
├── error.rs                // AppError (thiserror) + serializable ErrorInfo {code, message_key, detail}
├── state.rs                // Settings, ToolPaths, QueueHandle, caches
├── commands/               // thin IPC handlers only (validate → call service → emit events)
│   ├── inspect.rs  queue.rs  settings.rs  tools.rs  models.rs  system.rs  updates.rs
├── services/
│   ├── tools.rs            // manifest, resolution, provisioning, checksum, self-test
│   ├── jsruntime.rs        // QuickJS→Deno strategy + per-version memory (§5.3)
│   ├── inspect.rs          // yt-dlp -J classification → Inspected model (§6)
│   ├── queue/              // scheduler, lanes, persistence, retry policy (§7)
│   ├── ytdlp.rs            // arg builders, selectors, progress/print parser, error classifier (§8)
│   ├── ffmpeg.rs           // arg builders, progress parser, probe
│   ├── encoders.rs         // catalogue + capability probing + per-encoder args (§9)
│   ├── process.rs          // spawn with Job Object / process group, tree-kill, no console window
│   ├── transcribe/
│   │   ├── mod.rs          // trait Transcriber + TextOutputs
│   │   ├── subtitles.rs    // subs-first: fetch, pick, clean (§10.1)
│   │   ├── whisper.rs      // sidecar runner, chunking, model catalogue (§10.2)
│   │   ├── gemini.rs       // REST client (§10.3)
│   │   └── summary.rs      // optional Gemini summary (§10.4)
│   ├── models.rs           // Whisper model downloads (resume, checksum), import, delete
│   └── paths.rs            // app dirs, temp dirs, sanitising, default output dir
└── Cargo.toml
```

Rules:
* `commands/` has no logic. Pure functions (arg builders, parsers, selectors, filename/time handling, error classifier) are unit-tested without spawning anything.
* **Events, never polling.** Backend emits `queue:update` (diffs of item state, coalesced to ≤ 10/s per item), `tool:progress`, `model:progress`, `toast`. Frontend keeps a store keyed by item id.
* **IPC commands:** `inspect_urls`, `queue_add`, `queue_list`, `queue_cancel`, `queue_pause`, `queue_resume`, `queue_retry`, `queue_remove`, `queue_reorder`, `queue_clear_finished`, `settings_get/save`, `secret_set/clear`, `tools_status`, `tools_install`, `tools_update_ytdlp`, `tools_self_test`, `models_list/download/cancel/delete/import`, `encoders_probe`, `open_path`, `reveal_path`, `check_app_update`.
* **Single-instance:** a second launch focuses the first window and forwards any URL given on the command line / protocol handler.

### Item pipeline (state machine)
`Queued → Inspecting → Downloading(video|audio|subs) → PostProcessing → Encoding → Transcribing(subs|whisper|gemini) → Summarizing → Done | Failed | Cancelled | Paused`.
Every stage checks the item's cancellation token and reports progress with the same payload `{stage, percent|null, speedBps|null, etaSec|null, label, subIndex?, subTotal?}`.

---

## 4. Cross-platform process handling (critical)

1. **No console windows (Windows).** Every spawned tool uses `creation_flags(CREATE_NO_WINDOW = 0x08000000)`. Do **not** use `DETACHED_PROCESS` (the Python app does).
2. **Tree kill.** `child.kill()` kills only the direct child; yt-dlp spawns ffmpeg and the JS runtime; the Whisper sidecar may spawn nothing but must be killable the same way.
   * Windows: assign each child to a **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`win32job` or `windows` crate). Terminating the job kills the tree, even if the app crashes.
   * Unix: spawn in a new process group (`process_group(0)`/`setpgid`), cancel with `killpg(SIGTERM)`, wait ~2 s, then `killpg(SIGKILL)`.
   * On app exit (`RunEvent::ExitRequested`/`Exit`) cancel all running items.
3. **Always drain stdout AND stderr** concurrently (two tasks). Keep a ring buffer of the last ~200 stderr lines per item for error classification and "Copy details".
4. **Never hold a mutex across `.await` on a child.** Use `tokio::select! { status = child.wait() => …, _ = token.cancelled() => kill_tree() }`.
5. Decode tool output with `String::from_utf8_lossy`; pass `--encoding utf-8` to yt-dlp if non-UTF-8 output is observed on Windows.
6. **argv arrays, never shell strings.** Put the URL **after `--`**. Accept only `http://` and `https://` URLs.
7. Always pass `--ignore-config` to yt-dlp (a user's global `yt-dlp.conf` must not change behaviour).
8. **Absolute tool paths only**; the app never mutates `PATH`. Pass `--ffmpeg-location`, `--js-runtimes …:<abs path>` explicitly.
9. **Private temp dir.** Each item gets `<out>/.sdp-tmp-<itemid>/` (same filesystem as the output). Set `TMPDIR`/`TEMP`/`TMP` of child processes to a private per-app temp dir created with mode `0700` (matters for QuickJS, §5.3).
10. **Orphan cleanup at startup:** remove stale `.sdp-tmp-*` dirs left by crashes (only those belonging to no queued/interrupted item).

---

## 5. External tools: manifest, resolution, provisioning

### 5.1 Tools manifest (single source of truth)
Ship `tools.manifest.json` inside the app (versioned). For each tool and platform it pins: `name`, `version`, `url`, `sha256`, `archive_kind`, `executable_name`, `min_version`. Exceptions: **yt-dlp** is always "latest" (checksum from the release's `SHA2-256SUMS`), and **models** have their own catalogue (§10.2). The manifest can be refreshed by the app's own update (no code change needed for new tool versions).

### 5.2 Lookup order and provisioning
Per tool, first hit wins; validate by running `--version` (or equivalent) with a 5 s timeout:
1. Settings override. 2. App data dir `…/tools/<tool>/` (downloaded or updated copy). 3. **Bundled baseline** (yt-dlp, qjs: next to the executable, read-only). 4. System `PATH`. 5. Well-known locations — **required because GUI apps on macOS do not inherit the shell PATH**: `/opt/homebrew/bin`, `/usr/local/bin`, `/opt/local/bin`, `~/.deno/bin`, `~/.local/bin`; Linux also `/usr/bin`, `/snap/bin`; Windows `%USERPROFILE%\.deno\bin`, `%LOCALAPPDATA%\Microsoft\WinGet\Links`.

Provisioning rules: download over HTTPS to `*.tmp` with **resume (HTTP Range)**, verify sha256, `chmod +x` on Unix, atomic rename; archives are extracted with path-traversal protection; UI shows name, version, size, progress, Cancel; never block the UI. macOS: if a downloaded binary is killed on launch, try `xattr -d com.apple.quarantine <file>` then `codesign --force -s - <file>`; otherwise show a clear message. Everything that gets updated at runtime must live in a **writable** dir (app data).

#### 5.2.1 Bundled baseline: yt-dlp + qjs
* **How:** CI downloads the target-specific qjs (pinned in the manifest) and the **latest stable yt-dlp at build time** (recording its version), verifies sha256, renames them with the target-triple suffix and lists them in `bundle.externalBin` (verify placement: bundled executables normally sit next to the main binary, without the suffix). Binaries are **never committed** to git.
* **yt-dlp must be updatable, but the install dir is read-only.** On first launch copy the bundled yt-dlp to `<app data>/tools/yt-dlp/` and always run the copy; keep the bundled one as a fallback if the copy is missing or corrupt. Updates (§5.5) replace the copy only. The version inside the installer will be stale by definition: check for a newer one in the background, non-blocking, after the first launch.
* **qjs** runs directly from the bundled location; a newer pinned version from the manifest, if ever downloaded to app data, takes precedence (§5.2 order).
* **Composable download for everything else:** the Tools panel and the wizard show what is bundled, what is installed and what can be added (ffmpeg, Deno, Whisper sidecar, VAD, models), each with size and a single Install button. Features whose tool is missing are visible but disabled, with an "Install" action — never a dead end. Note: merging separate video+audio streams needs ffmpeg, so the wizard offers it right after install if it is not found.
* **macOS (risk — spike in M0):** bundled executables must be code-signed with the hardened runtime and notarized together with the app. `yt-dlp_macos` is a PyInstaller-built binary and may require extra entitlements (e.g. allow-unsigned-executable-memory, disable-library-validation; verify). **If it cannot be notarized cleanly, do not bundle yt-dlp on macOS: download it at first run into app data instead** (downloaded files do not go through the app's notarization), and keep qjs bundled.
* **Startup cost (measure in M0):** the official single-file builds are PyInstaller one-file bundles, which unpack themselves at every launch, and this design spawns a yt-dlp process per inspection and per queue item. If the overhead is noticeable, bundle the **unpacked** variant where yt-dlp publishes it (`yt-dlp_win.zip`, `yt-dlp_linux.zip`; no built-in auto-update, so our updater would replace the folder; verify per platform) instead of the single file.
* **Windows (risk — check in M0):** PyInstaller executables sometimes trigger antivirus/SmartScreen heuristics. Test the signed installer on a clean Windows machine with Defender enabled and record the result.
* **Linux:** inside an AppImage the bundled files live in a read-only mount; the copy-to-app-data step above handles updates. `.deb`/`.rpm` install system-wide, also read-only.
* **Licences:** yt-dlp (Unlicense) and QuickJS-NG (MIT) allow redistribution (verify); ship their notices in About.

| Tool | Role | Source (verify names) | Notes |
| :-- | :-- | :-- | :-- |
| **yt-dlp** | downloads | GitHub `yt-dlp/yt-dlp` latest release, **standalone** build: `yt-dlp.exe` (Win x64), `yt-dlp_macos` (universal), `yt-dlp_linux`, `yt-dlp_linux_aarch64` | Never the Python zipapp or `yt-dlp_min.exe`. Official standalone builds bundle the EJS challenge scripts and `curl_cffi` (needed for `--impersonate`, §6.3). **Bundled in the installer** (§5.2.1) and updated afterwards in app data. |
| **qjs** (QuickJS-NG) | JS runtime #1 | GitHub `quickjs-ng/quickjs` release (pin tag ≥ 0.12.0; bare binaries such as `qjs-linux-x86_64`, `qjs-linux-aarch64`, `qjs-darwin` or `qjs-darwin-arm64`/`-x86_64`, `qjs-windows-x86_64.exe`; names changed between tags) | **Rename to `qjs` / `qjs.exe`** (yt-dlp requires that name unless the full path is passed). Tiny compared with Deno. Pin sha256 in the manifest. **Bundled in the installer** (§5.2.1). |
| **deno** | JS runtime #2 (fallback) | GitHub `denoland/deno` zips: `deno-x86_64-pc-windows-msvc`, `deno-x86_64-apple-darwin`, `deno-aarch64-apple-darwin`, `deno-x86_64-unknown-linux-gnu`, `deno-aarch64-unknown-linux-gnu` | ≥ 2.3.0. Download `deno`, **not** `denort`. **Not bundled.** Lazy: only when the fallback triggers (with user consent). |
| **ffmpeg / ffprobe** | media | Prefer the system install. Offer a static build if absent (Windows/Linux: BtbN `FFmpeg-Builds`; Linux alt: John Van Sickle; macOS: static build source or `brew install ffmpeg`) | Needs `libopus`, `libmp3lame`, `libx264`; hardware encoders depend on the build (§9.3). Show licence (GPL/LGPL). |
| **sdp-whisper** | offline STT | **Our own CI build** of whisper.cpp's `whisper-cli` (§5.4), published as assets of our repo's release | Upstream publishes Windows zips and a macOS xcframework but **no Linux binaries**, so build all targets ourselves for consistency. |
| **Silero VAD model** | skip silence | HF `ggml-org/whisper-vad` (small file) | Optional, improves long/noisy audio (§10.2). |
| **Whisper models** | offline STT | HF `ggerganov/whisper.cpp` → `…/resolve/main/ggml-<id>.bin` | User-selected sizes (§10.2). |

### 5.3 JavaScript runtime strategy (QuickJS first, Deno fallback)
Facts (yt-dlp EJS wiki): Deno is the recommended and default runtime (sandboxed, min 2.3.0); Node, QuickJS and Bun (deprecated) are alternatives; QuickJS needs a recent version (QuickJS ≥ 2025-4-26 or QuickJS-NG ≥ 0.12.0, otherwise runs can take minutes), must be named `qjs`, and writes temporary files for each EJS execution (theoretical TOCTOU risk). Official standalone yt-dlp builds already include the EJS scripts.

Implementation:
1. **Force a single runtime per attempt.** yt-dlp enables Deno by default and prefers higher-priority runtimes (order: deno, node, quickjs, bun), so pass `--no-js-runtimes` **before** the runtime flag:
   * attempt A: `--no-js-runtimes --js-runtimes quickjs:<abs qjs>`
   * attempt B: `--no-js-runtimes --js-runtimes deno:<abs deno>`
2. **Auto mode (default):** use A. Fall back to B once when: the error classifier returns `YT_CHALLENGE` (§8.5); or the inspect/download made no progress for **90 s** while the JS challenge was being solved; or a YouTube URL returns zero video formats with a height. Download Deno on demand (ask once, remember the answer).
3. **Remember the outcome** per `(yt-dlp version, qjs version)`: if A failed and B worked, start with B for 24 h or until yt-dlp/qjs changes; then re-test A. Settings: *Auto (QuickJS→Deno)* | *QuickJS only* | *Deno only*.
4. If the EJS scripts are reported missing (non-standalone yt-dlp), retry with `--remote-components ejs:github` (works with QuickJS; the `ejs:npm` source only works with Deno/Bun).
5. Private TMP dir (§4.9) for any process using QuickJS.
6. **Self-test** (Settings → Tools): runs a known public video with `--simulate`, reports runtime used and elapsed time. Use it in CI on the three OSes (acceptance #3).
7. Apply the runtime flags to every yt-dlp call; they are harmless for non-YouTube sites. Keep the YouTube client list (`youtube:player_client=web_embedded,web,tv`, from the original) in one constant, user-overridable in an expert field, and pass it **only for YouTube hosts**.

### 5.4 Whisper sidecar build (CI) — keeps the app small and crash-safe
Why a sidecar CLI and not `whisper-rs`: no C++/CMake in the app's own build, no FFI, a crash or hang in native model code cannot take the app down, **cancel = kill the process**, progress comes from the CLI, and we control the whisper.cpp version (the `whisper-rs` crate vendors an older whisper.cpp than the newest upstream tags).
* A CI workflow builds `whisper-cli` from a **pinned whisper.cpp tag** for: Windows x64, macOS (universal arm64+x86_64, Metal), Linux x64 (and optionally Linux arm64, Windows ARM64 where a build path exists). Static where possible: `-DBUILD_SHARED_LIBS=OFF`; **portable CPU code** (`-DGGML_NATIVE=OFF`, so older CPUs do not crash with illegal instructions); Windows static CRT; macOS `-DGGML_METAL=ON` with the Metal library embedded (verify the option name, e.g. `GGML_METAL_EMBED_LIBRARY`); Linux built on the **oldest supported distro** (Ubuntu 22.04) with `-static-libstdc++ -static-libgcc`.
* Output asset: `sdp-whisper-<target-triple>[.exe]` + `SHA256SUMS`, attached to a release in our repo; `tools.manifest.json` points to it.
* Optional later: Vulkan build variant for Windows/Linux. Never required; CPU is the default for resilience, and the runner retries with `--no-gpu` (`-ng`) if a GPU build fails (§10.2).
* The sidecar CLI flags used (verify with `-h` of the pinned build): `-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`, `-of`, `--vad`, `-vm`, `-ng`, `-t`.

### 5.5 yt-dlp updater
Primary: run the binary's own updater on the **app-data copy** (`yt-dlp -U`, or `--update-to <channel>`); fallback: download the platform asset and verify against `SHA2-256SUMS`, swap atomically. **No restart needed** (each job starts a new process). Show the installed version in Settings. Channels: *stable* (default) and *nightly* (expert). Check for a newer release at most once per day in the background and non-blocking; also suggest the update when a `YT_CHALLENGE`/`YT_EXTRACT` error occurs (old yt-dlp versions often stop working with YouTube).

---

## 6. Link inspection and supported sources

### 6.1 Principle
yt-dlp supports many sites; the Python app only exposed YouTube-shaped behaviour. The port treats **every URL generically** and adds YouTube-specific refinements only when the host is YouTube.

### 6.2 Inspect command and result model
```
yt-dlp --ignore-config -J --flat-playlist --no-warnings
       --no-js-runtimes --js-runtimes <runtime>:<path> [--extractor-args "youtube:…" (YouTube hosts only)]
       [cookies / headers / proxy / --impersonate …] [--playlist-end 1000] -- <url>
```
* With `--flat-playlist`, a single video returns full detail (formats), while a playlist returns a light list: **one call classifies the URL** by the top-level `_type` (`video` vs `playlist`).
* Deserialize into:
  ```
  enum Inspected {
    Video   { id, title, duration, site (extractor_key), webpage_url, is_live, language?, thumbnail_url?,
              formats: Vec<FormatLite{ format_id, ext, height?, width?, fps?, vcodec?, acodec?, filesize?, protocol, note? }>,
              heights: Vec<u32>, has_video: bool, has_audio: bool },
    Playlist{ id, title, site, kind: Playlist|Channel|Mix|Other, count: Option<u32>,
              entries: Vec<EntryLite{ index, id, url, title?, duration?, available: bool }>, truncated: bool },
    Error(ErrorInfo)
  }
  ```
* Resolution list for a video = distinct `height >= 144` among formats with a video codec (exclude storyboards/`mhtml`); sorted descending; label `"{h}p"`. **Fallback:** if nothing, retry the filter without the codec condition (as the original). If there is still no height (audio-only source, or a generic/HLS site without heights), expose only **Best available** and **Audio only** — never fail just because heights are missing (the Python app does).
* Keep only a small cache per URL (`{title, duration, heights, site}`), cleared on "Clear"; never the whole JSON.
* Ignore stale responses (request id); cancel the previous inspection when the URL changes; debounce 500 ms; timeout 60 s (120 s for playlists); always cancellable.
* Concurrency: inspect up to 3 URLs in parallel when many are pasted.

### 6.3 Non-YouTube sources ("versatility")
1. **Generic by default.** Do not assume YouTube: no YouTube extractor args, no `bestvideo`-only selectors, `height` may be null, titles may be empty/duplicated, URLs may be direct media files or HLS/DASH manifests (yt-dlp's generic extractor handles them).
2. **Show the site** (`extractor_key`) as a badge on every item; unsupported links produce a clear "link not supported / no media found" message with a hint to update yt-dlp.
3. **Authentication/anti-bot toolbox**, per item or global, all optional:
   * cookies from browser (`--cookies-from-browser`) **or a cookies file** (`--cookies <file.txt>`, Netscape format) — the file path is more robust than browser decryption on Windows/Linux;
   * custom `Referer`/`User-Agent`/headers (`--referer`, `--add-header`), proxy (`--proxy`), rate limit (`--limit-rate`);
   * **Impersonation**: on `HTTP 403`/anti-bot errors from non-YouTube hosts, retry **once** with `--impersonate chrome` (official standalone builds include `curl_cffi`; check `--list-impersonate-targets` at startup and cache it).
4. **Live streams** (`is_live`): show a badge; ask "download from now / from start (`--live-from-start`)" or block with an explanation; never start silently. Warn that livestream downloads can be unbounded in size.
5. **DRM/protected media:** detect the yt-dlp error and show "DRM-protected content is not supported". Do not try to circumvent.
6. **Format selection that tolerates missing metadata** (§8.2) and a one-shot retry **without `-f`** when "Requested format is not available".
7. **Advanced format picker** (collapsed by default): a table of `FormatLite` (id, ext, resolution, fps, codecs, size, protocol) so the user can pick an explicit format id for odd sites.
8. **File naming** always includes the id by default (`%(title).150B [%(id)s].%(ext)s`), because titles on other sites are often empty or duplicated; a setting removes the id.
9. Respect geo/age/login errors with specific messages (§8.5) and suggest cookies.

### 6.4 YouTube playlists and channels
**URL cases** and behaviour:
| URL | Behaviour |
| :-- | :-- |
| Playlist (`list=PL…`) | Enumerate with `--flat-playlist`, open the selection panel. |
| Video + playlist (`watch?v=…&list=…`) | Ask once: *This video only* / *Whole playlist* (remember per session, setting to always ask / always video / always playlist). Single → re-inspect with `--no-playlist`. |
| Channel / tab (`/@name/videos`, `/shorts`, `/streams`) | Treated as a playlist; paginate (see below). |
| Mix / radio (`list=RD…`) | Dynamic and effectively endless: warn, cap to the first 50 by default. |
| Watch Later / Liked / private playlists | Need cookies: detect the auth error and offer to set cookies. |
| Search / feed pages | Not supported in v1; show a clear message. |

**Playlist selection panel:**
* Header: playlist title, site, entry count, total duration if known.
* Checkbox list with **Select all / none / invert**, shift-click ranges, text filter, "from # to #", reverse order toggle. Entries marked private/deleted/unavailable are shown disabled and unchecked.
* Options: **folder per playlist** (default on, sanitized name), **numbering prefix** (`001 - `), **quality preset** (max height, because each video has different heights: use the "max height" selector, §8.2), audio-only, subtitles/transcript, **skip already downloaded** (download archive, default on for playlists).
* Button "Add N items to queue".

**Enumeration and large lists:** list the first 1,000 entries (`--playlist-end 1000`), then offer "Load more" (`--playlist-items 1001:2000`); optionally use `--lazy-playlist` (verify) so the list starts appearing before enumeration finishes. Virtualize the list.

**Enqueuing:** enqueue **explicit per-entry URLs** (one queue item per video), not a single "playlist process". Each item runs its own yt-dlp process with `--no-playlist`. Benefits: per-item progress, retry, cancel, reorder, parallelism, and a failed/private video never aborts the rest. Entry URL: use the entry's `url` if http(s); otherwise reconstruct from `id` + `ie_key` (YouTube: `https://www.youtube.com/watch?v=<id>`).

**Naming:** the app supplies the index itself (yt-dlp does not know it in single-URL mode): `-o "<NNN> - %(title).150B [%(id)s].%(ext)s"` with the literal prefix escaped (`%` → `%%`), under `<out>/<Sanitized playlist title>/`. Sanitize folder names (reserved Windows names, trailing dots/spaces, max ~100 chars).

**Archive:** `--download-archive <app data>/archive.txt` when "skip already downloaded" is on. Provide "Ignore archive for this run" and "Clear archive" in Settings, because deleting files does not clean the archive.

**Pacing:** for ≥ 20 items add `--sleep-interval 1 --max-sleep-interval 4` (verify) and keep **max 2 concurrent items per host** by default, to reduce throttling/bot detection. Configurable.

**Failure policy:** continue on error; classify each failure (§8.5); "Retry failed" bulk action.

---

## 7. Queue engine

### 7.1 Model
Each queue item stores an **immutable snapshot of its options** (kind, quality, naming, embed, transcript settings, cookies/headers/proxy, runtime) so later changes to global settings never alter queued work. Items belong optionally to a **group** (playlist) with folder, index and group progress ("7/42").

### 7.2 Lanes and concurrency
Resource-aware scheduling with separate semaphores:
* **Download lane:** default 2 parallel items (1–4 configurable), plus the per-host cap of §6.4.
* **Encode lane:** 1 by default (CPU/GPU heavy).
* **Transcribe lane:** **1** (Whisper is CPU/GPU heavy; Gemini calls may run in a separate lane of 2).
An item holds only the lane of its current stage, so downloads continue while a previous item is transcribing.

### 7.3 Actions and policies
* **Pause/Resume:** pause kills the process and keeps partial files; resume re-runs the stage (yt-dlp continues from `.part`). "Pause all" for the whole queue.
* **Cancel:** kill the process tree (§4), delete the item's temp dir, never touch finished outputs or the original file of a previous stage.
* **Retry:** manual per item and "Retry failed". **Automatic retry** only for transient classes (`NETWORK`, HTTP 5xx, timeouts): up to 2 times with exponential backoff (e.g. 5 s, 20 s). Permanent classes (private, unavailable, DRM, unsupported, disk full) are never auto-retried.
* **Reorder:** drag and drop, "move to top"; the scheduler always picks the first runnable item.
* **Remove / Clear finished**; keep the last 500 finished items as history, then roll off.
* **Duplicate detection:** the same normalized URL + same options already queued → ask "add anyway?".

### 7.4 Persistence
`queue.json` in the app data dir: schema version, items, groups, order. **Atomic writes** (temp + rename), debounced (≤ 1 write/s while running), plus a write on every state change that matters (add/remove/finish). On startup items in an active stage become **Interrupted** and are shown with a "Resume" button (or auto-resume if the setting is on). Corrupt file → keep a `.bak`, start empty, show a toast.

### 7.5 Aggregation
Group progress = completed items / total, with the sum of known durations for an ETA when available. Global footer shows "N running · M queued · speed".

### 7.6 Events and throttling
Coalesce progress per item and emit at most ~10 updates/s; send only changed fields. The frontend must not re-render the whole list on every tick (keyed store + virtualized list, §11.7).

### 7.7 Notifications and housekeeping
Optional system notification when the **queue becomes idle** (not per item) with "Open folder". Orphan temp cleanup at startup (§4.10). Optional "keep the computer awake while the queue is running" (verify per-OS API; do not block shipping on it).

---

## 8. Download execution (yt-dlp CLI)

### 8.1 Never replicate these original options
`nocheckcertificate` and `prefer_insecure` (used by the original metadata call) disable TLS verification. **Do not pass `--no-check-certificates` or `--prefer-insecure`.**

### 8.2 Base arguments and format selectors
Common: `--ignore-config --no-playlist --newline --progress --no-warnings --color never`, `--ffmpeg-location <ffmpeg>`, runtime flags (§5.3), cookies/headers/proxy/impersonation (§6.3), `--concurrent-fragments 4`, `--retries 10 --fragment-retries 10 --socket-timeout 30` (verify defaults), optional `--limit-rate`.

Paths: `--paths home:<out_dir>` and `--paths temp:<out_dir>/.sdp-tmp-<itemid>`; `-o "<template>.%(ext)s"` with `--windows-filenames` (safe names on all OSes; the original regex `[^a-zA-Z0-9_. -]→-` destroys accents and non-Latin titles — do not port it). Default template `%(title).150B [%(id)s]`; playlist items get the numeric prefix (§6.4).

**Container:** `--merge-output-format mp4/mkv` (verify): prefer MP4, fall back to MKV when the streams are not MP4-compatible. Never assume the final extension is `.mp4`; later stages use the real path.

**Format selectors** (build in one pure function, unit-tested; `N` = height):
* *Exact height* (single video chosen from the list, parity with the original): `bv[height=N]+ba/bv[height<=N]+ba/b[height<=N]/b`
* *Max height* (presets, playlists, sites with unreliable heights): `bv*[height<=?N]+ba/b[height<=?N]/b` — the `?` keeps formats whose height is unknown (verify with `-F`).
* *Best*: `bv*+ba/b`
* *Audio only*: `-f ba/b -x --audio-format opus` (Ogg Opus `.opus`), also works for sources that only have muxed audio+video.
* *Explicit format id*: `-f <id>+ba/<id>` for video-only ids; `-f <id>` otherwise.
* If yt-dlp reports "Requested format is not available", retry **once** without `-f`.
* Optional setting (default off): "Prefer maximum compatibility (H.264/AAC)" using `-S vcodec:h264,res,acodec:m4a`.

### 8.3 Machine-readable output
(verify exact template syntax with the pinned yt-dlp)
```
--progress-template "download:SDP_DL|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.vcodec)s"
--progress-template "postprocess:SDP_PP|%(progress.status)s|%(progress.postprocessor)s"
--print "after_move:SDP_FILE|%(filepath)s"
```
* The **final path comes from `SDP_FILE`**, never guessed from the title.
* Percent = `downloaded / (total or total_estimate)`; unknown total → indeterminate bar. Speed shown in **Mbit/s** (`bytes/s × 8 / 1024²`, as the original); show ETA.
* Label the two streams "Video 1/2" and "Audio 2/2" and show "Merging…" during post-processing (the Python app shows "completed" when the first stream finishes).

### 8.4 Extras (all optional, per preset)
* **Embed:** `--embed-metadata --embed-thumbnail --embed-chapters`; convert thumbnails to JPG first (`--convert-thumbnails jpg`) because some containers cannot embed WebP. Treat embedding failures as warnings, not job failures (support differs per container; verify).
* **Subtitles with the video** (when the user wants them as sidecar files): see §10.1.
* **SponsorBlock (YouTube only):** default **off**. Modes: *mark as chapters* (`--sponsorblock-mark <cats>`) or *remove* (`--sponsorblock-remove <cats>`, forces ffmpeg cutting; warn about re-muxing and possible slight A/V cuts). Default categories when enabled: `sponsor,selfpromo,interaction`; user-selectable.

### 8.5 Error classification
Map the stderr ring buffer to stable codes and localized, actionable messages (verify wording against the pinned version):
| Pattern (substring, case-insensitive) | Code | UI hint |
| :-- | :-- | :-- |
| ffmpeg not found / spawn error on ffmpeg | `FFMPEG_MISSING` | open Tools |
| `HTTP Error 403`, `n challenge`, `nsig`, `Signature extraction failed`, "JS runtime"/challenge solving problems | `YT_CHALLENGE` | try the other JS runtime (§5.3), update yt-dlp, try cookies |
| `Sign in to confirm`, `not a bot`, age-restricted, private video, login required | `AUTH` | set cookies (browser or file) |
| `Video unavailable`, `not available in your country`, geo | `UNAVAILABLE` / `GEO` | — / proxy hint |
| `Requested format is not available` | `FORMAT` | one retry without `-f`, then choose another quality |
| `Unsupported URL`, `No video formats found`, generic-extractor failure | `UNSUPPORTED` | update yt-dlp; maybe not a media page |
| `DRM` | `DRM` | not supported |
| `HTTP Error 403`/anti-bot on non-YouTube host | `ANTIBOT` | one retry with `--impersonate chrome` |
| cookie DB / decrypt / keyring errors | `COOKIES` | close the browser, try a cookies file |
| `No space left` / `ENOSPC` | `DISK_FULL` | — |
| DNS/timeout/connection/5xx | `NETWORK` | auto-retry (§7.3) |
| `Extractor`/`Unable to extract` | `EXTRACT` | update yt-dlp |
| anything else | `UNKNOWN` | show last stderr lines + "Copy details" |

The original only showed "Download failed!". Always keep the raw detail one click away.

### 8.6 Browser cookies
`--cookies-from-browser <name>` with names `chrome, edge, firefox, brave, opera, vivaldi, chromium` and `safari` **only on macOS**; filter the dropdown by OS. Known limits to surface in the UI: Chromium-family decryption can fail on Windows (app-bound encryption / DB locked while the browser runs → close the browser, use Firefox, or use a cookies file); on Linux Chromium decryption may need the desktop keyring; macOS Safari needs Full Disk Access. Always offer the **cookies file** alternative (§6.3).

---

## 9. Media processing (ffmpeg)

### 9.1 Progress
Run with `-nostdin -hide_banner -progress pipe:1 -nostats`. Parse `out_time_us=` (microseconds; ignore `N/A`/negative values) over the expected output duration (from yt-dlp metadata, or `ffprobe`). **Read stderr too** (§4.3) and **check the exit code**: the Python app ignores it and then deletes the input.

### 9.2 Encode (video) and trim
Output name `<name>-encoded.mp4` (as the original). Delete the original **only if ffmpeg exited 0 and the output is non-empty**; otherwise keep it and report the error. Setting "Keep original after encode" (default off = same as the original app).

Base command: `ffmpeg -y -nostdin -ss <start>? -t <dur>? -i <in> -c:v <venc + args> -c:a <aenc> -movflags +faststart <out>`.

**Trim:** accept `SS(.ms)`, `MM:SS(.ms)`, `HH:MM:SS(.ms)`; validate `start < end` and both inside the media duration; reject garbage with an inline error (the original silently turns invalid input into 0 s). Put `-ss` **before** `-i` and use `-t (end − start)` (unambiguous; accurate because trim always re-encodes). Expected duration for progress = `(end or total) − start` (the original only computes this when both bounds are set). Trim implies Encode (coupling kept from the original).

### 9.3 Encoder catalogue and capability probing
The original detects the GPU vendor via WMI on Windows only. The port must work on all OSes **without OS-specific vendor queries**:
1. Candidates per OS — all: `libx264`, `libx265`; Windows: `*_nvenc`, `*_amf`, `*_qsv`; Linux: `*_nvenc`, `*_vaapi`, `*_qsv`; macOS: `*_videotoolbox` (for both h264 and hevc).
2. Filter by `ffmpeg -hide_banner -encoders`.
3. **Probe** survivors with a short real encode, e.g. `ffmpeg -hide_banner -f lavfi -i color=c=black:s=256x256:d=0.2 -c:v <enc> <args> -f null -` (VAAPI: add `-vaapi_device /dev/dri/renderD128 -vf format=nv12,hwupload`). 256×256 because hardware encoders reject tiny frames. Keep encoders that exit 0. Run in the background (limited concurrency, 10 s timeout each), cache by (ffmpeg path, mtime, version), refresh from Settings. Show "Detecting hardware encoders…" instead of blocking.
4. Labels (keep original wording): `x264`, `x265`, `Nvidia x264/x265`, `AMD x264/x265`, `Intel x264/x265`, plus `VideoToolbox x264/x265` (macOS) and `VAAPI x264/x265` (Linux).

**Per-encoder arguments** (starting points; validate by test and tune):
| Encoder | Args |
| :-- | :-- |
| `libx264` | `-preset medium -crf 20 -pix_fmt yuv420p` |
| `libx265` | `-preset medium -crf 24 -pix_fmt yuv420p -tag:v hvc1` |
| `h264_nvenc` / `hevc_nvenc` | `-preset p5 -rc vbr -cq 23 -b:v 0` (+ `-tag:v hvc1` for HEVC) |
| `h264_amf` / `hevc_amf` | `-quality quality -rc cqp -qp_i 22 -qp_p 24` (+ `-tag:v hvc1`) |
| `h264_qsv` / `hevc_qsv` | `-global_quality 23 -preset medium` (+ `-tag:v hvc1`) |
| `h264_videotoolbox` / `hevc_videotoolbox` | `-q:v 60` (fallback `-b:v` if rejected) (+ `-tag:v hvc1`) |
| `h264_vaapi` / `hevc_vaapi` | `-vaapi_device /dev/dri/renderD128 -vf format=nv12,hwupload -qp 23` |

If a hardware encoder fails at job time, **retry once with the CPU encoder** of the same codec and tell the user.

### 9.4 Audio-only encode
`aac` → `-c:a aac -b:a 192k` into **`.m4a`** (the original wrote raw `.aac`); `mp3` → `libmp3lame -q:a 2` `.mp3`; `opus` → `libopus -b:a 128k` `.opus` (the original used `.ogg`). Output `<name>-encoded.<ext>`, same delete-original and trim rules.

### 9.5 Auto-play and reveal
Use the `opener` plugin (ShellExecute / `open` / `xdg-open`); never `cmd /C start`. Failure to open never fails the item. Add "Reveal in folder".

---

## 10. Transcription and text outputs

Common rules: audio is taken from the **final media file** (after encode/trim). Outputs go next to the media (setting: `transcripts/` subfolder): `<name>.txt`, `<name>.srt`, `<name>.vtt`, `<name>.summary.md`, UTF-8. Errors are `Result<_, AppError>` — **never detect failure by searching the transcript for the word "Errore"** (the original does). Cancelling must stop the stage within ~2 s and leave no partial outputs.

### 10.1 Subtitles first (fast path, zero compute)
If the source already has subtitles, prefer them over running any model.
1. Choose languages: user setting, else the video's `language` metadata, else UI language + English.
2. Fetch **manual** subtitles first, then **automatic** captions if none: `yt-dlp --skip-download --write-subs [--write-auto-subs] --sub-langs "<langs>,-live_chat" --sub-format "srt/vtt/best" --convert-subs srt -o "sub.%(ext)s"` into the item's temp dir (`-live_chat` excluded; verify). For YouTube prefer the original-language auto track (often suffixed `-orig`; verify) over auto-translated tracks.
3. Mark the origin in the UI and in the sidecar name (`.it.srt`, `.it.auto.srt`).
4. Produce `.txt` by cleaning the subtitle file: strip tags/timestamps/styling, **de-duplicate rolling repeated lines** typical of auto-captions, join into paragraphs on long pauses.
5. Offer "Re-transcribe with Whisper" on the item if the auto captions look poor. Source order is configurable: *Subtitles → Whisper → Gemini* (default), *Whisper only*, *Gemini only*, *Subtitles only*.

### 10.2 Offline engine: Whisper (sidecar) — simple and resilient
**Pipeline**
1. **Prepare audio** with the user's ffmpeg: `ffmpeg -nostdin -i <file> -vn -ac 1 -ar 16000 -c:a pcm_s16le <tmp>.wav` (feeding a plain 16 kHz mono WAV avoids container/format surprises; 1 hour ≈ 115 MB of temp disk — check free space first).
2. **Chunking for very long audio:** whisper.cpp loads the whole audio in memory. Above **2 hours**, split into ~30-minute chunks at silence points (`ffmpeg … silencedetect`, nearest silence within ±20 s of each boundary; hard cut with 2 s overlap if none), transcribe sequentially and **offset timestamps** when merging SRT/VTT.
3. **Run** `sdp-whisper` (flags verify with `-h` of the pinned build):
   `sdp-whisper -m <model.bin> -f <wav> -l <auto|xx> [-tr] -pp -otxt -osrt -ovtt -of <outbase> [--vad -vm <silero.bin>] [-t N] [-ng]`
   * `-l auto` detects the language from the first seconds; **default to the video's language when known** (wrong detection happens with music/intros); a language dropdown overrides it. `-tr` = translate to English (Whisper can only translate *to English*; other target languages go through Gemini).
   * `--vad` (skip silence) **on by default** when the VAD model is installed: reduces hallucinations and speeds up silent stretches.
   * `-t`: default = let the sidecar choose; expose "CPU threads" in advanced settings.
4. **Progress:** parse `-pp` lines on stderr (`progress = N%`, verify format). Optionally parse stdout segment lines (`[hh:mm:ss.mmm --> …] text`) to show a **live transcript preview** in the item.
5. **Collect** `<outbase>.txt/.srt/.vtt`, move/rename to final names, delete temp WAV and partials.
6. **Failure handling:** non-zero exit → if the build used GPU, **retry once with `-ng`**; otherwise fail with the last stderr lines. Missing model/sidecar → offer the download instead of failing.
7. **Hallucination guard:** after a run, detect obvious loops (the same segment repeated ≥ 5 times) or suspicious silence-only output and warn the user ("try VAD, a larger model, or set the language").
8. Cancel = kill the process tree; remove partial outputs and temp WAV.

**Models: user chooses the size** (HF `ggerganov/whisper.cpp`, file `ggml-<id>.bin`; sizes from the repository table; indicative Italian error rates from FLEURS tests at Q8, lower is better). Ship this as `whisper.models.json` (id, label, file, url, size_bytes, sha256 pinned at implementation time from the HF file page/API, RAM estimate, tier, indicative WER, `multilingual: true`):
| Label | File id | Size | Indicative IT WER | Notes |
| :-- | :-- | :-- | :-- | :-- |
| Tiny | `tiny-q5_1` | 31 MiB | n/a | drafts only; poor for Italian |
| Base | `base-q5_1` | 57 MiB | n/a | very light |
| **Small** | `small-q5_1` | 181 MiB | ~8 % | lightest "usable" option |
| Small (higher quality) | `small-q8_0` | 252 MiB | ~8 % | |
| **Medium** | `medium-q5_0` | 514 MiB | ~4 % | |
| Medium (higher quality) | `medium-q8_0` | 785 MiB | ~4 % | |
| **Turbo — recommended** | `large-v3-turbo-q5_0` | 547 MiB | ~2.8 % | best quality/size trade-off |
| Turbo (higher quality) | `large-v3-turbo-q8_0` | 834 MiB | ~2.8 % | |
| **Large v3** | `large-v3-q5_0` | 1.1 GiB | ~2.5 % | most accurate, slowest |
| Turbo / Large full precision | `large-v3-turbo`, `large-v3` | 1.5 / 2.9 GiB | — | hidden under "Advanced" |
Multilingual models only (no `.en` variants in the main list; allow them under Advanced). Default selection: **Turbo q5_0**; if the machine has few cores (`available_parallelism()` ≤ 4) suggest **Small q5_1** and show a speed warning. Do not claim processing speeds in the UI; show an ETA computed from the first minute of progress.

**Download manager (resilience):** HTTP Range resume into `*.part`, retry with backoff, verify sha256, disk-space check before starting, progress events, cancel; configurable mirror/base URL (HF can be blocked); **"Import model file…"** for offline machines; list installed models with size and **Delete** (frees disk). Several models can coexist; one is "active" and can be overridden per item. The VAD model is fetched together with the first model (tiny file).

**GPU:** macOS build uses Metal (fallback `-ng`). Windows/Linux default CPU; an optional Vulkan sidecar variant can be added later (§5.4). Never block on GPU availability.

### 10.3 Cloud engine: Gemini (REST, no SDK)
Base `https://generativelanguage.googleapis.com`; authenticate with the header **`x-goog-api-key`** (never in the URL). Verify endpoints/fields against current docs.
1. **Prepare audio:** `ffmpeg -nostdin -i <file> -vn -ac 1 -ar 16000 -c:a libopus -b:a 24k -f ogg <tmp>.ogg` → MIME `audio/ogg`. Documented supported formats include WAV, MP3, AAC, FLAC, OGG, AIFF; if `libopus` is missing or the API rejects the file, fall back to MP3 (`audio/mpeg`) or FLAC. (Gemini downsamples audio to 16 Kbps, so 24 kbps mono is plenty.) The Files API is required above 20 MB request size, and the limit is 9.5 hours of audio per prompt.
2. **Upload** (resumable protocol): `POST /upload/v1beta/files` with `X-Goog-Upload-Protocol: resumable`, `X-Goog-Upload-Command: start`, `X-Goog-Upload-Header-Content-Length`, `X-Goog-Upload-Header-Content-Type`, JSON body `{"file":{"display_name":"…"}}`; read `x-goog-upload-url`; then `POST` the bytes with `X-Goog-Upload-Offset: 0`, `X-Goog-Upload-Command: upload, finalize`. **Stream from disk**, never load in RAM.
3. **Poll** `GET /v1beta/<file.name>` until `state == ACTIVE` (original: every 5 s). **Timeout (10 min)** and cancellation; `FAILED` → error. (The original loops forever.)
4. **Generate:** `POST /v1beta/models/<model>:generateContent` with `contents:[{parts:[{text:<prompt>},{file_data:{mime_type,file_uri}}]}]` (text first, file second, as the original). Prompts verbatim:
   * Original language: `Transcribe the following audio file into its original language. Include accurate punctuation.`
   * Target language X: `Transcribe the following audio file, and then translate the entire transcription into X.`
   Join all `candidates[0].content.parts[].text`. Handle `promptFeedback.blockReason` and `finishReason == MAX_TOKENS` (warn: possibly truncated; for very long audio split into chunks like §10.2.2). Output: **`.txt` only** (optionally a "with timestamps" mode via prompt, labelled experimental). SRT/VTT come only from Whisper or subtitles because model-generated timestamps are not reliable.
5. **Always cleanup** (success, error, cancel): `DELETE /v1beta/<file.name>` and the temp audio, via an RAII guard.
6. **Models — do not hard-code the original list.** Several 2.5-generation models were announced for shutdown (a shutdown date of 16 Oct 2026 for `gemini-2.5-flash` is reported, plus user reports of 404 "no longer available" before that date); the current list shows 3.x Flash models. Therefore: default to the **`-latest` aliases** (`gemini-flash-latest`, `gemini-flash-lite-latest`; they are hot-swapped on new releases, with a notice for breaking changes), refresh the list from `GET /v1beta/models` (keep those whose `supportedGenerationMethods` include `generateContent`), make the combobox editable, and on a 404/"model no longer available" error show a message that refreshes the list and suggests the alias. Stay on `generateContent`; the new Interactions API is still beta.
7. Map HTTP errors: 400/403 invalid key or no access, 429 quota/rate (retry with backoff, max 3), 5xx retry, network → `NETWORK`. Missing key → "Gemini API key missing" before doing any work.
8. Privacy notice in the UI: audio is uploaded to Google.

### 10.4 Optional summary (Gemini, text only)
Per-item toggle "Summarize". Input is the **transcript text**, not the audio (cheap, fast). Output `<name>.summary.md` in a selectable language, with a short summary + key points + (if timestamps exist) chapter-like sections. For transcripts too long for one request, use map-reduce (summarize chunks, then summarize the summaries). Needs the API key; show a cost/privacy hint the first time.

### 10.5 API key storage
OS keyring (`keyring` crate: Credential Manager, Keychain, Secret Service). **Linux fallback** when no Secret Service is available: config file with permissions `0600` and a visible note. Accept `GEMINI_API_KEY` from the environment. Never log it; never send it back to the frontend (only "set / not set").

---

## 11. UI/UX specification (redesign)

Goal: a calm, fast, **queue-centric** app that feels native on all three OSes. The CustomTkinter layout (450×600, three tabs, one job at a time) is **not** to be reproduced.

### 11.1 First-run Setup wizard
Shown once (re-openable from Settings → Tools). Steps: (1) welcome + language + theme; (2) **Tools check** — yt-dlp and qjs are already bundled; detect ffmpeg and offer "Install for me" with progress only for what is missing (Deno is **not** installed here; it is lazy, §5.3); (3) optional **Transcription**: pick a Whisper model (§11.6) or skip; (4) output folder. Every step is skippable; the app must remain usable if the user skips everything and installs things later. Never block on network failures: show retry / "use my own tool path".

### 11.2 Layout and navigation
* **Window:** default 980×680, minimum 640×520, no maximum. Under ~800 px the sidebar collapses to icons.
* **Sidebar:** Queue (home), Settings. A small status footer: running/queued counts, aggregate speed, tool problems (red dot opens Tools).
* **Queue page:** top **Add bar**, then the queue list (grouped by playlist, collapsible), footer with global actions (Pause all, Clear finished).
* Empty state: friendly illustration-free hint ("Paste a link to start"), supported-sites link, and the three most useful actions.

### 11.3 Add bar, presets and link inspection UX
* A single multi-line-capable input: **paste one or many links**, drag-and-drop text or a `.txt` of URLs, **Ctrl/Cmd+V anywhere** on the page when no field is focused (opt-in: "Add links pasted into the window").
* Inspection happens as links are added (debounced 500 ms as in the original); each link shows a **row with spinner → title, site badge, duration, thumbnail (optional), quality picker**. Errors are inline per link with a **Retry**/**Details** action, never a modal.
* **Preset chips** below the input apply to everything being added: `Video ▾ (Best / 1080p / 720p / …)`, `Audio only`, `Subtitles / Transcript ▾ (Off · Subtitles→Whisper · Whisper · Gemini)`, `More options ▾`. "More options" opens a drawer with Encode/Trim, Embed, SponsorBlock, Auto-play, output folder/naming, cookies/headers/impersonation, advanced format table (§6.3.7).
* User-defined **presets** ("Podcast: audio opus + transcript", "Lecture: 720p + subtitles") can be saved and selected; the last used preset is remembered.
* A prominent **Add to queue** button (Ctrl/Cmd+Enter) and an optional "Start immediately" toggle.

### 11.4 Playlist panel and non-YouTube UX
* Playlist links open the **selection panel** of §6.4 as a slide-over/dialog with virtualized checklist, filters and options; the primary button reads "Add N items".
* "Video + list" URLs show the **This video / Whole playlist** choice inline (§6.4).
* Non-YouTube links look the same as YouTube ones (site badge differs); live/DRM/unsupported cases show a dedicated, plain-language banner; auth problems show **"Use cookies"** with the two options (browser / file) right in the banner.

### 11.5 Queue items
A card/row per item: thumbnail (optional, fetched by the backend with a size cap and shown from a local file/data URL — the CSP forbids remote images), title, site badge, **stage chip** (Inspecting · Downloading · Merging · Encoding · Transcribing · Done · Failed · Paused), a determinate/indeterminate **progress bar**, speed, ETA, size, and the stage detail ("Video 1/2", "Whisper 37 %").
* Primary actions by state: Pause/Resume, Cancel, Retry, Open file, Reveal in folder, Remove.
* **Expandable details:** chosen options, output paths (media, subtitles, `.txt/.srt/.vtt`, summary), error detail with **Copy details**, last log lines, live transcript preview while Whisper runs.
* Context menu (right click) with the same actions + "Copy link", "Open page in browser".
* Items of a playlist are grouped under a **group header** with folder name, progress "7/42", and group actions (Pause group, Retry failed, Open folder).
* Finished items can be transcribed later: **"Transcribe…"** action on any file in the list (choose Whisper model/language) — also for files that were not downloaded by the app (drop a local media file to transcribe it).

### 11.6 Settings → Transcription & Models (model manager)
* **Engine order** selector (Subtitles → Whisper → Gemini, or custom), default language (Auto / video language / explicit), "Skip silence (VAD)", "CPU threads" (advanced), "Translate to English (Whisper)".
* **Whisper models table:** one row per catalogue entry (§10.2): name, **download size**, estimated RAM, quality/speed bars, indicative Italian error rate, badge **Recommended**; per row a state: *Not installed → Download (progress, pause, cancel)* · *Installed → Use / Delete*. A radio marks the **active** model. Show free disk space and warn before large downloads (> 1 GiB). Buttons: **Import model file…**, **Open models folder**, **Mirror/base URL** (advanced).
* **Gemini:** API key field (masked; "set / not set" status, Test button), model combobox (editable, **Refresh list**), privacy note.
* **Summary** defaults (language, on/off).

### 11.7 Performance, states and feedback
* **Virtualize** any list that can exceed ~200 rows (playlist panel, queue). Keyed store; patch single rows on `queue:update`; never re-render the whole list per tick.
* Progress events are coalesced (§7.6); animate with CSS transitions only (no JS animation loops).
* Every long operation has: spinner or progress, a **Cancel**, and a failure state with a retry. No frozen UI, no blocking modals for recoverable errors; use **toasts** for transient info and **inline banners** for persistent problems.
* Respect `prefers-reduced-motion`.

### 11.8 Other settings pages
* **General:** language (IT/EN), theme (System/Dark/Light), start page, "Add links pasted into the window", notifications on queue idle, "Keep original after encode".
* **Downloads:** default folder (default = OS Downloads), naming template (with id on/off), parallel downloads (1–4), per-host cap, pacing for large playlists, rate limit, skip-already-downloaded archive controls (clear), container preference, "Prefer H.264/AAC compatibility".
* **Network & Accounts:** cookies (from browser / cookies file), proxy, custom headers, impersonation toggle ("retry anti-bot errors with browser impersonation"), JS runtime mode (§5.3).
* **Tools:** table of yt-dlp / qjs / deno / ffmpeg / sdp-whisper with source, version, status, **Install / Update / Use my own path / Test**, plus "Test YouTube extraction" (§5.3.6) and the yt-dlp channel (stable/nightly).
* **About/Updates:** app version, **Check for updates** (§13.4), licences (ffmpeg GPL/LGPL, whisper.cpp MIT, yt-dlp Unlicense, QuickJS-NG MIT, Deno MIT — verify), link to logs, "Copy diagnostics" (versions, OS, tool paths — no secrets).

### 11.9 Visual design, i18n, accessibility, shortcuts
* **Design system:** CSS variables (colour, spacing 4/8 px grid, radius, elevation), light and dark palettes, system font stack, one accent colour, compact/comfortable density option. No external fonts or images.
* **Theme:** System/Dark/Light. CSS `prefers-color-scheme` plus the Tauri window theme API (`window.theme()` + `onThemeChanged`) as the source of truth for "System" (WebKitGTK is unreliable with the media query).
* **i18n:** IT and EN dictionaries (JSON), default from OS locale, runtime switch; all strings externalized; plural rules; no concatenated sentences.
* **Accessibility:** keyboard-navigable everywhere, visible focus, ARIA roles for list/progress/dialog, labels on all controls, no meaning conveyed by colour alone, contrast ≥ WCAG AA.
* **Shortcuts:** Ctrl/Cmd+V (paste links when idle), Ctrl/Cmd+Enter (add), Del (remove selected), Space (pause/resume), Ctrl/Cmd+, (settings), Ctrl/Cmd+F (filter queue). **macOS:** the default app menu with the **Edit** menu (Cmd+C/V/X/A) must be present, otherwise clipboard shortcuts do not work in inputs; keep the native context menu on text inputs.
* Responsive to 640 px width; no horizontal scroll.

---

## 12. Behaviours of the Python app that must NOT be ported

1. TLS verification disabled (`nocheckcertificate`, `prefer_insecure`) in the metadata call.
2. Deleting the source file after encode/trim even when ffmpeg failed (return code ignored, stderr discarded).
3. Failure detection by `"Errore" in transcription`.
4. Guessing the output path from the title (`title + ".mp4"`, `+ ".opus"`) instead of asking yt-dlp.
5. ASCII-only filename sanitising (`[^a-zA-Z0-9_. -]`).
6. Infinite polling loop for Gemini `PROCESSING`; silent `None` on VOSK failure followed by a success message.
7. "Download completed" + 100 % shown when the first stream (video) finishes, before audio/merge.
8. `get_system_theme()` returns "dark" on every non-Windows OS; theme detection via the Windows registry.
9. `deno.exe` hard-coded; `PATH` mutation; files written next to the executable (config, models, temp WAV, output) — not writable for installed apps and not portable.
10. Updater that downloads the Python **zipapp** and needs a restart.
11. Trim input silently turned into `0.0` on parse errors; trim duration only computed when both bounds are set.
12. Stale metadata responses can overwrite the result of a newer URL (no cancellation of the previous inspection).
13. `DETACHED_PROCESS` creation flag; no cancellation at all.
14. API key stored in plaintext JSON next to the exe.
15. The assumption that a video always has `height` formats (fails on audio-only or generic sources).
16. The hard-coded Gemini model list and the VOSK language catalogue of two languages.

---

## 13. Platform notes, packaging, updates, CI

### 13.1 Windows
WebView2 runtime is present on Windows 11 and most updated Windows 10; keep the Tauri `downloadBootstrapper` mode (small installer). NSIS installer preferred for size. Code signing recommended (SmartScreen); verify antivirus behaviour with the bundled yt-dlp (§5.2.1). Windows ARM64: build the app; verify availability of yt-dlp/ffmpeg ARM (or x64-emulated) builds; QuickJS-NG ships `qjs-windows-x86_64.exe` (run under emulation, verify); the Whisper sidecar may need a dedicated ARM64 build job.

### 13.2 macOS
Universal binary (`--target universal-apple-darwin`). Distribution requires Developer ID signing + notarization (hardened runtime), **including the bundled tools** (§5.2.1). Mind: PATH (§5.2), signatures/quarantine of downloaded tools (§5.2), Safari cookies permission (§8.6), VideoToolbox encoders (§9.3), Metal in the Whisper sidecar with `-ng` fallback.

### 13.3 Linux
Tauri v2 needs `libwebkit2gtk-4.1` and GTK3 at runtime (Ubuntu 22.04+/Debian 12+/Fedora current). Provide `.deb`, `.rpm` and AppImage. If the window is blank or broken on some GPU/Wayland combinations, document/set the fallback `WEBKIT_DISABLE_DMABUF_RENDERER=1`. Folder picker and `xdg-open` rely on portals/`xdg-utils`: show a friendly message if missing. Keyring may be unavailable (§10.5). VA-API needs `/dev/dri/renderD128` access. The Whisper sidecar and qjs are built/downloaded for glibc Linux x64 (and aarch64 where available).

### 13.4 App auto-update (Tauri updater)
* Use `tauri-plugin-updater` (+ `plugin-process` for relaunch). Generate a signing key pair once; **the private key lives only in CI secrets**; the public key goes in `tauri.conf.json` (`plugins.updater.pubkey`); set `bundle.createUpdaterArtifacts = true`.
* Endpoint: a static `latest.json` on GitHub Releases (`…/releases/latest/download/latest.json`). **Add a CI check that fails the release if `latest.json` or any `.sig` is missing** (the official GitHub action has been reported to skip the manifest when signatures are not collected).
* **Linux:** only the AppImage bundle is auto-updatable; for `.deb`/`.rpm` show "New version available" with a link instead.
* UX: check on startup at most once per day (non-blocking), show a non-modal banner "Update available — Install & restart"; never interrupt a running queue (offer to install when idle).
* Tool updates (yt-dlp, manifest-pinned tools) are independent of app updates (§5.5).

### 13.5 CI
GitHub Actions matrix (windows-latest, macos-latest incl. x64 target, ubuntu-22.04) with `tauri-apps/tauri-action`. Jobs: (a) `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, frontend type-check/build; (b) build bundles and **report artifact sizes** against the targets in §1; (c) **sidecar job:** build `sdp-whisper` for each target from the pinned whisper.cpp tag, produce `SHA256SUMS`, upload to the tools release, and update `tools.manifest.json`; (d) **smoke job per OS:** start the app headless-safe tests with fake tools, then run the real tool self-test (§5.3.6) in a nightly job.

---

## 14. Implementation plan (phases with exit criteria)

Do the phases in order; each ends with something runnable on all three OSes.

| Phase | Scope | Exit criteria |
| :-- | :-- | :-- |
| **M0 Skeleton** | Tauri v2 + Svelte shell, settings, i18n IT/EN, theme, `process.rs` (Job Object/process group, no console), tools manifest, **bundled yt-dlp + qjs (`externalBin`) with first-run copy of yt-dlp to app data**, on-demand provisioning UI, setup wizard (tools only) | App starts on 3 OSes; yt-dlp + qjs work **offline right after install**; ffmpeg detected or installed with checksums; killing a spawned tree works; **spike results recorded**: macOS notarization with bundled yt-dlp (§5.2.1) and Windows antivirus/SmartScreen behaviour; installer size budget fixed. |
| **M1 Single-video parity** | Inspect, resolutions, video/audio download, progress, final path via `SDP_FILE`, cancel, error classifier, cookies, QuickJS→Deno strategy + self-test | Original features work for one video; QuickJS used by default; automatic Deno fallback demonstrated. |
| **M2 Queue** | Queue engine, lanes, persistence, pause/resume/retry/reorder, notifications, new queue UI | Add 10 links, run 2 in parallel, restart the app mid-run → items resume. |
| **M3 Playlists & other sites** | Playlist/channel enumeration, selection panel, archive, folder/numbering, pacing; non-YouTube handling (§6.3): formats fallback, impersonation retry, cookies file, live/DRM messages, format table | A 100-item YouTube playlist and at least 5 non-YouTube sources (video page, HLS link, direct file, audio-only, login-gated with cookies) behave correctly. |
| **M4 Media processing** | ffmpeg encode/trim, encoder probing, audio encode, embed metadata/thumbnail/chapters, SponsorBlock | Hardware encoder fallback works; originals kept on failure. |
| **M5 Whisper** | CI-built `sdp-whisper`, model catalogue + download manager (resume, checksum, import, delete), pipeline (§10.2), live preview, SRT/VTT/TXT, "Transcribe a local file" | User picks a model size and gets correct IT/EN transcripts on 3 OSes; cancel kills the process; partial outputs removed. |
| **M6 Subtitles-first, Gemini, summary** | Subtitles fetch/clean, engine ordering, Gemini (dynamic models, cleanup guard), summary | Subtitles used when present; Gemini file deleted after success/error/cancel; summary produced. |
| **M7 Polish & release** | Accessibility pass, performance (5,000-item list), Tauri updater, signing/notarization, installers, docs | All acceptance items in §15 pass; release artifacts under size targets. |

---

## 15. Testing and acceptance

**Unit tests (no network, no processes):** resolution extraction (incl. storyboard exclusion, fallback, missing heights), format-selector builder (exact/max/best/audio/id), yt-dlp arg builder (URL after `--`, `--ignore-config`, `--no-js-runtimes` before runtime, no insecure flags), progress/print parser, ffmpeg `out_time_us` parser, time parsing/validation, trim duration math, filename/folder sanitising (reserved names, `%` escaping), playlist entry URL reconstruction, subtitle cleaner (rolling-duplicate removal), timestamp offset merge for chunked SRT/VTT, error classifier (§8.5), queue state machine and retry policy, settings/queue (de)serialisation with missing/unknown keys, model catalogue (checksums/sizes) and Range-resume logic.

**Integration tests:** fake `yt-dlp`, `ffmpeg`, `qjs`, `sdp-whisper` scripts (shell/batch) that emit canned output, to test events, cancellation, stderr draining, orphan cleanup, queue persistence and resume on all three OSes.

**Acceptance checklist (all must pass on every Tier 1 target, §1.1; Tier 2 targets: build + fake-tool tests + self-test in CI only):**
1. [ ] `cargo build --release` and the Tauri bundle build cleanly in CI; `clippy` and `fmt` clean; reported sizes: app binary ≤ 15 MB, installer size within the budget fixed after M0 (app + bundled yt-dlp + qjs; AppImage excepted); idle RSS of the Rust process ≲ 30 MB (tree measured separately).
2. [ ] Fresh machine, no tools: right after install yt-dlp + qjs work from the bundled copies (yt-dlp is copied to app data and updatable); the wizard installs ffmpeg when absent, with checksums; Deno, the Whisper sidecar and models are fetched only on demand; a public 1080p video downloads end-to-end.
3. [ ] **JS runtime:** the self-test passes with **QuickJS**; if it fails on a given OS the app falls back to Deno automatically, remembers the result, and the UI shows which runtime was used.
4. [ ] A 4K (or highest available) video and a vertical/Shorts video list the right resolutions and merge to MP4/MKV; audio-only produces `.opus`.
5. [ ] A 100-item YouTube playlist: selection panel responsive, subset selection, folder + numbering, archive skips re-downloads, a private/deleted entry does not stop the rest; "video + list" URL shows the choice.
6. [ ] Non-YouTube: a generic video page, an HLS/direct link, an audio-only source and a cookies-protected page work (or fail with the right actionable message); live and DRM cases are handled as specified; the anti-bot retry with impersonation triggers once.
7. [ ] Queue: parallel downloads, per-host cap, pause/resume, retry failed, reorder, restart recovery.
8. [ ] Encode/trim: aac/mp3/opus audio encode; video encode with CPU and (where available) hardware encoders; the hardware dropdown shows only probed encoders; forced failure falls back to CPU; trim ±0.1 s and rejects invalid times; originals untouched on failure.
9. [ ] **Whisper:** every model size in the catalogue can be downloaded (resume works after killing the connection), verified, selected and deleted; IT and EN transcripts are correct; `.srt/.vtt` timestamps are valid, including after chunking a > 2 h file; Import model works offline; cancel stops within ~2 s with no partial outputs.
10. [ ] Subtitles-first: manual subtitles used when present, auto captions otherwise, `.txt` free of rolling duplicates; falls back to Whisper when absent.
11. [ ] Gemini: key stored in keyring/0600 file; models list refreshes dynamically; a retired model name produces the guided message; the cloud file is deleted after success, error and cancel; summary generated.
12. [ ] **Cancel** during download, merge, encode, transcription kills the whole process tree within ~2 s (check the OS process list: no `yt-dlp`, `ffmpeg`, `qjs`, `deno`, `sdp-whisper` left), removes temp dirs, leaves finished outputs. Closing the app mid-job leaves no orphans.
13. [ ] Error cases show the right localized message: no ffmpeg, no JS runtime, invalid Gemini key, no network, invalid trim, unwritable folder, disk full, age-restricted video without cookies.
14. [ ] yt-dlp update works without restarting the app and shows the new version; the app updater updates from a signed `latest.json` (AppImage on Linux).
15. [ ] Theme follows the OS on Windows, macOS and at least GNOME + KDE; manual override persists; clipboard shortcuts work in all inputs on macOS; UI fully usable by keyboard.
16. [ ] No filename with accents/non-Latin characters is mangled; no console windows flash on Windows; no secrets appear in logs.

---

## 16. Out of scope now / design for later

* Search and feed pages as input; subtitle translation; per-word karaoke output.
* Whisper GPU (Vulkan/CUDA) sidecar variants (optional later, §5.4).
* Speaker diarization, Parakeet/other ONNX engines (the `Transcriber` trait allows adding them without UI changes).
* Scheduled downloads, RSS/channel subscriptions, browser extension.

---

## Appendix — what changed in rev. 3 (for reviewers)

* **yt-dlp + qjs bundled in the installer**; everything else composable on demand (§0.7, §5.2.1).
* VOSK/libvosk removed → **Whisper sidecar** with a user-selectable model catalogue (§5.4, §10.2).
* JS runtime: **QuickJS first, Deno fallback**, with correct yt-dlp flags (`--no-js-runtimes` first) and per-version memory (§5.3).
* New **queue engine**, **playlist/channel** handling, and **generic non-YouTube** handling (§6, §7).
* **Subtitles-first** transcripts, SRT/VTT outputs, Gemini summary, embeds, SponsorBlock (§8.4, §10).
* Gemini model handling rewritten (dynamic list, `-latest` aliases) because the original model names are partly retired (§10.3).
* New UI specification, updater/CI/packaging details, phased plan and expanded acceptance tests (§11, §13–§15).
