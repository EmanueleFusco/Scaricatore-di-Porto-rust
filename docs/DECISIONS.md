# DECISIONS

Questions, deviations, spec ambiguities and everything marked `TODO(verify)` that could not be
verified. Append only (compacted and unified into English in D143).

---

## 2026-10-12 — M0-A (skeleton, settings, i18n, theme, shell UI)

### D1 — Frontend: plain Vite + Svelte, **not** SvelteKit

`PHASE_PROMPTS.md` M0-A step 1 says to scaffold with the official Tauri v2 Svelte-TS template
(SvelteKit + `adapter-static`). Its production build emits an inline `<script>` in `index.html`,
which is blocked by `tauri.conf.json`'s strict CSP (`script-src 'self'`). Spec §1 asks for
"TypeScript + Vite + Svelte" and AGENTS.md §6 asks for a light frontend with no framework runtime,
so we use plain Vite + Svelte 5 with a ~40-line path router (`src/lib/router.svelte.ts`).
Result: strict `script-src 'self'` works as-is (`dist/index.html` has no inline `<script>`), and
payload drops from ~130 KB to **70 KB JS + 7.7 KB CSS**.

### D2 — Plugins added

Registered `dialog`, `opener`, and `single-instance` (all on the spec §1 allow-list). `single-instance`
implements spec §3 ("a second launch focuses the first window") without enabling its `deep-link`
feature. **No `tauri-plugin-shell`** (forbidden by spec §1).

### D3 — Devtools are off in release builds without a config flag

Spec §11.1 asks for "devtools off in release". Setting `app.windows[].devtools` in `tauri.conf.json`
would also disable them in `tauri dev`. Instead, the `devtools` cargo feature is omitted on the
`tauri` crate so wry's devtools API is not compiled into release builds.

### D4 — Lenient enum deserialisation in `Settings`

Unknown `language`/`theme` values degrade to defaults (`de_theme` / `de_language` in `state.rs`,
case-insensitive after trimming) rather than treating the whole `settings.json` as corrupt and
moving it to `.bak` (spec §2.8 resilience).

### D5 — Wire format is camelCase

`Settings`, `SettingsView`, `Recovery`, and `ErrorInfo` serialise with
`#[serde(rename_all = "camelCase")]` so TypeScript uses the field names directly; persisted
`settings.json` also uses camelCase keys.

### D6 — Settings file is only created on the first write

Loading a missing `settings.json` returns defaults in memory without touching the disk; writing
always creates it atomically.

### D7 — `npm run dev` outside Tauri degrades gracefully

`src/lib/stores/settings.svelte.ts` detects the absence of `__TAURI_INTERNALS__` and falls back to
in-memory defaults so UI development and CSP checks work in a standard browser.

### D8 — CI, `tools.manifest.json` and the wizard are not part of M0-A

Deferred to M0-C per phase breakdown. *(Superseded once M0-C and M7 completed them.)*

### D9 — `original_python/` is kept on disk, untracked

Upstream Python assets (`LICENSE`, `README.md`, `requirements.txt`, `images/`) and
`reference/ScaricatoreDiPorto.py` are kept on disk for reference and icon/licence parity but
excluded from git via `.gitignore` (see D17).

### D10 — Wayland: worked around in the dev launcher, not "fixed" in the app

On CachyOS / KDE Plasma Wayland / NVIDIA RTX 3080 (`webkit2gtk-4.1` 2.54.1), GTK/WebKitGTK failed
with `Gdk-Message: Error 71` or black GBM buffers (`Failed to create GBM buffer`). Initially
`scripts/run.mjs` injected `GDK_BACKEND=x11` and `WEBKIT_DISABLE_DMABUF_RENDERER=1` on Linux
Wayland sessions. *(Superseded in M7 by D135/D136, which moved `WEBKIT_DISABLE_DMABUF_RENDERER=1`
for Wayland + NVIDIA into `src-tauri/src/lib.rs` at startup.)*

### D11 — Icons: Lucide (`lucide-svelte`), not hand-written SVG

Replaced hand-written SVG paths with `lucide-svelte` (v1.0.1, ISC/MIT licence). Still renders
inline `<svg>` with no remote assets or icon fonts (spec §1, §11.9); tree-shaking verified (+11.7 kB
JS for used icons, zero Rust binary impact). Never hand-write SVG path data.

### D12 — CI designed around the **private** repository's minute budget

GitHub Actions multipliers on private repos are Linux 1x, Windows 2x, macOS 10x (2,000 free
min/month). CI was structured to avoid burning macOS/Windows minutes on every push, and Tier 2
targets (`macos-15-intel`, `windows-11-arm`, `ubuntu-24.04-arm`) run only via `workflow_dispatch`.
*(Further refined in D26 and D118: all workflows are manual `workflow_dispatch`.)*

### D13 — Bundle sizes measured on installable files only

`scripts/report-bundle-sizes.mjs` measures only final installable artifacts (`.deb`, `.rpm`,
`.AppImage`, `.exe`, `.msi`, `.dmg`, `.snap`, `.app`), ignoring intermediate bundler staging trees.

### D14 — Linux CI runner package list and Ubuntu version

Aligned Linux apt dependencies in CI with official Tauri v2 prerequisites (including
`libayatana-appindicator3-dev`, dropping unused `patchelf`). Initially targeted `ubuntu-22.04` for
older glibc compatibility. *(Superseded by D55 and D122: migrated all Linux jobs to `ubuntu-24.04`
due to GitHub Actions `ubuntu-22.04` deprecation/brownouts.)*

### D15 — Windows and macOS runner and bundler verification

1. Dropped `msi` from `bundle.targets` in `tauri.conf.json` (kept `["deb", "rpm", "appimage", "nsis", "app", "dmg"]`) because MSI requires optional VBSCRIPT features and spec §13.1 prefers NSIS.
2. Selected `windows-2022` (VS 2022) and `macos-26` (Apple Silicon) for Tier 1 CI runners.
3. Verified `PathResolver::download_dir()`, `tauri-plugin-single-instance`, and `windows_subsystem = "windows"`.

### D16 — Pre-push audit of tracked files and `.gitignore`

Verified zero secrets, tokens, personal paths, or binaries in git history. Extended `.gitignore` to
block signing keys (`*.key`, `*.pem`, `*.p12`, `*.pfx`, `*.keystore`, `*.jks`), backups (`*.bak`),
and crash dumps (`core`, `core.*`).

### D17 — Workflow prompt files and Python original removed from git history

Removed `COME_INIZIARE.md`, `PHASE_PROMPTS.md`, `original_python/`, and
`reference/ScaricatoreDiPorto.py` from git history via `git filter-repo` while keeping them on local
disk under `.gitignore`.

### D18 — Personal operational guide stays out of git

`GUIDA_OPERATIVA.md` is a local working document for the owner and is ignored via `.gitignore`.

---

## 2026-10-13 — M0-B (process layer, spec §4)

### D19 — `fake_tool` is a Cargo **example** / test binary, not a package `[[bin]]`

Declaring `fake_tool` as a package `[[bin]]` caused `tauri build` to package `/usr/bin/fake_tool`
inside user `.deb` installers. Keeping test binaries out of `[[bin]]` ensures user installers only
ship `scaricatore-di-porto`.

### D20 — Windows Job Object is attached right after `CreateProcess`

`tokio::process::Command` has no pre-spawn Job Object hook, so `services/process.rs` spawns the
child with `CREATE_NO_WINDOW`, creates the Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and
calls `AssignProcessToJobObject`. If assignment fails, `spawn()` kills the child and fails closed.

### D21 — On Unix, a reaped PID is never signalled again

To prevent `killpg(pgid, SIGKILL)` from hitting a recycled PID after child exit, `TreeHandle::forget_group()`
zeroes the stored PGID as soon as the child is reaped, and the grace loop polls `child.try_wait()`
to exit immediately when the direct child terminates.

### D22 — `TerminateJobObject` checks `ActiveProcesses == 0` first

