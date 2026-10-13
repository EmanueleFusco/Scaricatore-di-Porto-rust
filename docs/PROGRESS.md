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

---

## 2026-10-13 — M0-B ter / il Job Object è VERIFICATO su Windows (spec §4.2)

### La buona notizia, che stava dentro il fallimento

La prima run che ha **eseguito** i test su Windows (non solo compilato) ha dato:

* **60 test unitari: verdi** (61 su Linux: manca `create_private_dir_uses_mode_0700`, che è
  `#[cfg(unix)]`, e va bene così).
* **`process_tree`: 14 su 15 verdi.** In particolare:

  | test | esito | cosa dimostra |
  | :-- | :-- | :-- |
  | **`cancelling_kills_the_child_and_the_grandchild`** | **ok** | **il Job Object uccide l'albero su Windows.** Il pid del nipote non esiste più dopo la cancellazione. È la verifica più importante di M0-B, ed è reale. |
  | `dropping_a_running_tool_kills_the_tree` | ok | il `Drop` chiama `TerminateJobObject` e la `KILL_ON_JOB_CLOSE` tiene |
  | `a_large_stderr_output_does_not_deadlock` | ok | 40 000 righe su entrambi i flussi, nessun deadlock |
  | `a_single_huge_stderr_line_survives_intact` | ok | riga da 1 MiB intera |
  | `the_app_exit_hook_kills_every_running_tool` | **FAILED** | vedi sotto |
  | gli altri 10 | ok | `CREATE_NO_WINDOW`, argv, URL dopo `--`, temp dir privata, URL non-http rifiutato, kill idempotente, token già cancellato, registro |

Quindi, per la prima volta: **`CREATE_NO_WINDOW`, il Job Object, `TerminateJobObject` e il
tree-kill sono verificati in esecuzione su Windows**, non solo in compilazione.

### Il fallimento: di nuovo un test a forma di Linux

```rust
assert_eq!(first_result.exit_code, None, "killed by a signal");
// left: Some(1)  right: None
```

Il codice ha ragione di nuovo, ed è la **seconda volta** con la stessa classe di errore.

| | come muore il processo | `ExitStatus::code()` |
| :-- | :-- | :-- |
| Unix | `killpg(SIGKILL)` → morto per segnale | `None` |
| Windows | `TerminateJobObject(job, TREE_TERMINATE_EXIT_CODE)` | **`Some(1)`** |

Il secondo parametro di `TerminateJobObject` è documentato da Microsoft come *«The exit code to be
used by all processes and threads in the job object»*: cioè **assegna** il codice, non lo lascia
indeterminato. Il valore 1 è il nostro `TREE_TERMINATE_EXIT_CODE`, scelto in `process.rs`.

Corretto il test con l'aspettativa giusta per piattaforma e il perché per iscritto. E soprattutto
documentato sul campo `ProcessResult::exit_code`, che è la trappola per chi verrà:

> **Non leggere questo come «è stato ucciso?».** I segnali portabili sono `cancelled` e `success()`,
> e il classificatore di errori della spec §8.5 lavora sul testo di stderr, non su questo numero.

### Altro

* **`cargo test --no-fail-fast`** aggiunto ai job `check` e `process-tests`. Senza, il primo test
  fallito nasconde tutti gli altri: il fallimento nella lib del giro precedente ha impedito di vedere
  che `settings_ipc` non era mai partito, e si è perso un giro di CI intero per scoprirlo. Costa zero
  minuti di quota.

### Verified by running

* `cargo fmt --all -- --check` → pulito.
* `cargo clippy --all-targets -- -D warnings` → pulito (Linux) e su `x86_64-pc-windows-msvc`.
* `cargo test` su Linux → **61 + 15 + 5**, verdi.
* `npm run check:ci` → 8 blocchi `run: |` validi.

### Not verified / stubbed

* **La correzione di questo blocco non è ancora passata su Windows**: è verificata per tipo e per
  logica su entrambe le piattaforme, ma l'esito su runner arriva dalla prossima run.
* **`settings_ipc` (5 test) non è ancora partito su Windows**, quindi la domanda di **D15** —
  `std::fs::rename` che sostituisce il file su Windows — è **ancora aperta**. Con
  `--no-fail-fast` la prossima run la risponderà anche se qualcos'altro fallisce.
* **macOS: esito ancora non comunicato.** Se `process tests (macos-arm64)` è verde, insieme a
  questa run si chiude la verifica del livello processi su tutte e tre le piattaforme Tier 1.

### Next step

Rilanciare `process-tests` e chiudere: D15 (`rename` su Windows) ed esito macOS. Poi M0-C.

---

