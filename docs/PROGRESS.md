# PROGRESS

Running log, one block per step, newest last (format defined in `AGENTS.md` §9).

---

## 2026-10-12 — M0-A / repo preparation + project skeleton, settings, i18n, theme, shell UI

### Done

* **Repository prepared** per `COME_INIZIARE.md` §1 and `AGENTS.md` §3: `git init`, `.gitignore`,
  the spec moved to `docs/RUST_PORT_GUIDELINES.md`, the Python app moved to
  `reference/ScaricatoreDiPorto.py`, empty `docs/PROGRESS.md` / `DECISIONS.md` / `SPIKES.md`
  created, first commit on `main`, work on branch **`m0-skeleton`**.
* **Backend** (`src-tauri/src/`)
  * `error.rs` — `AppError` (thiserror) + `ErrorInfo { code, messageKey, detail }`, camelCase wire
    format, stable codes `IO / JSON / TAURI / SETTINGS_CORRUPT / UNSUPPORTED`.
  * `state.rs` — `Settings { schemaVersion, language, theme, defaultOutputDir }` with
    `serde(default)`, schema version 1, lenient enum deserialisation, atomic save, corrupt-file
    recovery to `.bak`, `AppState` behind an `RwLock`, `settings_view()`.
  * `services/paths.rs` — app config/data/cache dirs, `settings_file`, `tools_dir`,
    `private_temp_dir` (mode `0700` on Unix), `default_output_dir` (OS Downloads),
    `atomic_write` (temp file + `fsync` + `rename`, replaces the destination on all 3 OSes),
    `backup_file`, and `sanitize_component()` (Unicode-preserving, spec §12.5).
  * `commands/settings.rs` — thin `settings_get` / `settings_save`.
  * `lib.rs` — plugins (`dialog`, `opener`, `single-instance` as the **first** plugin), state
    management, exit hook placeholder, no `unwrap()`/`expect()` outside tests.
* **Config** — `tauri.conf.json`: window 980×680, min 640×520, no maximum, centred;
  strict `csp` + separate `devCsp`; `freezePrototype: true`. `capabilities/default.json` grants
  only `core:default`, the two window-theme permissions, `dialog:default`, `opener:default`.
  **No `tauri-plugin-shell`** (spec §1).
* **Cargo release profile** exactly as spec §1: `opt-level="s"`, `lto`, `codegen-units=1`,
  `panic="abort"`, `strip=true`.
* **Frontend** (plain Vite + Svelte 5, see DECISIONS.md D1)
  * `src/lib/i18n/` — `it.json` / `en.json`, `t()`, `tCount()` (one/other), interpolation,
    OS-locale detection, dictionary key-parity test.
  * `src/lib/styles/app.css` — CSS variables only, 4/8 px spacing grid, light + dark palettes,
    system font stack, `prefers-reduced-motion`, focus ring, 4/8 px density tokens.
  * `src/lib/theme.ts` + `stores/theme.svelte.ts` — `System / Dark / Light`; `window.theme()` +
    `onThemeChanged` is the source of truth, `prefers-color-scheme` is the fallback,
    `getCurrentWindow().setTheme(null)` for "System".
  * `src/lib/stores/settings.svelte.ts` — mirrors the Rust document, write-through on change,
    `isTauriRuntime()` fallback so `npm run dev` is usable in a browser.
  * `src/App.svelte`, `src/lib/components/{Sidebar,FooterStatus,Toast}.svelte`,
    `src/lib/pages/{QueuePage,SettingsPage}.svelte`, `src/lib/router.svelte.ts`.
  * Accessibility: skip link, `aria-label`ed nav/footer, `aria-current="page"` (plus a left bar
    and bold text, never colour alone), labelled selects, visible focus, `role="status"` /
    `role="alert"`.
* **Tests** — 32 Rust unit tests, 5 Rust IPC integration tests (`tauri::test::MockRuntime`,
  real `invoke` path, hermetic temp dirs), 28 frontend unit tests (vitest).

### Verified by running

```
cd src-tauri && cargo fmt --all -- --check          -> clean
cd src-tauri && cargo clippy --all-targets -- -D warnings -> Finished, 0 warnings
cd src-tauri && cargo test                          -> 32 + 5 passed, 0 failed
npm run check                                       -> svelte-check found 0 errors and 0 warnings
npm test                                            -> 3 files, 28 tests passed
npm run build                                       -> dist/ 70.28 kB JS + 7.70 kB CSS (23.8 + 2.1 kB gzip)
npx tauri build --no-bundle                         -> ok in 1m36s
```

* **Release binary: 4.58 MiB** (`src-tauri/target/release/scaricatore-di-porto`) — well under the
  spec §1 target of ≤ 15 MB. 313 Rust crates in the dependency tree.
* **Single-instance works**: with the app running, a second launch of the same binary exits
  immediately with code 0 instead of opening a second window (spec §3).
* **`npm run tauri dev` works**: Vite 8 + the plain Svelte setup compile and the app process stays
  alive; no version-mismatch warning, no CSP errors in the log.