Because Microsoft docs do not specify `TerminateJobObject` behaviour on an empty job, Windows
`kill_tree` queries `JobObjectBasicAccountingInformation` first and returns `Ok(())` immediately
when `ActiveProcesses == 0`.

### D23 — Cross-OS `process-tests` CI job

Added `process-tests` running `cargo test` on Windows and macOS in addition to Linux so Job Objects,
process groups, and atomic file replacement (`MoveFileExW`) are verified on real Tier 1 OSes.

### D24 — `tokio` `macros` feature in `[dependencies]`

Moved `tokio`'s `macros` feature from `[dev-dependencies]` to `[dependencies]` because
`tokio::select!` is used in production `services/process.rs` (spec §4.4).

### D25 — `unsafe impl Send + Sync` for Windows `TreeHandle`

`windows` 0.62 `HANDLE` wraps `*mut c_void` without `Send`/`Sync`. `TreeHandle` safely implements
`Send` and `Sync` because methods only read the handle via `&self` and `CloseHandle` runs once in
`Drop` (`&mut self`).

### D26 — Tier 2 isolated in `.github/workflows/tier2.yml`

Moved Tier 2 targets (`macos-15-intel`, `windows-11-arm`, `ubuntu-24.04-arm`) out of `ci.yml` into a
separate manual workflow (`tier2.yml`) so a manual `ci.yml` run does not burn ~83 Linux-equivalent
minutes on experimental targets.

### D27 — Terminated process tree exit codes differ between Unix (`None`) and Windows (`Some(1)`)

Unix `killpg(SIGKILL)` yields `exit_code == None`, whereas Windows `TerminateJobObject(hJob, 1)`
assigns `exit_code == Some(1)`. We do not normalise them because `ProcessResult::success()` checks
`!cancelled && exit_code == Some(0)` (`false` on both) and error classification (spec §8.5) matches
stderr patterns.

### D28 — Windows test binaries embed Common Controls v6 manifest via `rustc-link-arg-tests`

On Windows, integration tests using `tauri::test::mock_builder()` crashed before `main()` with
`0xc0000139 (STATUS_ENTRYPOINT_NOT_FOUND)` because `tauri-build` only embeds the Common Controls v6
manifest into `--bins`, not `--tests`. Fixed by adding `src-tauri/windows-app-manifest.xml` and
emitting `cargo:rustc-link-arg-tests=/MANIFEST:EMBED` + `/MANIFESTINPUT:...` in `src-tauri/build.rs`
when targeting Windows MSVC.

### D29 — M0-B verification scope

Confirmed `services/process.rs` passes all 80 unit and integration tests across Linux x64, Windows
x64, and macOS arm64 runners, including grandchild tree-kill via Job Object and `killpg`.

### D30 — Meaning of "yt-dlp + qjs work offline right after install" (spec §14 M0)

Means zero network bootstrap is required on first launch: `yt-dlp` and `qjs` are bundled inside the
installer, runnable immediately (`--version` works offline), avoiding macOS Gatekeeper quarantine
and Windows SmartScreen blocks on freshly downloaded executables.

---

## 2026-10-13 — M0-C (tools manifest, bundled yt-dlp + qjs, provisioning, UI)

### D31 — `tools.manifest.json` is embedded via `include_str!`

Compiled directly into the Rust binary rather than shipped as a loose resource file, avoiding
platform-specific resource path differences (`.deb`, `.AppImage`, `.app`, NSIS) and updating
automatically with the app binary (spec §5.1). Validated on every `cargo test`.

### D32 — Pinned `yt-dlp` (`2026.08.19`) and `qjs` (`v0.17.0`) assets and SHA-256 checksums

Verified asset filenames and SHA-256 checksums from upstream releases. QuickJS-NG (`v0.17.0`)
publishes separate `darwin-arm64` and `darwin-x86_64` binaries; `universal-apple-darwin` declares
`builtFrom` in `tools.manifest.json` and `scripts/fetch_bundled_tools.mjs` combines them with
`lipo -create`.

### D33 — Resolve `yt-dlp` `latest` tag via GitHub API / redirect rather than `/releases/latest/download/`

GitHub's `/releases/latest/download/<asset>` alias intermittently returned HTTP 404 on CDN edges
while explicit tag URLs succeeded. `fetch_bundled_tools.mjs` resolves the tag first and substitutes
`{tag}` into the download URL, recording the exact version (spec §5.2.1).

### D34 — `deno` omitted from manifest in M0-C

Deferred pinning Deno in `tools.manifest.json` until M1 rather than leaving a null placeholder.
*(Superseded in M1 by D59: Deno 2.9.7 pinned in `tools.manifest.json`.)*

### D35 — `reqwest` 0.12 with `rustls-tls` (`ring`) and pure-Rust archive crates

Used `reqwest 0.12` (`default-features = false`, `features = ["rustls-tls", "stream", "json"]`),
which uses `ring` and `webpki-roots` with zero OpenSSL or C/CMake dependencies (spec §1), plus
`sha2`, `zip` (`default-features = false, features = ["deflate"]`), `tar`, `flate2`, and `lzma-rs`.

### D36 — Per-tool version flag (`-version` for ffmpeg/ffprobe) and `n`-prefixed version parsing

`ffmpeg` and `ffprobe` use single-dash `-version` and emit banners like `ffmpeg version n9.0.2`.
The version parser strips leading `v`/`n` prefixes and accepts dotted numeric tokens (including
`yt-dlp` nightly `YYYY.MM.DD.NNNNNN`).

### D37 — `Transport::loopback_http()` restricts plain HTTP to loopback addresses

Production downloads enforce HTTPS (`Transport::HttpsOnly`). Tests use `Transport::LoopbackHttp`,
which strictly parses the URL host and only permits `localhost`, `127.0.0.0/8`, and `[::1]`,
rejecting userinfo tricks (`http://127.0.0.1@evil.com`) and prefix spoofing (`127.0.0.1.evil.com`).

### D38 — Unified path-traversal guard for `.zip` and `.tar` rejecting `\` on all OSes