## 2026-10-13 — M0-B quater / `settings_ipc` non partiva su Windows (bug noto di Tauri)

### Cosa è successo

Con `--no-fail-fast` i 15 test di integrazione sono arrivati a girare su Windows: **15/15 verdi**.
Poi è emerso un problema diverso, che prima non si vedeva:

```
Running tests\settings_ipc.rs
Caused by: process didn't exit successfully: `settings_ipc-....exe`
  (exit code: 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND)
```

**Non è un test che fallisce: il processo non parte**, prima di `main()`. E solo `settings_ipc` muore,
mentre `process_tree` sulla stessa macchina passa 15/15 — ed è la differenza che spiega tutto.

### Causa, verificata nelle fonti

`tauri-build` passa il manifest al binario applicativo con `embed-resource`, che emette
`cargo:rustc-link-arg-bins` (`embed-resource 3.0.12`, `src/lib.rs:443`). **`-bins`, non `-tests`**:
i binari prodotti da `cargo test` non lo ricevono. Senza quel manifest un eseguibile che importa i
simboli di `comctl32` v6 carica la `comctl32` legacy (v5), che non li esporta.

- `settings_ipc.rs` usa `tauri::test::mock_builder()` → importa la catena finestra/webview → comctl32 v6
- `process_tree.rs` non tocca Tauri → non importa quei simboli → parte senza problemi

Bug noto di Tauri, tre riferimenti concordi: issue **#13419**, issue **#14580** (ancora aperta),
discussione **#11179**.

### Done

* **`src-tauri/windows-app-manifest.xml`** creato, identico a `tauri-build/src/windows-app-manifest.xml`.
* **`build.rs`**: `cargo:rustc-link-arg-tests=/MANIFEST:EMBED` + `/MANIFESTINPUT:<path>`, emessi solo
  quando `CARGO_CFG_TARGET_OS=windows` **e** `CARGO_CFG_TARGET_ENV=msvc`.

### Verified by running

* XML del manifest valido, `assemblyIdentity` = `Microsoft.Windows.Common-Controls`.
* **I flag escono davvero**: cross-compilando per Windows **da Linux**
  (`cargo build --target x86_64-pc-windows-msvc -vv`) il build script emette entrambi; compilando per
  Linux **non ne emette nessuno** (0 occorrenze). È la prova che `CARGO_CFG_TARGET_*` è la strada
  giusta: un `#[cfg(windows)]` qui non avrebbe emesso nulla, e
  `cargo clippy --target x86_64-pc-windows-msvc` — la verifica che uso per controllare il codice
  Windows — sarebbe stata cieca su quel pezzo.
* `cargo fmt --all -- --check` → pulito.
* `cargo clippy --all-targets -- -D warnings` → pulito su Linux **e** su `x86_64-pc-windows-msvc`.
* `cargo test` su Linux → **61 + 15 + 5**, verdi.

### Not verified / stubbed

* **L'esito su runner Windows non è ancora arrivato.** La correzione è verificata per costruzione
  (i flag emessi dove devono) e per tipo, ma non in esecuzione: è la prossima run a dirlo.
* **D15 resta aperta**: `std::fs::rename` che sostituisce il file su Windows è proprio dentro
  `atomic_write_replaces_existing_content_and_leaves_no_temp_file`, uno dei test di `paths.rs` che
  risultano verdi nell'elenco. Quel test **è passato** su Windows: `rename` sostituisce la
  destinazione. La domanda è risolta, e l'asserzione sul `.bak` copre lo stesso percorso.
* macOS: esito di `process tests (macos-arm64)` ancora non comunicato.

### Next step

Rilanciare `process-tests`. Con la correzione attesa: Windows 61 + 15 + 5 verdi, e D15 chiusa.
Poi **M0-C**.

---

## 2026-10-13 — M0-B CHIUSO / il livello processi è verificato sulle tre piattaforme Tier 1

Il proprietario ha confermato che, nelle run di `process-tests`, **macOS (`macos-arm64`) e Linux
(`check`, `ubuntu-latest`) sono verdi** e che il fallimento era e resta solo Windows.

### Cosa significa, in concreto

Il livello processi di spec §4 non è più "compila su tre sistemi": è **stato eseguito** su tutti e
tre i target Tier 1 della spec §1.1.