* **Strict CSP verified against the production bundle**: `dist/index.html` contains **no inline
  `<script>` and no inline `<style>`**; the built app was served with the exact `csp` object from
  `tauri.conf.json` (`npm run check:csp` → `scripts/serve-dist-with-csp.mjs`) and loaded with
  **zero console messages**. Dark theme, Italian (from the OS locale), sidebar, empty queue state
  and status footer all rendered; switching to Settings navigated in-app (`/` → `/settings`,
  `aria-current` updated); switching language to English and theme to Light re-rendered
  immediately. Screenshots taken of both palettes and both languages.
* **Version alignment**: `tauri 2.12.3` / `tauri-build 2.7.2` / `plugin-opener 2.7.1` /
  `plugin-dialog 2.8.2` in Rust, matching `@tauri-apps/api 2.12.3` / `plugin-opener 2.7.1` /
  `plugin-dialog 2.8.2` in npm. (`tauri-build` has its own version line and is **not** tied to the
  `tauri` crate — noted because it caused a confusing resolution error.)

### Not verified / stubbed

* **The packaged Tauri window was never screenshotted.** The app launches and stays alive, but in
  this environment its window could not be raised onto the captured desktop, so there is no
  picture of the real window. The evidence that the UI is not blank is indirect but strong: the
  production bundle runs under the exact CSP in a browser with no console output, and the release
  binary starts and keeps running. **Superseded by the next block.**
* **Linux + this machine only.** Nothing was run on Windows or macOS; no `tauri build` bundle
  (`.deb`/`.rpm`/AppImage/NSIS/dmg) was produced, only `--no-bundle`.
* **Settings persistence was verified through the IPC tests, not by hand.** The test
  `settings_survive_a_restart` writes through `settings_save`, drops the app and builds a new one
  that reads the same values back — but nobody has yet clicked the two dropdowns in the real app.
  Please do that as the first manual check.
* **Theme "System" was not verified against a real OS theme change**: WebKitGTK reported dark via
  the media query, and the Tauri window API path could not be exercised outside the app.
* **Sidebar collapse under 800 px** is CSS-only and was not visually checked (no narrow viewport
  available in this environment).
* Window minimum-size enforcement (640×520) not checked by resizing a real window.
* Nothing from M0-B onwards is implemented: no `process.rs`, no tools, no manifest, no wizard, no
  CI workflow, `scripts/` only has the CSP helper.

### Spec items marked "verify" that I checked

* `tauri.conf.json` schema (`https://schema.tauri.app/config/2`, fetched and parsed) — confirmed
  `app.windows[].minWidth/minHeight`, `app.security.csp`/`devCsp` object form, and that
  `app.security` has **no** `devtools` key (it lives on the window config and requires the
  `devtools` cargo feature in release).
* `bundle.externalBin` naming rule read from the same schema (M0-C will need it).
* `@tauri-apps/api` 2.12.3 type definitions — confirmed `Window.theme()`,
  `Window.onThemeChanged()`, `Window.setTheme()` and their required permissions
  (`core:window:allow-theme`, `core:window:allow-set-theme`).