`safe_extract_path` rejects parent references (`..`), root prefixes, drive letters, symlink entries,
and backslashes (`\`) even on Unix hosts.

### D39 — Dev-mode bundled tool lookup also checks `<tool>-<triple>`

`tauri build` strips `-<triple>` when copying `bundle.externalBin` binaries next to the executable,
whereas `tauri dev` leaves `<tool>-<triple>[.exe]` in `src-tauri/binaries/`. In debug builds,
`bundled_candidates` checks both the plain name and the target-suffixed name.

### D40 — Single `ToolContext::lookup` entry point

Removed duplicate `lookup_from` / `lookup_for` helpers so all environment reading (`PATH`, home,
overrides) goes through `ToolContext::lookup`.

### D41 — `tools_install` placeholder in M0-C

Returned `UNSUPPORTED` in M0-C before on-demand tools were added. *(Superseded in M1 by D59 and in
M7 by D126/D134: `tools_install` installs `deno`, `ffmpeg`+`ffprobe`, and `sdp-whisper`.)*

### D42 — `yt-dlp` `minVersion` set to `2025.11.12`

Set `minVersion: "2025.11.12"` in `tools.manifest.json`. *(Confirmed in D56.)*

### D43 — `yt-dlp -U` is only run on the app-data copy

`tools_update_ytdlp` only runs `-U` on the copy inside `<app_data_dir>/tools/yt-dlp/` (bootstrapping
it from the bundled binary if needed, spec §5.2.1, §5.5). If the active `yt-dlp` comes from a user
override or system `PATH` (e.g. Homebrew/apt), it refuses to mutate it and instructs the user to
update it via their package manager.

### D44 — `yt-dlp` update classifier checks exit code and version change before output text

`classify_update` treats any non-zero exit code as failure first, compares pre- and post-update
versions second, and only inspects stdout/stderr text for "up to date" when exit code is 0 and
versions match.

### D45 — CI fetches and executes bundled tools in bundle jobs; offline manifest check in `check`

`bundle` and `bundle-macos-universal` run `npm run fetch:tools` followed by
`cargo test --test bundled_tools` before `tauri build`. `check` runs `npm run check:tools` offline.

### D46 — Multi-call `fake_tool` dispatches on `argv[0]`

The test binary `fake_tool` inspects its executable stem (`yt-dlp`, `qjs`, `deno`, `ffmpeg`,
`ffprobe`, `sdp-whisper`) so tool resolution, version probing (`--version` vs `-version`), and `-U`
updates are tested hermetically offline across all three OSes.

### D47 — Retry spawn on Unix `ETXTBSY` (`os error 26`) and `fsync` on tool copy

When `bootstrap_first_run` copies `yt-dlp` into app data and immediately probes `--version`, Linux
can transiently return `ETXTBSY` (`os error 26`). Fixed by `fsync`ing the file and directory in
`install_app_data_copy` and retrying `ETXTBSY` in `services/process.rs` (5 attempts, 20 ms apart).

### D48 — Override `TAURI_CONFIG` with empty `externalBin` in non-bundle CI jobs and validate parity

`tauri_build::build()` checks for `bundle.externalBin` paths on every `cargo check`/`clippy`/`test`
invocation. Non-bundling CI jobs set `TAURI_CONFIG='{"bundle":{"externalBin":[]}}'`, and
`npm run check:tools` verifies that `tauri.conf.json` `externalBin` and `tools.manifest.json`
`bundled: true` match.

### D49 — Cross-platform path tests use host-native absolute paths and parameterised well-known dirs

Unix paths like `/opt/mine/qjs` are relative on Windows (`Path::is_absolute` requires a drive or UNC
prefix), causing 5 path tests to fail on Windows. Tests now build host-native absolute paths
(`host_dir` / `host_exe`), and `well_known_dirs_for` is parameterised so both Windows and Unix
branches are unit-tested on every OS.

### D50 — Universal macOS builds require all three binary names (`arm64`, `x86_64`, `universal`)

`tauri build --target universal-apple-darwin` invokes Cargo once per slice (`build.rs` requires
`<tool>-aarch64-apple-darwin` and `<tool>-x86_64-apple-darwin`) and then bundles
`<tool>-universal-apple-darwin`. `fetch_bundled_tools.mjs` produces all three files (via `builtFrom`
+ `lipo -create` for `qjs`, and `lipo -thin` from `yt-dlp_macos` for `yt-dlp`), and CI verifies all
six binaries with `lipo -info`.

### D51 — Local Windows verification on Linux via `cargo-xwin` + Wine, and pure string path joining

Verified that Windows unit and `process_tree` tests can be cross-compiled and run locally via
`cargo-xwin` + Wine. Fixed `well_known_dirs_for` to format Unix and Windows paths with explicit `/`
and `\` separators rather than host-dependent `Path::join`.

### D52 — Guard against writing file paths instead of binary buffers in `fetch_bundled_tools.mjs`

Fixed a bug where `downloadSlice` returned a file path string that `writeFile` wrote as text into
`binaries/qjs-aarch64-apple-darwin`. `downloadSlice` now returns `{ bytes, path }` and `install()`
throws immediately if `typeof bytes === "string"`.

### D53 — Windows `binaries/` filename puts `.exe` after the target triple

`tauri_utils::external_binaries` expects `<tool>-<triple>.exe`, whereas combining `executable_name`
(`yt-dlp.exe`) with `-<triple>` produced `yt-dlp.exe-<triple>`. Added `fetched_name(tool, triple)`
so dev-mode lookup finds `<tool>-<triple>.exe` on Windows.

### D54 — Fallback tag resolution via GitHub `/releases/latest` HTTP 302 redirect and `GITHUB_TOKEN`

When unauthenticated GitHub API requests hit HTTP 403 rate limits on shared runner IPs, falling back
to `/releases/latest/download/` failed with 404. `fetch_bundled_tools.mjs` now passes `GITHUB_TOKEN`
when available and falls back to reading the `Location: .../releases/tag/<tag>` redirect header from
`https://github.com/yt-dlp/yt-dlp/releases/latest`, validating `<tag>` against path traversal.

### D55 — CI runner and action allowlist (`scripts/check-ci-runners.mjs`)

Migrated Linux CI jobs to `ubuntu-24.04` and upgraded GitHub Actions to Node 24 versions
(`checkout@v7`, `setup-node@v7`, `upload-artifact@v7`, `download-artifact@v8`,
`rust-cache@v2.9.2`). Added `scripts/check-ci-runners.mjs` (`npm run check:runners`) to reject
floating runner tags (`*-latest`), deprecated images (`ubuntu-22.04`), and pre-Node-24 actions.

---

## 2026-10-14 — M1 (single video: inspection, download, JS runtime, cancellation)

### D56 — Confirmed `yt-dlp` `minVersion: "2025.11.12"`

Verified in `yt_dlp/options.py` across upstream tags: `--js-runtimes` and `--no-js-runtimes` were
introduced in `2025.11.12`, and `--no-js-runtimes` must precede `--js-runtimes`. Closes D42's TODO.

### D57 — `ffmpeg` is checked pre-flight before starting a download

When `ffmpeg` is missing, `yt-dlp` exits 0 and leaves unmerged split streams (`*.f278.webm` +
`*.f251.webm`). `run_job` therefore resolves `ffmpeg` before spawning `yt-dlp` and fails fast with
`FFMPEG_MISSING`.

### D58 — `process.rs` supports line sinks on both stdout and stderr

In `yt-dlp`, `download:` progress templates print to stdout while `postprocess:` templates print to
stderr. `process.rs` provides `LineSinks` with callbacks for both streams while still populating the
200-line stderr ring buffer for error classification.

### D59 — Deno `v2.9.7` pinned in `tools.manifest.json` as optional on-demand tool

Added Deno `v2.9.7` (`bundled: false`, 6 platforms, `.zip`, verified SHA-256 checksums) to
`tools.manifest.json`. Treated as optional in `toolState` so the footer status dot does not flag a
fresh install when QuickJS is healthy.

### D60 — Use `-I :1000` (`--playlist-items`), not removed `--playlist-end`

Verified against `yt-dlp 2026.08.19 --help`: `--playlist-end` no longer exists; playlist bounds use
`-I, --playlist-items [START]:[STOP][:STEP]` (e.g. `-I :1000`).

### D61 — QuickJS→Deno fallback memory in `runtime-memory.json` (24 h TTL)

Persisted atomically at `<app_data_dir>/runtime-memory.json`, keyed by `(yt-dlp version, qjs version)`
with a 24-hour TTL (`now >= decided_at`). Corrupt files degrade to empty memory, and updating
`yt-dlp` automatically invalidates prior fallback entries (spec §5.3.3).

### D62 — Temporary M1 events `job:progress` / `job:done`

Used in M1 before the M2 queue existed. *(Superseded in M2 by `queue:update` and D107.)*

### D63 — Self-test video ID is `jNQXAC9IVRw` ("Me at the zoo")

The classic `youtube-dl` test video `BaW_jenozKc` is now unavailable on YouTube; switched the
self-test (spec §5.3.6) to `jNQXAC9IVRw`.

### D64 — Inspection timeout heuristic: 60 s for single video, 120 s for playlist-like URLs

URLs matching `list=`, `/playlist`, `/videos`, `/shorts`, or `/@` use a 120 s inspection timeout;
all other URLs use 60 s (spec §6.2).

### D65 — Renamed inner `PlaylistStub.kind` to `playlistKind`

Avoided duplicate `"kind"` JSON keys when serialising the internally tagged `Inspected` enum
(`#[serde(tag = "kind")]`).

### D66 — Verified `yt-dlp -U` channel switching on the real binary

Tested `--update-to nightly` and `--update-to stable` on a copy of the real binary. Because
`--update-to stable` from a newer nightly exits 0 without downgrading, `classify_update` (D44)
comparing versions before and after accurately reports `updated: false`.

### D67 — `YOUTUBE_PLAYER_CLIENTS` constant

Initially kept the original app's `youtube:player_client=web_embedded,web,tv` in a single constant.
*(Superseded by D74: no extractor args are passed by default; exposed as an optional expert setting.)*

### D68 — Live network self-tests in `tests/real_selftest.rs` are `#[ignore]`

Tests that hit live YouTube or download real tool archives are marked `#[ignore]` so `cargo test`
remains 100% offline and deterministic (AGENTS.md §6).

### D69 — `browserNamesForTriple` only offers Safari on macOS

Browser cookie options are filtered by the backend target triple so Safari only appears on
`*-apple-darwin` targets (spec §8.6).

### D70 — `.github/workflows/selftest.yml` is manual (`workflow_dispatch`)

The live YouTube self-test workflow runs only on manual dispatch to avoid burning private repo
minutes or failing on transient YouTube blocks.

### D71 — Detect degraded muxed 360p fallback via `--print before_dl:SDP_FORMAT|...`

When a YouTube JS/PO-token challenge fails mid-download, `yt-dlp`'s selector
`bv[height=N]+ba/bv[height<=N]+ba/b[height<=N]/b` silently falls back to `b[height<=N]` (muxed 360p,
format `18` without `+`) and exits 0. Added
`--print "before_dl:SDP_FORMAT|%(format_id)s|%(height)s|%(vcodec)s|%(acodec)s"`:
- If a **muxed** stream (no `+` in `format_id`) has height below requested, the attempt is aborted
  before downloading bytes, partials are cleaned, and the job retries once (with Deno if available);
  if still degraded, it fails with `YT_CHALLENGE`.
- If a **merged** stream (`+` in `format_id`, e.g. `137+140`) has a lower height, the video genuinely
  lacks the higher resolution; the download proceeds and records `requestedHeight`/`actualHeight`.

### D72 — Status footer reflects live state without duplicating per-card speed/ETA

Wired `FooterStatus.svelte` to live job/queue state and tool warnings, omitting speed and ETA that
are already shown on the active item card.

### D73 — Detect degraded inspection via `yt-dlp` stderr warnings and retry once

Removed `--no-warnings` from inspection (deliberate deviation from spec §6.2) because `yt-dlp`
prints `WARNING: [youtube] n challenge solving failed` / `Signature solving failed` on stderr when
returning a degraded 360p-only format list. When detected (or when zero heights are returned),
`inspect` retries once (with Deno if available, else the same runtime), keeps the best result
(`choose_next`), marks `video.degraded = true` if still degraded, and never caches degraded results.

### D74 — Do not force `youtube:player_client=web_embedded,web,tv` by default

In `yt-dlp 2026.08.19`, default clients are `('visionos', 'web')`, where `visionos` requires no PO
token while `web` requires a GVS PO token. Forcing the legacy `web_embedded,web,tv` list dropped
`visionos` and caused intermittent 360p-only extraction whenever PO token minting failed. Deviation
from spec §5.3.7: pass **no** `--extractor-args` by default (increasing 4K test formats from 31 to
49 and cutting self-test time from 7.4 s to 1.95 s); keep the override in Settings → Network &
Accounts ("YouTube player clients", empty = automatic).

### D75 — Cache resolved tools in `AppState` for the session

Because `yt-dlp` is a PyInstaller one-file bundle, probing `yt-dlp --version` costs ~0.46 s per
invocation. `AppState` now caches successful `(tool, override_path) → ResolvedTool` lookups for the
session, invalidating the cache on `tools_set_path`, `tools_update_ytdlp`, `tools_install`,
`tools_use_system`, `tools_status`, and `tools_test`.

### D76 — Inspection debounce: 500 ms when typing, 0 ms on paste

Typing in the URL input retains the 500 ms debounce (spec §2.9, §6.2), while a `paste` event starts
inspection immediately (`inspectDelay(true) === 0`).

### D77 — Inventory of historical Italian text in repository files

Recorded the 2026-10-14 inventory of Italian comments/logs across `docs/`, `.github/`, and
`scripts/` and the test-assertion traps to watch when translating them. *( docs/ translation
completed in D143.)*

### D78 — `docs/PROJECT_REPORT.md` does not exist in this repository

Noted in `AGENTS.md` §1 that `docs/PROJECT_REPORT.md` is not present on disk or in git so sessions
do not search for it.

---

## 2026-10-15 — M2 (the queue, persistence, scheduling, IPC, UI)

### D79 — `tauri-plugin-notification` 2.4 for optional queue-idle notifications

Added `tauri-plugin-notification` 2.4.0 (+0.17 MiB binary impact: 6.71 → 6.88 MiB) for spec §7.7
"system notification when the queue becomes idle" (off by default). Localized text is read from the
embedded `it.json`/`en.json` dictionaries in Rust so notifications work when the window is in the
background.

### D80 — `queue.json` stores item order implicitly in the `items` array

Omitted a separate `order` array in `queue.json` (spec §7.4) so item order has a single source of
truth (`items` vector order); `queue:update` sends the ID order vector only when reordering occurs.

### D81 — `queue:update` coalesces at ≤ 10/s and sends whole changed items

Coalesces progress updates at ≤ 10 updates/s (`services/queue/events.rs`, spec §7.6) and sends whole
changed `QueueItem` structs rather than field-level `Option<Option<T>>` patches, avoiding frontend/
backend state drift while the frontend store still patches single rows in place (spec §11.7).

### D82 — Scheduler lanes implemented as live counters rather than static semaphores

Download, encode, and transcribe capacities are tracked as counters in the scheduler planner
(`services/queue/scheduler.rs`) so mid-flight settings changes (e.g. reducing parallel downloads
from 2 to 1) take effect immediately on the next planning pass without revoking semaphores.

### D83 — The Add bar inspects URLs; the queue engine starts at the download stage

Pasted URLs are inspected in the Add bar (`inspect_urls`, 3 in parallel), and each queued item
freezes its options snapshot, title, site, and duration at enqueue time, avoiding a duplicate
`yt-dlp` inspection spawn when the item starts.

### D84 — Playlist URLs refused in M2 until M3 playlist panel

In M2, playlist inspection results showed a clear unsupported message instead of downloading
playlist metadata with `--no-playlist` (spec §12.12). *(Superseded in M3 when full playlist support
landed.)*

### D85 — Per-host cap key strips only leading `www.`

`queue::model::host_key` lowercases the host and strips a leading `www.` so `www.youtube.com` and
`youtube.com` share the per-host cap (spec §6.4).

### D86 — Resuming a `Paused` or `Interrupted` item transitions it to `Queued`

Only `Queued` items are eligible for the scheduler, so `queue_resume` transitions `Paused` and
`Interrupted` items back to `Queued` while preserving `paused_from` for lane and UI context.

### D87 — `Interrupted` items wait for the user's Resume click

On startup, items that were in an active stage when the app exited become `Interrupted` (spec §7.4)
and wait for `queue_resume` / `queue_resume_all` rather than restarting automatically.

### D88 — `QueueEngine::start()` spawns tasks via `tauri::async_runtime::spawn`

Because `QueueEngine::start()` runs inside Tauri's `setup` hook on the main thread (outside a direct
`tokio::runtime::Handle::current()` context), background tasks are spawned with
`tauri::async_runtime::spawn`.