| piattaforma | cosa ha verificato | come |
| :-- | :-- | :-- |
| **Linux x64** | `process_group(0)` + `killpg(SIGTERM)` → `killpg(SIGKILL)`, `killpg` di un gruppo vuoto = `ESRCH`, `forget_group` dopo il reaping, `SIGTERM` come default con `SIGKILL` dopo 2 s | 61 unit + 15 integrazione + 5 IPC, più 8 run consecutive di `process_tree` senza flake |
| **macOS arm64** | **stessi percorsi Unix su Darwin**: `killpg` con `link_name = killpg$UNIX2003` su x86_64, `SIGKILL` a `ExitStatus::code() == None`, `process_group` di `std` | run `process tests (macos-arm64)` verde |
| **Windows x64** | `CREATE_NO_WINDOW`, Job Object con `KILL_ON_JOB_CLOSE`, `AssignProcessToJobObject`, `TerminateJobObject`, `QueryInformationJobObject`, `CloseHandle` al drop, `unsafe impl Send/Sync` su `HANDLE` | 60 unit + 15 integrazione verdi (15/15), più il manifest Common Controls v6 |

Il caso che la spec §4.2 considera il piu` rischioso — **la cancellazione deve uccidere l'albero, non
solo il figlio** — e' verificato in esecuzione su Windows (Job Object) e su Linux/macOS (process
group). `child.kill()` da solo avrebbe lasciato vivo il nipote: il test lo controlla davvero,
leggendo il pid del grandchild e aspettando che sparisca.

### D15 chiusa

`atomic_write_replaces_existing_content_and_leaves_no_temp_file` e' **verde su Windows**. Quindi
`std::fs::rename` sostituisce la destinazione anche li, come suggerivano i documenti di `std`
(`MoveFileExW` + `MOVEFILE_REPLACE_EXISTING`): non piu' un'affermazione tratta dai docs, ma un fatto
misurato su runner.

### Precisazione onesta sulla data

Il verde di macOS e' stato misurato **prima** degli ultimi tre commit:

| commit | contenuto | effetto su macOS |
| :-- | :-- | :-- |
| `d1901bc` | aspettativa dell'exit code per piattaforma + `--no-fail-fast` + doc | nessuno: su Darwin l'albero termina per segnale, quindi `code()` e' `None`, che e' cio` che il test afferma |
| `79014ee` | manifest Common Controls v6 in `build.rs` | nessuno: condizionato a `CARGO_CFG_TARGET_OS == "windows"` |
| `fcd0d9e` | solo documentazione | nessuno |

Quindi il codice su Darwin non e' cambiato, ma **la conferma formale arriva dalla prossima run**, che
serve comunque per validare la correzione del manifest su Windows. Una sola run chiude entrambe le
coste.

### Stato della fase

M0-B e' finita:

* `services/process.rs` completo e verificato sulle tre piattaforme Tier 1;
* `fake_tool` come esempio Cargo (fuori dall'installer, D19);
* `process-tests` in CI, `tier2` isolato nel suo workflow (D23, D26);
* tre difetti trovati e corretti grazie alle run reali: due test a forma di Linux (D27 e la
  correzione dei percorsi) e il manifest mancante sui binari di test Windows (D28).

**Da verificare a mano dall'utente** (impossibile da qui): che l'app si apra e che un download reale
si cancelli correttamente. M0-C mette yt-dlp e qjs nel bundle, ed e' l la prima occasione per
provare il pulsante di cancellazione contro un processo vero.

### Next step

**M0-C**, se il verde su Windows è confermato.

---

## 2026-10-13 — M0-B / conferma finale: 80/80 verdi su Windows

### Done

Nessuna modifica al codice: questa è la **verifica** delle correzioni delle run precedenti.

### Verified by running (output integrale del job `process tests (windows-x64)` fornito dal proprietario)

| target | esito |
| :-- | :-- |
| `src/lib.rs` | **60 passed, 0 failed** |
| `tests/process_tree.rs` | **15 passed, 0 failed** |
| `tests/settings_ipc.rs` | **5 passed, 0 failed** |
| doc-tests | 0 |
| **totale** | **80 / 80** |

### La prova che il manifest era davvero la causa

I nomi dei binari di test sono **identici** fra la run in cui `settings_ipc` non partiva e questa:

```
prima (non partiva)                 adesso (5/5 ok)
settings_ipc-5ddef0135047f4f5.exe   settings_ipc-5ddef0135047f4f5.exe
process_tree-96a841a92988742c.exe   process_tree-96a841a92988742c.exe
```

**Stessi byte, esito opposto.** Fra le due run l'unica modifica al codice è `build.rs`, che non
cambia il codice generato ma aggiunge due flag al linker. Non è quindi un caso fortunato: è il
manifest Common Controls v6 a cambiare la possibilità di caricare l'eseguibile, isolato.

### D15 chiusa, e su due strade indipendenti

* `paths.rs` → `atomic_write_replaces_existing_content_and_leaves_no_temp_file ... ok`: `rename`
  sostituisce la destinazione su Windows, come suggerivano i documenti di `std`
  (`MoveFileExW` + `MOVEFILE_REPLACE_EXISTING`);
* `settings_ipc` → `settings_survive_a_restart ... ok`: stesso percorso, attraverso l'IPC.

### Altre due cose controllate, per onestà

* **60 test su Windows contro 61 su Linux**: `create_private_dir_uses_mode_0700` è
  `#[cfg(unix)]` (modalità 0700 è un concetto Unix). Atteso, non una mancanza.
* **`an_unwritable_settings_file_reports_a_localised_error` passa per il motivo giusto?** Sì,
  verificato: non usa giochi di permessi, mette un *file* al posto della cartella di configurazione,
  quindi la scrittura fallisce su ogni sistema. Era già pensato per non essere Linux-shaped, a
  differenza dei due test di `process.rs` corretti prima. Nessun terzo caso della stessa classe.

### Not verified / stubbed

* **`process_tree` impiega 4,63 s su Windows contro 0,35 s su Linux.** Non è un problema (è
  lontano da qualsiasi timeout) e dipende da costo di `CreateProcess` + `TerminateJobObject` e dalla
  grana di 25 ms con cui i test interrogano lo stato dei processi. Annotato, non investigato.
* **`CREATE_NO_WINDOW` resta verificato solo per compilazione**: nessun test headless può accorgersi
  di una finestra di console che non si apre. Serve guardare l'app a mano su Windows.
* Nessun processo reale coinvolto: il falso tool non è `yt-dlp`. La prova che
  `yt-dlp → ffmpeg → runtime JS` muoiano tutti insieme arriva da M1.

### Next step

**M0-C.** Le condizioni di uscita di M0-B sono soddisfatte e verificate.

---

## 2026-10-13 — M0-B / audit delle mie stesse note, e un "verify" che avevo saltato

Il proprietario ha chiesto se `docs/PROGRESS.md` fosse stato aggiornato secondo `AGENTS.md`. Controllato
invece di dare per scontato, e **non era a posto**. Questo blocco registra cosa mancava e cosa è stato
corretto, invece di riscrivere di nascosto i blocchi precedenti.

### Le due lacune, e perché sono un problema

`AGENTS.md` §9 fissa sei intestazioni obbligatorie per ogni blocco. Confrontando i sette blocchi di
M0-B:

| intestazione | blocco 1 | blocchi 2-7 |
| :-- | :-- | :-- |
| Done · Verified by running · Not verified / stubbed · Decisions · Next step | ✅ | ✅ |
| **Spec items marked "verify" that I checked** | ✅ | ❌ **assente in 6 blocchi** |
| **Needs a human** (§10.5) | ✅ | ❌ **assente in 6 blocchi** |

Non è una questione di forma. La prima intestazione serve a distinguere **"non c'era niente da
verificare"** da **"non ho verificato"**: senza la riga, chi legge non può sapere quale dei due sia
successo. La seconda serve a non perdere gli impegni che spettano a una persona.

Nei blocchi 3, 4, 5 e 7 (che riguardano `spec §4`) la risposta è onesta ma andava scritta: **`spec §4`
non contiene alcun item marcato "verify"**. La convenzione della spec (riga 7) definisce "verify" come
*«the exact flag / asset name / URL / version may have changed upstream»*, cioè nomi di flag, asset,
URL e versioni: sono tutte cose di `spec §5`/`§8`, non di `§4`. Avevo controllato, ma non l'avevo
scritto.

### Il "verify" che avevo davvero saltato

`spec §1.1`, riga 42: *«Check which runners are currently available (**verify**: macOS Intel, Windows
ARM64, Linux arm64 hosted runners)»*.

Quando ho spostato `tier2` in `tier2.yml` ho **copiato le tre label senza riverificarle**. Non era un
rischio teorico: D15 aveva già cambiato un label (`windows-latest` → `windows-2022`) proprio perché il
primo si era mosso.

**Ora verificato** contro `actions/runner-images`, README ufficiale:

| label | stato |
| :-- | :-- |
| `macos-15-intel` | esiste — macOS 15, x64 |
| `windows-11-arm` | esiste — Windows 11 Arm64, arm64 |
| `ubuntu-24.04-arm` | esiste — Ubuntu 24.04 Arm64, arm64 |

Ricontrollate anche le altre label usate nel workflow: `windows-2022` esiste; `windows-latest` è
Windows Server 2025 con VS 2026 (conferma D15); `macos-latest` è macOS 26 **arm64** (conferma D15);
`ubuntu-latest` è 24.04 (conferma D14); `ubuntu-22.04` esiste. Nota non usata ma rilevante:
`macos-14` e `macos-14-arm64` sono **deprecati** e non vanno scelti.

### Cosa invece NON ho scritto, e va bene così

In chat avevo ipotizzato che il calo dell'AppImage da 108 MiB (M0-A) a **76,9 MiB** (misurato nella run
#12) fosse dovuto a una versione diversa di squashfs/appimagetool. **Quella ipotesi non è stata messa
nei documenti**, e non lo sarà: è una congettura, non una misura, e i due numeri sono reali ma la
causa no. Se serve, si indaga con `mksquashfs -version` sui due runner.

### Needs a human — la voce che mancava

Riepilogata qui, perché le condizioni di uscita di M0-B sono soddisfatte ma tre cose restano a chi
scrive il codice e non a chi lo fa girare:

1. **aprire l'app su Windows** e confermare che non compare nessuna finestra di console: `CREATE_NO_WINDOW`
   è verificato solo per compilazione, nessun test headless può accorgersene;
2. **scaricare l'installer Windows** (`bundle (windows-x64)` → artifact `windows-x64` → l'`.exe`) e
   provarlo: nessuno ha ancora installato un binario prodotto dalla CI;