* `tauri::test::get_ipc_response` availability (tauri 2.12.3 `test` feature) — used by the
  integration tests. Note: `Builder::setup` does **not** run on `MockRuntime` (it only runs on the
  runtime's `Ready` event), so the tests manage the state on the built app instead.
* `std::fs::rename` replaces the destination on Windows (`MoveFileExW` +
  `MOVEFILE_REPLACE_EXISTING`) — this is why `atomic_write` needs no extra dependency.
* Tauri CLI version check: it compares **major/minor** between the Rust crates and the npm
  packages and refuses to run on a mismatch.

### Decisions/deviations (also in DECISIONS.md)

D1 (plain Vite + Svelte instead of SvelteKit — the template's inline bootstrap script breaks the
strict CSP), D2 (plugins added), D3 (devtools off in release without a config flag), D4 (lenient
enum deserialisation), D5 (camelCase wire format), D6 (settings file only written on first change),
D7 (non-Tauri fallback), D8 (CI/manifest/wizard are M0-C), D9 (`original_python/` kept).

### Next step

M0-B — `services/process.rs`: safe cross-platform spawn, Job Object / process-group tree kill,
concurrent stdout+stderr draining with a 200-line stderr ring buffer, `CancellationToken` +
`tokio::select!`, per-app private temp dir, with a `fake_tool` test binary and the four tests
listed in `PHASE_PROMPTS.md` M0-B.

---

## 2026-10-12 — M0-A / Wayland crash on this machine + window captured

### Done

* **Diagnosed the crash reported by the owner.** `npm run tauri dev` printed
  `Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.` and the process
  died immediately, so nothing could be tested by hand.
  Environment: **CachyOS, KDE Plasma 6.7.5, Wayland session** (`XDG_SESSION_TYPE=wayland`) with
  XWayland on `:0`; `webkit2gtk-4.1 2.54.1`, `gtk4 4.24.1`, `glib 2.90.1`, `mesa 26.2.4`,
  NVIDIA RTX 3080 (proprietary driver, modeset enabled).
* **Root cause is NOT our code — verified.** The pristine Tauri template
  (`create-tauri-app svelte-ts`, its untouched scaffold, none of our code) was built and launched on
  the same Wayland session: it dies with the **identical** `Error 71`. So this is
  GTK4/WebKitGTK against this compositor, not our configuration and not our Rust code.
* **Measured matrix** (release binary, fresh start each time, window captured with
  `magick import -window` and its mean pixel value measured — `0.0000` would mean a black window):

  | backend | outcome | window | mean brightness | `Error 71` | GBM warnings |
  | :-- | :-- | :-- | :-- | :-- | :-- |
  | `GDK_BACKEND=wayland` | **dies** | none | — | **1** | 0 |
  | `GDK_BACKEND=x11` | **alive** | yes | **0.137** (renders) | 0 | 2 (harmless) |

  So **both** environment variables are needed on this machine:
  * `GDK_BACKEND=x11` — without it the app dies with `Error 71`.
  * `WEBKIT_DISABLE_DMABUF_RENDERER=1` — the window renders **black on screen** otherwise
    (the two `Failed to create GBM buffer` messages). With it, **the GBM errors disappear entirely**
    and the log is empty. This is the fallback spec §13.3 prescribes.
* **`npm run app` updated** (`scripts/run-dev.mjs`): runs `tauri dev`, and on Linux only, when the
  session is Wayland *and* XWayland is available *and* the user has not already set `GDK_BACKEND`,
  it injects both variables and prints exactly what it is doing and why. One command, no hidden
  behaviour, unchanged on machines where Wayland works.
* **`devCsp` relaxed for HMR only**: `connect-src` now ends with `ws: wss:` instead of hard-coded
  `ws://localhost:1421` / `http://localhost:1420`, which were wrong anyway once `devUrl` became
  `http://127.0.0.1:1420`. The production `csp` is untouched.

### Verified by running

* `npm run app -- --help` → prints the explanation and forwards the arguments to the CLI.
* Release binary + `GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1`: alive, **0 GBM errors**,
  empty log.
* The window's X11 buffer contains the finished shell — sidebar with **Coda** marked current, the
  empty-queue card (*"Incolla un link per iniziare"*), the status footer (*"In attesa — Nessun
  processo in esecuzione"*), **Italian**, **dark** palette taken from the OS (KDE Breeze-Dark), so
  theme `System` works end to end. 980×680 as configured, `WM_NAME "Scaricatore Di Porto"`.
* **No error banner in the real app**, which means the `settings_get` IPC round trip to Rust
  succeeded (a failure would render the `role="alert"` toast).

### Not verified / stubbed

* **Whether the window is actually visible on screen is still unverified by me.** My capture tool
  (`magick import -window`) reads the X11 window buffer directly and therefore bypasses the
  compositor, so it cannot tell "correct pixels" from "correct pixels that the GPU never shows".
  The disappearance of the GBM errors is strong evidence that the black window is gone, but the
  owner must confirm it visually.
* The **underlying GTK bug** behind `Error 71` was not identified — no upstream bug search was
  done. It is recorded as an environment limitation, not a project defect.
* Clicking the language/theme dropdowns **by hand is still pending**: the capture tool cannot
  synthesise input, so `settings.json` has still never been written on this machine.
* macOS and Windows remain completely untested.

### Two measurement mistakes of mine, recorded so they are not repeated

1. **Stale instance.** For a while I reported contradictory results ("MORTO" for configurations that
   had worked minutes earlier). A leftover test copy of the binary (`/tmp/opencode/app-base`) was
   alive and held the `com.scaricatorediporto.app.SingleInstance` D-Bus name, so every later launch
   was treated as a second instance and exited at once. **When the app "exits immediately with no
   error", check whether a previous instance is still running** (`gdbus ... GetNameOwner`, `ps`).
2. **Wrong measurement device.** I concluded "the window renders, brightness 0.137" from
   `magick import`. That reads the X11 backing buffer and **bypasses the compositor**, so it was
   measuring memory, not the screen: the window was in fact black on screen. The real signal was
   the `Failed to create GBM buffer` line I had dismissed as harmless. **Capture through the
   compositor (`spectacle`) or trust the user's eyes, never `magick import`, for display bugs.**

### Decisions/deviations

D10 in `docs/DECISIONS.md`: the Wayland crash and the black window are worked around in the dev
launcher rather than "fixed" in the app, because the same crash happens with the unmodified Tauri
template.

---

## 2026-10-12 — M0-A / why the release binary and `npm run app` behaved differently

### Done

* **Owner asked why `./target/release/scaricatore-di-porto` did not start while `npm run app` did.**
  Measured, not guessed:

  | how it is launched | outcome | window | `Error 71` |
  | :-- | :-- | :-- | :-- |
  | release binary, **no env vars** | **dies** | none | **1** |
  | release binary, `x11` + `no-dmabuf` | **alive** | yes | 0 |
  | debug binary (what `npm run app` runs), **no env vars** | **dies** | none | **1** |

  **The release binary is not broken.** Both binaries fail identically without the environment
  variables; `npm run app` only appeared to work because `scripts/run.mjs` injects them. The
  release binary also has the frontend embedded (no `Could not connect to 127.0.0.1`), because it
  was built with `--features tauri/custom-protocol`.
* **`scripts/run.mjs` replaces `scripts/run-dev.mjs`** and supports two modes, so both routes
  behave identically:
  * `npm run app` → `tauri dev` (Vite dev server + debug binary)
  * `npm run app:binary` → the compiled release binary; if it is missing it prints the right build
    command and warns that a bare `cargo build --release` is not enough.
* Verified: `npm run app:binary` → alive, window created, 0 `Error 71`, 0 GBM errors.

### Not verified / stubbed

* Still not verified by me that the window is visible **on screen** (the capture tool bypasses the
  compositor); the owner is the only one who can confirm the display bugs are gone.

### Next step

M0-B, unchanged.

---

## 2026-10-12 — M0-A / icons replaced with Lucide + settings persistence confirmed by the owner

### Done

* **Owner confirmed the settings round trip by hand**: language and theme changed in the UI survive
  closing and relaunching the app. This was the last unverified item of M0-A.
* **Owner reported the gear icon in the sidebar was malformed.** Root cause was my own method: the
  two sidebar icons were hand-written inline SVG `<path>` data recalled from memory. AGENTS.md §2.3
  ("Do not invent […] verify them") applies to SVG path data like any other value.
* **Replaced with `lucide-svelte` 1.0.1** (see DECISIONS.md D11): `download`, `list-video`,
  `settings`, imported individually from `lucide-svelte/icons/<name>`.
  * Still **inline SVG**, as spec §1 requires — no icon font, no remote asset.
  * **Tree-shaking measured, not assumed**: bundle 70.28 kB → 81.97 kB JS for three icons;
    `airplay` / `airplane` / `banana` / `trophy` absent from the bundle, the three used paths present.
  * **ISC licence** (Feather-derived icons MIT) — must appear in the About screen (spec §11.8, M7).
* Project rule recorded: never hand-write SVG path data.

### Verified by running

* Release binary rebuilt and captured: the sidebar now shows a correct gear, a correct
  list-with-play icon for **Coda**, and a correct download arrow in the brand. 980×680 as configured.
* `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (32 + 5),
  `npm run check` (0 errors/warnings), `npm test` (28) — all green.

### Not verified / stubbed

* The icon set has **not** been exercised at small sizes on Windows/macOS, and only 3 of ~1600
  icons are in use; the rest get picked as the UI grows (M2 needs play/pause/folder/trash/retry).
* Still not verified by me: that the packaged window is visible **on screen** (see the previous
  block — the capture tool bypasses the compositor).

### Another of my mistakes: `cargo build --release` alone produces a useless binary

While rebuilding to screenshot the icons I got `Could not connect to 127.0.0.1` inside the release
app. Tauri decides dev-vs-dist with `dev: cfg!(not(feature = "custom-protocol"))`
(`tauri-macros/src/context.rs`), and **that feature is what `tauri build` turns on**. So:

```
cargo build --release                                      # compiles, runs, but loads devUrl -> broken
cargo build --release --features tauri/custom-protocol     # real standalone binary
npx tauri build                                           # the supported way
```

Recorded because it looks like a packaging bug and is not one.

### Next step

M0-B, unchanged.

---

## 2026-10-12 — M0-C item 8 (pulled forward) / cross-OS CI

### Done

* Owner asked how to test on Windows and macOS. Answer: only CI can do it from here; cross-
  compiling to Windows is possible but useless (the result cannot be run), and cross-compiling to
  macOS from Linux is impossible (needs Xcode + Apple SDK).
* **`.github/workflows/ci.yml` written** (spec §13.5), shaped around the owner's **private**
  repository and its minute budget (DECISIONS.md D12): lint+tests on Linux only at every push, the
  3-OS bundle matrix only when an artifact is actually wanted, and the Tier 2 targets
  (macOS Intel, Windows ARM64, Linux ARM64) only on `workflow_dispatch`.
* **`scripts/report-bundle-sizes.mjs`** puts the artifact sizes in the job summary (spec §1).
  Verified locally against a real bundle.

### Verified by running

* `npx tauri build` produces bundles on this machine: `.deb` **1,9 MiB**, `.rpm` **1,9 MiB**,
  `.AppImage` **108,0 MiB** (the AppImage is big on purpose — it embeds WebKitGTK, and spec §1
  excludes it from the size targets). App binary alone: **4,58 MiB**, under the ≤ 15 MB target.
* `scripts/report-bundle-sizes.mjs` reports exactly those numbers.
* `npm run check` (0 errors/warnings) and `npm test` (28) green; the workflow YAML has no tabs and
  parses into the expected jobs.

### Not verified / stubbed

* **The CI has never run.** The first execution is the real test. Most likely to need a fix: the
  Ubuntu system package list and the `universal-apple-darwin` build.
* No Windows or macOS build exists yet — nothing has been compiled for those targets.
* `scripts/fetch_bundled_tools.mjs`, `tools.manifest.json` and the sidecar job (spec §13.5 c) are
  still missing: they belong to M0-C, which is not finished.

### Next step

M0-B, unchanged.

---

## 2026-10-12 — M0-A / Windows and macOS audit

### Done

* Owner confirmed the policy: **oldest supported product, for the widest compatibility**. That
  decision drove the two changes below.
* Audited Windows x64 and macOS against GitHub's runner README, Tauri's prerequisites page (updated
  20 Aug 2026) and the published Tauri config schema — not from memory. Full reasoning and the
  corrected facts are in DECISIONS.md D15.
* **`bundle.targets`: `"all"` → explicit list without `msi`.** `"all"` would also build an MSI on
  Windows, and Tauri's docs say MSI packaging needs the VBSCRIPT optional feature on the build
  machine, which may be absent on the Windows runner images. Spec §13.1 prefers NSIS and never
  mentions MSI.
* **CI now uses `windows-2022` instead of `windows-latest`**, because `windows-latest` moved to
  Windows Server 2025 with **Visual Studio 2026** in June 2026; the older image is both the
  conservative choice and the one matching the owner's policy.

### Verified by running

* `npx tauri build` with the new target list still produces all three Linux bundles
  (`.deb` 1.94 MiB, `.rpm` 1.94 MiB, `.AppImage` 108.01 MiB) — the list does not break Linux.
* `npm run check` (0 errors/warnings), `npm test` (28), `cargo fmt --check`,
  `cargo clippy -D warnings`, `cargo test` (32 + 5) all green.
* **First CI run: every job green** (30m 17s wall clock) — `check` 7m08s, `bundle` linux-x64 7m30s,
  windows-x64 9m12s, macos-arm64 2m35s, macos universal 3m52s.
* **The Rust code now compiles and links for Windows x64, macOS arm64 and macOS universal** — the
  first time anything has ever been built for those targets.

### Not verified / stubbed

* **No CI runner ever opened a window.** Green means "it compiles, links and bundles", not "it
  works". Manual testing on real Windows and macOS hardware is still outstanding.
* **The tests ran only on Linux.** `cargo test` lives in the `check` job (ubuntu only); the `bundle`
  matrix does not run tests. So the open question from D15 — whether `std::fs::rename` replaces the
  destination on Windows — is **still untested**.
* Tier 2 targets (macOS Intel, Windows ARM64, Linux ARM64) show green but were **skipped**: that job
  only runs on `workflow_dispatch`. The literal `${{ matrix.label }}` in the run summary is how
  GitHub renders a skipped matrix that never expanded — it does not mean they were tested.
* The artifacts themselves have not been downloaded or run by anyone yet.
* My prediction that `bundle-macos-universal` would be the most likely failure was **wrong**: it
  passed in 3m52s.

### Next step

M0-B, unchanged.

---

## 2026-10-12 — M0-A chiusa / PASSAGGIO DI CONSEGNE

Read this block first if you are a new session continuing the work.

**Stato:** M0-A finita e verificata. Il branch `main` e` pulito, tutto committato, working tree
vuota. Versione `0.1.0`.

**Cosa funziona, verificato:** la shell dell'app (sidebar, coda vuota, footer), le impostazioni
persistenti su tre sistemi, i18n IT/EN con cambio a runtime, tema System/Dark/Light, single-instance,
CI su Linux/Windows/macOS con pubblicazione delle release in bozza.

**Cosa NON esiste ancora:** `services/process.rs` e tutto il resto. Nessuna coda, nessun download,
nessun tool esterno. L'app non scarica niente: e' una shell, e va detto cosi' a chi la usa.

**Prossimo passo: M0-B.** Il testo del task e' in `PHASE_PROMPTS.md`, sezione "M0-B". Il file
e' **sul disco ma non tracciato in git** (D17: escluso da GitHub, tenuto in locale per gli
agenti). Se non lo trovi, controlla `.gitignore` prima di chiederlo.

```bash
git switch -c m0-process      # non lavorare direttamente su main
```

### Tre cose che ti risparmieranno tempo

1. **`reference/ScaricatoreDiPorto.py` e' sul disco ma non in git.** Se non lo vedi con
   `ls` o `git ls-files`, e' solo perche' e' in `.gitignore`: non e' sparito. Serve dalla **M1**
   (lavoro di parita'), per M0-B non serve. Stessa cosa per `PHASE_PROMPTS.md` e
   `COME_INIZIARE.md`.
2. **Su questa macchina l'app non parte senza aiuto.** Sessione Wayland + WebKitGTK = `Error 71`
   e finestra nera. Usa `npm run app` (mette lui le variabili) e non `npm run tauri dev` diretto.
3. **I check del turno precedente passavano tutti.** Se un check fallisce subito, il problema e'
   tuo, non preesistente: 32 + 5 test Rust, 28 test frontend, clippy pulito, `npm run check:ci`
   valido.

### Cosa NON e' stato verificato, e conta

* Nessun runner ha mai aperto una finestra. "Verde" significa "compila, linka, impacchetta".
* **I test girano solo su Linux.** `cargo test` sta nel job `check` (ubuntu); i job `bundle`
  compilano ma non eseguono la suite. Se M0-B aggiunge test che dipendono dal sistema
  (spawn, Job Object, process group), vanno eseguiti esplicitamente anche su Windows e macOS,
  altrimenti la copertura e' solo-linux anche quando sembra multi-OS.
* `std::fs::rename` che sostituisce il file su Windows: documentato, non testato su Windows.

---

## 2026-10-13 — M0-B / process layer (`services/process.rs`, spec §4)

Read this block first if you are a new session continuing the work.

### Done

* **`src-tauri/src/services/process.rs`** (about 1 100 lines with its tests) is the only module allowed
  to spawn a process. Public API:
  * `spawn_tool(ToolCommand, CancellationToken, LineSink) -> Result<ProcessResult, AppError>`;
  * `spawn(cmd, sink) -> RunningTool` + `RunningTool::wait(token)` when the caller wants to cancel
    from elsewhere too;
  * `kill_all_running()` / `running_count()` — the registry the app-exit hook walks;
  * `ProcessResult { label, pid, exit_code, cancelled, stderr_tail[≤200], duration_ms }`.
* **`ToolCommand`** builds an argv array, never a string. The URL is a *separate field*, validated as
  `http`/`https` only and appended after `--` by `argv()`, so "URL last" is a property of the type.
  `validate()` refuses a relative program path (spec §4.8) and any NUL byte.
* **Windows**: `creation_flags(CREATE_NO_WINDOW = 0x0800_0000)`, no `DETACHED_PROCESS`; one unnamed
  Job Object per invocation with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so a **crash** of the app kills
  the tree; kill via `TerminateJobObject`.
* **Unix**: `process_group(0)` before the spawn; cancel = `killpg(SIGTERM)`, up to 2 s, then
  `killpg(SIGKILL)`.
* **Both pipes drained concurrently** by two tasks, with a 200-line stderr ring buffer
  (`RingBuffer`) and a byte-level `LineReader` that survives partial reads, CRLF, 1 MiB single lines
  and split multi-byte characters.
* **Cancellation** is `tokio::select! { biased; child.wait(), token.cancelled() }`. No lock in the
  module is ever held across an `.await`: the registry mutex is taken in single statements only.
* **`RunningTool::drop` kills the tree**, so "dropping means stopped" is true on all three OSes (and it
  is what keeps the Windows KILL_ON_JOB_CLOSE promise meaningful).
* **`error.process`** added to `AppError`/`ErrorCode` and to `it.json`/`en.json` (`"Errore di un
  programma esterno"` / `"External program error"`).
* **`lib.rs`**: the previously empty `on_exit()` now calls `process::kill_all_running()`.
* **New CI job `process-tests`** (windows-2022 + macos-latest, on tag or manual dispatch), and the
  `release` job now depends on it. This is the gap M0-A's handoff pointed at: until now `cargo test`
  only ever ran on Linux.
* **`fake_tool`**, a dependency-free Rust fake external tool
  (`src-tauri/tests/bin/fake_tool.rs`): prints to either stream, writes a line of any size, sleeps,
  exits with a chosen code, prints an environment variable, echoes its argv, and spawns a **grandchild**
  that sleeps.

### Verified by running

Every command below was executed on this machine (CachyOS, x86_64):

| command | result |
| :-- | :-- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo test` | **80 passed, 0 failed** (60 unit + 15 `process_tree` + 5 `settings_ipc`) |
| `cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | clean — this is what compiles the Job Object code |
| `cargo check --all-targets --target x86_64-pc-windows-msvc` | clean (tests included) |
| `npm run check` | 0 errors, 0 warnings |
| `npm test` | 28 passed |
| `npm run check:ci` | 8 `run: |` blocks valid, YAML parses to 6 jobs |
| `npx tauri build --bundles deb` | `.deb` **1.94 MiB**, only `usr/bin/scaricatore-di-porto` inside |

* **Sizes measured**: app binary `target/release/scaricatore-di-porto` **4 823 984 B (4.60 MiB)**,
  against the 4.58 MiB recorded in M0-A — about **+20 KiB** for the whole process layer, far under
  the ≤ 15 MB target of spec §15. `Cargo.lock` gained exactly one new package, `tokio-macros`, a
  proc-macro crate that adds nothing at runtime.
* **The four scenarios of the M0-B prompt are really tested**, not asserted in the abstract:
  1. normal run — exit code 0/3, stdout callback, stderr tail;
  2. **40 000 lines on both streams at once** (≈500 KB each, far beyond a pipe buffer) finishes in
     0.36 s. A single-pipe reader would block there for ever; the `timeout` turns that into a failure
     rather than a hung CI. A single 1 MiB stderr line also comes back intact;
  3. cancel — the grandchild's pid is polled until it is gone. It **is** gone, and the cancel returns
     in well under the 2 s of spec §4.2;
  4. killing a finished tool succeeds, and succeeds twice more.
* Ran `cargo test --test process_tree` **8 times in a row**: 15/15 every time, no flake.
* Also tested: drop-kills-the-tree, the app-exit hook killing two tools and being idempotent, a
  finished tool leaving the registry, a non-`http` URL refused before anything is spawned, the private
  temp dir reaching the child, and `fake_tool args` proving on a real command line that the URL really
  lands after `--`.
* macOS API surface checked by compiling a scratch crate that mirrors exactly the `#[cfg(unix)]` lines
  of `process.rs` for `aarch64-apple-darwin` **and** `x86_64-apple-darwin`: clean.

### Not verified / stubbed

* **Nothing Windows has ever been *run*.** `cargo clippy --target x86_64-pc-windows-msvc` proves the
  Job Object code **compiles**; it does not prove `AssignProcessToJobObject` succeeds on a real
  Windows runner, nor that the cancellation kills a grandchild there. Only the new `process-tests` CI
  job can say. Until that job has run once, treat "tree-kill works on Windows" as **unverified**.
* **macOS has not been compiled at all.** Cross-compiling from Linux is impossible here:
  `objc2-exception-helper` needs an Objective-C compiler for the target and `cc` rejects
  `-arch arm64`. Only the libc/tokio symbols were checked (see above).
* `std::fs::rename` over an existing file on Windows (D15) is still untested — but the new
  `process-tests` job runs the whole suite there, so the question gets answered on its first run.
* **`READER_DRAIN_TIMEOUT` (5 s) is defensive code with no test.** It only matters if something
  *escapes* the job/process group and holds the pipe open; building that scenario needs `setsid` on
  Unix and breakaway flags on Windows. Documented in D25/DECISIONS rather than dressed up as tested.
* **Spec §4.10** (orphan cleanup of stale `.sdp-tmp-*` directories at startup) is **deliberately not
  done here**: "only those belonging to no queued/interrupted item" needs the queue, which is M2. What
  *is* in place is the naming rule (`ITEM_TEMP_PREFIX`, `item_temp_dir`) and the private per-app temp
  dir of §4.9 (`temp_env` sets `TMPDIR`/`TEMP`/`TMP`).
* Spec §4.5 mentions passing `--encoding utf-8` to yt-dlp if non-UTF-8 output shows up on Windows.
  Not applicable yet: the output decoding (`from_utf8_lossy`, spec §4.5) is done and tested; the flag
  belongs to the yt-dlp argument builder of M1.

### Spec items marked "verify" that I checked

| item | how | result |
| :-- | :-- | :-- |
| `CREATE_NO_WINDOW = 0x08000000` | Windows SDK value for the flag | correct; `DETACHED_PROCESS` deliberately not used |
| `JobObjectExtendedLimitInformation` = 9, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` = 8192 | `windows` 0.62.2 generated sources | correct |
| `CreateJobObjectW` needs the `Win32_Security` feature (it takes `SECURITY_ATTRIBUTES`) | `#[cfg(feature = "Win32_Security")]` on the binding | true; feature enabled |
| `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` needs `Win32_System_Threading` (it embeds `IO_COUNTERS`) | same | true; feature enabled |
| `PCWSTR::null()` satisfies `P: Param<PCWSTR>` | `impl<T,U> Param<T, CopyType> for U where U: CanInto<T>` + `impl<T> CanInto<T> for T` | compiles on the Windows target |
| `tokio::process::Child` exposes the process handle | tokio 1.53.2 sources: `Child::raw_handle()` (windows-only); it does **not** implement `AsRawHandle` itself | used `raw_handle()` |
| `Child::wait` is cancel-safe, `try_wait` fuses the exit status | tokio 1.53.2 `process/mod.rs` | true; the `select!` relies on it |
| a tokio child that is not reaped keeps its pid reserved | tokio keeps the `StdChild` alive; the global reaper only reaps orphans | true, which is what makes `killpg` safe (D21) |
| `libc::killpg` / `SIGTERM` / `SIGKILL` / `ESRCH` exist on macOS | libc 0.2.190 `unix/mod.rs` (shared) and `bsd/apple/mod.rs` | yes, with the `killpg$UNIX2003` link name on macOS x86_64 |
| `Command::process_group` exists on tokio and is `#[cfg(unix)]` | tokio 1.53.2 `process/mod.rs` | yes, uses `std`'s, which is stable since Rust 1.64 |
| `TerminateJobObject` on a job with no processes | MS documentation | **not documented** → the code queries `ActiveProcesses` first (D22) |

### Decisions/deviations (also in DECISIONS.md)

D19 fake tool is a Cargo *example* (a `[[bin]]` was measured installing `/usr/bin/fake_tool` in the
`.deb`) · D20 the Job Object is attached after `CreateProcess`, with the race that implies · D21 a
reaped pid is never signalled again · D22 `TerminateJobObject` on an empty job · D23 the new
`process-tests` CI job and its quota cost · D24 `tokio/macros` belongs in normal dependencies ·
D25 `TreeHandle` declares `Send`/`Sync` because `windows::HANDLE` does not.

### Next step

**M0-C**: `tools.manifest.json`, `scripts/fetch_bundled_tools.mjs`, bundled yt-dlp + qjs via
`bundle.externalBin`, `services/tools.rs` with the lookup order of spec §5.2, the first-run copy of
yt-dlp into app data, the on-demand downloader with Range resume, `yt-dlp -U`, the Settings → Tools
page and the first-run wizard. `PHASE_PROMPTS.md` "M0-C" has the full list.

Two things that are now easier than they were:

* `spawn_tool` exists, so the tool self-test (`yt-dlp --version`, 5 s timeout) is a few lines and can
  be tested offline against `fake_tool` with `SDP_FAKE_TOOL`;
* `item_temp_dir` exists, so the M2 queue has its per-item temp rule (spec §4.9) already.

### Needs a human

* Push the branch and read the **`process-tests`** result: it is the first real answer about Windows
  and about `rename`-over-destination on Windows.
* Merge `m0-process` into `main` only after that CI run, and only if the Windows job is green.

---

## 2026-10-13 — M0-B bis / Tier 2 isolato in un workflow separato (spec §1.1, D12)

### Done

* **`.github/workflows/tier2.yml`** creato, e il job `tier2` **rimosso** da `ci.yml`.
  `tier2.yml` ha un solo trigger (`workflow_dispatch`) e un solo pulsante. In `ci.yml` la
  condizione che lo richiamava non esiste più: è strutturalmente impossibile che un avvio manuale
  di `CI` trascini i tre target sperimentali.

### Verified by running

* `npm run check:ci` → **8 blocchi `run: |` controllati** (7 in `ci.yml`, 1 in `tier2.yml`),
  entrambi i file ok. Il controllo legge già tutta la cartella, quindi il file nuovo è coperto.
* Parsing YAML di entrambi: `ci.yml` → trigger `push`/`pull_request`/`workflow_dispatch`, job
  `check`, `process-tests`, `bundle`, `bundle-macos-universal`, `release`; `tier2.yml` → trigger
  `workflow_dispatch` **solo**, job `tier2`.

### Not verified / stubbed

* Il workflow `tier2.yml` **non è mai partito** e non può partire finché questo commit non è su
  `main` (`workflow_dispatch` richiede che il file esista sul branch di default).
* I tre target Tier 2 restano, come prima, "compilano e superano i test in CI" e nient'altro:
  nessuno ha girato l'app su quei sistemi.

### Decisions/deviations (also in DECISIONS.md)

D26 — perché `if: github.event_name == 'workflow_dispatch'` non significa "solo quando lo voglio",
ma "a ogni avvio manuale", e cosa è costato (calcolato con i moltiplicatori di D12: ~83
minuti-equivalente Linux per run, ~43% della quota mensile gratuita).

### Next step

Aspettare la run manuale già avviata: quando `check` diventa verde deve comparire
`process tests (windows-x64)` e `process tests (macos-arm64)`. È l'unica verifica ancora mancante
per M0-B.

---

## 2026-10-13 — M0-B bis / la prima run Windows: due test scritti a forma di Linux

### Done

* **Corretti due test che fallivano solo su Windows**, e altri due che passavano per il motivo
  sbagliato (vedi sotto). Il codice di `validate()` non è stato toccato: aveva ragione.
* Aggiunto `sample_tool()` / `absolute_path()` nella sezione test: costruiscono un percorso
  **assoluto sulla piattaforma in esecuzione** (`C:\tools\yt-dlp.exe` su Windows,
  `/usr/tools/yt-dlp` su Unix e macOS) e contengono un `assert!(path.is_absolute())` interno, così
  se un domani l'ipotesi si rompe il test fallisce rumorosamente invece di mentire in silenzio.
* Aggiunto `a_backslash_path_is_relative_on_unix_and_absolute_on_windows`, che fissa per iscritto la
  differenza di piattaforma invece di lasciarla nella memoria di chi scrive i test.

### Causa vera

```rust
let command = ToolCommand::new("/usr/bin/yt-dlp").url("https://example.org/v");
assert!(command.validate().is_ok(), "must accept");
```

Su Windows `Path::new("/usr/bin/yt-dlp").is_absolute()` è **`false`**: niente prefisso di drive, niente
radice UNC. Un percorso che comincia con `/` lì è *relativo*. `validate()` lo rifiutava
correttamente (spec §4.8 vuole percorsi assoluti), il test no.

Il guaio più subdolo: `validate_refuses_a_non_http_url` e `validate_refuses_a_nul_byte_anywhere`
usavano lo stesso percorso, quindi su Windows **fallivano prima sul controllo "assoluto"** e non
arrivavano mai a provare né lo schema URL né il byte NUL. Verde, ma incapaci di scovare la
regressione per cui esistono. Ora l'asserzione controlla anche *perché* è fallita:

```rust
assert!(error.to_string().contains("http"),
        "must fail on the scheme, not on something else: {error}");
```

Lezione per le fasi seguenti: **nei test, mai un percorso hardcoded.** Ogni `PathBuf` in un test
deve passare da un helper che conosce la piattaforma, o il test è Linux-shaped e mente.

### Verified by running

* `cargo fmt --all -- --check` → pulito.
* `cargo clippy --all-targets -- -D warnings` → pulito (Linux).
* `cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings` → pulito. **Compila,
  non esegue:** la correzione è verificata per tipo e per logica, non per comportamento.
* `cargo test` su Linux → **61 unit** (era 60) + 15 `process_tree` + 5 `settings_ipc`, tutti verdi.

### Not verified / stubbed

* **I 15 test di `process_tree` non sono mai arrivati a girare su Windows.** `cargo test` si ferma
  dopo il fallimento della lib, quindi i test di integrazione — che sono quelli che provano davvero
  Job Object, `CREATE_NO_WINDOW` e il tree-kill — **non sono partiti**. Vanno rilanciati.
* **`process tests (macos-arm64)`: esito non ancora comunicato.** Se è verde, è la prima verifica
  reale che `process_group` + `killpg` funzionino su Darwin. Se è saltato, il buco resta aperto.
* **Domanda di D15 ancora aperta:** `settings_survive_a_restart` (cioè
  `std::fs::rename` che sostituisce il file su Windows) sta in `settings_ipc`, che per lo stesso
  motivo non è mai partito su Windows.

### Next step

Rilanciare `process-tests` e arrivare ai test di integrazione su Windows. Se `AssignProcessToJobObject`
fallisce sul runner, il piano B è documentato in D20/D21 (`taskkill /T /F` come fallback quando il
Job Object non è assegnabile, per esempio sotto un runner che è già dentro un job).