### D89 — Delete a cancelled running item's temp directory before recording `Cancelled`

When a running item is cancelled or removed, the engine kills the process tree, deletes the item's
`.sdp-tmp-*` directory outside the state lock, and only then records `Cancelled` so the directory
never outlives the row.

### D90 — `queue_pause` on a `Queued` item parks it as `Paused` instead of cancelling

Calling `queue_pause` on an item that is `Queued` sets `state = Paused` (`paused_from = Queued`)
instead of `Cancelled`, and leaves already-parked or finished items untouched.

### D91 — Persist item state as running to `queue.json` **before** spawning the tool

`start_item` writes `queue.json` after transitioning an item to `Downloading` and *before* spawning
the child task, guaranteeing that a crash or `kill -9` mid-download always recovers the item as
`Interrupted` on next launch (spec §7.4).

### D92 — Atomic writes use a unique per-write temp filename (`.<file>.tmp-<pid>-<counter>`)

`paths::atomic_write` previously used `.<file>.tmp-<pid>`, which caused concurrent writers in the
same process to corrupt each other's temp file and fail on `rename`. Added a process-wide
`AtomicU64` counter so concurrent atomic writes never collide.

### D93 — `save_settings` updates disk and in-memory state under a single write lock

`AppState::save_settings` holds the settings lock across both the atomic file write and the
in-memory update (writing the file first) so overlapping saves cannot leave memory and disk out of
sync.