3. quando si farà M0-C, **provare la cancellazione contro un `yt-dlp` vero**: è la prima occasione in
   cui l'albero è `yt-dlp → ffmpeg → runtime JavaScript` e non un finto programma Rust.

### Next step

**M0-C**, senza altri prerequisiti da questa parte.

---

## 2026-10-13 — M0-C (manifest, tool inclusi, provisioning, pagina Strumenti, CI)

### Done

* **`tools.manifest.json`** (spec §5.1): `yt-dlp` in modo *always latest* con checksum
  dal `SHA2-256SUMS` della release, `qjs` (QuickJS-NG) con tag e sha256 fissati.
  Solo questi due sono inclusi (spec §0.7). `deno` **non** c'è: D34.
* **`src-tauri/src/services/manifest.rs`**: tipi serde + validazione, 22 test. Il
  manifesto è compilato nel binario (D31).
* **`scripts/fetch_bundled_tools.mjs`**: scarica, verifica, rinomina, colloca in
  `src-tauri/binaries/<nome>-<triple>[.exe]`. Risolve il tag via API GitHub
  perché l'alias `latest` è risultato inaffidabile (D33). `--check` valida il
  manifesto offline, `--verify-pins` riverifica gli sha256.
* **`bundle.externalBin`** con `yt-dlp` e `qjs`; a runtime si cercano accanto
  all'eseguibile (verificato in `tauri-bundler`).
* **`services/tools.rs`**: le cinque fonti della lookup di spec §5.2 come lista
  pura e ordinata; ogni candidata è validata **eseguendola** con timeout di 5 s.
  Copia in app data al primo avvio; `-U` solo sulla copia nostra (D43).
* **`services/download.rs`**: HTTPS, resume con `Range`, sha256, `chmod +x`,
  rename atomico, zip/tar/tar.gz/tar.xz con un solo guard di path traversal.
* **`commands/tools.rs`**: 9 comandi sottili + copia al primo avvio in `setup`.
* **UI**: pagina Strumenti (tabella, azioni per riga, path e banner), procedura
  iniziale con i passaggi 1, 2 e 4 di spec §11.1 (il 3 è un segnaposto
  saltabile per M5), Impostazioni a schede, dizionari IT/EN completi.
* **CI**: `npm run check:tools` nel job `check` (offline), fetch dei tool + test
  sui tool **reali** + `ls` nel job `bundle` e in quello universale.
* **`docs/SPIKES.md`** riscritto: sei prove con il metodo, cosa può andare storto
  e i campi da riempire.

### Verified by running

Tutti i comandi di `AGENTS.md` §4, più quelli nuovi:

| controllo | esito |
| :-- | :-- |
| `cargo fmt --all -- --check` | pulito |
| `cargo clippy --all-targets -- -D warnings` | pulito |
| `cargo test` | **179 verdi** (133 lib + 3 bundled_tools + 9 download + 15 process_tree + 6 settings_ipc + 13 tools) |
| `npm run check` | 0 errori, 0 warning |
| `npm test` | **72 verdi** in 5 file |
| `npm run build` | ok, 108,71 kB JS |
| `npm run check:ci` | 8 blocchi `run: \|` validi |
| `npm run check:tools` | manifesto valido, 2 tool inclusi |
| `node scripts/fetch_bundled_tools.mjs` | yt-dlp **2026.08.19**, qjs **0.17.0**, checksum verificati |