### D94 — Queue integration tests use deadline polling, never fixed sleeps

Replaced fixed `sleep` calls in `tests/queue_engine.rs` with deadline-bounded polling helpers to
eliminate false test flakes caused by asynchronous persistence and event flushing.

### D95 — `RuntimeMemoryStore::remember` uses the same single-lock write discipline as D93

Applied D93's lock-and-write-first ordering to `RuntimeMemoryStore::remember`.

### D96 — `start_item` checks `stopped` before acquiring the state lock

Prevents an in-flight scheduler pass from spawning new child processes or rewriting `queue.json`
after `QueueEngine::shutdown()` has been called during app exit.

### D97 — Frontend `queueStore.load()` never rolls rows backward when overtaken by events

`queue.svelte.ts` records `seq` before invoking `queue_list`. If `queue:update` events advance `seq`
while the IPC call is in flight, `applySnapshot` only adds missing rows and keeps the newer item
states and sequence number.

### D98 — Add bar supports both immediate download ("Download" / "Scarica") and "Add to queue"

Following hands-on feedback, the Add bar automatically inspects typed/pasted links and provides two
buttons: primary **Download** (which adds items and moves them to the top of waiting items) and
secondary **Add to queue** (FIFO), plus per-row exact resolution selection alongside the global cap
preset.

### D99 — Parameterised height i18n keys and automated frontend/Rust i18n key guards

Replaced fixed resolution keys with parameterised `queue.item.quality.exact` / `.max` (`{height}`),
and added automated tests in `src/lib/i18n/usage.test.ts` and Rust (`message_key_in_dictionaries`)
verifying that every literal key, error code, state, action, and `toast.*` key exists in both
`en.json` and `it.json`.

### D100 — Priority download uses `queue_move_to_top` when other items are waiting

The primary **Download** button calls `queue_add` and, only when other `queued` items exist ahead of
the new batch, calls `queue_move_to_top` in reverse order to place the batch at the head of the
waiting queue without changing the IPC surface.

### D101 — Increased fake download window in restart test

Raised the `veryslow` window in `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`
from 3 s to 8 s. *(Superseded by D112 and D115, which made the test deterministic via file markers.)*

### D102 — Queue page flex layout and scroll containment

Removed the hardcoded `max-height: calc(100vh - 320px)` on the queue viewport so an expanded Add bar
never paints over the queue action buttons or status footer; the page scrolls cleanly on small
viewports while the virtual list scrolls internally when space permits.

### D103 — Fixed-shape queue cards (single status line) and measured virtual-list row pitch

Queue cards now maintain a constant vertical layout in every state (header, progress bar, single
clamped status line, actions) so transient `NA` speed/ETA ticks from `yt-dlp` never cause layout
jitter. The virtual list dynamically measures the real pixel pitch between rendered rows on resize
and slice changes instead of assuming a 92 px constant.

### D104 — New queue items stay FIFO with scroll-into-view and 2.5 s highlight pulse

Appended items preserve FIFO ordering (preventing playlists from being starved by later links),
while the virtual list smoothly scrolls to newly added off-screen items (`scrollOffsetFor`) and
pulses an accent ring for 2.5 s (`.item--new`, disabled under `prefers-reduced-motion`).

### D105 — Inline Deno install offer and stream stage detail on queue cards

Moved the one-click Deno install banner (on `DENO_MISSING` failures) and the live stream stage
detail (`Video 1/2`, `Merging…`) directly onto `QueueItem.svelte` cards.

### D106 — Session URL metadata cache (cap 50) in the Add bar

Moved the session `MetadataCache` (up to 50 non-degraded single-video inspections, bypassed on
explicit Retry, cleared on Clear) into the Add bar store (`spec §2.9`).

### D107 — Removed unreachable M1 frontend stores; kept M1 backend commands

Deleted dead M1-only frontend files (`stores/job.svelte.ts`, `stores/inspect.svelte.ts`, `job.ts`)
whose responsibilities moved to the queue and Add bar, while retaining the M1 backend commands and
`services/job.rs` (which the queue engine calls underneath).

### D108 — Fake tools answer `-version` and require `--paths home:` and URL after `--`

`fake_ytdlp` copied as `ffmpeg` previously fell through on `-version` (single dash) and wrote a
16-byte `Fake title [fakeid].mkv` into `src-tauri/`. Fixed `fake_ytdlp` to answer `-version` with an
ffmpeg banner and to exit 2 if invoked for a download without `--paths home:` and a URL after `--`.

### D109 — Separate CI artifacts per Linux package format (`.deb`, `.rpm`, `.AppImage`)

Split the Linux `bundle` artifact upload into `scaricatore-di-porto-linux-x64-deb`, `-rpm`, and
`-appimage` (each with `if-no-files-found: error`) so testers can download the `.deb` without
pulling the ~148 MB AppImage.

### D110 — Stall watchdog test margin on Windows CI (5 s)

Increased the stall watchdog timeout in `a_stalled_challenge_switches_runtime_instead_of_failing`
from 300 ms to 5 s so Windows process creation + Defender scanning can write its initial log line
before the watchdog kills the stalled QuickJS process and falls back to Deno.

### D111 — Upload only installable files from `bundle` and guard `release` assets

Restricted CI artifact upload globs to installable packages (`deb/*.deb`, `rpm/*.rpm`,
`appimage/*.AppImage*`, `nsis/*.exe`, `dmg/*.dmg`, `*.app.zip`) to exclude 490+ MB of unpacked
staging trees (`AppDir`, `squashfs-root`), and added a release-job guard (tested in
`scripts/ci-steps.test.mjs`) that fails if any non-installer file reaches `assets/`.

### D112 — Deterministic unit vs integration ownership for mid-run `queue.json` state

Removed the racy mid-run disk read from the integration test
`the_queue_survives_a_restart_and_the_interrupted_item_can_resume` (which already had a
deterministic unit test `the_file_says_an_item_is_running_before_the_tool_is_spawned`), using a
URL-specific marker file (`.fake-started-<host>`) to synchronise shutdown while the fake tool is
alive.

### D113 — Corruption recovery test uses `fx.idle_engine()` to avoid concurrent task writes

In `the_queue_file_survives_a_corruption_with_a_backup`, used `fx.idle_engine()` (whose background
scheduler tasks are never started) to populate `queue.json` before corrupting it, preventing an
finishing item task from overwriting the corrupted file.

### D114 — Always pass `--encoding utf-8` to `yt-dlp` (`ytdlp::ENCODING_ARGS`, spec §4.5, §8.3)

On Windows, a piped stdout defaults to the legacy ANSI code page (e.g. `cp1252`), causing `yt-dlp`'s
`--print after_move:SDP_FILE|%(filepath)s` to strip or mangle non-ASCII characters while writing the
actual media file with Unicode wide-char APIs — making `job.rs` report "file is not there". Because
PyInstaller bundles ignore `PYTHONIOENCODING`, `ytdlp::ENCODING_ARGS` (`--encoding utf-8`) is now
passed on every `yt-dlp` download and inspection call across all platforms.

### D115 — Wait for `queue.json` to record the parked state before reopening in restart test

Because `finish_item` updates in-memory state before persisting `queue.json`, the restart test now
polls `wait_for_parked_on_disk` before dropping and reopening the engine, and matches the marker's
host to the exact running item ID.

### D116 — Retry transient Windows file-locking errors (`ERROR_SHARING_VIOLATION` 32, `ERROR_ACCESS_DENIED` 5) on spawn

Extended `is_text_file_busy` in `services/process.rs` so Windows retries `CreateProcessW` for up to
1 s (50 ms interval) when an antivirus or indexer transiently holds a freshly copied tool binary
(`os error 32` or `5`), matching the Unix `ETXTBSY` (26) retry of D47.
`TODO(verify)`: confirm on a real Windows machine whether first-launch tool copies ever trigger 32
or 5 in practice.

### D117 — Split `is_text_file_busy` error codes and unit tests by OS family (`#[cfg(windows)]` vs `#[cfg(not(windows))]`)

`raw_os_error()` returns Win32 codes on Windows and errno values on Unix (where 26 is `ETXTBSY` on
Unix but `ERROR_NOT_DOS_DISK` on Windows, and 5/32 are `EIO`/`EPIPE` on Unix). Gated both the
constants and the unit tests per OS family so each platform only retries its own transient spawn
codes.

### D118 — All GitHub Actions workflows are manual (`workflow_dispatch` only)

Changed `ci.yml`, `process-tests.yml`, `selftest.yml`, and `tier2.yml` to trigger exclusively on
`workflow_dispatch` so pushes never consume private repository minutes unasked. `process-tests.yml`
("Checks and process tests") runs `check` plus `cargo test` across Linux, Windows, and macOS without
building installers; `ci.yml` ("CI") runs everything and creates the draft release when dispatched
on a `v*` tag.

---

## 2026-10-18 — M5 (Whisper sidecar CI, model manager, and offline transcription pipeline)

*(Note: M3 and M4 followed the spec directly without numbering new entries in this file; their implementation details are in `docs/PROGRESS.md`.)*

### D119 — Whisper sidecar (`sdp-whisper`) build workflow and model download manager (M5-A)

Spec §5.4, §10.2, §11.6, `whisper.models.json`.

1. **Pinned `whisper.cpp` sidecar (`sdp-whisper`) version is `v1.9.5`** (`ggml-org/whisper.cpp`), built in `.github/workflows/whisper-sidecar.yml`:
   - Linux (`x86_64`, `aarch64`): static linkage with `-static-libgcc -static-libstdc++`.
   - Windows (`x64`, `arm64`): static C++ runtime (`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`).
   - macOS (`arm64`, `x86_64`, `universal`): Metal enabled (`GGML_METAL=ON`).
2. **Model catalogue (`whisper.models.json`) pinned via Hugging Face Git LFS**:
   Contains 11 Whisper ggml models (Tiny, Base, Small, Medium, Large-v3, Large-v3-Turbo with quantized variants) and Silero VAD (`silero-v5.1.2` from `ggml-org/whisper-vad`, `ggml-silero-v5.1.2.bin`, 885,098 bytes, sha256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`). Validated offline/online via `scripts/fetch_models_manifest.mjs` (`npm run check:models`).
3. **Automatic VAD co-download**: `download_model` checks if `ggml-silero-v5.1.2.bin` exists and transparently downloads and verifies it before downloading the requested Whisper model.
4. **Resumable HTTP `Range` download and disk-space pre-flight**: streams into `<model>.part` using `Range: bytes=N-`, checks free disk space (accounting for existing `.part` bytes), supports pause/resume/cancel, and removes corrupt `.part` files on SHA-256 mismatch.
5. **Settings schema v7 & IPC**: added transcription settings and `models_list`, `models_download`, `models_pause_download`, `models_cancel_download`, `models_delete`, `models_import`, `models_set_active`, `models_open_folder`.

### D120 — Whisper offline transcription pipeline, chunking, and local media action (M5-B)

Spec §10.2, §3, §11.5, §14 row M5, §15 item 9.

1. **Audio preparation**: 16 kHz mono WAV via `ffmpeg -vn -ac 1 -ar 16000 -c:a pcm_s16le` inside the item's isolated job temp directory, guarded by a pre-flight disk-space check (`32,000 bytes/s * duration + 200 MiB`) and RAII cleanup guards (`CleanupFile`, `ChunkGuard`).
2. **Chunking for audio > 2 hours (7200 s)**: streams `ffmpeg` `silencedetect=noise=-30dB:d=0.5` line-by-line to pick split points within ±20 s of 30-minute boundaries (falling back to a hard cut with 2.0 s overlap), then merges `.txt`, `.srt`, and `.vtt` with timestamp offsetting, boundary deduplication, and continuous cue renumbering.
3. **Pinned `sdp-whisper` CLI flags**: `-m <model> -f <wav> -l <lang> [-tr] -pp -otxt -osrt -ovtt -of <out_base> [--vad -vm <silero_vad>] -t <threads> [-ng]`.
4. **GPU-to-CPU fallback (`-ng`)**: on non-zero exit when launched without `-ng` on a GPU-capable build (macOS Metal), automatically retries once with `-ng` (CPU) and emits a warning toast.
5. **Hallucination loop detection**: inspects output for >= 5 identical consecutive segments or suspicious silence-only output and emits a `HallucinationWarning` toast (spec §10.2 item 7) without mutating the written transcript text.
6. **Queue & UI integration**: bounded by `max_parallel_transcriptions`, supports post-download transcription, standalone "Transcribe a local file…", and re-transcribing completed items into collision-free `<stem>.{txt,srt,vtt}` paths.

### D121 — Errata for D119 and D120: Silero VAD metadata, `-ng` retry, and hallucination guard (B5)

Confirmed and locked by `docs_progress_and_decisions_reflect_whisper_vad_and_hallucination_reality` in `src-tauri/tests/whisper_pipeline.rs`:
1. Silero VAD in `whisper.models.json` is `ggml-silero-v5.1.2.bin` (885,098 bytes, SHA-256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`) from `ggml-org/whisper-vad`.
2. `whisper.rs` retries with `-ng` on non-zero exit when `-ng` was not already passed on GPU builds.
3. `detect_hallucination` only emits a warning toast and never trims or alters transcript text.

### D122 — Whisper sidecar CI: macOS Intel deduplication and Ubuntu 24.04 runner standard (M1)

Removed the duplicate `macos-15-intel` job from `build-tier2` in `whisper-sidecar.yml` because `build-tier1` on `macos-26` already produces `sdp-whisper-x86_64-apple-darwin` (with Metal embedded) via `lipo -thin x86_64` from the universal binary. Added a duplicate-filename guard in the `Collect and compute SHA256SUMS` packaging step, and standardised on `ubuntu-24.04` with `-static-libgcc -static-libstdc++`.

### D123 — Whisper sidecar CI: `GGML_NATIVE=ON` for Windows ARM64 MSVC build (spec §5.4, §1.1)

In `whisper.cpp` v1.9.5 (`ggml/src/ggml-cpu/CMakeLists.txt:118`), MSVC targeting ARM64 requires `GGML_NATIVE=ON` (failing CMake configuration with `FATAL_ERROR` if `OFF`). Set `-DGGML_NATIVE=ON` for the Tier 2 `windows-11-arm` build step in `whisper-sidecar.yml`.

### D124 — Restrict `tauri::test::mock_builder()` to `tests/*.rs` and pre-allocate `.part` in NTFS disk-space test

1. Because `cargo:rustc-link-arg-tests` in `build.rs` embeds `windows-app-manifest.xml` (Common Controls v6) only into integration test binaries (`tests/*.rs`) and not into the `--lib` unit test runner (D28), calling `tauri::test::mock_builder()` inside `src/` crashes `--lib` tests on Windows with `0xc0000139`. Moved `models_download_throttles_rapid_chunk_progress_events` to `tests/models.rs` and added `src_unit_tests_do_not_call_tauri_mock_builder`.
2. In `download_model_accounts_for_existing_part_file_in_disk_space_check`, sized the 200 MiB `.part` file *before* querying `free_disk_space` so Windows NTFS cluster allocation is already reflected in `GetDiskFreeSpaceExW`.