**Il criterio di uscita di M0, provato per davvero** (spec §14: "yt-dlp + qjs
work offline right after install"):

* `cargo test --test bundled_tools` esegue i **binari veri** attraverso la lookup
  dell'app, senza rete:
  ```
  yt-dlp: 2026.08.19 (from …/src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu)
  qjs: 0.17.0 (from …/src-tauri/binaries/qjs-x86_64-unknown-linux-gnu)
  ```
  e per qjs anche uno script vero (`console.log('sdp-qjs-ok')`), perché è qjs
  che yt-dlp esegue per la sfida EJS: un interprete che risponde solo a
  `--version` passerebbe ogni altro test e fallirebbe sul primo link.
* `npx tauri build` completo. Contenuto del `.deb` **verificato**, non supposto:
  ```
  /usr/bin/scaricatore-di-porto   5 039 440 byte
  /usr/bin/yt-dlp               40 446 224 byte
  /usr/bin/qjs                   2 562 504 byte
  ```
* L'app avviata davvero (build di debug, `XDG_*` su una cartella di prova):
  ```
  scaricatore-di-porto: yt-dlp installed in the app data dir at
    …/com.scarikatorediporto.app/tools/yt-dlp/yt-dlp
  ```
  e la **copia in app data risponde**: `yt-dlp --version` → `2026.08.19`.
  Viene creata anche la cartella temporanea privata (spec §4.9).
* Dimensioni misurate (`report-bundle-sizes.mjs`): binario applicativo **4,81 MiB**
  (obiettivo spec §1 ≤ 15 MB ✅), `.deb` **41,36 MiB**, `.rpm` **41,36 MiB**,
  AppImage **147,27 MiB** (escluso dal budget).

### Not verified / stubbed

* **Solo Linux x64.** Tutto il resto — Windows, macOS arm64 e universale, Tier 2 —
  aspetta la CI. In particolare `lipo` per il `qjs` universale è eseguito solo
  su runner macOS e qui non è mai girato.
* **`yt-dlp -U` non è mai stato eseguito contro il vero yt-dlp**: l'aggiornatore
  è provato solo con il falso tool multi-call. Il `-U` vero riscrive il proprio
  file mentre è in esecuzione, e questo è esattamente il caso che tocca la copia
  in app data: va provato a mano.
* **Nessun download è mai arrivato dalla rete dall'app**, perché nel manifesto
  non c'è ancora nessuna entry installabile (D34, D41). Il downloader è provato
  contro un server in loopback che serve corpo, resume, checksum sbagliato, 404 e
  connessione interrotta.
* **La pagina Strumenti e la procedura non sono state guardate con gli occhi.**
  `svelte-check` e `npm run build` passano, l'app parte, ma nessuno ha guardato
  la tabella con un tool mancante.
* **`quarantine_repair` su macOS non è eseguibile qui.** È best-effort, solo
  macOS, solo per i file in app data, e dopo una sonda già fallita.
* La catena `download → extract → install` non è stata esercitata end-to-end con
  un archivio reale: i test costruiscono archivi sintetici per non avere una
  dipendenza di rete in una suite che deve essere offline.

### Spec items marked "verify" that I checked

* **Nomi degli asset GitHub** → presi dalle API GitHub, non dalla memoria
  (D32). yt-dlp `2026.08.19`: `yt-dlp_linux`, `yt-dlp_linux_aarch64`,
  `yt-dlp.exe`, `yt-dlp_macos`. QuickJS-NG `v0.17.0`: `qjs-linux-x86_64`,
  `qjs-linux-aarch64`, `qjs-windows-x86_64.exe`, `qjs-darwin-arm64`,
  `qjs-darwin-x86_64`. **Il nome è cambiato rispetto alle versioni precedenti
  della spec**: non esiste più `qjs-darwin` generico.
* **checksum qjs** → calcolati al primo download e messi nel manifesto, perché
  QuickJS-NG **non pubblica** checksum.
* **licenze** → yt-dlp **Unlicense**, QuickJS-NG **MIT**, lette dai file LICENSE.
* **`externalBin`** → letto in `tauri-utils/src/config.rs` (suffisso
  `<nome>-<triple>[.exe]`, percorsi relativi a `tauri.conf.json`) e in
  `tauri-bundler` (`copy_binaries` toglie `-<target>`; destinazione
  `Contents/MacOS`, `/usr/bin`, `$INSTDIR`, squashfs dell'AppImage).
* **flag di yt-dlp** → `-U, --update` e `--update-to [CHANNEL]@[TAG]` letti nel
  `--help` del binario scaricato.
* **flag di versione** → `ffmpeg -version` (non `--version`), verificato su un
  `ffmpeg` vero; banner reale `ffmpeg version n9.0.2 …` (D36).
* **banner reali** → `2026.08.19` (yt-dlp), `0.17.0` (qjs),
  `ffmpeg version n9.0.2 …` (ffmpeg), tutti riprodotti come stringhe di test.
* **feature di reqwest** → `rustls-tls` esiste solo sulla linea 0.12, e in 0.13
  usa **ring**, non aws-lc-rs (D35).
* **`minVersion` di yt-dlp** → **non** verificato al livello della singola
  release: si è potuto verificare solo che la release `2025.11.12` annuncia
  «an external JavaScript runtime is now required». `TODO(verify)` in M1 (D42).
* **label dei runner** → quelle della M0-B sono state riverificate e non sono
  cambiate; nessuna nuova etichetta è stata introdotta qui.

### Bugs trovati e corretti dai test (otto)

Ognuno è stato verificato **revertendo la correzione** e rilanciando il test che
la copre: senza il fix tutti e otto diventano rossi, per la ragione giusta.
Non è una verifica banale — un test che copre un fix ma resta verde anche senza
di esso non copre niente.

1. Un aggiornamento **fallito** veniva riportato come riuscito quando il testo non
   conteneva "up to date" (D44).
2. Il retry interno del downloader riusava l'offset **di prima del loop**, quindi
   ripeteva la stessa richiesta invece di riprendere.
3. `Transport::loopback_http()` accettava `http://127.0.0.1.evil.com` e
   `http://127.0.0.1@evil.com` (D37).
4. Il guard di estrazione non rifiutava la barra rovesciata su Linux, dove è un
   nome innocuo ma sulla stessa app su Windows è un traversal (D38).
5. In un build di sviluppo la lookup **non trovava** i tool inclusi, perché
   `tauri dev` non copia nulla e il file conserva il suffisso `-<triple>` (D39).
6. Il parser delle versioni leggeva "versione sconosciuta" per ogni `ffmpeg` 7 o
   successivo, per via del prefisso `n` (D36).
7. **`ETXTBSY`**: Linux rispondeva "yt-dlp non c'è" perché la copia appena
   scritta era ancora "busy" (D47). È l'unico dei sette che avrebbe colpito
   l'utente finale con una diagnosi sbagliata.
8. **`scripts/fetch_bundled_tools.mjs`** produceva `qjs.exe-x86_64-pc-windows-msvc.exe`:
   `destinationPath` riceveva `executableName` (che su Windows è già `qjs.exe`)
   invece del nome del tool, e il bundler non avrebbe trovato nulla. Trovato dal
   test JS `uses the tool name, not the asset name`. Da solo avrebbe fatto fallire
   **solo** la build Windows, con un errore (`externalBin`) che non nomina il
   file: il tipo di difetto che si scopre al primo utente Windows.

I primi sette sono nel backend, l'ottavo è nello script di build: nessuno dei due
era visibile a `clippy` né a `cargo build`, e nessuno sarebbe emerso senza
eseguire i test.

A questi si aggiunge il caso che la validazione del manifesto ha **rifiutato la
mia stessa prima stesura** (deno dichiarato "pinned" senza versione), che è il
miglior argomento a favore di avere la regola scritta prima dei dati.

**Il #7 ha una prova debole rispetto agli altri.** Il test che copre il ritento
verifica l'helper (`errno == 26`), non il comportamento sotto carico: `ETXTBSY`
non è riproducibile a comando. La prova vera è stata empirica e statistica —
prima del fix la suite falliva 1 volta su 3, dopo 14 run su 14 verdi. Va tenuto a
mente se un giorno quel test torna rosso: il sospetto è il filesystem, non la
logica.

### Needs a human — AGENTS.md §10.5

1. **`docs/SPIKES.md` S1**: firmare e notarizzare su macOS **con dentro**
   `yt-dlp_macos` e `qjs`. Se non passa, la spec ha già deciso il fallback
   (scaricare yt-dlp al primo avvio su macOS): va applicato, non deciso.
2. **S2**: l'installer firmato su una macchina Windows pulita con Defender
   acceso. Nessuno ha ancora installato un binario prodotto dalla CI.
3. **S3**: il costo di avvio di `yt-dlp` one-file contro la variante spacchettata.
   Il metodo è pronto; il numero no.
4. **S4**: il budget dell'installer. I numeri ci sono (41,4 MiB), la soglia no.
5. **S5**: cancellare un download vero e guardare l'elenco dei processi. È la
   prima occasione in cui l'albero è `yt-dlp → ffmpeg → runtime JavaScript`.
6. **Guardare la pagina Strumenti** con l'app aperta: una riga "non trovato", una
   riga "troppo vecchio", il pulsante "Usa il mio percorso", la procedura
   iniziale.
7. **Provare `-U` sul vero yt-dlp**, che riscrive il proprio file mentre gira.

### Next step

**M1**, con il precondition che questa fase si chiuda con la CI verde sulle tre
piattaforme. Il primopezzo di M1 che tocca M0-C è `services/jsruntime.rs`: i
flag `--no-js-runtimes --js-runtimes quickjs:<path>` vanno verificati con
`--help` sul binario fissato, e lì si chiude anche il `TODO(verify)` su
`minVersion` (D42).

## 2026-10-13 — M0-C / Cinque test che sapevano una sola risposta

La prima run di `m0-tools` sulla CI: `check` verde dopo D48, ma rossi
`process tests (windows-x64)` e `bundle (windows-x64)`. Il log: 5 test falliti,
tutti sulla gestione dei path. **Non un bug di produzione: i test erano scritti
solo per l'host su cui li avevo scritti.** Dettagli e prove in D49.

- **Done**
  - `well_known_dirs` separata in `well_known_dirs_for(home, windows,
    user_profile, local_app_data)`: entrambi i rami sono ora eseguibili da una
    sola macchina, come già faceva `override_candidate`.
  - Il ramo Windows composto come testo: `Path::join` prende il separatore
    dell'host e restituiva `C:\Users\tester/.deno\bin`.
  - Test riscritti perché chiedano di *questa* macchina: nuovi helper `host_dir`
    e `host_exe` in `services::tools`, path nativi anche in `state.rs` e in
    `tests/settings_ipc.rs`.
  - Aggiunte le asserzioni che prima non esistevano: la lista Unix non deve
    contenere `WinGet`, quella Windows deve essere esattamente le due directory
    nell'ordine di spec §5.2, e con un ambiente vuoto la lista Windows è vuota.
- **Verified by running**
  - `cargo fmt --all -- --check` → OK; `cargo clippy --all-targets -- -D warnings`
    → OK; `cargo test --no-fail-fast` → **179 verdi** (133 lib + 3 bundled_tools
    + 9 download + 15 process_tree + 6 settings_ipc + 13 tools).
  - `npm run check` → 0 errori, 0 warning; `npm test` → **77 verdi**;
    `npm run check:tools` → manifest OK.
  - **Revert-check**: invertendo l'ordine delle due directory Windows il test
    va rosso con `left: […WinGet\Links, ….deno\bin]` / `right: […\.deno\bin,
    …WinGet\Links]`; ripristinato torna verde. Il test custodisce davvero.
  - Risposte di `std` verificate dalla sorgente (Rust 1.95.0,
    `library/std/src/sys/path/windows.rs`): `is_absolute = has_root &&
    prefix().is_some()`, quindi `C:\opt\bin1` assoluta, `/opt/mine/qjs` **no**;
    `extension()` usa `rsplit_file_at_dot`, quindi `qjs.exe` → `Some("exe")` e
    l'override non viene ricomposto. Non dalla memoria.
- **Not verified / stubbed**
  - **I cinque test non sono stati eseguiti su Windows.** Il giudice è il runner
    Windows della CI. Le ragioni sono verificate dalla sorgente di std, ma
    "verificato" e "sarà verde" restano due cose diverse.
  - `cargo check --all-targets --target x86_64-pc-windows-msvc` non gira qui:
    si ferma su `ring` (`failed to find tool "lib.exe"`). Nessun `mingw-w64`
    disponibile e wine non è utilizzabile senza un linker. Quindi nemmeno la
    compilazione dei test per Windows è stata controllata da questa macchina.
  - Ancora non diagnosticato: `bundle (macos-universal)`. Ipotesi aperta, nessun
    log.
- **Spec items marked "verify" che I checked**
  - `Path::is_absolute` su Windows → verificato dalla sorgente std (citata sopra).
  - Nient'altro in questa sessione.
- **Decisions/deviations**
  - D49: i test non sono stati indeboliti, sono stati riscritti; e la funzione
    è stata resa dipendente dai soli suoi argomenti perché altrimenti il ramo
    Windows verrebbe asserito solo dal runner che già falliva.
  - D48 (sessione precedente) restava il vero autore del fallimento di `check`.
- **Next step**
  1. Committare e rilanciare il workflow: `process tests (windows-x64)` deve
     andare verde, e con lui `bundle (windows-x64)` se la causa era una sola.
  2. Il log di `bundle (macos-universal)`, che è un'altra storia e non l'ho
     ancora guardato. Ipotesi: il passo `lipo -info` aggiunto in M0-C, mai
     eseguito.
  3. **Attenzione alla disciplina**: se dopo questa run che ha prodotto gli
     artifact si aggiunge un commit, gli artifact non corrispondono più a
     `main`. Rilanciare il workflow prima della merge.