---

## 2026-10-20 — M6 (Subtitles-first pipeline, Gemini cloud engine, summaries, and secret storage)

### D125 — Transcription Phase M6: Subtitles-first pipeline, Gemini REST engine, summaries, and secret storage (spec §10.1, §10.3, §10.4, §10.5)

1. **`keyring 3.6` with zero OpenSSL**: added `keyring` v3.6 (`apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`, `linux-native`); verified `cargo tree -i openssl-sys` returns 0 matches (spec §1).
2. **Secret storage hierarchy & Linux `0600` fallback (spec §10.5)**: resolves `GEMINI_API_KEY` env var → OS Keyring (`service = "scaricatore-di-porto"`, `user = "gemini_api_key"`) → `<app_config_dir>/gemini_api_key` fallback (created with mode `0600` on Unix when D-Bus Secret Service is unavailable). The key is redacted in `Debug` impls, marked sensitive on HTTP headers, never logged, and never sent to the frontend (`GeminiKeyStatus` only exposes `has_key` and `source`).
3. **Subtitles downloader & cleaner (`services/transcribe/subtitles.rs`, spec §10.1)**: runs `yt-dlp --write-subs --write-auto-subs --skip-download`, strips WebVTT headers, inline word timestamps (`<00:00:01.520><c>`), and HTML tags, deduplicates rolling auto-caption overlaps, breaks paragraphs on pauses >= 2.0 s, and writes collision-free `<name>.txt` and `<name>.<lang>[.auto].srt`.
4. **Gemini Cloud Engine (`services/transcribe/gemini.rs`, spec §10.3)**: direct REST client over `reqwest` + `rustls-tls`. Transcodes audio via `ffmpeg` to 16 kHz mono Opus in Ogg (`-c:a libopus -b:a 24k -ar 16000 -ac 1`, with MP3/WAV fallback), streams upload from disk via Google Files API Resumable Upload (`POST /upload/v1beta/files` + streaming `POST`), polls until `ACTIVE` (10-min timeout, cancellable), enforces verbatim transcription via prompt instructions in `contents.parts`, retries HTTP 429 with backoff, discovers `generateContent` models dynamically from `GET /v1beta/models` (`-latest` first, offline fallback), and guarantees remote file deletion via RAII `GeminiFileGuard` (`DELETE /v1beta/files/{name}`) on success, error, or cancel.
5. **Map-reduce summarization (`services/transcribe/summary.rs`, spec §10.4)**: splits transcripts exceeding `MAX_SUMMARY_CHUNK_CHARS` (`50_000` chars) on paragraph boundaries, summarizes chunks, reduces them into `<name>.summary.md`, and includes timestamped chapter sections when timed cues are present.
6. **Pipeline modes & error preservation**: supports `subtitles_whisper_gemini` (default), `whisper_only`, `gemini_only`, and `subtitles_only`, records `item.transcript_origin`, and preserves actionable Whisper errors when no Gemini key is configured in fallback mode.

---

## 2026-10-23 & 2026-10-24 — M7 (Polish, on-demand tools, updater, docs, and release fixes)

### D126 — M7: on-demand `ffmpeg`/`ffprobe` provisioning and archive extraction (spec §5.1, §5.2)

Added generic `tools::install_tool` supporting plain binaries, `.gz` (`ArchiveKind::Gz`), `.zip`,
`.tar.gz`, and `.tar.xz` with SHA-256 verification and path-traversal guards. Installing `ffmpeg`
also provisions `ffprobe` as its companion so the Setup Wizard and Tools page only require a single
Install action. *(FFmpeg version upgraded from 7.0.2 to 9.0.2 in D140; `sdp-whisper` release URLs
corrected and enabled in D134.)*

### D127 — M7: in-app updates, signed `latest.json` manifest, and MSRV 1.90 (spec §13.4)

1. Added `tauri-plugin-updater` 2.14.0 and `tauri-plugin-process` 2.4.0 (bumping `rust-version` to **1.90**).
2. With `bundle.createUpdaterArtifacts = true` (Tauri v2 mode), Linux produces raw `*.AppImage` + `*.AppImage.sig` (only AppImage self-updates on Linux; `.deb`/`.rpm` link to the Releases page via `canSelfUpdate()`), macOS produces `*.app.tar.gz` + `.sig`, and Windows NSIS produces `*-setup.exe` + `.sig`.
3. `scripts/generate-latest-json.mjs` builds `latest.json` for `linux-x86_64`, `windows-x86_64`, `darwin-aarch64`, and `darwin-x86_64` from the collected `.sig` files in the CI `release` job.
4. The app checks at most once per 24 h (timestamp in `localStorage`), never installs without user action, and refuses to install while the queue is active. *(Repository slug corrected in D134; signing key rotated in D137.)*

### D128 — M7: minimal append-only log and privacy-safe diagnostics without `tauri-plugin-log` (spec §11.8)

`services/logging.rs` appends `[unix-seconds] LEVEL: message` (newlines sanitised) to
`<app_log_dir>/scaricatore-di-porto.log` without pulling in `tauri-plugin-log`.
`services/diagnostics.rs` builds the **Copy diagnostics** report (app version, OS/arch, paths, tool
statuses, and the last 80 log lines) and is unit-tested to never include `api_key`, `AIza`, or
`cookie` strings.

### D129 — M7: bilingual user documentation

Initially added `.it.md` translations alongside English user docs. *(Superseded by D138: `.it.md`
files removed in favour of English-only repository documentation.)*

### D130 — M7: UI quality-of-life, keyboard shortcuts, Setup Wizard, and collapsible error details (spec §11.5, §11.9)

1. **Queue**: added find-as-you-type filter (`Ctrl/Cmd+F`), row checkboxes, Shift-click range selection, select-all-shown, and bulk pause/resume/cancel/remove bar while keeping the 5,000-item list virtualised (< 20 DOM nodes).
2. **Shortcuts (`src/lib/shortcuts.ts`)**: separated `isTypingTarget` from `isInteractiveTarget` so `Ctrl/Cmd+F` works after clicking a checkbox while `Delete`/`Space` do not hijack focused buttons/links; added `Ctrl/Cmd+,` for Settings.
3. **Setup Wizard**: wired real tool installs (single and batch), recommended Whisper model download/cancel/resume, transcription engine order, output folder picker, and focus trap.
4. **Error banners & `ToolSource`**: localized `message_key` shown first with raw technical details inside a collapsed `<details>`; fixed `ToolSource::PathEnv` to serialize as `"path"`.

### D131 — M7: MIT `LICENSE` file

Added root `LICENSE` (MIT, Copyright (c) 2026 Emanuele Fusco) matching `package.json` and
`Cargo.toml`, with third-party component licences listed in **Settings → About → Licences**.

### D132 — M7: installer size measurements without a hard CI failure gate (spec §1, §15 item 1)

Measured on local Linux release build (2026-10-23):
- Application binary: **8.5 MiB** (well under spec §1's ≤ 15 MB limit; +1.6 MiB over M6 due to `tauri-plugin-updater` pulling `reqwest 0.13` alongside `0.12`).
- `.deb` / `.rpm`: **43.2 MiB** (dominated by bundled `yt-dlp` ~38.6 MiB and `qjs` ~2.4 MiB for offline first run, D30).
- `.AppImage`: **148.9 MiB** (excluded by spec §1 as it embeds WebKitGTK).
`scripts/report-bundle-sizes.mjs` reports all bundle sizes in CI without failing the build on installer size.

### D133 — M7: Linux idle memory measurement and the 30 MB target (spec §15 item 1)

Measured on Linux x64 / CachyOS / KDE Wayland + NVIDIA RTX 3080 after 30 s idle: main process
`scaricatore-di-porto` uses **191,920 kB VmRSS / 87,832 kB PSS / 44,628 kB RssAnon** (dominated by
shared library mappings: `libnvidia-gpucomp.so` 27.8 MB, `libLLVM.so` 26.3 MB, `libwebkit2gtk`
19.7 MB, `libjavascriptcoregtk` 12.5 MB, `[anon:WKFastMalloc]` 11.4 MB). Spec §15 item 1's ≲ 30 MB
idle RSS target is not met on this Linux WebKitGTK + NVIDIA stack and remains recorded as open in
`docs/ACCEPTANCE.md` pending measurement on a clean VM without the NVIDIA/LLVM driver stack.

### D134 — M7 correction: repository slug `EmanueleFusco/Scaricatore-di-Porto-rust` and published `sdp-whisper` assets

Corrected the GitHub repository slug from `EmanueleFusco/ScaricatoreDiPorto` to
`EmanueleFusco/Scaricatore-di-Porto-rust` across `tools.manifest.json`, `src-tauri/tauri.conf.json`,
About links, README, and tests. Populated the verified URLs and SHA-256 checksums for all 7
published `whisper-v1.9.5` `sdp-whisper` assets in `tools.manifest.json` and verified end-to-end
download + `sdp-whisper -h` execution.

### D135 — Linux Wayland: apply `WEBKIT_DISABLE_DMABUF_RENDERER=1` inside `src-tauri/src/lib.rs` (spec §13.3)

Moved the WebKitGTK Wayland DMA-BUF crash workaround (upstream WebKit bug 280210) from
`scripts/run.mjs` into `apply_webkit_dma_buf_fallback()` in `src-tauri/src/lib.rs` before the Tauri
builder initialises, respecting any existing user `WEBKIT_DISABLE_DMABUF_RENDERER` env var.

### D136 — Restrict the automatic Wayland DMA-BUF fallback to NVIDIA drivers (spec §13.3)

Refined `apply_webkit_dma_buf_fallback()` in `src-tauri/src/lib.rs` to only set
`WEBKIT_DISABLE_DMABUF_RENDERER=1` on Linux Wayland when an NVIDIA proprietary driver is detected
(`/proc/driver/nvidia/version` or `/dev/nvidia0`), keeping hardware-accelerated DMA-BUF rendering on
AMD/Intel Wayland systems.

### D137 — Rotated Tauri updater signing key pair (spec §13.4)

Generated a new minisign key pair on 2026-10-24 (`~/.config/scaricatore-di-porto/updater.key`, mode
`0600`, outside the repo; key ID `34F4F78211F08D2A`), updated `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`, and verified `.AppImage.sig` Ed25519 signature validation. The private
key must be stored in GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY` (with empty
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).

### D138 — English-only repository documentation

Removed `README.it.md`, `docs/TROUBLESHOOTING.it.md`, and `docs/SUPPORTED_SITES.it.md`, and rewrote
`README.md` in concise English per owner instruction. The application UI remains bilingual (IT/EN).

### D139 — Co-locate `ffmpeg` and `ffprobe` in `<tools_dir>/ffmpeg`, allow switching to system `PATH`, and refresh live Windows `PATH` (spec §5.2, §8.2, §11.8)

1. **`ffprobe` co-location & migration**: `yt-dlp --ffmpeg-location` requires `ffmpeg` and `ffprobe` in the same directory. `app_data_dir_for` now maps both to `<tools_dir>/ffmpeg/`, `migrate_legacy_ffprobe_dir()` moves any legacy `<tools_dir>/ffprobe/ffprobe[.exe]` automatically, and `sibling_companion_override` resolves `ffprobe` alongside a custom `ffmpeg` override path (and vice versa).
2. **`tools_use_system` ("Use system PATH")**: added `systemPath` detection on `ToolStatus` and the `tools_use_system` IPC command so users can switch from an `appData` copy or custom override back to a valid system `PATH` binary without restarting.
3. **Live Windows Registry `PATH` reading**: on Windows, `live_path_dirs()` merges `std::env::var_os("PATH")` with `HKCU\Environment\Path` and `HKLM\...\Session Manager\Environment\Path` (with `%VAR%` expansion) so newly installed system tools are detected on **Re-check** without restarting the app.

### D140 — Upgrade on-demand FFmpeg to 9.0.2, single-pass companion extraction, and stream-indexed thumbnail/HEVC encoding fixes (spec §5.1, §5.2, §8.4, §9.2, §11.2)

1. **FFmpeg 9.0.2 upgrade**: updated `tools.manifest.json` to FFmpeg `9.0.2` across Windows x64 (`GyanD/codexffmpeg`), Linux x64/arm64 (`Tyrrrz/FFmpegBin`), and macOS arm64/x64 (`Tyrrrz/FFmpegBin`) with verified SHA-256 hashes, and upgraded `find_executable` in `services/download.rs` to breadth-first recursive directory search.
2. **Single-pass companion extraction**: `install_tool` extracts both `ffmpeg` and `ffprobe` from the same downloaded `.zip` archive in a single pass.
3. **Attached thumbnail stream mapping (`exit code -22` and `-1094995529`)**:
   - Replaced invalid `-c:v:disp:attached_pic` / `-map 0:v:disp:attached_pic?` with `-map 0:disp:attached_pic?`.
   - When `embed_thumbnail` is enabled during video re-encode, qualified primary video encoder flags with stream index `:0` (`-c:v:0 <encoder>`, `-tag:v:0 hvc1`, `-b:v:0`, `-q:v:0`) and copied the thumbnail stream with `-c:v:1 copy -disposition:v:1 attached_pic` so FFmpeg neither re-encodes the JPEG cover art as video nor applies `hvc1` FourCC tags to the `mp4v` thumbnail stream.

### D141 — Generic transcription naming, reactive engine layout, dynamic transcribe stage status, custom scrollbars, and FFmpeg license accuracy (spec §10.1–§10.3, §11.5–§11.9)

1. Renamed top-level download option labels from "Whisper transcription" to "Transcription" / "Transcribe" (`en.json`, `it.json`) and moved the **Engine** dropdown to the top of `AddBar.svelte`, disabling Whisper-specific controls when `gemini_only` or `subtitles_only` is selected.
2. Emitted stage-specific progress labels (`transcribingSubtitles`, `transcribingWhisper`, `transcribingGemini`) from `services/queue/engine.rs`.
3. Added theme-integrated thin scrollbars (`scrollbar-width: thin` and `::-webkit-scrollbar`) in `src/lib/styles/app.css`.
4. Updated FFmpeg/FFprobe license strings in `src/lib/about.ts` and `tools.manifest.json` to `LGPL-2.1-or-later / GPL-2.0-or-later` per `ffmpeg.org/legal.html`.

### D142 — Cross-platform mutability in `live_path_dirs` (spec §5.2)

Declared `let dirs = path_dirs(...);` immutable at the top of `live_path_dirs()` in
`src-tauri/src/services/tools.rs` and rebound `let mut dirs = dirs;` only inside the
`#[cfg(windows)]` Registry block, eliminating `-D unused-mut` failures on Linux and macOS CI without
`#[allow(unused_mut)]`.

---

## 2026-11-06 — Documentation maintenance

### D143 — Synthesis and English unification of `docs/PROGRESS.md`, `docs/DECISIONS.md`, and `docs/SPIKES.md`

`docs/PROGRESS.md` (4,702 lines / 353 KB) and `docs/DECISIONS.md` (3,627 lines / 227 KB) had grown
large enough to saturate agent context windows due to ~100 micro-step review blocks, repeated test
boilerplate, and verbose conversational essays (with M0–M1 entries in Italian). Per owner request:
- Compacted `docs/DECISIONS.md` while keeping every decision ID (`D1`–`D143`), technical root cause,
  verified flag/hash/measurement, cross-reference, and test-asserted invariant intact, translating
  D1–D76 into English.
- Compacted `docs/PROGRESS.md` by consolidating M0-A through M6 into per-milestone blocks (with all
  review fixes folded in) while keeping the M7 and post-M7 blocks detailed at the bottom for
  `AGENTS.md` §10 session startup.
- Translated and tightened `docs/SPIKES.md` (`S1`–`S6`) into English.
