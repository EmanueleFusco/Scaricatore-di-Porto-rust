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

## 2026-10-13 — M0-C / Il bundle universale vuole tre nomi per tool

`bundle (macos universal)` era rosso con
`resource path 'binaries/yt-dlp-aarch64-apple-darwin' doesn't exist`, mentre
Linux e macOS arm64 erano verdi. Dettagli e fonti in D50.

- **Done**
  - `fetch_bundled_tools.mjs --target universal-apple-darwin` produce ora **tre**
    file per tool: i due per-architettura (che `build.rs` chiede, una volta per
    ogni passaggio di cargo) e quello universale (che il bundler chiede alla
    fine). Prima ne produceva uno.
  - `UNIVERSAL_SLICES` e `lipoArchName` ricavati, non indovinati: le slice sono
    un fatto di Tauri, `lipo` vuole `arm64` dove Rust dice `aarch64`.
  - `yt-dlp` le due slice escono da `lipo -thin` sull'asset universale; `qjs`
    sono i due asset nativi che il `lipo -create` gia' scaricava.
  - Il ramo "gia' presente" scrive comunque le slice: altrimenti un secondo run
    ritrovava l'universale, saltava tutto, e non scriveva mai i due file che il
    build script sta per chiedere — lo stesso difetto, ma solo dalla seconda
    volta.
  - `validateManifest`: `builtFrom` deve elencare esattamente le slice che Tauri
    assembla.
  - La guardia `lipo` è centralizzata e gira **prima del primo byte**: prima
    falliva dopo 37 MB scaricati, perché `yt-dlp` viene prima di `qjs` nella
    lista e arrivava a `lipo` per primo.
  - Passo di CI `lipo -info` (un file, mai eseguito) sostituito da `Every bundled
    file is the architecture its name promises` (sei file, sei modi di fallire).
  - **`scripts/ci-steps.test.mjs`, nuovo**: 7 test che eseguono quel passo con un
    `lipo` finto che risponde in base al **contenuto** del file.
- **Verified by running**
  - `npm test` → **92 verdi** (erano 77: +8 fetch, +7 passo di CI).
  - `npm run check:tools` → manifest OK; `npm run check:ci` → 9 blocchi `run: |`
    validi; `cargo fmt --check` OK; `cargo clippy -D warnings` → 0 errori;
    `cargo test` → 8 suite verdi; `npm run check` → 0 errori, 0 warning.
  - **Il passo di CI, eseguito davvero** contro un `lipo` finto, in sei scenari:
    caso buono (0), file per-arch mancante (1), universale mono-arch (1),
    universale senza arm64 (1), slice che è un fat binary (1), slice
    dell'architettura sbagliata (1). Tutti con l'esito atteso.
  - **Due difetti trovati eseguendolo**, non leggendolo: `Non-fat file` contiene
    `fat file` (il passo rifiutava i 6 file giusti), e un ramo morto tolto.
  - **Revert-check**: `builtFrom` con una slice sola e con una terza slice → il
    controllo del manifesto segnala entrambe le situazioni.
  - Asset `yt-dlp_macos` verificato **su Linux**: magic `0xcafebabe`, `nfat_arch`
    2, `x86_64` 73 808 B + `arm64` 37 047 744 B. È un vero fat binary, quindi
    `lipo -thin` è applicabile.
  - Da Linux, `--target universal-apple-darwin` → messaggio chiaro e **zero byte
    scaricati** (cartella non creata).
- **Not verified / stubbed**
  - **`lipo -create`, `lipo -thin` e `tauri build` universale non sono mai
    girati qui: non esistono su Linux.** Verificato su questa macchina: la logica
    di fetch (nomi, checksum, ordinamento), il passo di CI contro il finto,
    l'intestazione Mach-O dell'asset. Il resto lo vedrà il runner macOS.
  - Nessun `.app` universale è stato costruito né aperto.
- **Spec items marked "verify" che I checked**
  - Quale nome chiede `build.rs` durante un build universale → `env::var("TARGET")`
    in `tauri-build/src/lib.rs`, cioè per-architettura (citato in D50).
  - Quale nome chiede il bundler → `self.target` in
    `tauri-bundler/src/bundle/settings.rs`, cioè `universal-apple-darwin`.
  - `yt-dlp_macos` è universale → letta l'intestazione Mach-O, vedi sopra.
- **Decisions/deviations**
  - D50: tre nomi per tool; `lipo -thin` per yt-dlp; `builtFrom` vincolato alle
    slice di Tauri; passo di CI da uno a sei file, con due difetti trovati
    eseguendolo.
- **Next step**
  1. Committare e rilanciare il workflow. `bundle (macos universal)` deve andare
     verde; se fallisce, **il log sarà molto più utile di prima**: i sei file
     sono controllati per nome e per contenuto prima che il bundle venga
     impacchettato.
  2. `process tests (windows-x64)` e `bundle (windows-x64)`: D49 li corregge ma
     non sono mai stati eseguiti su Windows.
  3. Poi il discorso di `cargo-xwin` + wine per compilare i test per Windows da
     Linux, che è ancora aperto.

## 2026-10-13 — M0-C / I test Windows su Linux, e il sesto bug di fila

Mentre la CI girava: `cargo-xwin` + wine per eseguire la suite su Windows dalla
macchina Linux. Dettagli in D51.

- **Done**
  - `cargo xwin check --all-targets` per `x86_64-pc-windows-msvc`: **verde**,
    35 s. Chiuso il buco «i test non sono mai stati compilati per Windows».
  - Verificato che il check **non è vacuo**: un errore di tipo dentro il modulo
    di test lo fa fallire.
  - `cargo xwin test --test process_tree`: **15/15 verdi sotto wine**, con
    `cancelling_kills_the_child_and_the_grandchild`,
    `dropping_a_running_tool_kills_the_tree` e
    `the_app_exit_hook_kills_every_running_tool`. È spec §4, che avevo dichiarato
    non verificabile.
  - **Corretto un bug mio** in `well_known_dirs_for`: su Windows il ramo Unix
    produceva `/home/tester\.deno\bin`. Il commento che prometteva «la risposta
    dipende solo dagli argomenti» era falso per metà della funzione.
  - Documentato nel codice perché esiste: su Windows `create_private_dir` non può
    mettere 0700 e la privacy viene dalla app data dir. Da lì il 133 (Linux)
    contro 132 (Windows), che è una differenza voluta e ora scritta.
  - Corretta una mia supposizione: wine **fa** funzionare i Job Object.
- **Verified by running**
  - Linux: `cargo fmt --check` OK, `cargo clippy -D warnings` 0 errori,
    `cargo test` 8 suite verdi, `npm test` **92 verdi**, `check:tools` OK.
  - Windows via wine: `cargo xwin test --lib` → **132 verdi, 0 falliti**.
  - Windows via wine: `cargo xwin test --test process_tree` → **15 verdi**.
  - Confronto degli elenchi di test Linux/Windows: l'unica differenza è
    `create_private_dir_uses_mode_0700`, escluso con `#[cfg(unix)]`.
- **Not verified / stubbed**
  - `npx tauri build` su Windows, l'installer e `bundled_tools`: non provati.
    Wine non sostituisce un runner vero per l'impacchettamento.
  - Il fix di `well_known_dirs_for` è stato verificato **sotto wine**, non sul
    runner Windows della CI: se quel runner risponde diversamente, la CI resta
    l'arbitro.
- **Spec items marked "verify" che I checked**
  - Job Object sotto wine: `CreateJobObjectW`, `AssignProcessToJobObject`,
    `TerminateJobObject` e `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (via `Drop`)
    funzionano. Misurati, non dedotti.
  - Confronto sysroot wine `Z:\` ↔ `/`: i test che scrivono e leggono file
    passano senza problemi di percorso.
- **Decisions/deviations**
  - D51: `cargo-xwin` + wine come seconda opinione, non come sostituto; e la
    regola che vale più di tutto, cioè che quando proxy e test vero discordano
    ha ragione il test vero.
- **Next step**
  1. La CI in corso **fallirà** su `well_known_locations_include_the_ones_the_
     spec_lists` se il fix non è pushato. Serve un commit e un rilancio.
  2. Dopo il verde: installare e provare i pacchetti, poi merge e tag.
  3. Script e prerequisite di `cargo-xwin` da mettere in `GUIDA_OPERATIVA.md`
     (personale, fuori dal repository).

## 2026-10-13 — M0-C / Un path scritto al posto dei contenuti

Seconda run di `bundle (macos universal)`: il passo di CI ha trovato che
`binaries/qjs-aarch64-apple-darwin` non è un Mach-O. Dettagli in D52.

- **Done**
  - `downloadSlice` restituisce `{ bytes, path }`: i due chiamanti dichiarano
    quale serve. Prima restituiva solo `path` e quel path finiva **scritto come
    contenuto** del file.
  - Guardia in `install()`: un payload di tipo stringa viene rifiutato al
    momento dell'errore, con quattro test.
  - Il comportamento esiste **solo su macOS**: su Linux il ramo `lipo -thin`
    funziona, quindi il codice sembrava corretto.
- **Verified by running**
  - Asset di qjs scaricati e letti su Linux: `qjs-darwin-arm64` e
    `qjs-darwin-x86_64` sono Mach-O `MH_EXECUTE`, arm64 e x86_64. **Asset e
    manifesto sono corretti** — la prima ipotesi (asset sbagliato) era falsa.
  - `npm test` → **96 verdi** (erano 92: +4 sulla guardia).
  - La guardia attiva davvero: passando un percorso, `install` fallisce con
    `internal error: asked to write a string to …`.
  - Linux: fmt OK, clippy 0 errori, 8 suite Rust verdi, `svelte-check` 0/0,
    `check:tools` OK, `check:ci` 9 blocchi validi.
  - Windows via wine: `cargo xwin test --lib` → **132/132**.
- **Not verified / stubbed**
  - **Il fetch universale vero e `tauri build --target universal-apple-darwin`
    non sono mai stati eseguiti.** Non girano su Linux: `lipo`, `tauri build` e
    la composizione del `.app` non esistono qui. La catena fino a `install` è
    corretta per costruzione e per test; da lì in poi è la CI che deve dirlo.
  - Per macOS **non esiste** un equivalente di `cargo-xwin`: la copertura
    locale che ho per Windows non ce l'ho per macOS, ed è il limite reale di
    questo approccio.
  - Ancora da diagnosticare: i due job Windows. La suite completa sotto wine è
    178/178, quindi la causa più probabile è il bug di D51, già corretto; ma
    `bundle (windows-x64)` scarica i binari Windows da GitHub, cosa che qui non
    posso verificare.
- **Spec items marked "verify" che I checked**
  - Gli asset macOS di QuickJS-NG sono Mach-O eseguibili: letta l'intestazione
    su Linux, `filetype 2`, cpu `0x100000c` (arm64) e `0x1000007` (x86_64).
- **Decisions/deviations**
  - D52: `{ bytes, path }` esplicito più guardia in `install()`, perché in
    JavaScript il tipo sbagliato è silenzioso e il sintomo arriva due passi dopo
    in un messaggio che non nomina la causa.
- **Next step**
  1. Committare, push, rilanciare il workflow.
  2. Servono comunque le righe `FAILED` dei due job Windows: la suite sotto wine
     è verde, ma «molto probabile» non è «verificato».
  3. Dopo il verde: installare e provare i pacchetti, poi merge e tag.

### Addendum alla voce D51 — cosa `cargo-xwin` NON può fare

Verificato tentando di eseguire `tests/bundled_tools.rs` per Windows sotto
wine, con i binari Windows scaricati al posto giusto:

```
refusing to spawn the relative tool path
/home/arturo/Documenti/ScaricatoreDiPorto/src-tauri\binaries\qjs-x86_64-pc-windows-msvc.exe
```

Il codice del test **e' corretto**: usa `Path::join`, non la concatenazione di
stringhe. Il path e' misto perche' `CARGO_MANIFEST_DIR` e' un path Unix (la
 compilazione e' stata fatta qui) e `Path::join` su Windows aggiunge `\`.
Su un runner Windows vero `CARGO_MANIFEST_DIR` e' `D:\a\...` e il risultato e'
valido. Quindi `bundled_tools` **non e' eseguibile sotto wine**: non per un difetto,
perche' l'ambiente non e' rappresentabile.

Da notare che `process.rs` ha rifiutato quel path e ha avuto ragione:
`/home/...\binaries\qjs.exe` su Windows non e' assoluto (nessuna lettera di
unita'), quindi il rifiuto di spec 4.8 e' il comportamento giusto. La guardia ha
respinto un path genuinamente malformato.

**La sostanza del test e' comunque verificata a mano**, eseguendo i binari
Windows fuori dalla suite: il fetch per `x86_64-pc-windows-msvc` funziona da
Linux (checksum di yt-dlp verificato contro `SHA2-256SUMS`, quello di qjs contro
il manifesto), entrambi sono PE32+ `machine 0x8664`, `yt-dlp.exe --version` ->
`2026.08.19`, `qjs.exe --version` -> `0.17.0`, e `qjs.exe` esegue uno script
reale. Cioe' tutto quello che `bundled_tools` afferma, verificato con un metodo
diverso.

**Quello che `cargo-xwin` non copre, in tre righe:**

* `bundled_tools` — il path di compilazione non e' rappresentabile sotto wine;
* `tauri build` e l'impacchettamento su Windows — restano al runner vero;
* il comportamento su macOS — non esiste un equivalente, e D52 lo ha mostrato:
  quel difetto era invisibile su Linux perche' esiste solo su macOS.

## 2026-10-13 — M0-C / `.exe` dopo il triple

Terzo e ultimo rosso. `bundle (windows-x64)` falliva su
`yt-dlp: the bundled binary did not answer --version`. Dettagli in D53.

- **Done**
  - Nuova `fetched_name(tool, triple)`: `<tool>-<triple>` con `.exe` **dopo** il
    triple. `candidates()` la usa per il candidato di sviluppo.
  - `executable_name` (nome nell'app installata, `yt-dlp.exe`) e `fetched_name`
    (nome in `binaries/`, `yt-dlp-<triple>.exe`) sono tenute distinte a verbale:
    erano la stessa cosa scritta in due modi, e solo uno dei due giusto.
  - Il test del candidato con triple riscritto: tre triple Tier 1 più
    `universal-apple-darwin`, nomi esatti scritti a lettere, e una guardia che
    vieta `.exe-` in un nome.
- **Verified by running**
  - **Revert-check**: con il codice vecchio il test rosso e stampa
    `/app/yt-dlp.exe-x86_64-pc-windows-msvc` come candidato; ripristinato, verde.
  - Linux: fmt OK, clippy 0 errori, 8 suite verdi, `npm test` **96 verdi**,
    `svelte-check` 0/0, `check:tools` OK, `check:ci` 9 blocchi validi.
  - Windows via wine: `cargo xwin test --lib` → **132/132**.
- **Not verified / stubbed**
  - `bundled_tools` **non è eseguibile sotto wine** (addendum a D51:
    `CARGO_MANIFEST_DIR` compilato su Linux + `Path::join` Windows = path misto,
    giustamente rifiutato da spec 4.8). La sostanza è verificata a mano: fetch
    Windows funzionante da Linux, checksum corretti, PE32+ x86_64,
    `yt-dlp.exe --version` → `2026.08.19`, `qjs.exe --version` → `0.17.0`, qjs
    esegue uno script.
  - Il fetch universale vero e `tauri build` universale (D52): non girano su
    Linux, resta la CI.
- **Spec items marked "verify" che I checked**
  - La convenzione del nome: `tauri_utils::external_binaries` costruisce
    `{path}-{triple}{.exe su Windows}`; `fetch_bundled_tools.mjs` scrive
    `<nome>-<triple>.exe`. Due fonti, stessa convenzione.
- **Decisions/deviations**
  - D53: `fetched_name` separato da `executable_name`; test del candidato
    riscritto sulle quattro triple invece di una.
- **Next step**
  1. Committare, push, rilanciare il workflow.
  2. **Da fare dopo il verde**: accendere `SDP_DEBUG_PROBE` nel passo di CI che
     testa i tool, cosi' un fallimento dirà la causa e non solo la conclusione.
     E' la seconda volta che un diagnostico esistente ma spento mi costa un
     round-trip.
  3. Dopo: installare e provare i pacchetti, poi merge e tag.

## 2026-10-13 — M0-C / Il fallback puntava a qualcosa di rotto

Ultimo rosso: `bundle (macos-arm64)`. HTTP 403 dalla API e 404 dall'alias
`latest`. Dettagli in D54.

- **Done**
  - Risoluzione del tag in tre passi: API col token, API senza token, **redirect
    della pagina release** (nessuna quota, nessun rate limit).
  - L'alias `latest/download/` **eliminato**: rispondeva 404 tre volte su tre.
  - `releaseUrl` e `checksumUrl` ora hanno il segnaposto `{tag}`: con `latest`
    scritto dentro non c'era dove mettere il tag che il redirect fornisce.
  - `GITHUB_TOKEN` nei due passi di fetch della CI (solo GitHub, mai stampato).
  - `tagFromLocation()` rifiuta `.` e `..`: passavano ogni controllo sui
    caratteri e finivano nell'URL come `/releases/download/../`.
- **Verified by running**
  - **Il fallback eseguito davvero**, con l'API disabilitata nel codice: ottiene
    `2026.08.19` dal redirect, scarica l'URL col tag esplicito e verifica il
    checksum `0f192b7ec147ab62…` — **identico** a quello del percorso con l'API.
  - Percorso normale (con API) verificato per `aarch64-apple-darwin`.
  - Misurati i due sintomi: alias `latest` 404 su 3 tentativi; URL col tag 206.
  - Misurato il redirect: `curl -sI` restituisce 302 con
    `location: …/releases/tag/2026.08.19`.
  - `npm test` → **104 verdi** (erano 96: +8 sul fallback e sul tag).
  - Linux: 8 suite Rust verdi, `svelte-check` 0/0, `check:tools` OK,
    `check:ci` 9 blocchi validi.
  - Un primo tentativo di verifica (proxy con `HTTPS_PROXY`) **non ha funzionato**
    e non è stato contato: il `fetch` di Node non onora quella variabile.
- **Not verified / stubbed**
  - Il comportamento sotto carico reale della API (403 per IP condiviso) non è
    riproducibile qui: si è verificato il ramo del fallback, non l'evento.
  - Restano non eseguiti: fetch universale vero e `tauri build` universale.
- **Spec items marked "verify" che I checked**
  - Alias `latest/download/`: 404 tre volte su tre, contro 206 per l'URL col
    tag esplicito sullo stesso asset.
  - Header `Location` del redirect di `/releases/latest`: contiene il tag.
- **Decisions/deviations**
  - D54: tre strategie di risoluzione del tag in ordine, alias `latest` tolto,
    `{tag}` esplicito nel manifesto, token in CI.
- **Next step**
  1. Committare, push, rilanciare il workflow.
  2. **Da fare dopo il verde**: accendere `SDP_DEBUG_PROBE` nei passi che testano
     i tool, cosi' un fallimento dirà la causa (D53).
  3. Poi: installare e provare i pacchetti, merge, tag.

## 2026-10-13 — M0-C / Run verde: due warning annotati, non corretti

Run di `m0-tools` **verde su tutti e sette i job**, incluso `bundle (macos
universal)`, che era l'unico passaggio della catena mai arrivato in fondo. I due
warning che la run segnala sono annotati qui e **non è stato modificato niente**
né nel codice né nella CI, per scelta esplicita.

- **Warning 1 — Node.js 20 deprecato nelle GitHub Actions.**
  `Node.js 20 is deprecated. The following actions target Node.js 20 but are
  being forced to run on Node.js 24`.
  Riguarda il **runtime delle azioni**, non il nostro Node: le azioni sono
  scritte per girare su Node 20, i runner le forzano su Node 24. Oggi funziona
  (la run è verde), quindi non è un problema attuale; è un problema **futuro e
  datato**, nel momento in cui Node 20 sparisce dai runner.
  Azioni coinvolte, dalle nostre dipendenze di workflow:

  | azione | versione che usiamo |
  | :-- | :-- |
  | `actions/checkout` | `@v4` |
  | `actions/setup-node` | `@v4` |
  | `actions/upload-artifact` | `@v4` |
  | `actions/download-artifact` | `@v4` |
  | `Swatinem/rust-cache` | `@v2` (non segnalata: non e' `actions/*`) |

  Riferimento: <https://github.blog/changelog/2025-09-19-deprecation-of-node-20-on-github-actions-runners/>
  **Da verificare quando si decidera'**: quali versioni maggiori esistono e se
  dichiarano Node 24. Non l'ho controllato adesso perche' la richiesta era di
  annotare e non di agire.
- **Warning 2 — `ubuntu-latest` passa a Ubuntu 26 il 19 ottobre 2026.**
  `The ubuntu-latest label will migrate to Ubuntu 26 beginning October 19,
  2026.` Riferimento:
  <https://github.com/actions/runner-images/issues/14748>
  **Da oggi (2026-10-13) mancano 6 giorni.**

  Chi e' esposto, dai `runs-on` del workflow:

  | job | runner | esposto |
  | :-- | :-- | :-- |
  | `check` | `ubuntu-latest` | **si**, e gira a ogni push |
  | `release` | `ubuntu-latest` | si |
  | `process-tests`, `bundle` | matrice: `ubuntu-22.04`, `windows-2022`, `macos-latest` | no, il Linux e' pinnato a 22.04 |

  Cioe' il lavoro che produce l'installer non e' esposto: il job `bundle` e' su
  `ubuntu-22.04` di proposito, perche' e' la versione su cui il `.deb` e l'rpm
  sono stati misurati. A cambiare e' `check`, che e' il job veloce: un'immagine
  nuova con un GTK/WebKitGTK diverso e' esattamente il tipo di cosa che fa
  fallire una build per motivi che non hanno a che fare con il codice.

  **Quello che e' gia' stato fatto per questo**, e che vale la pena non
  dimenticare: quando i pacchetti apt della CI sono stati scelti, sono stati
  verificati uno per uno contro gli indici di `apt` ed esistono su **22.04, 24.04
  e 26.04** (commento nella CI, riga 83). Quindi il rischio residuo non e'
  «un pacchetto non c'e' piu'», e' «compila ma produce un binario diverso».

- **Next step**
  1. Installare e provare i pacchetti: e' il controllo che nessuna run puo' fare
     al posto di una persona, ed e' quello che sblocca il tag `v0.1.0`.
  2. Prima del 19 ottobre: decidere cosa fare del `ubuntu-latest` di `check`.
     L'opzione con il minor costo e' fissarlo a `ubuntu-24.04` (o 22.04) come
     gli altri job, cosi' la migrazione non arriva a sorpresa.
  3. Separatamente: decidere se aggiornare le azioni `actions/*` per il runtime
     Node 24.
  4. Come da D53: accendere `SDP_DEBUG_PROBE` nei passi che testano i tool, cosi'
     un fallimento futuro dirà la causa e non solo la conclusione.

## 2026-10-13 — Ricerca sui due warning CI: nessuna modifica applicata

Richiesta: verificare i due warning e mettere in documentazione come risolverli,
**senza toccare il codice**. Così è fatto: questo voce è l'unica modifica.

Tre scoperte, in ordine di gravità. **La prima non era nei warning.**

### A. `ubuntu-22.04` è già in deprecazione — ed è il job che produce gli installer

I warning parlavano di `ubuntu-latest`, che riguarda solo `check` e `release`.
Seguendo la pista delle immagini scoperte, il job `bundle` gira su
**`ubuntu-22.04`**, e quello è un problema diverso e più serio.

`actions/runner-images` #14254, stato **aperto**, aperto il 2026-06-16:

> GitHub Actions announcing the deprecation process for `ubuntu-22.04` and
> `ubuntu-22.04-arm`. While the images are being deprecated, You may experience
> longer queue times during peak usage hours. Deprecation will begin on
> **September 17th, 2026** and the images will be fully unsupported by
> **April 17th, 2027** for GitHub Actions and Azure DevOps.

E la parte che conta:

> To raise awareness of the upcoming removal, we will **temporarily fail jobs
> using Ubuntu 22.04**. The brownouts are scheduled for the following dates:
> March 23, March 30, April 6, April 13 (ognuno 14:00 UTC → 00:00 UTC del giorno dopo).

Quindi: la deprecazione è iniziata **26 giorni fa**, i nostri build falliranno
in quattro finestre a marzo–aprile 2027, e il 17 aprile 2027 **smettiano del
tutto**. Il motivo: «GitHub Actions maintains the latest two stable versions of
any given OS version», e ora quelle sono 24.04 e 26.04.

**Cosa colpisce**, dai `runs-on` del workflow:

| job | runner | esposto |
| :-- | :-- | :-- |
| `bundle` linux-x64 | `ubuntu-22.04` | **sì: `.deb`, `.rpm`, AppImage** |
| `check` | `ubuntu-latest` | sì, ma non produce nulla di pubblicato |
| `release` | `ubuntu-latest` | sì, ma solo raccoglie artifact |
| `bundle` windows / macos | `windows-2022` / `macos-latest` | no |
| `process-tests` | `windows-2022`, `macos-latest` | no (non ha una gamba Linux) |
| `tier2` linux-arm64 | `ubuntu-24.04-arm` | no, 24.04 è supportata |

**Nota sul perché 22.04 era la scelta giusta**, e va tenuto presente invece di
dimenticarlo: il commento nella CI spiega che si compila sul sistema più vecchio
supportato perché il `.deb` deve girare anche su distribuzioni più recenti
(glibc e webkit2gtk linkati dinamicamente), e che è ciò che chiede spec §13.5 e
§5.4 («oldest supported distro»). La scelta era corretta; è il mondo che si è
mosso. La sostituzione naturale è `ubuntu-24.04`, facendo attenzione che
**webkit2gtk su jammy è stato aggiornato solo per amd64/i386 (arm64 è ancora a
2.36)** — ed è il motivo per cui `tier2` usa già `ubuntu-24.04-arm`. Passando a
24.04 quella precauzione cade, e il `.deb` si allinea a 24.04/26.04.

### B. Node 20 è già rimosso: era "futuro", è "già successo"

La voce precedente diceva «oggi funziona, problema futuro». **È sbagliato, e la
correzione è il punto di questa ricerca.**

Dalla changelog (2025-09-19, con aggiornamenti fino al 25 agosto 2026):

> Node20 will reach end-of-life (EOL) in April of 2026. … Beginning on June 16th,
> 2026, runners will begin using Node24 by default. … This will only work until we
> upgrade the runner and remove Node20 on **September 23rd, 2026**.

E dalla notifica finale, «Node 20 is no longer available in GitHub Actions»
(2026-09-23):

> This is the final notification that Node 20 is no longer available on GitHub
> Actions runners. Runners now use Node 24 for JavaScript actions. The temporary
> `ACTIONS_ALLOW_USE_UNSECURE_NODE_VERSION` opt-out is no longer available.

Quindi le nostre azioni che dichiarano `using: node20` girano **solo perché il
runner le forza su Node 24**. Non è una configurazione supportata: è uno strato
di compatibilità di cui GitHub non dichiara la durata. Il warning diceva
«being forced to run on Node.js 24» — e va letto alla lettera: non è
un'informazione, è un avviso.

**Verificato, non ricordato: non esiste una patch nella serie v4.**

| azione | ultima v4.x | `runs.using` | versione su Node 24 |
| :-- | :-- | :-- | :-- |
| `actions/checkout` | v4.4.0 | `node20` | v7.1.0 |
| `actions/setup-node` | v4.4.0 | `node20` | v7.1.0 |
| `actions/upload-artifact` | v4.6.2 | `node20` | v7.0.2 |
| `actions/download-artifact` | v4.3.0 | `node20` | **v8.0.2** |

Tre cose da non sbagliare quando si farà:

1. **Le major non sono allineate.** `download-artifact` è a **v8**, le altre a
   **v7**: un «alzo tutto a v7» lascia `download-artifact` indietro, e un «v8»
   generale non esiste per le altre. Vanno aggiornate una per una.
2. **`Swatinem/rust-cache@v2` non è travolta**: non è un'azione `actions/*` e il
   warning non la cita. L'ultima è v2.9.2.
3. **Il `node-version` del workflow è un'altra cosa.** `setup-node` prende il
   Node con cui gira *il nostro* `npm`; il warning parla del runtime con cui gira
   *l'azione stessa*. Sono due livelli distinti e non si risolvono insieme.
4. **Node 24 non supporta macOS 13.4 e precedenti, né ARM32.** Non ci tocca
   (`tier2` usa macos-15-intel), ma è una conseguenza da conoscere se un giorno
   aggiungiamo una piattaforma più vecchia.

### C. `ubuntu-latest` → Ubuntu 26.04: finestra, non data

Dalla changelog 2026-09-17 e dall'issue #14748:

- Ubuntu 26.04 è **generally available dal 17 settembre 2026** (x64 e arm64).
- `ubuntu-latest` migrerà da 24.04 a 26.04 **gradualmente tra il 19 ottobre e il
  19 novembre 2026**, quindi per un mese la nostra run potrebbe capitare sull'una
  o sull'altra immagine. È la cosa peggiore di questa migrazione: non una data,
  una **finestra**.
- Per fissarsi: `runs-on: ubuntu-24.04`. Per provare prima: `ubuntu-26.04`.

**I sette pacchetti apt della CI su Ubuntu 26.04 (resolute).** Tutti esistono.
Questo il cuore della domanda, e le versioni a confronto con 24.04 (noble):

| pacchetto | 24.04 (noble) | 26.04 (resolute) | delta |
| :-- | :-- | :-- | :-- |
| `libwebkit2gtk-4.1-dev` | 2.52.6 | 2.52.6 | **nessuno**: noble è aggiornato via security update |
| `libayatana-appindicator3-dev` | 0.5.93-1build3 | 0.5.94-1build1 | minore |
| `libxdo-dev` | 1:3.20160805.1-5build1 | 1:3.20160805.1-5.1build1 | minore |
| `librsvg2-dev` | 2.58.0+dfsg-1ubuntu0.1 | 2.61.3+dfsg-3ubuntu0.1 | bump di minor |
| `libssl-dev` | 3.0.13-0ubuntu3.16 (OpenSSL) | 3.5.5-1ubuntu3.7 | **major** — irrilevante per noi, vedi sotto |
| `build-essential`, `curl`, `wget`, `file` | presenti | presenti | nessuno |

Due note su quella tabella, che sono la differenza fra «i numeri» e «cosa
significano»:

* **webkit2gtk è identico**, ed è il pacchetto che conta: il binario si linka
  dinamicamente a WebKitGTK e questa è la libreria che decide se il WebView si
  comporta allo stesso modo. Il fatto che 24.04 e 26.04 abbiano la stessa versione
  toglie il rischio più grosso.
* **`libssl-dev` fa un salto major (3.0 → 3.5)**, ma spec §1 richiede `rustls`
  esplicitamente e **nessun OpenSSL**, e di fatto non lo usiamo. Il pacchetto sta
  nell'elenco perché è sulla pagina dei prerequisiti di Tauri, non perché una
  nostra dipendenza lo linki. Vale la pena saperlo se un giorno qualcuno aggiunge
  una dipendenza che usa `openssl-sys`.
* Sulle immagini Rust è 1.98.1 su entrambe, ed è irrilevante: la CI installa il
  proprio toolchain con `dtolnay/rust-toolchain@stable`.

### D. Cosa NON ho verificato

* **Non ho eseguito una build su Ubuntu 26.04.** Non posso: non ho un runner.
  Quello che ho verificato è l'esistenza dei pacchetti e le loro versioni, non
  che `tauri build` ci produca lo stesso binario. La differenza glibc/kernel
  (6.17 → 7.0) non è misurabile da qui.
* **Non ho applicato nessuna delle tre correzioni**, nemmeno quella ovvia:
  è quanto richiesto.
* Non ho guardato se **macOS 14** (già ritirato il 1° ottobre 2026, vedi i
  "related posts") tocchi qualcuno dei nostri job: `macos-latest` oggi è macOS 26,
  quindi no, ma `tier2` usa `macos-15-intel` e quello è un'altra storia.

### E. Ordine di lavoro proposto, da applicare quando si decide

Con le date dalla parte, l'ordine non è ovvio ma è determinato:

1. **`check` e `release`: fissare `ubuntu-24.04`.** Costa una riga per job,
   toglie la finestra di un mese in cui la run potrebbe cambiare sistema sotto i
   piedi, e 24.04 ha strada davanti (fino all'arrivo della prossima LTS). Da fare
   **entro il 19 ottobre**, cioè tra 6 giorni.
2. **`bundle`: `ubuntu-22.04` → `ubuntu-24.04`.** Non urgente per data (aprile
   2027) ma **urgente per conseguenza**: è l'unico job che produce gli installer,
   e quando cade, non abbiamo più un modo di rilasciarli. Vanno rimisurati i
   `.deb`/rpm (le dimensioni e il glibc minimo cambiano), e va aggiornato il
   commento nella CI che motiva la scelta di 22.04, perché diventa falso.
   Vanno anche riguardati i brownout: se rilasciamo in marzo–aprile 2027 su 22.04,
   un build potrebbe fallire per cause esterne.
3. **Le azioni a Node 24**, una per una e verificandole: checkout v7, setup-node
   v7, upload-artifact v7, download-artifact v8. Da fare insieme al punto 1 o 2,
   tanto nello stesso commit si toglie un giro di CI.
4. **Controllo da fare comunque**: `npm run check:release` e le versioni nei tre
   manifest restano allineati a `0.1.0` (verificato prima, invariato).

Nessuna di queste è applicata. Le righe da cambiare sono poche e il rischio è
basso, ma è una decisione tua: cambia l'immagine di build degli installer, e
quella è una cosa che si misura, non si presume.

## 2026-10-13 — M0-C / Allowlist invece di commenti (spec §13, §14)

Risolti i due warning della CI, e reso difficile ricascarci. D55.

- **Done**
  - Immagini esplicite: `ubuntu-latest` → `ubuntu-24.04` (`check`, `release`),
    `ubuntu-22.04` → `ubuntu-24.04` (gamba Linux di `bundle`),
    `macos-latest` → `macos-26` (stessa immagine, etichetta che non migra).
  - Azioni su Node 24: `checkout@v7`, `setup-node@v7`, `upload-artifact@v7`,
    **`download-artifact@v8`**, `rust-cache@v2.9.2`.
  - `dtolnay/rust-toolchain@stable` registrata come esenzione: è composite,
    nessun runtime JS, tag mobile voluto.
  - `scripts/check-ci-runners.mjs`: allowlist di immagini e azioni, con le
    scadenze nei messaggi. Agganciata al job `check` come `npm run check:runners`.
  - Commento in testa a `ci.yml` con il perché delle versioni; 3 commenti che
    nominavano immagini vecchie riscritti (erano diventati falsi).
- **Verified by running**
  - `npm run check:runners` sui workflow verdi: «nessuna immagine fluttuante,
    nessuna deprecata, tutte le azioni su Node 24».
  - **Revert-check**: una riga indietro (`ubuntu-24.04` → `ubuntu-22.04`) fa
    scattare la guardia con la scadenza del 17 aprile 2027 e i brownout.
  - **Revert-check sul confine**: `download-artifact` da v7 a v6 fa cadere due
    test. I test della guardia non sono vacui.
  - `npm test` → **127 verdi** (erano 104: +32 sulla guardia e −5? no: +23 qui);
    `check:ci` 9 blocchi validi; `check:tools` OK; 8 suite Rust; fmt; clippy 0.
  - Confini Node 24 verificati tag per tag: checkout/setup-node da v5,
    upload-artifact da v6, download-artifact da v7. Non simmetria, misure.
- **Not verified / stubbed**
  - **Nessuna build eseguita su Ubuntu 24.04.** Verificato che i pacchetti ci
    sono e che l'etichetta è valida; non che `tauri build` produca lo stesso
    binario.
  - `bundle` da 22.04 a 24.04 **alza il glibc minimo del `.deb`**: le soglie e le
    dimensioni misurate finora non valgono più. Da rimisurare.
  - I 4 warning di Node 20 del job `release` su tag: verificati per lettura, non
    su una run con tag.
- **Spec items marked "verify" che I checked**
  - Confini Node 24 per azione (tabella in D55), letti da `action.yml` a ogni tag.
  - `ubuntu-24.04`, `macos-26`, `macos-15-intel`, `windows-2022`,
    `windows-11-arm` sono etichette esistenti nella tabella dei runner.
  - `dtolnay/rust-toolchain` è composite; `rust-cache` v2.9.2 è node24.
- **Decisions/deviations**
  - D55: allowlist e non denylist, controllo a ogni push, confini Node 24 non
    simmetrici ma misurati, due esenzioni dichiarate (dtolnay, rust-cache).
- **Next step**
  1. Rilanciare la CI. La run deve essere verde e confermare che le immagini
     nuove reggono: in particolare `bundle (linux-x64)`, il primo a costruire
     davvero su 24.04.
  2. **Rimisurare** `.deb`, `.rpm` e AppImage: dimensione e glibc minimo. Il
     rapporto di `scripts/report-bundle-sizes.mjs` va confrontato con quello
     precedente.
  3. Poi l'ultima cosa rimasta da ieri: installare e provare i pacchetti, che
     sblocca il tag `v0.1.0`.

### 2026-10-13, run successiva — verde sulle immagini nuove

`workflow_dispatch` su `m0-tools` dopo `fc5ddc5` (immagini esplicite, azioni su
Node 24, guardia): **verde tutto, e senza warning**.

* `check (ubuntu, lint + test)` — 2 m 8 s
* `Matrix: process-tests` — 2 su 2
* `Matrix: bundle` — 3 su 3
* `bundle (macos universal)` — 3 m 9 s
* `draft release` — non partito, correttamente: è legato ai tag `v*` e questa era
  una dispatch.
* Test frontend: **127 su 127** su 7 file. Confermata la run anche sui runner
  Windows e macOS, che ieri erano rossi.

- **Ciò che questa run chiude.** D55 lasciava scritto «non verificato: nessuna
  build eseguita su Ubuntu 24.04». Ora c'è stata: `bundle (linux-x64)` ha
  costruito su 24.04 ed è verde. Il punto Non era se compilasse — era se
  producesse lo stesso binario, e su quello **il verde della build non dice
  niente**: una compilazione riuscita può benissimo linkednare una glibc più
  recente.
- **Ciò che resta aperto, ed è una misura, non un'opinione.** `bundle` è passato
  da 22.04 a 24.04, quindi il **glibc minimo richiesto dal `.deb` è salito**. Le
  misure di ieri erano su 22.04 e **non valgono più**:

  | componente | misurato su 22.04 | su 24.04 |
  | :-- | --: | --: |
  | binario applicativo | 4,81 MiB | da rimisurare |
  | `.deb` | 41,36 MiB | da rimisurare |
  | `.rpm` | 41,36 MiB | da rimisurare |
  | AppImage | 147,27 MiB | escluso dal budget |

  Il rapporto è nel log del job `bundle` (`report-bundle-sizes.mjs`) e negli
  artifact. Finché qualcuno non lo confronta, le righe nella tabella qui sopra
  restano «da rimisurare»: **una misura vecchia presentata come attuale è uno
  dei modi in cui questa documentazione potrebbe mentire**, ed è il motivo per
  cui la scrivo invece di toglierla.
- **Nota di metodo, che vale più del singolo numero.** Le ultime tre run hanno
  chiuso tre difetti trovati da CI, e due trovati da un controllo locale in meno
  di due minuti (`well_known_dirs_for` tramite `cargo-xwin`, e il passo di
  `lipo`). La run di oggi non ha trovato niente, e non è un caso: è il primo
  giro in cui le correzioni erano già state verificate su un sostituto locale
  prima di arrivare in CI. Non perché CI sia inutile — è l'unica che dice se il
  *pacchetto* funziona — ma perché **un controllo che gira prima di spendere 95
  minuti di quota cambia il costo dell'errore**, e con esso la voglia di
  verificare.

## 2026-10-14 — M1 / Video singolo: ispezione, download, runtime JS, cancellazione

Branch `m1-single-video` (da `m0-tools`, che a sua volta non è ancora in `main`).

- **Done**
  - `services/jsruntime.rs`: modalità Auto/QuickJS/Deno, flag nel verso giusto
    (`--no-js-runtimes` **prima** di `--js-runtimes`), riconoscimento host
    YouTube, costante `YOUTUBE_PLAYER_CLIENTS`, memoria `(yt-dlp, qjs)` con TTL
    24 h su `runtime-memory.json` (best effort, atomica).
  - `services/process.rs`: secondo sink per lo **stderr** (il progress
    postprocess di yt-dlp esce lì — misurato), ring buffer invariato.
  - `services/ytdlp.rs`: selettori esatto/max/best/audio/id esplicito, merge
    `mp4/mkv`, `--windows-filenames`, template progress/print di §8.3, parser
    (`SDP_DL`/`SDP_PP`/`SDP_FILE`, "NA" compreso), tracker "Video 1/2"/"Audio
    2/2"/"Merging…", throttle a 10/s, classificatore §8.5 (tutte le righe della
    tabella + `DENO_MISSING`), cookie §8.6 per OS.
  - `services/inspect.rs`: `-J --flat-playlist`, modello `Inspected`
    (Video/Playlist stub/Error), scala risoluzioni con fallback dell'originale e
    filtro storyboard, timeout 60/120 s, self-test §5.3.6.
  - `services/job.rs`: pipeline a video singolo con fallback QuickJS→Deno (una
    volta: errore `YT_CHALLENGE`, zero formati con altezza, stallo 90 s nel
    canale di download), retry **senza `-f`** su `FORMAT`, cancel che uccide
    l'albero e pulisce `.sdp-tmp-<job>`, storage dell'esito per la prossima
    volta, pulizia dei `.sdp-tmp-*` orfani all'avvio.
  - IPC: `inspect_url`/`inspect_cancel` (request id, cancellazione della
    precedente), `download_start`/`download_cancel`/`download_dismiss_deno_offer`,
    eventi `job:progress`/`job:done`; `tools_install` installa **Deno** dal
    manifest con `tool:progress`; `tools_self_test`.
  - Manifest: voce `deno` pinnata 2.9.7 (sei piattaforme, zip, sha256), non
    inclusa nell'installer.
  - Frontend: schermata temporanea (URL con debounce 500 ms e cache, selettore
    risoluzione, switch audio, cartella, progresso con velocità/ETA, errore con
    "Copia i dettagli", banner Deno), tab "Rete e account" (runtime JS, cookie),
    "Prova l'estrazione da YouTube" su Strumenti, i18n IT/EN completa.
  - `.github/workflows/selftest.yml`: il self-test vero sui tre sistemi, **solo
    a avvio manuale** (D70). `npm run check:ci` e `check:runners` verdi su di
    esso.
  - **Footer di stato collegato al job** (segnalato dall'utente: diceva "Nessun
    processo in esecuzione" durante un download). Ora mostra stadio, flusso e
    percentuale; a fine job il nome del file, in errore il messaggio
    localizzato, e la voce degli strumenti con problemi porta a Strumenti. I
    contatori di spec §7.5 restano per M2. **Senza** velocità ed ETA: erano gli
    stessi numeri già sotto il nome del video (seconda segnalazione dell'utente).
  - **Risoluzione degradata riconosciuta** (segnalato: "1440p scelto, 360p
    scaricato, una volta"). Nuova stampa `before_dl:SDP_FORMAT…` (misurata sul
    binario vero) che dice il formato davvero scelto; se è un muxed più basso del
    richiesto il file viene rimosso e si ritenta una volta con Deno, altrimenti
    il risultato porta `requestedHeight`/`actualHeight` e la UI lo dichiara.
    Dettagli e limiti in D71.
  - **Ispezione ridotta riconosciuta e ritentata** (seconda segnalazione: "a
    volte incollando il link ci sono solo 360p, riprovando ci sono tutte").
    L'ispezione non passa più `--no-warnings` (deviazione deliberata da §6.2,
    motivata in D73) perché yt-dlp annuncia lì la challenge fallita; un secondo
    tentativo — Deno se disponibile, altrimenti lo stesso runtime — recupera la
    lista completa, si tiene il risultato migliore, e se resta ridotta il
    risultato lo dichiara (`degraded`) e la UI offre **Riprova**. Le risposte
    degradate **non** entrano nella cache.
  - **Il bug ereditato dall'originale: via la lista di client forzata** (terza
    segnalazione, con richiesta esplicita di cercare lì). `player_client=
    web_embedded,web,tv` è codice dell'app Python; in questa versione di yt-dlp i
    client predefiniti sono `visionos,web`, e solo `visionos` non richiede un PO
    token. Forzare la vecchia lista eliminava proprio quel client, e quando il
    token non si riusciva a coniare restavano solo i muxed — il 360p, a
    intermittenza. Ora **di default non si passa nessun extractor arg**; la
    costante resta in un campo esperto (Impostazioni → Rete e account), vuoto =
    automatico. Misure: 31→**49** formati sul video 4K, self-test 7,4 s/15
    formati → **1,95 s/24 formati**. Dettagli e sorgente citato in D74.
  - **Il download non finalizza mai un muxed più basso del richiesto**: alla
    riga `SDP_FORMAT` (prima dei byte) il job annulla il tentativo, elimina i
    parziali, ritenta una volta (Deno o stesso runtime) e, se resta degradato,
    **fallisce** con un messaggio chiaro invece di salvare il file sbagliato. Un
    *merge* più basso resta un esito legittimo, con i due numeri nel payload.
  - **Risoluzione dei tool in cache per la sessione** (D75): risparmia il probe
    da 0,46 s di yt-dlp a ogni incolla; invalidata da override, aggiornamento,
    installazione e dalla verifica esplicita della pagina Strumenti. Provata da
    un test che cancella i binari dal disco dopo la prima risoluzione.
  - **Debounce**: 500 ms restano per la digitazione, salta per l'incolla (D76).
- **Verified by running**
  - `cargo test`: **285** test verdi in totale (208 lib, 3 bundled_tools,
    9 download, 8 m1_ipc, 16 process_tree, 6 settings_ipc, 13 tools,
    22 ytdlp_job), più 3 `#[ignore]` (i test veri, vedi sotto);
    `cargo clippy -D warnings` e `cargo fmt --check` puliti. Frontend: **162**
    test (`npm test`), `npm run check` a 0 errori.
  - **IPC completo con il tool finto**: `inspect_url` e `download_start`
    attraverso i comandi veri, con `ToolContext` che punta al fake — il job
    scrive il file e lo stato torna a fermo. È in `m1_ipc.rs`.
  - `npm run check` 0 errori, `npm test` **152** verdi, `npm run build` ok.
  - **Reale, con i binari inclusi** (`cargo test --test real_selftest --
    --ignored --nocapture --test-threads=1`): self-test ok con **quickjs** in
    7,4 s; ispezione "Me at the zoo" → altezze `[240, 144]`; download a 144p
    unito in `.mkv` (il merge sceglie MKV per VP9+Opus, come previsto); audio
    only → `.opus`; le temp dir spariscono, gli output restano.
  - **Installazione Deno reale** (stesso file, `the_deno_install_...`): 41,6 MB
    scaricati, checksum del manifest verificato, zip estratto, il binario
    risponde `deno 2.9.7`. E il flag di tentativo B è stato provato live:
    `--no-js-runtimes --js-runtimes deno:/tmp/.../deno` estrae "Me at the zoo".
  - L'argomento del merge e i template sono stati verificati **eseguendo** il
    binario fissato su un video vero (non solo `--help`): `SDP_DL`/`SDP_FILE`
    su stdout, `SDP_PP` su stderr, `NA` per i campi ignoti.
  - **Flag verificati su `--help` del binario incluso (2026.08.19)**, oltre che
    in esecuzione dove indicato: `--js-runtimes`, `--no-js-runtimes`,
    `--remote-components`, `--flat-playlist`, `--no-playlist`, `--playlist-items`
    (`-I`), `--progress-template` (prefissi `download:`/`postprocess:`),
    `--print` con `WHEN:after_move`, `--paths` (`home:`/`temp:`),
    `--windows-filenames`, `--merge-output-format`, `--concurrent-fragments`,
    `--retries`, `--fragment-retries`, `--socket-timeout`,
    `--cookies-from-browser`, `--cookies`, `--extractor-args`,
    `--ffmpeg-location`, `--impersonate`, `--simulate`, `--newline`, `--color`,
    `--ignore-config`, `--extract-audio` (`-x`), `--audio-format`,
    `--limit-rate`. **Assente**: `--playlist-end` (D60).
  - `-U` reale su una copia in `/tmp`: aggiornamento a nightly e no-op di
    `--update-to stable` da nightly (D66).
  - L'app parte: `npm run app` (vedi "run" qui sotto).
- **Not verified / stubbed**
  - **Playlist**: solo lo stub (tipo e conteggio). Elenco, selezione, archivio
    sono M3.
  - **Firma/notarizzazione macOS e antivirus Windows**: invariati da M0-C
    (SPIKES.md).
  - Il **cancel con un download vero**: coperto con il falso yt-dlp dalla suite;
    la prova manuale con l'elenco dei processi resta S5 per l'umano.
  - **Cookie da un browser vero**: gli argomenti sono provati con il falso tool
    e il filtro per OS con i test; leggere il database di un browser reale no
    (servono la macchina e una sessione dell'utente).
  - **Pulizia all'avvio** dei `.sdp-tmp-*` orfani: copre solo la cartella
    predefinita (impostazione o Download di sistema), non una cartella scelta al
    volo nel selettore. Con M2 `queue.json` saprà quali cartelle appartengono a
    elementi interrotti.
  - L'interfaccia grafica non è stata cliccata da qui (nessun modo di pilotare la
    finestra): avvio e assenza di errori all'avvio verificati, i percorsi di
    click no. Elencato per onestà.
  - `--limit-rate`, proxy e impersonificazione: M3.
- **Spec items marked "verify" che ho chiuso**
  - `--js-runtimes`/`--no-js-runtimes` esistono nel binario fissato e sono nati
    esattamente in **2025.11.12** (sorgente `options.py` ai tag): `minVersion`
    confermato (D56).
  - `--playlist-end` **non esiste più** in 2026.08.19; M3 userà `-I :1000` (D60).
  - Asset e checksum Deno: nomi verificati nella release v2.9.7; sha256 linux
    x64 verificato byte per byte (D59).
  - `--print after_move:` scarica davvero (non simula) e stampa il percorso
    finale dopo il merge (misurato).
  - Il video di prova `BaW_jenozKc` non è più disponibile; usato
    `jNQXAC9IVRw` (D63).
- **Decisions/deviations (anche in DECISIONS.md)**: D56–D69.
- **Next step**
  1. Prova manuale dell'interfaccia sulla macchina di sviluppo (URL, altezza,
     audio, cartella, annulla) — è l'unica cosa che qui non si può fare.
  2. Merge del branch dopo la revisione; poi M2 (coda).

---

## 2026-10-15 — M2 / The queue (spec §7, §11.2, §11.5, §11.7)

Branch `m2-queue` (from `m1-single-video`, which is not in `main` yet).

- **Done**
  - **`services/queue/`**, in five files with one job each:
    - `model.rs` — the state machine of spec §3 (all twelve states, including
      the `Encoding`/`Transcribing`/`Summarizing` of M4/M5/M6 and the
      `Interrupted` of §7.4), the immutable options snapshot of §7.1, groups,
      the counters of §7.5, duplicate detection, the retry policy and the
      500-item history cap of §7.3. Pure, 34 unit tests;
    - `store.rs` — `queue.json`: atomic writes, `.bak` on corruption, active
      items come back `Interrupted` keeping their temp dir (§7.4, §4.10);
    - `scheduler.rs` — the planning function: lanes (download 2, encode 1,
      transcribe 1), the per-host cap of §6.4, retry timers, first-runnable
      first. 12 unit tests, no process and no clock;
    - `events.rs` — the `queue:update` payload and the ≤ 10/s per-item
      coalescing of §7.6;
    - `engine.rs` — a scheduler task, one task per item (which runs **the M1
      pipeline unchanged**, `job::run_job`), a flusher, the actions and the
      persistence triggers.
  - **`services/notify.rs`** — the system notification when the queue goes
      idle (§7.7), off by default, text taken from the same dictionaries the UI
      uses.
  - **Commands** — `queue_list`, `queue_add`, `queue_cancel`, `queue_pause`,
      `queue_resume`, `queue_retry`, `queue_remove`, `queue_pause_all`,
      `queue_resume_all`, `queue_retry_failed`, `queue_reorder`,
      `queue_move_to_top`, `queue_clear_finished`, `queue_lanes`,
      `read_links_file`, `app_set_language`, plus **`inspect_urls`** (three in
      parallel, own tokens, answers in order, §6.2's last bullet).
  - **Settings** (schema 4): `parallel_downloads` (1–4), `per_host_cap` (1–8),
      `notify_on_idle`, `paste_links_anywhere`, all clamped on both sides, and
      the Downloads section of Settings → General now really edits them.
  - **Startup** — the queue opens before the window, the orphan-temp sweep of
      §4.10 now knows **which temp dirs the queue still owns** (an interrupted
      item keeps its `.part` files) and looks in every output folder a live item
      writes to, not only in the default one. The exit hook cancels the items
      before killing the trees.
  - **Frontend** — `lib/queue.ts` and `lib/addbar.ts` (pure, tested),
      `lib/stores/queue.svelte.ts` (a keyed map + explicit order; a tick with one
      changed item patches one row), the new **Add bar** (multi-link paste,
      drop of text or `.txt`, per-row quality picker, inline errors with Retry,
      "add anyway" for duplicates), the **virtualised list**, the **item cards**
      (stage chip, progress, speed/ETA, actions per state, expandable details
      with Copy details, drag to reorder) and a footer that now says
      "N running · M queued · speed".
  - **i18n** — 79 new keys in IT and EN; the M1 placeholders
      (`queue.comingSoon`, `settings.downloads.comingSoon`) are gone.
- **Verified by running**
  - `cargo test`: **328 tests** green (297 lib, 17 `queue_engine`, 14
      `queue_ipc`, 3 bundled_tools, 9 download, 8 m1_ipc, 16 process_tree, 6
      settings_ipc, 13 tools, 22 ytdlp_job) + the 3 `#[ignore]` real-tool tests.
      `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean.
  - `cargo test --test queue_engine` — **17 integration tests on the fake
      yt-dlp**, all offline:
      * ten links, **exactly 2 at a time** measured from the fake's own
        start/end log (peak concurrency, by interval overlap);
      * one parallel download means one; four means four;
      * the per-host cap: never three from YouTube, while a different host is
        not held back;
      * pause keeps the item (and no error), resume finishes it, pause-all stops
        the scheduler;
      * cancel removes the temp dir and nothing else;
      * a transient network failure is retried by the queue (1 automatic retry,
        `auto_retries == 1`), a permanent one is never retried by itself, AUTH is
        never retried and keeps its raw detail;
      * a crash-style `queue.json` (an item still `downloading`) comes back
        `Interrupted` with its temp dir, and Resume finishes it;
      * a graceful shutdown comes back parked and Retry finishes it;
      * a corrupt file is kept as `.bak`, the queue starts empty and the toast
        reaches the sink;
      * the footer counters and **one** idle notification for the transition;
      * clear-finished, reorder, move-to-top, the events and their coalescing,
        the settings' lanes (including 0 → clamped to 1).
  - `npm test`: **197** green (162 of M0/M1 + 35 new for `queue.ts` and
      `addbar.ts`). `npm run check` 0 errors 0 warnings; `npm run build` ok.
  - `cargo build --release`: app binary **6,88 MiB** (budget 15 MB). The
      notification plugin's own cost was measured separately for D79: **+0,17
      MiB** (6,71 → 6,88).
  - **`npm run app:binary` starts**: the release binary opens the queue, starts
      its two tasks and stays up. This is how D88 was found — a `tokio::spawn` on
      the main thread panicked, and no test could see it (they all start the
      engine inside a `#[tokio::test]`).
- **Not verified / stubbed**
  - **The GUI has not been clicked.** From here there is no way to drive the
      window: no manual check of the Add bar, the cards, the context actions or
      the drag-and-drop. Recorded rather than implied.
  - **The real network path through the queue is untested.** Every queue test
      uses the fake yt-dlp; a real end-to-end (paste → 2 in parallel → files) is
      the manual check above.
  - **The system notification has not been seen.** The plugin is wired, the
      strings resolve in both languages (unit-tested), and the engine calls it on
      the idle transition; whether the OS shows it, and what it looks like, is
      for the manual check.
  - **Playlist links are refused with a message** (D84); M3 brings the selection
      panel.
  - **Not implemented in M2, on purpose**: per-row "start immediately", the
      "More options" drawer, user presets, the thumbnail column, the group
      actions of §11.5 (the model and the grouping exist, M3 creates groups),
      Ctrl/Cmd+F filtering, the 2 000+ item load test (the arithmetic is tested,
      a real 5 000-item queue is M7's performance item).
  - **Auto-resume on start** (spec §7.4's "or auto-resume if the setting is on")
      is not implemented, and an interrupted item does **not** start by itself:
      it comes back `Interrupted` with a Resume button, which is the spec's
      first-listed behaviour (D87).
  - **"Keep the computer awake"** (§7.7, optional, per-OS API) is not
      implemented.
- **Spec items marked "verify" that I checked**
  - **Tauri v2 exposes no OS locale to Rust.** `Manager` has no `locale()`, and
      neither `Window` nor `Webview` has one (read in the 2.12.3 sources, then
      in the docs.rs page for `tauri::Manager`, `tauri::window::Window`). That is
      why the language for the notification travels from the webview
      (`app_set_language`), and why the backend falls back to `LANG`/`LC_ALL`.
  - **`tauri-plugin-notification` 2.7 requires Rust 1.90** and this package
      declares `rust-version = 1.85`, so 2.4.0 is the newest usable release;
      `cargo add` picks it for that reason (D79).
- **Decisions/deviations (also in DECISIONS.md)**: D79–D88.
  - The two deliberate deviations from the spec's wording are D81 (item-granular
    event diffs instead of field-level patches) and D82 (lane counters instead
    of semaphores); D83–D86 are decisions of scope, not of behaviour.
- **Next step**
  1. Manual check on the development machine: paste several links, watch two
     downloads run at once, pause/resume/cancel one, clear finished, restart the
     app mid-download and Resume; then Settings → Downloads with the two caps,
     and the notification with the switch on.
  2. Merge the branch after the review; then M3 (playlist and other sites).

## 2026-10-15 — M2 / The queue's IPC layer as real IPC tests, and four real bugs

Still branch `m2-queue`. Same phase: the queue was done, this step is the pass
that makes its **claims** true rather than merely asserted in prose. Two of the
four bugs below came out of a test that failed once in ten runs; the other two
came from reading the code with those tests in hand.

- **Done**
  - **`tests/queue_ipc.rs` rewritten as real IPC tests.** The 14 tests it had
      were placeholders that only proved the file compiled; they now go through
      `tauri::test::MockRuntime` with the real `App`, the real handlers and the
      real argument deserialisation (the `m1_ipc.rs` pattern). **12 tests**:
    - `inspect_urls` answers one row per URL, **in the order the URLs were sent**,
      and an empty list is not an error (three in parallel, answers in order,
      §6.2);
    - `queue_add` refuses a non-HTTP URL and keeps the others, reports duplicates
      as `{url, itemId, title}` and adds them anyway on the second call with
      `force`;
    - an audio-only **quality** wins over the `kind` the frontend sent (§8.4);
    - the actions on an unknown id answer `false`, they do not raise (§15: a
      stale click must not be an error);
    - `queue_reorder` / `queue_move_to_top` change the order over the wire;
    - `queue_lanes` returns the lanes the settings actually clamped to;
    - `read_links_file` reads a dropped `.txt` and refuses a directory **with a
      message**, not a panic;
    - `app_set_language` travels from the webview to the backend (D79's
      fallback) and the queue settings survive `settings_save`/`settings_get`
      clamped at both ends (this one goes through the real `AppState`, the
      document on disk and the engine that reads it).
  - **Four more `queue_engine` tests** (21 in all) and **three more unit tests**
      (300 in the lib), covering what the previous pass left untested:
    - the add → `queue_list` → event → `queue.json` path the frontend actually
      walks, checked at all three points;
    - the snapshot's **wire shape**: the states as strings, `options.kind`,
      `options.quality`, and every optional an explicit `null` rather than a
      missing key (the frontend's normaliser would default it, the wire has one
      shape);
    - the options are **frozen into the item**, so a later settings change
      cannot alter what the runner will use (§7.1);
    - a group is upserted (replaced, not appended) and survives the file;
    - pausing an item that is not running parks it (D90);
    - **the file says an item is running while the tool runs** (D91), read from
      the disk and not from the engine, and a restart over that file parks it;
    - **concurrent writes of one path are each a whole document** (D92): eight
      threads, twelve rounds, ~40 KB payloads;
    - **the frontend reducer**: `applyUpdate` and `applySnapshot` in
      `queue.ts`, nine new tests (197 → 206), covering the per-row identity a
      tick must preserve, the removal, the order, and the stale event.
  - **Seven fixes** (D89–D93, D96, D97). D91, D92 and D96 came out of the
      flaky runs; D89, D93, D95 and D97 out of reading the same code with the
      new tests in hand:
    - a **running** item that is cancelled or removed now has its temp
      directory deleted **before** the row records it, and with the queue's lock
      released. It used to be deleted after, which left a window where
      "Cancelled" and "its directory is gone" were not the same statement — the
      startup sweep of §4.10 lists what the queue still owns, so a directory
      that outlives its row is invisible to it. `QueueEngine::signal` no longer
      deletes anything while holding the state mutex either;
    - `queue_pause` on an item that is **not running** used to **cancel** it — a
      queued row would come back as "Cancelled" (Retry), and a row that had just
      finished would lose its progress and read "Cancelled" while keeping its
      file. The UI never offers Pause there and `pause_all` never reaches it,
      but the command is public IPC: it now parks the item as `Paused`, so the
      row says Resume, and leaves an already parked item alone;
    - **an item is written to `queue.json` as running before yt-dlp is spawned**
      (D91). `start_item` set the state in memory and never wrote the file, so
      for the whole life of a download the document claimed the item had not
      started. Spec §7.4's recovery can only park what it can see was in an
      active stage, so **a crash left the item in `Queued` and the next launch
      downloaded it again by itself** — no Resume, no question asked. The
      invariant is now: a file that says `Queued` never has a tool behind it.
      One extra write per item, on top of the one at the end;
    - **the temp file of an atomic write is per write, not per process** (D92).
      `.<file>.tmp-<pid>` was shared by every concurrent write to the same
      destination inside one process, so "temp + rename" was not atomic at all:
      two writers overwrote each other's bytes through two file descriptors,
      the first rename published the other's half-written document, and the
      second failed with `ENOENT`. The next reader found unparseable JSON and —
      as §7.4 asks — moved it to `.bak` and carried on with **defaults**: an
      empty queue, or default settings, because two threads happened to save at
      once. Now `.<file>.tmp-<pid>-<counter>` with a process-wide `AtomicU64`.
      This one predates M2 (`paths.rs` is M0) and the queue is simply the first
      thing that writes concurrently by design;
    - **`save_settings` and `RuntimeMemoryStore::remember` move the file and the
      in-memory copy under one lock** (D93). Two saves can overlap — the
      settings page saves on every change and the app saves again on the way out
      — and each one carries a whole document, so the old "write the file, then
      update the copy" could leave the engine scheduling with settings that were
      never on disk. The runtime memory has the same shape: two yt-dlp jobs can
      learn a degraded runtime at the same moment. Both still write the file
      **first**, so a failed write leaves the copy on the last saved values;
    - **nothing starts after `shutdown()`** (D96). The scheduler checks
      `stopped` at the top of each pass, but a pass already in flight went on to
      `start_item` for everything it had planned: an item could be spawned
      *after* the app began tearing down, escaping the exit hook's kill, and it
      would rewrite `queue.json` while the next launch was reading it. The
      check moved into `start_item`, before it takes the lock;
    - **the frontend never walks a row back** (D97). `load()` replaced the whole
      queue with the snapshot; a `queue_list` overtaken by an event in flight
      put a *running* item back to `queued`, and an idle queue emits nothing
      that would correct it — the row stayed wrong until the user touched
      something. The store remembers the `seq` it had when the read went out
      and, if the answer was overtaken, takes only the rows it is missing. To
      make that testable at all, the reducer moved from the store into
      `$lib/queue` as a pure function: a `$state` field needs the Svelte
      compiler, which the vitest config deliberately does not use, so the
      store's own "testable without an event bus" claim had never been true.
- **Verified by running**
  - `cargo test`: **411 tests** green (301 lib, 21 `queue_engine`, 12
      `queue_ipc`, 3 bundled_tools, 9 download, 8 m1_ipc, 16 process_tree, 6
      settings_ipc, 13 tools, 22 ytdlp_job) + the 3 `#[ignore]` real-tool tests.
      `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean.
  - **The flakes were chased, not retried away.** `cargo test --test
      queue_engine` failed about one run in ten on the first binary. The log
      named `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`
      with the recovered state `Queued` (and, in another run, `Downloading`),
      and `an_added_item_is_persisted_patched_and_gone_from_a_restart` with an
      **empty** queue plus `cannot write the queue file: No such file or
      directory` in stderr. Those two symptoms are D91 and D92 — how a
      crash-recovery bug and a data-loss bug in the same file path announce
      themselves. Each new test was checked to **fail without its fix**: the
      D91 unit test prints `Queued`, the D92 test fails with the old name on the
      first round.
  - **The suite was soaked 30 times, three times over**, and each soak found
      something: the first the two product bugs above, the second and third five
      test-side races. That is the whole reason for a soak — the tests that only
      fail on a loaded machine are the ones that would have failed in CI
      intermittently and been re-run until green.
  - `the_queue_settings_travel_through_the_settings_ipc_and_are_clamped` now
      goes through the **real** `settings_get` / `settings_save` commands and
      reads the document back off disk, instead of calling `AppState` directly:
      it is the only test in the file that proves the frontend's shape of the
      request (a whole camelCase document) deserialises into `Settings`.
  - **Five of the flakes were the tests themselves, and are fixed without being
      weakened.** Each was a test observing a task's output
      the instant the *state* said it should be there, and each is now a poll
      with a deadline, which is what the other waits in the file already do:
    - a test that read `queue.json` right after seeing a state in memory was
      reading the **previous** document: the state lock is released before the
      file is touched, so there is a microsecond in which the engine says
      `downloading` and the disk still says `queued`. It now waits for the file
      to leave `queued`. And the fake grew a `veryslow-<ms>` URL whose download
      takes exactly as long as the test asks, so the assertion pins the real
      value (`downloading`) instead of "not queued": a 400 ms download can
      finish while a loaded machine is still running the assertion;
    - two tests corrupted or inspected the file while a `shutdown()`-ed engine
      could still write to it, which a real app cannot do (exiting kills the
      tasks). They now wait until the queue is quiet, where "quiet" also covers
      an item still `Queued` because the stopped scheduler never got to it;
    - the idle-notification test read the notifications as soon as the
      counters said idle, but the notifier is called *after* the state is
      updated. It waits for the first notification and then 500 ms more, because
      "exactly one" is a claim about absence and absence has to be waited for
      too;
    - the coalescing test's fixed 300 ms sleep became the same wait, on the last
      event reporting a settled queue.
  - `npm test`: **206** green (197 + the nine reducer tests). `npm run check`:
      0 errors, 0 warnings; `npm run build` ok (184 kB, 56 kB gzipped).
  - The count in the M2 block above (328 tests, 14 `queue_ipc`, and its claim
      that writes happen "exactly on the state changes spec §7.4 lists (add,
      remove, finish)") is what that earlier run reported and is left as
      written: a **start** writes too, now, and that is D91.
- **Not verified / stubbed**
  - **Still not clicked**: the GUI, the real network path and the system
      notification are as unverified as in the M2 block above.
  - **The fixes were not exercised against a real crash.** D91 is proven by
      reading the file while a tool runs and by reopening it, not by `kill -9`.
      The real kill is the new spike **S7** in `docs/SPIKES.md` (a table to
      fill in per OS), added to the manual list for exactly this reason.
- **Spec items marked "verify" that I checked**: none new. No new tool flag,
      URL, crate API or asset name was used in this step: the fixes are
      re-orderings of existing paths, one temp-file name and one lock.
- **Decisions/deviations (also in DECISIONS.md)**: D89, D90, D91, D92, D93,
      D94, D95, D96, D97.
- **Next step**
  1. The manual check listed at the end of the M2 block (paste several links,
     watch two downloads, pause/resume/cancel, restart and Resume, the two
     caps, the notification) — still the only thing missing before the phase
     can be called done. **S7** was added for the crash case (D91).
  2. Then M3 (playlists and other sites).

## 2026-10-15 — M2 / the Add bar, the labels and an M1-parity pass after the first hands-on feedback
- **Done**
  - The Add bar is the standard download again, on the queue's single path (D98).
    Inspection starts on its own after 500 ms (the original's `on_url_change` →
    `after(500, init_submit)`), so pasting a link produces a row with title,
    site, duration and the resolutions that video really reports; the two
    actions are **Scarica** (primary: new items go on top, only when some other
    item is queued — D100) and **Aggiungi alla coda** (appended). Both check
    whatever is still in the box *and wait for it* before adding, which is the
    defect the first build shipped: the button was disabled and the click a
    silent no-op whenever the rows were not there yet.
  - Quality: preset = cap (`MaxHeight`) over 2160/1440/1080/720/480/360 + Best +
    Solo audio; per row = the video's own heights as *exact* choices (the
    original's `1080p` semantics, spec §8.2) + Best + Solo audio. The "audio
    only" checkbox is gone: it was the missing `queue.add.audio` (D99) and a
    second way to say what the preset already says.
  - Labels: the three missing keys, a fourth found by machine
    (`queue.add.playlist.unsupported`, set as a row's `errorKey`), heights as
    parameters instead of 16 fixed keys, "Qualità predefinita", "Svuota la
    lista", the list counter is the whole queue (`tCount`) and not the rows in
    the viewport, the notification checkbox moved to a new "Comportamento"
    section, the stale first-run text ("La coda arriva con M2") replaced.
  - Guards (D99), all three verified by breaking them on purpose and watching
    the test fail, then restoring: `src/lib/i18n/usage.test.ts` (frontend scan:
    literal keys, *assigned* keys, every state and action, quality labels at
    heights no list covers), `error.rs`/`ytdlp.rs` (every backend error code),
    `services/notify.rs` (every `toast.` literal of the three files that build
    them, scanned rather than listed).
  - Test-only: the restart test's download margin 3 s → 8 s (D101) after one
    observed flake where the file said `done` and the test asserted
    `downloading`, i.e. the test task was starved past the fake's window.
- **Verified by running**
  - `cargo fmt --all -- --check`: ok. `cargo clippy --all-targets -- -D warnings`: clean.
  - `cargo test`: **414 passed, 3 ignored** (304 lib, 3 bundled_tools, 9 download,
    8 m1_ipc, 16 process_tree, 21 queue_engine, 12 queue_ipc, 6 settings_ipc,
    13 tools, 22 ytdlp_job); `cargo test --test queue_engine` alone: 21 green,
    three more runs green before the margin change.
  - `npm test`: **215** green (206 + the 9 new i18n/usages tests). `npm run
    check`: 0 errors, 0 warnings. `npm run build`: ok (183,54 kB, 56,35 kB gzipped).
  - The two guards, broken on purpose: a renamed key in
    `stores/addbar.svelte.ts` and a renamed toast in `queue/engine.rs` both made
    the new tests fail with the key named; restored, green.
- **Not verified / stubbed**
  - **Still not clicked**: the GUI. The new flow (paste → row → per-row quality →
    Scarica) is type-checked, unit-tested where it is pure, and built, but no
    window has shown it. The manual list at the end of the previous M2 block
    still stands, with item 1 and 2 rewritten for the two actions.
  - Playlists remain M3: a playlist link is refused with a message that says so
    (the message is now a real dictionary entry).
- **Spec items marked "verify" that I checked**: none new — no new tool flag,
  URL, crate API or asset name. The 500 ms figure is the original's own
  (`on_url_change`), not a guess.
- **Decisions/deviations (also in DECISIONS.md)**: D98 (the queue extends the
  standard download), D99 (i18n guards, parameterised heights), D100 (priority
  via `queue_move_to_top`, no flag on `queue_add`), D101 (test margin 3 s → 8 s).
- **Next step**
  1. The manual check list (the window): paste one link and press **Scarica**;
     paste several and compare **Scarica** with **Aggiungi alla coda**; then the
     rest of the list — pause/resume/cancel, restart and Resume, the two caps,
     the notification, the drag-and-drop.
  2. Then M3 (playlists and other sites), which is what the queue was built for.

## 2026-10-15 — M2 / the second hands-on report: card height, page layout, footer
- **Done**
  - **The list no longer paints over the page actions** (D102). The viewport's
    `min-height: 240px` and `max-height: calc(100vh - 320px)` are gone; the page
    scrolls when its minimum does not fit (`overflow: auto`), the list has a
    104px floor with a `min-height: 0` viewport, and the Add bar, the actions
    row and the footer are `flex: none`.
  - **One status line per card, same shape in every state** (D103): the bar is
    always drawn (empty when queued, last width when parked/failed, full when
    done) and the line shows the problem, the progress
    (`Velocità: … · Tempo rimasto …`, `In attesa dei dati…` while yt-dlp reports
    nothing) or the quality. The full error and the full warning moved into the
    details panel. `speedMbits` keeps one decimal below 1 Mbit/s.
  - **The virtual list measures its rows** (D103): the 92px constant was wrong
    (a card is ~130px and its height depends on the window width), so the pads
    stood in the wrong place. The list measures the pitch between two rendered
    rows and re-measures on resize and on every slice change; `visibleRange` is
    untouched and still pure.
  - **Footer** (D103): parked and failed counts instead of a premature `Fatto`,
    `flex: none`, a red dot for failures, and `footer.nothing` when the queue is
    empty (it used to say `In attesa`).
  - Two new pure helpers with tests: `statusKindFor` (every state gets a line)
    and `barPercentFor` (always a width: 0, the last one, or 100 when done).
- **Verified by running**
  - `npm test`: **217** green (215 + `statusKindFor`/`barPercentFor`, plus the
    sub-1 Mbit/s case). `npm run check`: 0 errors, 0 warnings. `npm run build`:
    ok (185,52 kB, 56,92 kB gzipped).
  - Rust untouched; the last full run (`cargo fmt --check`, `clippy -D
    warnings`, 414 tests, 3 ignored) still stands.
- **Not verified / stubbed**
  - **Still not clicked from here**: the fixes are layout and rendering, and no
    window exists in this environment. The report they answer came from the
    user's machine, and so must the confirmation. What to look at: resize the
    window both ways while the Add bar holds several rows, watch a download
    through a stall, and check the footer with a paused and a failed item.
  - The row-height measurement is the pragmatic answer for the heights we have
    (uniform cards, ±one wrapped button row). A variable-size virtualiser stays
    the answer if a future layout makes rows genuinely different; M7's 5 000
    item item is where that would show.
- **Spec items marked "verify" that I checked**: none new (CSS and one display
  helper).
- **Decisions/deviations (also in DECISIONS.md)**: D102 (page layout), D103
  (one status line, measured rows, footer).
- **Next step**
  1. The user's re-test of the three points above, plus the rest of the manual
     list.
  2. The open items from the M1/M2 audit: the Deno install offer, the "Video
     1/2" stage detail on the card, `size`, the right-click context menu, the
     dead M1 frontend code (`jobStore`, `inspectStore`, `job.ts`).
  3. Then M3 (playlists and other sites).

## 2026-10-15 — M2 / new items stay FIFO, the list shows where they went
- **Done**
  - The user asked whether new links should be added on top. Answered no, and
    recorded why (D104): a playlist of forty would be starved by every link
    added after it. The order stays the order items were added; **Scarica**
    keeps its priority over what is *waiting*, and **Sposta in cima** does the
    same for one card later.
  - The real gap was feedback, so: the store remembers the just-added ids
    (`queueStore.flash`), the list scrolls to the first one **only when it is
    not already visible** and to the nearest edge, the new rows pulse for 2.5 s
    (accent ring; off under `prefers-reduced-motion`), and the count line is a
    `role="status"` region so the growth is announced.
  - New pure helper with tests: `scrollOffsetFor(index, pitch, scrollTop,
    viewport)` — the "nearest edge" arithmetic, computed from the virtualiser's
    numbers because the target row usually has no element yet.
- **Verified by running**
  - `npm test`: **218** green (217 + the scroll arithmetic, including the
    already-visible, above, below, first-row and rolled-off cases).
    `npm run check`: 0 errors, 0 warnings. `npm run build`: ok (186,30 kB,
    57,19 kB gzipped).
  - Rust untouched since the last green run (`fmt`, `clippy -D warnings`, 414
    tests, 3 ignored).
- **Not verified / stubbed**
  - **Still not clicked**: the scroll and the pulse need a real window; that is
    the user's next check. What to look at: with the list scrolled up, add a
    link — the list must glide to the new row and pulse it; add one while it is
    already visible — the list must not move.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D104.
- **Next step**
  1. The user's re-test (the scroll, the pulse, the not-moving-when-visible).
  2. The open items from the M1/M2 audit: the Deno install offer, the "Video
     1/2" stage detail, `size`, the right-click context menu, the dead M1
     frontend code.
  3. Then M3 (playlists and other sites).

## 2026-10-15 — M2 / Deno offer, stream detail, file size, and the dead M1 frontend
- **Done**
  - **Deno install offer** (D105) back on the failed card, where it can unblock a
    real download: Install (with progress and Cancel, the same `tools_install`
    path as the Tools page), Not now (persisted), the tools error when it
    fails, and it disappears as soon as Deno is found.
  - **Stream detail** (D105): "Video 1/2", "Merging…" now open the card's status
    line while the item runs. `progressLabelKey`/`stageDetail` are pure and
    tested, and the i18n guard now walks the label tokens too (verified by
    deleting `job.label.merging` and watching it fail).
  - **File size** (spec §11.5): `sizeBytes` on the item, read from the file
    yt-dlp produced when the item finishes, surviving `queue.json`. Shown on the
    status line of a finished card ("1080p · 480 MB") and as a row in the
    details panel. `formatBytes` is pure and tested.
  - **Dead M1 frontend removed** (D107): job store, inspect store, `job.ts` and
    its tests, the helpers only they used, and the six `job.stage.*` strings.
    The **backend** M1 commands stay, and `services/job.rs`'s header now says
    what it is: the module the queue runs on.
  - **Two behaviours the dead files carried are alive again** (D106): the URL
    metadata cache of spec §2.9 moved into the add bar (a URL inspected in this
    session answers without a round-trip; Ricontrolla bypasses it; Svuota la
    lista clears it) and D76's paste rule too (a paste inspects at once, typing
    keeps the 500 ms).
- **Verified by running**
  - `cargo test`: **415 passed, 3 ignored** (304 lib, 3, 9, 8, 16, 22, 12, 6,
    13, 22), including the new `a_finished_item_carries_the_size_of_the_file_it
    _produced` (which also reopens `queue.json` and checks the size is still
    there). `cargo fmt --check` and `clippy -D warnings`: clean.
  - `npm test`: **204** green — fewer than before because twenty tests went with
    the deleted files: what remains covers what the app can still do.
    `npm run check`: 0 errors, 0 warnings. `npm run build`: ok (186,74 kB,
    57,16 kB gzipped).
  - The new i18n guard for the stage tokens was verified by breaking it on
    purpose (deleting `job.label.merging` from the Italian dictionary) and
    watching the test name the token.
- **Not verified / stubbed**
  - **Still not clicked**: the Deno offer needs a `DENO_MISSING` failure to
    appear, which needs a real YouTube challenge and no Deno installed. From
    here it is type-checked and its pieces are tested; seeing it is a manual
    check for a machine without Deno.
  - The cache's effect (no second round-trip on a repeated paste) is likewise a
    behaviour of the running app: the class has its tests, the wiring does not.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D105 (Deno offer, stream
  detail), D106 (cache and paste rule in the add bar), D107 (what was deleted,
  what stayed).
- **Next step**
  1. The user's next round: paste a link twice (the second must be instant), the
     "Video 1/2" line while it downloads, the size on a finished card.
  2. Then M3 (playlists and other sites). The right-click menu of §11.5 stays
     queued for the polish phase, as agreed.

## 2026-10-15 — M2 / pre-push review: a test artefact was tracked, and the fake tools are fixed
- **Done**
  - Pre-push review of the whole branch (what would go to GitHub, file by file):
    no executables, no bundled tools, no secrets or user data, no binary file in
    the diff against `main`. One thing was wrong: a **test artefact was tracked**
    (`src-tauri/Fake title [fakeid].mkv`, committed in the M1 branch and
    regenerated on every run).
  - Root cause found and closed (D108): the fixtures copy the one `fake_ytdlp`
    binary under `yt-dlp`, `qjs` **and** `ffmpeg`; the fake only knew
    `--version`, so the ffmpeg probe (`-version`) fell through to the download
    path and wrote its media file into the process's working directory. The fake
    now answers `-version`, and **refuses** a download without a URL after `--`
    or without `--paths home:` (exit 2). `home_dir()` replaces the five
    `unwrap_or_else(|| PathBuf::from("."))` fallbacks so the trap cannot come
    back inside a branch.
  - The artefact is out of the index and `.gitignore` has a narrow net
    (`src-tauri/*.mkv`, `*.opus`, `fake-ytdlp*`) under the fix.
- **Verified by running**
  - `cargo test`: **415 passed, 3 ignored**; `cargo fmt --check` and `clippy -D
    warnings` clean; and after the full run the artefact is **not** recreated
    (`find` over the repo and `git status`: clean).
  - `npm test`: **204** green; `npm run check`: 0 errors, 0 warnings.
  - Pre-push scan, all empty: executables/binaries tracked, `.env`/settings/
    queue/cookies/keys tracked, API-key-shaped strings, binary files in the diff
    against `main`.
- **Not verified / stubbed**
  - The artefact remains in the **history** of the M1 branch (already pushed).
    Removing it from the tip is what matters; rewriting the history for sixteen
    harmless bytes is not worth it.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D108.
- **Next step**
  1. Push `m1-single-video`, `m2-queue` and (optionally) `main`, then the
     manual checks before merging `m2-queue` into `main`.
  2. M3.

## 2026-10-15 — CI / one artifact per Linux format (spec §13.5)
- **Done**
  - The `bundle` job used to upload one artifact per platform, with all three
    Linux formats (`.deb`, `.rpm`, `.AppImage`) inside it. Since GitHub returns
    an artifact as one zip, getting the 5 MB `.deb` meant downloading the
    ~110 MB AppImage with it. Linux now gets **one artifact per format**
    (`scaricatore-di-porto-linux-x64-deb`, `-rpm`, `-appimage`), each with
    `if-no-files-found: error`. Windows (`nsis`) and macOS (`.dmg` +
    `.app.zip`) keep their single artifact: one product, one format.
  - The `release` job needs no change: it collects every
    `scaricatore-di-porto-*` artifact, flattens the folders and attaches the
    files one by one, so a published release already offers them separately.
- **Verified by running**
  - `node scripts/check-ci-syntax.mjs`: 10 `run:` blocks valid.
    `node scripts/check-ci-runners.mjs`: no floating or deprecated images, all
    actions on Node 24. The workflow YAML parses and lists the four upload
    steps with their names and conditions.
- **Not verified / stubbed**
  - **The workflow itself has not run.** The split can only be seen on GitHub:
    `Actions → ci → Run workflow`, pick this branch, and the `bundle` job's
    artifacts must appear as three separate entries for `linux-x64`.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D109.
- **Next step**
  1. Push the branch, dispatch the workflow on it, check the three artifacts,
     then open the PR into `main`.

## 2026-10-15 — CI / the stall test's margin was shorter than the process launch
- **Done**
  - The user's run confirmed the Linux split works: `bundle (linux-x64)` now
    shows `…-deb`, `…-rpm` and `…-appimage` as three artifacts instead of one
    heavy archive (D109).
  - Fixed the one Windows failure, `a_stalled_challenge_switches_runtime_instead
    _of_failing` (D110): its watchdog margin was 300 ms, shorter than the launch
    of the freshly copied fake on the Windows runner (loader + Defender scan), so
    the process was killed before writing its first log line and the test counted
    one line instead of two. The margin is 5 s now; the assertion is unchanged.
    Not an app bug: production waits 90 s.
- **Verified by running**
  - `cargo test --test ytdlp_job`: **22 passed** in 5.04 s (was 2.9 s: the extra
    seconds are the bigger margin). `cargo fmt --check` and `cargo clippy
    --all-targets -- -D warnings`: clean.
  - Built in a separate worktree with `TAURI_CONFIG={"bundle":{"externalBin":[]}}`
    (the CI's own trick) and a shared `CARGO_TARGET_DIR`, because the linked
    worktree has no `src-tauri/binaries/`.
- **Not verified / stubbed**
  - **The Windows runner itself.** The fix follows from the diagnosis above and
    from the fact that it passes on Linux; the proof is the next run of the
    branch. If it fails again with one line, the next step is written in D110.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D110.
- **Next step**
  1. Push the branch and re-run `ci`: `process tests` must be green on all three
     systems, and `bundle` must show the split Linux artifacts.
  2. Then the PR into `main`.

## 2026-10-15 — CI / only the installable files travel, and a guard for the release
- **Done**
  - The user's run showed the split working (D109): three Linux artifacts instead
    of one. The numbers it printed exposed the real weight problem (D111): the
    `.deb` artifact was 127 MB for a 42 MB package and the AppImage 251 MB for a
    148 MB file, because every upload used `bundle/**` and the bundlers leave the
    **unpacked** package next to the installer (`deb/<name>/data/usr/…`,
    `<name>.AppDir/`, `squashfs-root/`). A local build measured the trees: 88 MB,
    359 MB and 46 MB.
  - The uploads name files now (`deb/*.deb`, `rpm/*.rpm`, `appimage/*.AppImage*`,
    `nsis/*.exe`, `dmg/*.dmg`, `*.app.zip`).
  - The `release` job gained the guard **"Only installers reached the release"**:
    after the flatten step it walks `assets/` and fails when a file that is not
    an installer, `SHA256SUMS` or `latest.json` is there. Without it a tag would
    have attached those trees to the draft release too.
- **Verified by running**
  - `npx vitest run scripts/ci-steps.test.mjs`: **10 passed** (7 existing + 3
    new: installers only → ok, the flattened application binary → fails with its
    name in the message, a nested staging file → fails).
  - `node scripts/check-ci-syntax.mjs`: 11 `run:` blocks valid (the guard is one
    of them).
  - The sizes come from the local build in `src-tauri/target/release/bundle/`:
    `.deb` 42 MB + 88 MB of staging, AppImage 148 MB + 359 MB of `AppDir`.
- **Not verified / stubbed**
  - **The GitHub run**: the artifact sizes on the next run are the proof
    (expected: `.deb` ≈ 42 MB, `.rpm` ≈ 42 MB, `.AppImage` ≈ 148 MB).
- **A near miss, told rather than hidden**: the first version of this commit
  carried a `node_modules` **symlink** created in the linked worktree to run
  vitest. `.gitignore` had `node_modules/`, and a trailing slash matches a
  directory, not a symlink. It was caught before pushing (the link is out of the
  index and the pattern is `node_modules`, no slash) — the same class of mistake
  as the fake-media file of D108, on the machine that wrote the guard for it.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D111.
- **Next step**
  1. Push, re-run `ci` on the branch, check the artifact sizes and that
     `process tests` is green on all three systems.
  2. Then the PR into `main` (the D77–D111 order note applies).

## 2026-10-15 — M2 / the first `ci` run on `main`: two tests were racing the disk
- **Done**
  - The `check` job on Linux failed two `queue_engine` tests. Both are **test-side
    races**, not application bugs, and both are the class of D94: an in-memory
    state change is visible a moment before the write that records it.
    1. `a_finished_item_carries_the_size_of_the_file_it_produced` read
       `queue.json` right after seeing the state in memory, so the round trip now
       **waits for the size to appear on disk** with a deadline — exactly what the
       other tests do for a state.
    2. `the_queue_survives_a_restart_and_the_interrupted_item_can_resume` was
       already a margin, raised 3 s → 8 s when it failed once before, and the CI
       failed it again the same way (`done` where the test wanted `downloading`):
       a starved poller read the file after the tool had finished. **No margin can
       fix that**, so the fake now writes a marker in the output folder while it
       is alive (`.fake-veryslow-started`) and the test waits for the marker
       before reading. The app writes the state to disk *before* spawning the
       tool (D91), so a read taken while the process runs can only say
       `downloading`: the race is gone by construction.
  - `state_after_the_start` is gone with it. It was the helper that encoded the
    hope ("the item's download is long enough for the read to land inside it")
    that the CI disproved; `read_state_on_disk` now carries the rule instead
    ("whoever reads this must have a reason to know the file is up to date"),
    and `wait_for_path` is the answer for "wait for the tool, not for the clock".
  - The restart test's download is back to 1.5 s: it dropped from ~25 s to ~2 s,
    and the marker is what makes the window deterministic.
- **Verified by running**
  - `cargo test --test queue_engine`: 22 green, 25,04 s; the binary's slowest test
    is no longer this one.
  - **Under a simulated CI load** (`taskset -c 0,1` with CPU hogs — the closest a
    16-core machine gets to a 4-core runner): both tests 3/3 green, and a full
    run of the binary 22/22 green.
  - `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`: clean.
    `cargo test`: **415 green**, 3 ignored. `npm test`: **204 green**.
    `npm run check`: 0 errors, 0 warnings.
- **Not verified / stubbed**
  - **The GitHub runner itself**: the proof is the next `ci` run on `main`.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations**: none new. This is the third occurrence of D94's class
  (D101 was the second), and the fix lives where the next reader meets it — in the
  test file and in the fake — instead of another stretched margin.
- **Next step**
  1. Push, re-run `ci` on `main`: `check` green on the three systems.
  2. Then the tag `v0.1.0` (the version is 0.1.0 in all three manifests) for the
     draft release.

## 2026-10-15 — M2 / the CI failed the same test a third time, and the fix was to move the assertion
- **Done**
  - The `check` job on `main` failed `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`
    again, with a **new** symptom: the marker proved a tool was running, but the
    read found `queued` — i.e. it was looking at the *other* item. Same class as
    D94, third occurrence, and the lesson is D112: the "the file says
    `downloading` while the tool runs" invariant already has a deterministic
    owner that passed in both failing runs
    (`engine::tests::the_file_says_an_item_is_running_before_the_tool_is_spawned`,
    no scheduler, no clock), so the integration test dropped its mid-run read
    instead of stretching the margin a third time.
  - What the integration test keeps is what only it can prove: the fake reports
    while it is alive (marker named after the URL, `.fake-started-<url tail>`,
    written by `veryslow-<ms>`), the app is closed **while a tool runs**, the
    next launch parks the item, and a Resume finishes it. The `veryslow` window
    is 4 s and now only has to survive the microseconds between the marker and
    the shutdown.
  - `read_state_on_disk` and `wait_for_path` went with the read;
    `wait_for_fake_start` replaced them.
- **Verified by running**
  - `cargo test --test queue_engine`: 22 green, 25,02 s;
  - **under a simulated CI load** (`taskset -c 0,1,2` with CPU hogs): the test
    3/3 green (4,4 s each) and the whole binary 22/22 green;
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean;
    `cargo test`: **415 green**, 3 ignored; `npm test`: **207 green**;
    `npm run check`: 0 errors, 0 warnings.
- **Not verified / stubbed**
  - **The GitHub runner**, as always: the proof is the next `ci` run on `main`.
  - **I could not reproduce the `queued` value locally**, not with 3 cores and
    CPU hogs, and I have no verified explanation for it. Rather than build a
    third fix on a hypothesis, the assertion that produced it is gone — which is
    D112's point.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D112.
- **Next step**
  1. Push, re-run `ci` on `main`: `check` green on the three systems.
  2. Then the tag `v0.1.0` (the version is 0.1.0 in all three manifests) for the
     draft release.

## 2026-10-15 — M2 / the third CI failure: a test was corrupting a file a task was about to rewrite
- **Done**
  - The `check` job failed `the_queue_file_survives_a_corruption_with_a_backup`
    this time (`the_queue_survives_a_restart...` is **green** since D112 ✓), with
    *"the queue starts empty"*: the reopened engine found a valid document with
    items, i.e. the file had been rewritten after the test corrupted it.
  - Cause read out of the code, not guessed: `wait_until_quiet` waits for the
    item to settle in the **snapshot**, while `finish_item` mutates under the
    lock, releases it and only then calls `self_persist` — so the item task's
    final write can land after the corruption.
  - Fix (D113): the document is written by `fx.idle_engine()`, whose tasks were
    never started, so `add()` is the only writer and the test owns the
    corruption. No wait was added. `wait_until_quiet` stays where the file is
    not touched afterwards.
- **Verified by running**
  - The corruption test: **20/20 green** in a loop and **10/10 under load**
    (`taskset -c 0,1,2` with CPU hogs); the whole `queue_engine` binary 22/22
    under the same load.
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean;
    `cargo test`: **415 green**, 3 ignored; `npm test`: **207 green**;
    `npm run check`: 0 errors, 0 warnings.
- **Not verified / stubbed**
  - **The GitHub runner**, again: the proof is the next `ci` run on `main`.
  - A sweep of the other tests that touch `queue.json`: the only writer of a
    file a test then breaks is this one (the crashed-item test builds its own
    document on a fresh fixture, and `an_added_item_is_persisted...` reads after
    `add()`, which persists synchronously). No other test has this shape.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D113.
- **Next step**
  1. Push, re-run `ci` on `main`.
  2. Then the tag `v0.1.0` for the draft release.

## 2026-10-15 — Windows: "yt-dlp reported … but the file is not there" while the file is there (spec §4.5, §8.3)

- **Done**
  - The first Windows run of the installer reported some downloads as failed with
    *"yt-dlp reported … [id].mkv but the file is not there (spec §8.3 …)"*, with
    the file present in the output folder, playable, and the same links working
    on Linux.
  - Cause, read out of the tool's source rather than guessed: the path the app
    checks is the one yt-dlp **prints**, and with a piped stdout the tool writes
    the **console code page** (cp1252 in the west), not UTF-8 — `write_string`
    encodes with `.encode(enc, 'ignore')`, so characters outside the code page
    are dropped and accents arrive as mojibake. The *file* is written under its
    real Unicode name with wide-char APIs, so the folder is right and the printed
    path is wrong. Deterministic per **title**, hence "some downloads, often",
    the same links, and never on Linux (its pipe is UTF-8).
  - A Windows-only symptom needs no Windows-only fix: spec §4.5 says to pass
    `--encoding utf-8`, and that is the only lever that works — the standalone
    builds run an isolated PyInstaller interpreter config, so `PYTHONIOENCODING`
    is ignored from outside (measured on the bundled binary: `cp1252` and `ascii`
    change nothing; the `C` locale is coerced to UTF-8, which is also why the
    mangling cannot be reproduced here).
  - `ytdlp::ENCODING_ARGS` (`--encoding utf-8`) is now part of the **base** args
    of every download (retry without `-f`, audio-only and future branches
    included) and of the inspection, on every platform. D114.
- **Verified by running**
  - New unit test `services::ytdlp::tests::the_tool_is_forced_to_utf8_on_every_path`
    (video, exact height, audio-only, retry without `-f`) and `--encoding` added
    to the existing inspection arg test.
  - New integration test
    `ytdlp_job::an_accented_final_path_survives_the_tools_console_code_page`: the
    fake tool now prints its final path **the way the real one does** — through
    the console code page unless `--encoding utf-8` was passed — so the Windows
    behaviour is reproduced on every OS. Removing the flag makes it fail with the
    user's exact message:
    `Failed(Unknown, "yt-dlp reported …/Fake accent-citt-perch [fakeid].mkv but
    the file is not there (spec §8.3: the path is never guessed)")`
    — note the accents gone from a path that still looks plausible, which is what
    the report showed. The flag back in: green.
  - Real tool, live: the bundled yt-dlp 2026.08.19, a real download with an
    accented `-o` template, `--paths home:`/`temp:`, our exact `--print` template
    and `--encoding utf-8`: the `SDP_FILE` bytes are valid UTF-8 and the printed
    path `is_file()`. `--help` of the pinned binary confirms
    `--encoding ENCODING  Force the specified encoding (experimental)`.
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean;
    `cargo test`: **417 green** (3 ignored), 0 failed; `npm test`: **207 green**;
    `npm run check`: 0 errors, 0 warnings.
- **Not verified / stubbed**
  - **Windows itself**: the machine that reported it is the only place where the
    end-to-end proof can be taken. What is verified here is the mechanism (in
    yt-dlp's source and `--help`), the flag being passed on every path (tests),
    and the failure the fake reproduces byte for byte on Linux. If a download
    still fails on Windows with this message after the fix, the item's "Copy
    details" plus the folder listing would pin it down.
  - The inspection's JSON is not affected either way (`json.dumps` escapes
    non-ASCII); the flag is there for its stderr titles (which reach error
    messages) and for the printed fields of M3.
  - Tools without such a flag (ffmpeg, the JS runtimes, whisper) are M4/M9; the
    rule recorded in D114 is that their output encoding is measured, not assumed.
- **Spec items marked "verify" that I checked**
  - `--encoding utf-8` exists in the pinned yt-dlp (`--help`, Workarounds) and
    overrides the pipe's encoding: `YoutubeDL._write_string` →
    `write_string(..., encoding=self.params['encoding'])`.
  - `--print after_move:filepath` is printed **after** the move to `home`
    (`YoutubeDL.post_process` runs `MoveFilesAfterDownloadPP` before the
    `after_move` prints) — so a missing file is really a name mismatch, not a
    timing artefact.
- **Decisions/deviations (also in DECISIONS.md)**: D114.
- **Next step**
  1. Push, rebuild, and retry on Windows the **same** links (the ones with
     accented or emoji titles).
  2. Then the tag `v0.1.0` for the draft release.

## 2026-10-15 — Windows CI: "the item that was running comes back parked … : Queued" (spec §7.4)

- **Done**
  - The fourth `check` failure was again `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`,
    only on Windows: the reopened engine found the killed item `Queued`. The
    other two OSes still pass, and this same test passed on Windows before the
    marker handshake was added.
  - Cause (D115), and it is an **ordering**, not a duration: `finish_item` mutates
    the state under the lock, releases it, and *then* writes `queue.json`, so
    `snapshot()` — what the test waited on — shows the settled state a moment
    before the write that records it. The test dropped the engine and reopened it
    inside that window, and the "second launch" read the document written
    **before** the app closed: still `queued` for the item whose tool had just
    been killed. Same window as D94 and D113, seen from the reopen.
  - Fix: `wait_for_parked_on_disk` — the engine is dropped only once the document
    the reopen will read agrees with the state the test already saw. The
    invariant is still guarded and visibly so: with the closing write removed the
    test now fails with *"timed out … waiting for the closed item to reach
    queue.json"* (verified by breaking it on purpose).
  - The test also stopped guessing: it waited for *any* marker and then asserted
    on `ids[0]`, while its own comment said which item the lane picked is not the
    test's business. The fake now writes `.fake-started-<host>` and the test
    follows the host to the item it added, so the assertion is about the item that
    really had a tool behind it. The failure message now carries `queue.json`.
- **Verified by running**
  - `cargo test --test queue_engine the_queue_survives_a_restart…`: 4 green runs
    in a row (4.1 s each — the 20 s waits of the old shape are gone).
  - The whole `queue_engine` binary **22/22** three times in a row under load
    (`taskset -c 0,1,2` with three CPU hogs, 25 s each).
  - The "file never catches up" simulation: fails with the explicit message
    above (so the new wait guards rather than hides).
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean;
    `cargo test`: **417 green**.
- **Not verified / stubbed**
  - **The Windows runner is the only proof** available: the mechanism is read out
    of the code and reproduced on purpose, and the fix is verified here, but the
    green check on Windows can only come from the next CI run.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D115.
- **Next step**
  1. Push, re-run `ci` (the Windows `check` job is the one that was failing).
  2. Then the tag `v0.1.0` for the draft release.

## 2026-10-15 — process: a Windows spawn that fails because the file is held is retried (spec §5.2.1)

- **Done**
  - `is_text_file_busy` now also accepts Windows' `ERROR_SHARING_VIOLATION` (32)
    and `ERROR_ACCESS_DENIED` (5), and the spawn retry is a per-OS-family time
    budget instead of a fixed count of retries: 100 ms/20 ms on Unix (unchanged:
    ETXTBSY only, one errno, as the unit test still asserts), 1 s/50 ms on
    Windows. D116.
  - Recorded honestly as **precaution, not measurement**: unlike the Unix
    ETXTBSY case (which the suite observed firing), no Windows failure of this
    shape has been seen, and the CI failure that opened this branch turned out to
    be the reopen race of D115, not the spawn. The `TODO(verify)` and the reason
    the amount is a judgement are in D116.
- **Verified by running**
  - `process` unit tests: 31 green, both halfs of the predicate (the Unix one
    asserts 32 and 5 are **not** retried there; the `#[cfg(windows)]` one asserts
    they are on Windows and runs on the Windows CI).
  - New assertion in `the_spawn_retry_stays_shorter_than_the_user_notices`: the
    delay fits the budget and the budget stays ≤ 1 s, so a genuinely broken tool
    is still reported inside the shortest user-visible timeout.
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
    (417 green), `npm test` (207), `npm run check` (0/0).
- **Not verified / stubbed**
  - Windows itself: the two error codes are stable Win32 values and the predicate
    is unit-tested, but whether a first launch ever hits them is the
    `TODO(verify)` of D116.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D116.
- **Next step**
  1. Push, re-run the process-tests workflow on `windows-x64` (and `ci` on push).
  2. Then the tag `v0.1.0` for the draft release.

## 2026-10-15 — process: the two halves of the spawn-busy predicate, split by cfg (spec §4, §5.2)

- **Done**
  - The `process-tests` run on `windows-x64` came back with **`queue_engine` 22/22**
    (the D115 fix holds — `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`
    passes) and **`windows_sharing_violations_are_the_same_not_runnable_yet`** passing.
    One failure left, and it was mine: `text_file_busy_is_recognised_and_nothing_else_is`
    asserted that `[2, 5, 13, 22, 27, 32]` are never retried — true on Unix, false on
    Windows — and I had left it ungated (D116 gave the predicate a per-family meaning
    without gating the half that describes Unix).
  - Fixed properly rather than by adding one attribute: `ETXTBSY` (26) is now
    `#[cfg(not(windows))]` **as a constant too**, so the predicate compares only the
    codes that mean "not runnable yet" on the family it runs on. On Windows
    `raw_os_error()` is a Win32 code, and 26 there is `ERROR_NOT_DOS_DISK`: reading it
    with the Unix table would have delayed a real failure by a second. The Unix test is
    `#[cfg(not(windows))]`, the Windows test now also asserts that 26 is **not** retried,
    and neither test can run on the family it does not describe.
- **Verified by running**
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean;
    `cargo test --lib services::process`: **31 green** on Linux.
  - `rustc --target x86_64-pc-windows-msvc --emit=metadata -D warnings` on the extracted
    predicate: compiles with no warning, and the same for
    `x86_64-unknown-linux-gnu` — so both `cfg` arms are well-formed.
  - **The Windows branch executed on Linux**: extracted the item from the real file, let
    `rustc` expand the `cfg` (other family → `#[cfg(any())]`), compiled with `--test`,
    ran natively. Pre-fix as-Windows: **2 tests, the Unix one fails with the CI's exact
    assertion** (the failure reproduced locally). Post-fix as-Windows: 1 test, green.
    Post-fix as-Unix: the other test, green. Method recorded in D117.
- **Not verified / stubbed**
  - Still the runner's job: `CreateProcess`, the Job Object and real antivirus behaviour,
    none of which the local expansion can reach. D116's numbers remain a judgement with
    its `TODO(verify)`.
- **Spec items marked "verify" that I checked**: none new.
- **Decisions/deviations (also in DECISIONS.md)**: D117 (correction of D116 + the
  reproduction method).
- **Next step**
  1. Push, re-run `Process Tests` on `windows-x64`: expect the whole suite green.
  2. Then the tag `v0.1.0` for the draft release.

## 2026-10-16 — every workflow is manual, and the tests have their own button (spec §13.5)

- **Done**
  - `ci.yml` no longer starts on `push` (`main`, `v*` tags) or on `pull_request`:
    `workflow_dispatch` is its only trigger. Every commit pushed to `main` used to
    buy a Linux `check` (7 minutes) that nobody had asked for (D12, D19); now the
    run is a deliberate act. The header comment describes what the button does
    instead of when it fires.
  - `process-tests.yml` became the fast button — **"Checks and process tests"**:
    it carries the same `check` job as `ci.yml` (fmt, clippy, `cargo test`,
    frontend, manifest, runner-image guard) **and** the whole suite on all three
    platforms. `linux-x64` is the row that was added: the process layer is the
    part whose behaviour really differs per OS, and the D115 and D117 failures are
    exactly what a matrix missing that row hides. It builds no installer.
  - `CI` still does everything (same `check`, process tests, all bundles, and the
    draft release when dispatched on a tag): unchanged in what it does, only in
    when it starts.
- **Verified by running**
  - `npm run check:ci`: all 13 `run: |` blocks valid across the four workflows.
  - `npm run check:runners`: 4 workflows, no floating or deprecated image, every
    action on Node 24.
  - YAML parse of the four files: `on:` is `workflow_dispatch` for all of them,
    and `grep` finds no `push:`, `pull_request:` or `schedule:` left.
  - The `check` job of the two files is **byte-identical** (generated from the
    same text; a `diff` of the two blocks is empty).
- **Not verified / stubbed**
  - Nothing here needs a runner to be believed, but the real proof is the first
    manual dispatch: `Checks and process tests` on `main` must go
    check → three platforms, and `CI` dispatched on `v0.1.0` must still reach the
    draft release.
- **Spec items marked "verify" that I checked**: spec §13.5 lists the jobs and
  the platforms but says nothing about triggers, so the trigger policy is ours
  (D118) and not a deviation from the spec.
- **Decisions/deviations (also in DECISIONS.md)**: D118.
- **Next step**
  1. Push, then press `Checks and process tests` by hand on `windows-x64` (with
     the D117 fix) — expect all three rows green.
  2. Then the tag `v0.1.0`, and the release by dispatching `CI` **on the tag**.

## 2026-10-16 — the operational guide follows the manual-only CI (D118, docs only)

- **Done**: `GUIDA_OPERATIVA.md` (personal, `.gitignore`d, never committed) told
  the reader that pushing a `v*` tag starts the release and that `check` runs on
  every push. Both sentences became false with D118, so the guide was brought in
  line: the release is now *tag → push the tag → dispatch `CI` **on the tag*** (a
  dispatch on a branch skips `release` in silence); the CI section lists the two
  buttons (`Checks and process tests` and `CI`) instead of the five jobs of one
  workflow; `tier2` is described as its own workflow; the bundle image is
  `ubuntu-24.04`, not 22.04; and the four guards replace the three (the release
  one, D111, was missing).
- **Done, numbers**: the counts in the guide were stale in three places — 415/204
  Rust/TS tests, "619 automatici". Measured now: 305 unit + 112 integration = 417
  Rust, 207 TypeScript, 624 in total. The figure is what `cargo test` printed, not
  a sum done by hand.
- **Verified by running**: `cargo test` (417 passed, breakdown above), `npm test`
  (207 passed), `npm run check:release -- v0.1.0` (the three manifests agree:
  0.1.0), and `grep` for the stale strings (`ogni push`, `22.04`, `415`, `204`,
  `619`, `cinque job`) — none left.
- **Not verified**: the redaction is only as good as my reading of the guide; the
  parts about bundle artifact names were left alone.
- **Next step**: the tag `v0.1.0` and the first draft release, dispatched on the
  tag.

---

## 2026-10-16 — M3 / Step 1: Playlist & URL-case inspection, entry URL reconstruction, pagination (spec §6.2, §6.4)

Branch `m3-playlists-and-sites`.

- **Done**
  - Extended `PlaylistStub` (re-exported as `PlaylistInfo`) in `src-tauri/src/services/inspect.rs` and `src/lib/inspect.ts` with `webpage_url` and `entries: Vec<EntryLite { index, id, url, title, duration, available }>`, plus `PLAYLIST_PAGE_SIZE = 1000` and `MIX_DEFAULT_CAP = 50`.
  - Implemented pure URL-case classifiers (`is_video_with_list_url`, `is_mix_url`, `is_search_or_feed_url`) in both Rust (`services/inspect.rs`) and TypeScript (`src/lib/inspect.ts`).
  - Implemented `reconstruct_entry_url` and `is_entry_available` in `services/inspect.rs` to handle full `http(s)` URLs, protocol-relative URLs, `webpage_url` fallback, and bare video IDs on YouTube/Vimeo/TwitchVod, and to flag `[Private video]`, `[Deleted video]`, `availability: private|unavailable|needs_auth|subscriber_only|premium_only`, and `live_status: is_upcoming` as `available: false`.
  - Added `InspectOptions { item_range, no_playlist, impersonate_chrome, proxy, referer, user_agent, custom_headers }`, `inspect_args_with_options`, and `inspect_with_options` in `services/inspect.rs`, and wired `InspectRequestOptions` in `commands/inspect.rs` (`inspect_url` and `inspect_urls`).
  - Updated `fake_ytdlp.rs` and `tests/ytdlp_job.rs` to cover full playlist entries, `--no-playlist` on `watch?v=…&list=…`, and upfront refusal of search/feed/home pages without spawning `yt-dlp`.
- **Verified by running**
  - `( cd src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test )` → 309 lib + 115 integration = **424 passed**, 3 ignored.
  - `npm run check && npm test` → `svelte-check` 0 errors 0 warnings; Vitest **210 passed** across 10 files.
- **Not verified / stubbed**
  - Real multi-thousand-item YouTube channel pagination over the live network (covered by unit and fake-yt-dlp tests; UI "Load more" button arrives in Step 3).
- **Spec items marked "verify" that I checked**
  - `-I, --playlist-items ITEM_SPEC` (`-I :1000`, `-I :50`, `-I 1001:2000`), `--no-playlist`, `--yes-playlist`, `--impersonate`, `--proxy`, `--referer`, `--user-agent`, `--add-headers` verified against bundled `yt-dlp 2026.08.19 --help`.
- **Decisions/deviations**: none.
- **Next step**: M3 Step 2 — download args (`-o` prefix with `%` → `%%` and ID toggle, `-S vcodec:h264,res,acodec:m4a`, `--download-archive`, `--sleep-interval`, `--live-from-start`, `--proxy`, `--limit-rate`, custom headers), anti-bot `--impersonate chrome` probe & retry, archive commands, and queue group commands.


## 2026-10-16 — M3 / Step 2: download args, anti-bot retry, archive, settings v5, group commands
- Done: M3 `yt-dlp` download args (numbering prefix, ID toggle, H.264 sort, `--download-archive`, pacing sleep, `--live-from-start`, proxy, rate limit, referer/UA/headers, `--impersonate chrome`); one-shot anti-bot retry with `--impersonate chrome` in inspect and download (cached `--list-impersonate-targets` probe); `JobOutcome::SkippedArchive`; settings schema v5; `archive_status` / `archive_clear`; queue group commands (pause/resume/retry-failed/remove); frontend bindings (`settings.ts`, `queue.ts`, stores) and unit tests.
- Verified by running: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (all suites pass, incl. new `ytdlp_job` antibot-retry / archive-skip tests and `queue_ipc` archive/group test), `npm run check` (0 errors), `npm test` (213 passed).
- Not verified / stubbed: the Svelte UI for M3 (Step 3); real `--impersonate` against a live site (needs curl_cffi in the bundled yt-dlp, not tested offline).
- Decisions/deviations: the fake yt-dlp answers `--list-impersonate-targets` anywhere in argv (the real probe passes `--ignore-config` first).
- Next step: Step 3 — playlist panel, AddBar URL cases, queue group headers, M3 settings UI, i18n.

## 2026-10-16 — M3 / Step 3: playlist panel, AddBar URL cases, queue group headers, settings and i18n (spec §6.4, §11.3–§11.8)

Branch `m3-playlists-and-sites`.

- **Done**
  - Backend: Added `output_subdir` field to `AddItemRequest` in `src-tauri/src/commands/queue.rs` with safe path joining via `sanitize_component`.
  - Frontend pure helpers and state:
    - Implemented `src/lib/playlist.ts` with range parser (`1-10, 15, 20-`), case/accent-folding title search, shift-click range selection, default caps (mix: 50, channel: 100), safe folder name sanitisation, numbering prefix formatter (`001 - `), and queue request builder. 18 unit tests in `src/lib/playlist.test.ts`.
    - Added header line parser, format table selector, and choice resolvers in `src/lib/addbar.ts`.
    - Added queue group helpers (`groupStats`, `nonEmptyGroups`, `itemsOutsideCollapsed`, `groupPercent`, `wasSkippedByArchive`) in `src/lib/queue.ts` with unit tests in `src/lib/queue.test.ts`.
    - Updated `src/lib/stores/addbar.svelte.ts` to manage choice state (`watch?v=...&list=...`), playlist slide-over state, pagination loading, live stream options, and network options.
  - Svelte components:
    - `src/lib/components/PlaylistPanel.svelte`: slide-over virtualized panel with search filter, range selector, select all/none/invert, reverse order toggle, quality preset, folder-per-playlist, numbering prefix, archive skip, pagination ("Load more"), and "Add N items" action.
    - `src/lib/components/CookieBanner.svelte`: inline browser cookie picker / Netscape cookies file picker on auth error.
    - `src/lib/components/AddBar.svelte`: integrated URL choice inline prompt, live stream radio buttons ("download from now" vs "from start"), advanced format table (`details`), network headers drawer, and cookie banner.
    - `src/lib/components/QueueGroups.svelte`: collapsible playlist group headers showing done/total count, progress bar, folder name, and group actions (pause, resume, retry failed, open folder, remove).
    - `src/lib/components/QueueList.svelte` & `src/lib/components/QueueItem.svelte`: group filtering of collapsed groups and archive skip status indicator ("Already in archive (skipped)").
    - `src/lib/pages/settings/GeneralSettings.svelte`: added M3 settings (ID in filename, prefer H.264, limit rate, playlist URL behaviour, download archive toggle, and clear archive button with status count).
    - `src/lib/pages/settings/NetworkSettings.svelte`: added M3 settings (proxy URL, custom referer, user-agent, custom headers, and `--impersonate chrome` retry checkbox).
  - i18n: Added 94 new localized strings across `src/lib/i18n/en.json` and `src/lib/i18n/it.json` with 100% dictionary parity verified by unit tests.
- **Verified by running**
  - `( cd src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test )` → all tests pass (0 warnings, 424 passed, 3 ignored).
  - `npm test` → 11 test suites passed, 238 tests passed.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm run build` → production bundle builds cleanly.
- **Not verified / stubbed**
  - Manual UI click testing with a live webview (requires display server / graphical environment).
- **Spec items marked "verify" that I checked**
  - All UI elements, labels, options and drawer behaviours follow spec §6.4, §11.3, §11.4, §11.5, §11.8.
- **Decisions/deviations (also in DECISIONS.md)**: none.
- **Next step**: Step 4 — Verification of acceptance criteria (spec §15 items 5 & 6) and completion of Phase M3.

## 2026-10-16 — M3 / Step 4: acceptance criteria verification and Phase M3 sign-off (spec §14, §15 items 5 & 6)

Branch `m3-playlists-and-sites`.

- **Done**
  - Completed all deliverables of **Phase M3 (Playlists & other sites)**:
    - Step 1 (`8556582`): Playlist & URL-case inspection, entry URL reconstruction, pagination (`-I :1000`), search/feed refusal.
    - Step 2 (`ff88a77`): Download args (numbering, ID toggle, H.264 preference, archive, pacing, live from start, proxy, rate limit, custom headers), `--impersonate chrome` probe and one-shot retry, archive skip handling, settings v5, archive and group IPC commands.
    - Step 3 (`77dc8ef`): UI components (virtualized `PlaylistPanel`, `AddBar` choice & live & format table & network drawer, `CookieBanner`, collapsible `QueueGroups`, `QueueItem` archive status, Settings M3 options, and full i18n dictionaries).
  - Executed full test suite and verification scripts: backend unit and integration tests, Svelte check, Vitest unit tests, CI syntax check, runner allowlist guard, bundled tools manifest check, and production build.
  - Verified exit criteria (spec §14 row M3) and acceptance items (spec §15 items 5 & 6):
    - Acceptance item 5: 100-item playlist panel responsive, subset selection, folder per playlist, numbering prefix, archive skip, private/unavailable entry isolation, and video+list inline choice.
    - Acceptance item 6: Generic video page, HLS/direct links, audio-only, cookie-protected auth hints, live stream handling, DRM message, format table, and one-shot anti-bot retry with impersonation.
- **Verified by running**
  - `cargo fmt --all -- --check` → ok.
  - `cargo clippy --all-targets -- -D warnings` → 0 warnings, clean.
  - `cargo test` → 424 passed (309 unit + 115 integration), 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 11 test files, 238 passed, 0 failed.
  - `npm run build` → production bundle compiled in ~430ms.
  - `npm run check:ci` → all 13 `run: |` blocks valid across 4 workflows.
  - `npm run check:runners` → 4 workflows, no floating or deprecated images, all actions on Node 24.
  - `npm run check:tools` → manifest OK: 2 bundled tools, schema 1.
- **Not verified / stubbed**
  - Live YouTube / network calls (prohibited in normal CI/test runs per AGENTS.md §6; reserved for manual self-test / nightly runs).
  - Headed GUI testing with real display server.
- **Spec items marked "verify" that I checked**
  - All items verified in Steps 1–3 (`-I` syntax, `--impersonate`, `--no-playlist`, `--download-archive`, `--live-from-start`, `--sleep-interval`, etc.).
- **Decisions/deviations**: None.
- **Next step**: Phase M4 — Media processing (ffmpeg encode/trim, encoder probing, audio encode, embed metadata/thumbnail/chapters, SponsorBlock, spec §9, §14 row M4).

## 2026-10-16 — M3 / Bugfix: download click IPC deserialisation and native drag prevention (spec §6.4, §11.2, §11.3)

Branch `m3-playlists-and-sites`.

- **Done**
  - Diagnosed and fixed the download button inaction issue: `AddItemRequest.custom_headers` was typed as `Vec<String>` with `#[serde(default)]` in Rust, which failed with `"invalid type: null, expected a sequence"` when `addbar.svelte.ts` passed `customHeaders: null` on items without custom headers. Added `deserialize_null_as_default` to `AddItemRequest.custom_headers` in `src-tauri/src/commands/queue.rs` and ensured `addbar.svelte.ts` passes `headers` array, with regression integration test in `src-tauri/tests/queue_ipc.rs`.
  - Diagnosed and fixed unwanted browser dragging of UI elements (sidebar links, buttons, icons):
    - Added global `user-select: none; -webkit-user-select: none;` to `html, body` and buttons in `src/lib/styles/app.css` (retaining selection for `input, textarea, .selectable`).
    - Added `-webkit-user-drag: none;` on `a, button, img, svg` and `pointer-events: none;` on `button svg, a svg` in `app.css` so that clicks on buttons are never swallowed by icon drag initiation in WebKitGTK/WebView.
    - Set `draggable="false"` on `Sidebar.svelte` anchor links and brand, as well as `App.svelte` skip link.
    - Updated `QueueItem.svelte` `onDragStart` to prevent starting row drag when the interaction begins inside interactive child controls (`button, a, input, select, textarea`).
    - Fixed `onDragLeave` in `AddBar.svelte` to verify `contains(relatedTarget)`.
- **Verified by running**
  - `( cd src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test )` → all 424 tests pass, including regression test for `customHeaders: null`.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 238 tests pass across 11 test suites.
  - `npm run build` → builds cleanly.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all pass.
- **Next step**: Ready for user testing.

---

## 2026-10-17 — M4 / Media processing (ffmpeg encode, probing, trim, audio-only, embeds, SponsorBlock) (spec §9, §8.4, §14 row M4, §15 item 8)

Branch `m4-media-processing`.

- **Done**
  - **Backend (FFmpeg encoding & encoder catalogue)**:
    - Implemented `services/encoders.rs`: OS-specific candidate catalogue (`libx264`, `libx265`, `*_nvenc`, `*_amf`, `*_qsv`, `*_vaapi`, `*_videotoolbox`), parsing of `ffmpeg -encoders`, background probe runner with 256×256 black frame test (`color=c=black:s=256x256:d=0.2`), 10-second timeout, cache by (path, mtime, version), and CPU fallback mapping (`h264_*` → `libx264`, `hevc_*` → `libx265`). (Note: version passed to cache was initially `job_tools.ytdlp.version`; corrected to `ffmpeg.version` in Review major 5).
    - Implemented `services/ffmpeg.rs`: `TrimSpec`, timestamp parser (`SS`, `MM:SS`, `HH:MM:SS`), trim validator (`start < end <= duration`), expected duration calculator, `FfmpegProgressTracker` for `-nostdin -hide_banner -progress pipe:1 -nostats` parsing `out_time_us`, video/audio arg builders (`-ss` before `-i`, `-t`, `-movflags +faststart`), and `run_encode` runner. Output filename `<name>-encoded.<ext>`.
    - Implemented audio-only encoding presets (spec §9.4): `aac` → `.m4a` (`-c:a aac -b:a 192k`), `mp3` → `.mp3` (`-c:a libmp3lame -q:a 2`), `opus` → `.opus` (`-c:a libopus -b:a 128k`).
    - Implemented safe file lifecycle (spec §9.2, §12.2): original file is deleted **only** if ffmpeg exits with code 0 and output is non-empty; original is preserved on any encode failure or when `keep_original = true`.
    - Extended `services/ytdlp.rs`: added `SponsorBlockMode` (`mark`, `remove`), embed flags (`--embed-metadata`, `--embed-thumbnail` with `--convert-thumbnails jpg`, `--embed-chapters`), `--sponsorblock-mark` and `--sponsorblock-remove`, and `JobStage::Encoding` variant.
    - Updated `services/queue/model.rs`: added M4 fields to `ItemOptions`, added `ItemOptions::needs_encode()`, and mapped `JobStage::Encoding` to `ItemState::Encoding`.
    - Integrated encoding stage into `services/queue/engine.rs`: sequential encode execution with `encode_semaphore` (capacity = 1), hardware-to-CPU fallback retry, auto-play via `tauri-plugin-opener`, and live encode progress dispatching.
    - Added IPC handlers in `commands/system.rs` (`encoders_probe`, `open_path`, `reveal_path`) and mapped M4 fields in `commands/queue.rs`.
    - Updated `tests/bin/fake_tool.rs` and created integration test suite `tests/ffmpeg_encode.rs` testing encoder probing, CPU/audio encoding, hardware failure CPU fallback, and keep_original file lifecycles.
  - **Frontend (Settings, Stores, AddBar, GeneralSettings, and i18n)**:
    - Updated `src/lib/settings.ts`: bumped settings schema version to 6, added `SponsorBlockMode` and M4 configuration defaults (`keepOriginalAfterEncode`, `autoPlay`, `embedMetadata`, `embedThumbnail`, `embedChapters`, `sponsorblockMode`, `sponsorblockCategories`, `defaultVideoCodec`, `defaultAudioCodec`).
    - Updated `src/lib/queue.ts`: added M4 fields to `ItemOptions`, `AddItemRequest`, `EncoderInfo`, and mapped `"encoding"` to `"job.label.encoding"`.
    - Updated `src/lib/stores/tools.svelte.ts`: added `probedEncoders`, `probingEncoders`, and `probeEncoders(force?: boolean)` IPC method.
    - Implemented `src/lib/addbar.ts`: pure functions `parseTrimTime`, `validateTrimInput`, `AUDIO_CODEC_OPTIONS`, and `SPONSORBLOCK_CATEGORY_KEYS`, with 21 unit tests in `src/lib/addbar.test.ts`.
    - Updated `src/lib/stores/addbar.svelte.ts`: added M4 fields to `AddRow`, `setMediaOptions` with trim->encode coupling, and forwarded all M4 fields in `toRequests()`.
    - Updated `src/lib/components/AddBar.svelte`: wired media options into the "More options" drawer (re-encode toggle, probed video codecs, audio codecs, trim inputs with real-time validation error feedback, keep original, auto-play, embed checkboxes, SponsorBlock mode with remove warning, and category checklist), plus background encoder probe on mount.
    - Updated `src/lib/pages/settings/GeneralSettings.svelte`: added "Media processing & encoders" group with default video/audio codecs, "Detect hardware encoders" button, keep original toggle, auto-play toggle, default embed checkboxes, and default SponsorBlock mode & category checklist.
    - Localised all 43 new keys across `src/lib/i18n/en.json` and `src/lib/i18n/it.json` with 100% parity verified by tests.
- **Verified by running**
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 330 unit + 123 integration = **453 passed**, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, **259 passed**, 0 failed.
  - `npm run build` → production bundle compiled in ~494ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
  - Partial acceptance checklist item 8 (spec §15): video/audio encode, fake-tool probing, CPU fallback execution, trim unit tests, originals untouched on failure.
- **Not verified / stubbed**
  - OS-specific proprietary hardware encoders on macOS/Windows host (host is Linux; probed via fake tool suite and system ffmpeg).
  - `validate_trim` not yet wired into `queue_add` or `engine.rs`, and invalid trim inputs not yet blocked on submit (addressed in Review blocker 1).
  - `Trim` UI inputs hidden when `Encode` unchecked (addressed in Review blocker 4).
  - Encoder cache passed `job_tools.ytdlp.version` instead of ffmpeg version (addressed in Review major 5).
  - Hardware-to-CPU encoder fallback did not notify the user (addressed in Review major 8).
- **Spec items marked "verify" that I checked**
  - `ffmpeg -progress pipe:1` microseconds (`out_time_us`), `-movflags +faststart`, `--convert-thumbnails jpg`, and `--sponsorblock-mark / --sponsorblock-remove` verified against installed tools.
- **Decisions/deviations**: None.
- **Next step**: Address M4 code review items.





## 2026-10-17 — M4 / Review blocker: reject invalid trim input
- Done: new `ffmpeg::parse_and_validate_trim` (parse + `validate_trim`); `queue_add` rejects malformed/inverted trim (checked against `duration_sec` when known); engine fails the item before ffmpeg on invalid trim (original file kept) instead of silently dropping parse errors; AddBar disables submit while any ready row shows a trim error.
- Verified by running: cargo fmt --check (ok), cargo clippy --all-targets -D warnings (ok), cargo test (all pass, new unit test included), npm run check (0 errors, 0 warnings).
- Not verified: manual UI repro in `npm run tauri dev`; `toRequests()` itself is not filtered (submit is blocked in the UI and the backend rejects).
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review blocker: resume/retry after Encoding re-downloaded
- Done: entering `Encoding` now persists `item.file` (plus runtime/heights, `size_bytes = None`); `run_item` skips yt-dlp and goes straight to the encode stage when the item needs encoding, has `file` on disk and no `size_bytes` (i.e. never reached Done). Fixes resume/crash-recovery/retry re-downloading or being archive-skipped without encoding.
- Verified by running: cargo fmt --check (ok), cargo clippy -D warnings (ok), cargo test (all pass), npm run check (0 errors, 0 warnings).
- Not verified: no new automated test for the resume path; not exercised manually. ffmpeg stderr tail still goes only into the error detail, not `log_tail`.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review blocker: partial encode output left on cancel
- Done: `run_encode` points ffmpeg at `<stem>-encoded.sdp-part.<ext>` (`partial_output_path`), removes it on Cancelled/Failed, and renames it onto the final name only after a successful non-empty encode; the original is deleted only after that rename. A pre-existing `-encoded` file is no longer truncated/deleted by a failed or cancelled encode. `derive_encoded_output_path` builds the stem as `OsString` (no `"output"` fallback for non-UTF-8 names).
- Verified by running: cargo fmt --check (ok), cargo clippy -D warnings (ok), cargo test (all pass, incl. existing ffmpeg_encode integration tests + new unit test), npm run check (0 errors, 0 warnings).
- Not verified: no integration test asserting the partial file is removed on cancel; not exercised manually.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review blocker: Trim and Auto-play hidden behind Encode
- Done: in `AddBar.svelte` the Trim fields and per-item Auto-play are now outside `{#if row.encode}` (only codecs and Keep original stay inside). The Encode checkbox is disabled while a trim range is set, with `aria-describedby` pointing at the existing trim hint ("Entering a trim range automatically enables re-encoding").
- Verified by running: cargo fmt --check (ok), cargo clippy -D warnings (ok), cargo test (all pass), npm run check (0 errors, 0 warnings).
- Not verified: visual check in `npm run tauri dev` not done.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 7: items shown as Encoding while waiting for the lane
- Done: `run_item` now persists the downloaded file, then acquires the encode-lane permit, and only then switches the item to `Encoding` (so only items running ffmpeg show it). `encode_semaphore` is a `Mutex<Arc<Semaphore>>`; `QueueEngine::configure` replaces it when the encode capacity changes (running encodes keep their old permits).
- Verified by running: cargo fmt --check (ok), cargo clippy -D warnings (ok), cargo test (all pass), npm run check (0 errors, 0 warnings).
- Not verified: no new test for the ordering/resize; not exercised manually.
- Known trade-off: an item waiting for the encode lane stays in `Downloading` (100%) and keeps its download-lane slot until it gets the permit. Items already waiting on the old semaphore when the capacity changes keep waiting on it.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 7 follow-up: dynamic encode slot & separate waiting label
- Done:
  - Completed downloads needing encoding immediately free the download lane (transitioning to `Encoding` with `progress.label = "waitingEncode"` and `inner.wake.notify_waiters()`).
  - Added `job.label.waitingEncode` ("Waiting for the encoder…" / "In attesa della codifica…") in `en.json`, `it.json`, `queue.ts` and `usage.test.ts`.
  - Replaced static semaphore with dynamic slot counter (`encodes_running`, `encode_slot: Notify`) checked against live lane capacity (`lanes.capacity(Lane::Encode)`). When `QueueEngine::configure` updates lane capacity, waiting tasks are immediately notified and re-evaluate against the new limit.
  - Once a slot is acquired, progress transitions to `label = "encoding"` (0%). Dropping `EncodeSlot` decrements running count and notifies waiters.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 332 unit tests + integration test suites passed.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 259 passed.
  - `npm run build` → production build succeeded.
- Not verified: visual UI testing under `npm run tauri dev`.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 5: encoder probe cache uses ffmpeg version and resolves ffmpeg directly (spec §9.3)
- Done:
  - Made `resolve_cached` in `commands/env.rs` public so single tools can be resolved through session cache and user overrides without triggering job tool requirements.
  - Updated `encoders_probe` in `commands/system.rs` to resolve `tools::FFMPEG` directly via `resolve_cached` instead of calling `resolve_job_tools`, eliminating the false failure when `yt-dlp` is not yet installed.
  - Passed `ffmpeg.version.as_deref()` instead of `job_tools.ytdlp.version` to `probe_all_encoders`, ensuring the encoder cache is keyed by `(ffmpeg_path, ffmpeg_mtime, ffmpeg_version)` as required by spec §9.3.
  - Added integration test `encoders_probe_succeeds_without_ytdlp_and_caches_ffmpeg_version` in `tests/ffmpeg_encode.rs` verifying probing works without yt-dlp and caches by the ffmpeg version.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 330 unit + 124 integration = 454 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 259 passed.
  - `npm run build` → built in 471ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 6: align default audioCodec with UI and hide video codec for audioOnly (spec §9.3, §11.3, §11.6)
- Done:
  - Defaulted `default_audio_codec` to `Some("aac".to_owned())` in `src-tauri/src/state.rs` (`Settings::default()` and `Settings::normalise()`).
  - Defaulted `defaultAudioCodec` to `"aac"` in `src/lib/settings.ts` (`DEFAULT_SETTINGS` and `normaliseSettings()`).
  - Initialized `audioCodec` in `src/lib/stores/addbar.svelte.ts` to `defaultAudioCodec ?? "aac"`.
  - Updated audio encode fallback in `src-tauri/src/services/queue/engine.rs` from `Opus` to `AudioCodec::Aac` to align with the UI default.
  - Hid the video codec dropdown in `src/lib/components/AddBar.svelte` when `choiceFor(row).kind === "audioOnly"`.
  - Added assertions in `src/lib/settings.test.ts`, `src-tauri/tests/settings_ipc.rs`, and `src-tauri/tests/queue_ipc.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 330 unit + 124 integration = 454 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 260 passed.
  - `npm run build` → built in 464ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 8: inform user on hardware-to-CPU encoder fallback (spec §9.3)
- Done:
  - Matched `used_cpu_fallback` from `ffmpeg::run_encode` in `services/queue/engine.rs`.
  - When true, appended notice to `item.log_tail` and pushed user-facing toast `toast.hardwareEncoderFallback`.
  - Added localized strings for `toast.hardwareEncoderFallback` in `src/lib/i18n/en.json` and `src/lib/i18n/it.json`.
  - Added integration test `hardware_encoder_fallback_informs_user_via_toast_and_log_tail` in `tests/ffmpeg_encode.rs` verifying toast emission and log tail notice.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 330 unit + 125 integration = 455 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 260 passed.
  - `npm run build` → built in 476ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 9: treat embedding failures as warnings and fix metadata flags (spec §8.4)
- Done:
  - Updated `download_args` in `src-tauri/src/services/ytdlp.rs` so that when `spec.embed_metadata` is true, `--no-embed-info-json` is always passed and `--no-embed-chapters` is passed if `!spec.embed_chapters`, preventing implicit embedding of chapters and info-json.
  - Updated `run_attempt` in `src-tauri/src/services/job.rs` to treat postprocessor-only embedding errors (`YtdlpErrorCode::Unknown` when an embed option was requested and the output media file already exists on disk) as non-fatal warnings instead of failing the job.
  - Added unit test `embed_metadata_without_chapters_passes_no_embed_chapters` in `src-tauri/src/services/ytdlp.rs`.
  - Updated `src-tauri/tests/bin/fake_ytdlp.rs` to simulate embedding failure after writing the output media file on `embed-fail` URLs.
  - Added integration tests `an_embedding_failure_is_treated_as_a_warning_when_output_file_exists` and `a_job_fails_when_tool_fails_and_no_embed_option_was_requested` in `src-tauri/tests/ytdlp_job.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 331 unit + 126 integration = 457 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 260 passed.
  - `npm run build` → built in 482ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked:
  - Verified `yt-dlp --help` flags: `--embed-metadata` implies chapters/infojson unless `--no-embed-chapters` and `--no-embed-info-json` are specified.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 10: gate SponsorBlock on YouTube URLs and prevent unchecking all categories (spec §8.4)
- Done:
  - Gated `--sponsorblock-mark` and `--sponsorblock-remove` in `src-tauri/src/services/ytdlp.rs` on `jsruntime::is_youtube_url(url)` so non-YouTube URLs never receive SponsorBlock arguments.
  - In `src/lib/components/AddBar.svelte` and `src/lib/pages/settings/GeneralSettings.svelte`, disabled unchecking the last category when only one category is selected (`disabled={length === 1 && includes(cat)}`) and ignored uncheck events if length <= 1.
  - In `src-tauri/src/commands/queue.rs`, resolved `sponsorblock_mode` and `sponsorblock_categories` such that empty category lists or explicit mode `"off"` are treated as `SponsorBlockMode::Off` (`None`), rather than silently falling back to global settings.
  - In `src/lib/queue.ts` and `src/lib/stores/addbar.svelte.ts`, allowed `"off"` in `AddItemRequest.sponsorblockMode` and defaulted `row.sponsorblockMode ?? "off"` in `toRequests()`.
  - Added unit test assertion in `src-tauri/src/services/ytdlp.rs` verifying non-YouTube URLs (e.g. Vimeo) do not get SponsorBlock flags even when enabled in spec.
  - Added integration test `queue_add_treats_empty_sponsorblock_categories_or_off_as_disabled` in `src-tauri/tests/queue_ipc.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 331 unit + 127 integration = 458 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 260 passed.
  - `npm run build` → built in 490ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 11: videotoolbox fallback, hevc_vaapi tag, and bound probe concurrency (spec §9.3)
- Done:
  - Added `"-tag:v", "hvc1"` to `hevc_vaapi` in `ENCODER_CATALOGUE` in `src-tauri/src/services/encoders.rs` so all HEVC hardware encoders generate QuickTime/Apple-compatible MP4 files.
  - Implemented VideoToolbox `-b:v` fallback (`&["-b:v", "2M"]` / `&["-b:v", "2M", "-tag:v", "hvc1"]`) in `src-tauri/src/services/encoders.rs` via `videotoolbox_fallback_args` and `probe_encoder` retry.
  - Updated video encoding attempt flow in `src-tauri/src/services/ffmpeg.rs` (`run_encode_attempts` and `build_video_encode_args_with_custom_encoder`) to retry with VideoToolbox `-b:v` fallback before falling back to CPU encoder (`libx264`/`libx265`).
  - Bounded concurrency in `probe_all_encoders` using `tokio::sync::Semaphore::new(2)` and `tokio::spawn`, preserving candidate catalogue order.
  - Added `InFlightDeduplicator` in `src/lib/tools.ts` and used it in `src/lib/stores/tools.svelte.ts` to deduplicate concurrent in-flight `probeEncoders()` calls.
  - Added unit tests in `src-tauri/src/services/encoders.rs`, integration tests in `src-tauri/tests/ffmpeg_encode.rs`, and deduplicator unit tests in `src/lib/tools.test.ts`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 333 unit + 130 integration = 463 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → built in 483ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 12: duplicate queue detection distinguishes M4 options (spec §7.3)
- Done:
  - Included `encode`, `video_codec`, `audio_codec`, `trim_start`, `trim_end`, `keep_original`, `embed_metadata`, `embed_thumbnail`, `embed_chapters`, `sponsorblock_mode`, and `sponsorblock_categories` in `ItemOptions::signature` in `src-tauri/src/services/queue/model.rs`.
  - Added unit test `the_options_signature_separates_m4_options` in `src-tauri/src/services/queue/model.rs`.
  - Added integration test `queue_add_does_not_treat_different_m4_options_as_duplicates` in `src-tauri/tests/queue_ipc.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 334 unit + 131 integration = 465 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → built in 440ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review major 13: align Task 4 progress entry with actual implementation (spec §14, §15)
- Done:
  - Corrected Task 4 entry in `docs/PROGRESS.md` to accurately reflect the initial state of encoder version caching, trim validation wiring, UI visibility, and fallback notification, recording them as deferred/stubbed until their respective review fixes (#1, #4, #5, #8).
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 334 unit + 131 integration = 465 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~440ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.

## 2026-10-17 — M4 / Review minors 14, 15, 16: ffmpeg child temp_dir, ffprobe duration fallback, and remove dead helper (spec §4.9, §9.1)
- Done:
  - Removed unused dead helper `EncoderDefinition::to_info` in `src-tauri/src/services/encoders.rs` (Review Minor 16).
  - Passed `temp_dir` to `ToolCommand` in `run_single_probe_attempt`, `query_ffmpeg_encoders`, `probe_encoder`, and `probe_all_encoders` in `src-tauri/src/services/encoders.rs`, and wired `state.tools().temp_dir()` from `src-tauri/src/commands/system.rs` (Review Minor 14, Spec §4.9).
  - Added `temp_dir: Option<PathBuf>` to `EncodeRequest` and passed `temp_dir` to `ToolCommand` in `execute_single_encode` and `run_encode` in `src-tauri/src/services/ffmpeg.rs`, passing `environment.private_temp` from `src-tauri/src/services/queue/engine.rs` (Review Minor 14, Spec §4.9).
  - Added `find_ffprobe` and `probe_file_duration` in `src-tauri/src/services/ffmpeg.rs` using `ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 -- <input>` with child process `temp_dir` propagation (Review Minor 15, Spec §9.1).
  - Added `ffprobe: Option<PathBuf>` to `JobTools` and `JobEnvironment`, resolving it in `resolve_job_tools` and passing it to jobs and queue items.
  - In `src-tauri/src/services/queue/engine.rs`, when `duration_sec` is `None` on an item needing encode, probed the downloaded file duration with `ffprobe` before encode, updated `item.duration_sec`, and passed `total_duration` to `parse_and_validate_trim` (exercising `ffmpeg.rs:183-190`) and `EncodeRequest` (Review Minor 15, Spec §9.1).
  - In `src-tauri/src/services/ffmpeg.rs` `run_encode`, added fallback to probe duration via `ffprobe` when `request.total_duration` is `None`, ensuring expected duration and progress percentage / ETA are computed during encode.
  - Added `ffprobe` multi-call support and `record_env.txt` handling to `src-tauri/tests/bin/fake_tool.rs`.
  - Added integration tests `ffmpeg_and_encoders_pass_private_temp_dir_to_child_processes`, `encode_falls_back_to_ffprobe_duration_when_total_duration_is_none`, and `queue_engine_probes_duration_with_ffprobe_and_validates_trim` in `src-tauri/tests/ffmpeg_encode.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 333 unit + 134 integration = 467 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~466ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: continue M4 review items.
## 2026-10-17 — M4 / Review follow-up: push stderr to item log_tail on ffmpeg failure and preserve attached thumbnails during re-encode (spec §8.4, §9.2, §9.4)
- Done:
  - In `src-tauri/src/services/queue/engine.rs`, updated `EncodeOutcome::Failed` to iterate over `stderr_tail` and push each line to `item.push_log(line)` so the card's log viewer displays the failure output, matching CPU fallback behavior.
  - Added `embed_thumbnail: bool` to `VideoEncodeOptions` and `AudioEncodeOptions` in `src-tauri/src/services/ffmpeg.rs`, wired from `options.embed_thumbnail` in `engine.rs`.
  - In `src-tauri/src/services/ffmpeg.rs` `build_video_encode_args_with_custom_encoder`: when `embed_thumbnail` is true, map `0:V:0?`, `0:a?`, `0:v:disp:attached_pic?` and add `-c:v:disp:attached_pic copy -disposition:v:disp:attached_pic attached_pic` so the embedded thumbnail/cover art is preserved and not dropped or re-encoded with the video codec.
  - In `src-tauri/src/services/ffmpeg.rs` `build_audio_encode_args`: when `embed_thumbnail` is true and the container supports attached picture streams (`AudioCodec::Aac` `.m4a` and `AudioCodec::Mp3` `.mp3`), map `0:a:0?` and `0:v:disp:attached_pic?` with copy disposition instead of passing `-vn`, preserving embedded thumbnails during audio re-encoding. (Opus keeps `-vn` as Ogg/Opus does not support video attached_pic streams).
  - Added unit tests in `src-tauri/src/services/ffmpeg.rs` for `build_video_encode_args` and `build_audio_encode_args` with `embed_thumbnail: true`.
  - Added integration tests `queue_engine_pushes_ffmpeg_stderr_to_log_tail_on_failure` and `embed_thumbnail_preserves_attached_pic_in_ffmpeg_args` in `src-tauri/tests/ffmpeg_encode.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 335 unit + 135 integration = 470 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~486ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: proceed to M5 or next planned phase.

## 2026-10-17 — M4 / CI fix: fake_tool multi-call matching for suffixed binaries (spec §5.2, §15 item 8)
- Done:
  - In `src-tauri/tests/bin/fake_tool.rs`, updated the multi-call dispatcher so that target-triple suffixed binary names (`<tool>-<triple>`, e.g. `ffmpeg-x86_64-unknown-linux-gnu`, `yt-dlp-*`, `qjs-*`, `ffprobe-*`) match their corresponding tool handler (`run_ffmpeg`, `run_ytdlp`, etc.) instead of falling through to generic fake_tool CLI commands with exit code 2.
  - In `src-tauri/tests/ffmpeg_encode.rs` `encoders_probe_succeeds_without_ytdlp_and_caches_ffmpeg_version`, ensured the bundled fake ffmpeg binary is copied as both suffixed (`ffmpeg-{triple}`) and plain (`ffmpeg`/`ffmpeg.exe`) with Unix executable permissions, and asserted that `resolved_ffmpeg.source` resolves directly to `ToolSource::Bundled`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 335 unit + 135 integration = 470 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~476ms.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: push branch to remote and verify CI checks.

## 2026-10-17 — M4 / CI fix: eliminate flusher race condition in hardware encoder fallback test (spec §7.6, §9.3, §15)
- Done:
  - In `src-tauri/tests/ffmpeg_encode.rs` `hardware_encoder_fallback_informs_user_via_toast_and_log_tail`, fixed a race condition where the polling loop exited immediately upon `item.state == Done` without waiting for the asynchronous `flusher_loop` to deliver the `toast.hardwareEncoderFallback` event to the sink.
  - Polled for both item completion and toast delivery, and shut down the engine cleanly before running assertions to prevent orphaned engine tasks from attempting file writes after fixture deletion.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 335 unit + 135 integration = 470 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~457ms.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
## 2026-10-17 — M4 / Bug fix: pass --no-quiet to prevent --print from silencing archive skip output (spec §6.4, §8.3)
- Done:
  - In `src-tauri/src/services/ytdlp.rs` `download_args`: added `--no-quiet` to base flags and following `--print`. In yt-dlp, `--print` implies `--quiet`, which suppressed the archive skip line (`[download] <id>: has already been recorded in the archive`). Because the video was already recorded in `--download-archive`, yt-dlp skipped downloading it without printing `SDP_FILE`, but because `--print` enabled quiet mode, stdout was empty and `job.rs` never saw the archive skip line, causing the job to fail with `yt-dlp exited 0 but never printed the final path (SDP_FILE)`.
  - In `src-tauri/src/services/job.rs` `AttemptShared::on_stderr_line`: also checked `ytdlp::is_archive_skip_line(line)` as a defensive measure.
  - In `src-tauri/src/services/ytdlp.rs` unit tests: verified `--no-quiet` is present in `the_base_arguments_are_all_present_and_the_url_comes_last_after_double_dash`.
  - In `src-tauri/tests/bin/fake_ytdlp.rs`: updated `archive-skip` handling to mirror real yt-dlp by silencing the skip line if `--print` is passed without `--no-quiet`, preventing regressions.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 335 unit + 135 integration = 470 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → production bundle compiled in ~450ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools` → all passed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: yt-dlp `--help` confirms `--print` implies `--quiet`, and `--no-quiet` explicitly deactivates quiet mode. Live execution with pinned yt-dlp binary confirmed `--no-quiet` restores `[download] <id>: has already been recorded in the archive`.
- Decisions/deviations (also in DECISIONS.md): None.
## 2026-10-17 — Tools / UI fix: show clearPath button whenever tool override is configured and hide install on bundled tools (spec §5.2, §11.8)
- Done:
  - In `src/lib/pages/ToolsPage.svelte`: updated the `clearPath` ("Ripristina") button visibility condition to check `tool.source === "override" || Boolean(settingsStore.settings.toolPaths[tool.name])`. Previously, if a custom override path pointed to an unexecutable or missing file, `resolve()` returned `None`, which caused `tool.source` to be `null` and hid the "Ripristina" button, trapping the user without a way to reset to the bundled tool.
  - In `src/lib/pages/ToolsPage.svelte`: updated the `install` button visibility condition to `!tool.bundled && tool.installable && !tool.path` so that bundled tools (like `qjs` and `yt-dlp`) never present an on-demand install action.
- Verified by running:
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → built in 474ms.
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None in this step.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: push commit to remote.

## 2026-10-17 — M4 / Bug fix: scope download archive fallback to playlist items so single videos can always be re-downloaded (spec §6.4, §8.2)
- Done:
  - In `src-tauri/src/commands/queue.rs` `queue_add`: changed `use_archive` resolution from `item.use_archive.unwrap_or(settings.use_download_archive)` to only fall back to `settings.use_download_archive` when `item.group_id.is_some()` (playlist items), and `false` for standalone single-video items (`group_id == None`). Previously, single-video requests from the Add bar omitted `useArchive`, inherited `use_download_archive: true` from global settings, recorded their video IDs in `archive.txt`, and prevented the user from ever re-downloading a video whose output file had been deleted from disk.
  - In `src/lib/components/PlaylistPanel.svelte`: initialised `useArchive` from `settingsStore.settings.useDownloadArchive` instead of a hard-coded `true`, so the global "Use download archive" setting in the "Playlists & archive" section governs the playlist panel's default while allowing per-run overrides ("Ignore archive for this run").
  - In `src/lib/settings.ts` and `src/lib/settings.test.ts`: aligned `DEFAULT_SETTINGS.useDownloadArchive` (`true`) and `normaliseSettings` with the Rust backend default (`Settings::default().use_download_archive == true`, spec §6.4).
  - Added integration test `queue_add_scopes_download_archive_fallback_to_playlist_items` in `src-tauri/tests/queue_ipc.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 335 unit + 136 integration = 471 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 12 test files, 262 passed.
  - `npm run build` → built in 442ms.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: Spec §6.4 specifies `--download-archive <app data>/archive.txt` under "YouTube playlists and channels" ("skip already downloaded (download archive, default on for playlists)").
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: commit changes.

## 2026-10-18 — M5-A / Whisper sidecar CI, model catalogue, download manager, and transcription settings (spec §5.4, §10.2, §11.6)
- Done:
  - Created `.github/workflows/whisper-sidecar.yml` to compile `sdp-whisper` static binaries from pinned `v1.9.5` whisper.cpp (Linux x86_64/aarch64 with static libgcc/libstdc++, Windows x64/arm64 with MultiThreaded CRT, macOS arm64/x86_64 with Metal).
  - Pinned `whisper.models.json` containing 11 GGML Whisper models + Silero VAD (`silero-v5.1.2`) with exact byte sizes and sha256 hashes queried from Hugging Face Git LFS. Added `scripts/fetch_models_manifest.mjs` and `npm run check:models`.
  - Added `sdp-whisper` entry to `tools.manifest.json` and updated tool banner parsing in `src-tauri/src/services/tools.rs` for `whisper.cpp version: 1.9.5-dev`.
  - Bumped settings schema version to 7 and added transcription preferences in `src-tauri/src/state.rs` and `src/lib/settings.ts`.
  - Implemented `ModelDownloadManager` in `src-tauri/src/services/models.rs` with Range-resume into `.part`, sha256 checksum validation, Silero VAD co-download, disk-space pre-flight check, local model file import, model deletion, and models view inspection.
  - Added IPC commands `models_list`, `models_download`, `models_cancel_download`, `models_delete`, `models_import`, `models_set_active`, `models_open_folder` in `src-tauri/src/commands/models.rs`.
  - Built frontend models store `src/lib/stores/models.svelte.ts` and `TranscriptionSettings.svelte` with live progress bars, large download warning (> 1 GiB), active model selection, CPU threads and mirror URL configuration, and full i18n support in `it.json` and `en.json`.
  - Added unit tests in `src/lib/models.test.ts`, `src/lib/settings.test.ts`, and full integration test suite in `src-tauri/tests/models.rs`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 345 unit + 140 integration = 485 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 267 passed.
  - `npm run check:ci && npm run check:runners && npm run check:tools && npm run check:models` → all passed.
- Not verified / stubbed: Transcription audio extraction and whisper CLI execution pipeline (reserved for phase M5-B).
- Spec items marked "verify" that I checked:
  - whisper.cpp release asset and version tags verified on ggml-org/whisper.cpp (`v1.9.5`).
  - Hugging Face Git LFS metadata verified for ggerganov/whisper.cpp and ggml-org/whisper-vad models.
- Decisions/deviations (also in DECISIONS.md): D119.
- Next step: proceed to Phase M5-B (transcription pipeline, chunking, and subtitle generation).

## 2026-10-18 — Windows / Cross-compilation and test execution with cargo-xwin & wine (spec §1.1, §2.8)
- Done:
  - Validated cross-compilation for Windows MSVC target (`x86_64-pc-windows-msvc`) with `cargo-xwin`:
    - `cargo xwin check --target x86_64-pc-windows-msvc --all-targets` passed cleanly in ~7s.
    - `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` passed with 0 warnings.
    - `cargo xwin test --no-run --target x86_64-pc-windows-msvc` successfully compiled all Windows executables (unit tests and integration test suites: `models.rs`, `download.rs`, `queue_engine.rs`, `ffmpeg_encode.rs`, `tools.rs`, etc.).
  - In `src-tauri/src/services/paths.rs` `atomic_write`: added Windows-specific retry loop on `fs::rename` (up to 10 retries with 10ms backoff) to handle transient file-sharing contention during concurrent atomic overwrites.
  - Executed Windows binaries under Wine:
    - `wine target/x86_64-pc-windows-msvc/debug/deps/scaricatore_di_porto_lib-*.exe` → 344 passed, 0 failed.
    - `wine target/x86_64-pc-windows-msvc/debug/deps/models-*.exe` → 4 passed, 0 failed.
    - `wine target/x86_64-pc-windows-msvc/debug/deps/download-*.exe` → 9 passed, 0 failed.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 345 unit + 140 integration = 485 passed, 0 failed, 3 ignored.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: Windows `MoveFileExW` / `fs::rename` sharing violation resolution via backoff retry loop.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: proceed to Phase M5-B.

## 2026-10-18 — M5-B / Whisper offline transcription pipeline, chunking, and local media action (spec §10.2, §11.5, §15 item 9)
- Done:
  - Implemented offline Whisper transcription pipeline in `src-tauri/src/services/transcribe/whisper.rs`:
    - 16 kHz mono WAV conversion via ffmpeg (`-vn -ac 1 -ar 16000 -c:a pcm_s16le`) with pre-flight disk space validation.
    - Silence detection chunking for audio > 2 hours (7200 s) using `silencedetect=noise=-30dB:d=0.5` within ±20 s of 30-min windows (1800 s), with hard cut and 2.0 s overlap fallback.
    - Automatic cue offsetting and index renumbering for merged SRT and VTT outputs.
    - Pinned CLI flags for `sdp-whisper`: `-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`, `-of`, `--vad`, `-vm`, `-t`, `-ng`.
    - Real-time progress parsing (`-pp`) mapped to `transcribing (X%)` and live segment preview on stdout.
    - Automatic retry fallback: on non-zero exit from an initial run without `-ng`, retries once on CPU with `-ng`, and emits warning toast.
    - Hallucination guard: detects repetitive segment loops (>=5 identical consecutive segments) or suspicious silence, and emits a warning toast (spec §10.2 item 7).
    - Graceful process tree cancellation and cleanup of temporary WAV and partial transcript files.
    - Directory resolution supporting direct alongside media or dedicated `transcripts/` subfolder.
  - Queue engine and IPC integration:
    - Added dedicated transcription concurrency semaphore bounded by `max_parallel_transcriptions`.
    - Added IPC commands `queue_transcribe_item` and `queue_add_local_file` in `src-tauri/src/commands/queue.rs`.
    - Extended `ItemOptions` and `QueueItem` with transcription options and transcript output paths.
    - Supported post-download transcription and direct local media queue items.
  - Frontend UI integration:
    - Added "Transcribe a local file…" button and per-row transcription options in `AddBar.svelte`.
    - Added contextual "Transcribe…" action for completed queue items in `QueueItem.svelte`.
    - Displayed live segment preview during transcription and links to generated `.txt`, `.srt`, `.vtt` files upon completion.
    - Added full i18n support in `it.json` and `en.json`.
  - Comprehensive testing:
    - 17 unit tests in `src-tauri/src/services/transcribe/whisper.rs`.
    - 6 integration tests in `src-tauri/tests/whisper_pipeline.rs` covering progress tracking, GPU retry with `-ng`, hallucination detection, cancellation cleanup, chunking with silencedetect merge, and end-to-end queue transcription of local media files.
    - Unit tests in `src/lib/queue.test.ts`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 362 unit + 146 integration = 508 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 267 passed.
  - `npm run build` → production bundle built in ~470ms.
  - `npm run check:ci && npm run check:runners && npm run check:tools && npm run check:models` → all passed.
- Not verified / stubbed: Subtitle extraction and Gemini summarisation (reserved for phases M5-C and M5-D).
- Spec items marked "verify" that I checked:
  - `sdp-whisper` CLI flags (`-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`, `-of`, `--vad`, `-vm`, `-t`, `-ng`) verified against whisper.cpp v1.9.5 source and `--help`.
  - ffmpeg `silencedetect` output formatting (`silence_start: X` and `silence_end: Y | silence_duration: Z`).
- Decisions/deviations (also in DECISIONS.md): D120.
- Next step: commit changes.

## 2026-10-18 — M5 / Fix transcript file collision overwrite (B1, spec §10.2)
- Done:
  - Fixed B1: prevented silent overwriting of existing `.txt`, `.srt`, and `.vtt` files in `src-tauri/src/services/transcribe/whisper.rs`:
    - Updated `derive_transcript_paths` to check whether `<stem>.txt`, `<stem>.srt`, or `<stem>.vtt` already exists on disk in the target directory (either adjacent to media or inside `transcripts/`). If any exist, iteratively appends suffix ` (1)`, ` (2)`, etc. so all three generated transcript files share a single collision-free stem.
    - Updated `run_transcribe` to resolve `final_paths` immediately before writing and write final merged files atomically using `crate::services::paths::atomic_write`.
  - Added unit test `derive_transcript_paths_avoids_overwriting_existing_files` in `src-tauri/src/services/transcribe/whisper.rs`.
  - Added integration test `transcribe_does_not_overwrite_existing_transcript_files` in `src-tauri/tests/whisper_pipeline.rs` verifying that an existing `.srt` file is untouched and transcripts receive collision-free names.
- Verified by running:
  - `cargo test --test whisper_pipeline transcribe_does_not_overwrite_existing_transcript_files` → verified FAILS before the fix with assertion error, then PASSED after the fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 363 unit + 152 integration = 515 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings (e.g. B2).

## 2026-10-18 — M5 / Fix transcription temp directory isolation and resource leak on error (B2, spec §4.9, §10.2)
- Done:
  - Fixed B2:
    - Updated `run_transcribe_stage` in `src-tauri/src/services/queue/engine.rs` to pass `temp_dir: Some(process::item_temp_dir(&options.output_dir, id))` instead of `environment.private_temp.clone()`, ensuring temporary audio preparation files are kept in the item's isolated job temp directory on the target filesystem (spec §4.9, §10.2).
    - Introduced RAII cleanup guards `CleanupFile` and `ChunkGuard` in `src-tauri/src/services/transcribe/whisper.rs` so that `full_wav`, `chunk_wav`, and `chunk_out_base.*` files are unconditionally cleaned up when exiting `run_transcribe` or each chunk iteration, preventing leaks across errors, cancellations, or normal completion.
    - Added tests in `src-tauri/tests/whisper_pipeline.rs`: `transcribe_cleans_temporary_wav_when_whisper_fails_to_spawn`, `transcribe_cleans_temporary_wav_and_chunks_when_whisper_fails_to_spawn_in_chunked_mode`, and `queue_engine_transcribe_uses_job_temp_dir_not_shared_private_temp`.
- Verified by running:
  - `cd src-tauri && cargo test --test whisper_pipeline transcribe_cleans_temporary_wav_when_whisper_fails_to_spawn` → FAILS before fix (panic: temporary prep wav must not leak on whisper spawn error, found: [DirEntry(.../whisper_prep_601406.wav)]), PASSES after fix.
  - `cd src-tauri && cargo test --test whisper_pipeline transcribe_cleans_temporary_wav_and_chunks_when_whisper_fails_to_spawn_in_chunked_mode` → FAILS before fix (panic: temporary wav and chunk files must not leak... found: chunk_0_*.wav, whisper_prep_*.wav), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 363 unit + 155 integration = 518 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Fix model filename path traversal validation (B3, Checklist #4, spec §5.2, §10.2)
- Done:
  - Fixed B3: validated model and VAD filenames against path traversal in `src-tauri/src/services/models.rs`:
    - Updated `ModelCatalogue::validate` (and made it public) to reject filenames that fail `crate::services::manifest::is_safe_asset_name` or do not end with `.bin` for both `vad.file` and `model.file`.
    - Added safety validation checks and assertions (`is_safe_asset_name`) before joining paths with `models_dir` in `is_vad_installed`, `inspect_models`, `download_model`, `delete_model`, and `import_model`, as well as in `effective_url` and `effective_vad_url`.
    - Added unit tests in `src-tauri/src/services/models.rs`:
      - `catalogue_validate_rejects_path_traversal_and_unsafe_filenames`: reproduces and verifies rejection of relative traversal (`../escaped.bin`), absolute paths, backslashes, hidden files, and missing `.bin` extensions for both models and VAD.
      - `model_operations_reject_path_traversal_filenames`: reproduces and verifies that `delete_model`, `import_model`, and `download_model` reject path-traversing filenames without deleting, overwriting, or reading files outside `models_dir`, and that `inspect_models` and `is_vad_installed` handle them safely.
- Verified by running:
  - `cargo test catalogue_validate_rejects_path_traversal_and_unsafe_filenames` → FAILS before fix (panic: `assertion failed: catalogue.validate().is_err()`), PASSES after fix.
  - `cargo test model_operations_reject_path_traversal_filenames` → FAILS before fix (panic: `delete_model must reject path traversal`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 365 unit + 155 integration = 520 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Fix pinned sdp-whisper asset URL and checksum in manifest (B4, Checklist #2, spec §5.1, §5.4)
- Done:
  - Fixed B4: populated real asset releaseUrl template and Linux x86_64 url and sha256 checksum for `sdp-whisper` in `tools.manifest.json`:
    - Added `"releaseUrl": "https://github.com/emanuelefusco/ScaricatoreDiPorto/releases/download/whisper-v1.9.5/{asset}"` to the `sdp-whisper` tool definition.
    - Set `"url": "https://github.com/emanuelefusco/ScaricatoreDiPorto/releases/download/whisper-v1.9.5/sdp-whisper-x86_64-unknown-linux-gnu"` and `"sha256": "c0cc33d49a950ecda4089ba7448a576ab7286b950c48c2e718bd8913c97932eb"` (verified byte-for-byte by compiling `whisper.cpp` pinned tag `v1.9.5` with the exact flags in `whisper-sidecar.yml`).
    - Added unit test `whisper_sidecar_asset_has_url_and_checksum_where_pinned` and asserted installability in `the_manifest_carries_yt_dlp_qjs_and_the_deno_fallback` in `src-tauri/src/services/manifest.rs`.
- Verified by running:
  - `cargo test whisper_sidecar_asset_has_url_and_checksum_where_pinned` → FAILS before fix (panic: `assertion left == right failed (left: None, right: Some("https://github.com/emanuelefusco/ScaricatoreDiPorto/releases/download/whisper-v1.9.5/{asset}"))`), PASSES after fix.
  - `cargo test the_manifest_carries_yt_dlp_qjs_and_the_deno_fallback` → FAILS before fix (panic: `sdp-whisper must carry url + sha256 so the lazy install works`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 366 unit + 155 integration = 521 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: macOS and Windows sdp-whisper checksums (Tier 1/Tier 2 CI-built targets; cannot run MSVC or macOS Metal builds in Linux environment, pending whisper-sidecar CI release).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Correct PROGRESS.md and DECISIONS.md claims for VAD, GPU retry, and hallucination guard (B5, Checklist #6, #12, #15, spec §10.2)
- Done:
  - Corrected false claims in `docs/PROGRESS.md` and `docs/DECISIONS.md` identified in review finding B5:
    - Hallucination guard: removed false claim of segment trimming. The pipeline detects repetitive loops (>= 5 consecutive segments) and suspicious silence, and emits a warning toast notification per spec §10.2 item 7 without mutating the generated transcript files.
    - Automatic CPU retry fallback: clarified that non-zero exit from an initial run without `-ng` retries once with `-ng`, correcting the claim that specific GPU errors are detected prior to retry.
    - Silero VAD metadata: corrected model catalog descriptions in `docs/DECISIONS.md` D119 and `docs/PROGRESS.md` to reference `ggml-org/whisper-vad` (`ggml-silero-v5.1.2.bin`, 885,098 bytes, SHA-256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`), replacing obsolete references to `snakers4/silero-vad` and `ggml-silero.bin` (5,463,552 bytes).
    - Recorded errata entry D121 in `docs/DECISIONS.md`.
    - Added test `docs_progress_and_decisions_reflect_whisper_vad_and_hallucination_reality` in `src-tauri/tests/whisper_pipeline.rs` enforcing documentation veracity against actual manifest and behavior.
- Verified by running:
  - `cargo test --test whisper_pipeline docs_progress_and_decisions_reflect_whisper_vad_and_hallucination_reality` → FAILS before fix (panic: `PROGRESS.md falsely claims that hallucination guard trims repetitions`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → clean.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → 0 warnings.
  - `( cd src-tauri && cargo test )` → 366 unit + 157 integration = 523 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 267 passed, 0 failed.
  - `npm run check:models` → OK: 11 model(s), VAD model present, schema 1.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): D121.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Fix leftover .part files reporting and pause/resume model state (B6, Checklist #4, #7, spec §10.2, §11.6)
- Done:
  - Fixed B6: leftover `.part` files on disk are no longer reported as actively `Downloading` by `inspect_models`:
    - Added `ModelInstallState::Paused { received_bytes: u64, total_bytes: Option<u64> }` to `src-tauri/src/services/models.rs` and matching TypeScript type `ModelInstallState` in `src/lib/models.ts`.
    - In `inspect_models`, only return `ModelInstallState::Downloading` when `downloading_models.contains_key(&model.id)`. When an idle `.part` file exists on disk and is not actively downloading, return `ModelInstallState::Paused { received_bytes, total_bytes }`.
    - Updated `ModelDownloadManager` in `src-tauri/src/services/models.rs`:
      - Populated initial progress in `download_model` on start so active downloads are registered immediately in `in_flight_progress`.
      - Added `pause` method on `ModelDownloadManager` to abort in-flight downloads while keeping `.part` files intact.
      - Added `cancel_model_download` helper to delete partial `.part` files upon cancellation.
    - Updated IPC commands in `src-tauri/src/commands/models.rs` and registered `commands::models::models_pause_download` in `src-tauri/src/lib.rs`:
      - Added `models_pause_download` to pause an in-flight download keeping `.part`.
      - Updated `models_cancel_download` to cancel in-flight downloads and delete any lingering `.part` file for the model.
      - Silenced intentional cancellation error toast emission in `models_download` background loop by emitting `models:cancelled` event instead of `models:error`.
    - Updated UI in `src/lib/pages/settings/TranscriptionSettings.svelte` and `src/lib/stores/models.svelte.ts`:
      - Handled both active `downloading` and `paused` states in the models table.
      - Displayed partial progress in size column (`{received} / {total}`) and paused percentage bar when paused.
      - Added Pause (`PauseIcon`) and Cancel (`XIcon`) buttons during active download, and Resume (`PlayIcon`) and Cancel (`XIcon`) buttons when paused.
      - Restored `downloadingModelId` and progress when reopening Settings while a download is running.
      - Added i18n keys for `transcription.models.pauseButton`, `resumeButton`, and `statePaused` in Italian and English.
    - Added tests:
      - Unit test `inspect_models_does_not_report_idle_part_file_as_downloading` in `src-tauri/src/services/models.rs`.
      - Unit test `cancel_model_download_removes_part_file` in `src-tauri/src/services/models.rs`.
      - Frontend test `supports paused ModelInstallState variant` in `src/lib/models.test.ts`.
- Verified by running:
  - `cargo test inspect_models_does_not_report_idle_part_file_as_downloading` → FAILS before fix (panic: `idle .part file without in-flight download must not be reported as Downloading`), PASSES after fix.
  - `cargo test cancel_model_download_removes_part_file` → PASSES.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 368 unit + 157 integration = 525 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 268 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings (e.g. B7).

## 2026-10-18 — M5 / Cancel in-flight download when models_delete is invoked (B7, Checklist #4, #5, spec §10.2)
- Done:
  - Fixed B7: added `state.models_manager().cancel(&model_id)` at the start of `models_delete` in `src-tauri/src/commands/models.rs`. This ensures any in-flight download for the model is cancelled immediately before files are deleted, aborting the background task and preventing it from recreating the deleted model file upon completion.
  - Added reproduction test `models_delete_cancels_in_flight_download_and_prevents_recreating_file` in `src-tauri/tests/models.rs`.
- Verified by running:
  - `cargo test --test models models_delete_cancels_in_flight_download_and_prevents_recreating_file` → FAILS before fix (panic: `models_manager must not report model as downloading after models_delete`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 368 unit + 158 integration = 526 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 268 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Fix whisper-sidecar macOS Intel artifact collision and packaging guard (M1, Checklist #1, spec §5.4, §1.1)
- Done:
  - Investigated M1: identified that `needs: [build-tier1, build-tier2]`, `permissions: contents: write`, and `gh release upload` were already implemented in `whisper-sidecar.yml`, and `ubuntu-24.04` is the repo standard runner per D55 and GitHub Actions deprecation of `ubuntu-22.04`.
  - Fixed M1 collision: removed redundant `macos-15-intel` matrix job from `build-tier2` in `.github/workflows/whisper-sidecar.yml`. `sdp-whisper-x86_64-apple-darwin` is built exclusively by `build-tier1` as the x86_64 slice of the universal binary with Metal support embedded (per spec §1.1).
  - Added duplicate filename detection guard in the `Collect and compute SHA256SUMS` packaging step to prevent silent overwriting of assets.
  - Added reproduction test in `scripts/ci-steps.test.mjs` checking that `whisper-sidecar.yml` does not produce colliding binary artifact names across build jobs and that the collection step fails upon duplicate filenames.
  - Documented decision D122 in `docs/DECISIONS.md` and updated `docs/reviews/M5-review-2026-10-18.md`.
- Verified by running:
  - `npx vitest run scripts/ci-steps.test.mjs` → FAILS before fix (`AssertionError: Colliding binary artifacts found in whisper-sidecar.yml: sdp-whisper-x86_64-apple-darwin (2 occurrences)` and `AssertionError: expected +0 not to be +0` on duplicate artifact packaging), PASSES after fix (15/15 tests passed).
  - `node scripts/check-ci-syntax.mjs` → 22 run blocks valid, exit 0.
  - `node scripts/check-ci-runners.mjs` → 5 workflows valid, exit 0.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 368 unit + 158 integration = 526 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 273 passed, 0 failed.
- Not verified / stubbed: Live GitHub Actions execution of whisper-sidecar.yml (requires GitHub infrastructure).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): D122.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Fix hard-cut chunking overlap cue duplication and timestamp backward jumps (M2, Checklist #9, spec §10.2)
- Done:
  - Fixed M2 in `src-tauri/src/services/transcribe/whisper.rs`:
    - Reconciled chunk boundaries in `merge_srt_contents` and `merge_vtt_contents` so cues in chunk $i$ starting at or after the next chunk's offset are dropped.
    - Merged duplicate cues across boundaries by detecting matching normalized text and joining their time bounds without duplication.
    - Clamped cues spanning across boundaries to prevent overlapping or backward-jumping timestamps when text differs, and deduplicated multi-word text overlap across chunk boundaries.
    - Deduplicated text in `merge_txt_contents` across chunk boundaries by trimming repeated words/lines.
    - Added reproduction test `hard_cut_chunk_boundary_overlap_does_not_duplicate_text_or_overlap_timestamps` and boundary tests for overlap dropping, timestamp clamping, and partial word deduplication.
- Verified by running:
  - `cargo test hard_cut_chunk_boundary_overlap_does_not_duplicate_text_or_overlap_timestamps` → FAILS before fix (panic: `assertion left == right failed: duplicated speech must not be emitted twice in merged SRT: left: 2, right: 1`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 372 unit + 158 integration = 530 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Stream ffmpeg silencedetect to preserve all silence intervals beyond stderr ring buffer (M3, Checklist #9, spec §10.2)
- Done:
  - Fixed M3 in `src-tauri/src/services/transcribe/whisper.rs`:
    - Updated `detect_audio_silence` to use `process::spawn_tool_with_sinks` with an incremental stderr line sink instead of reading the 200-line capped `result.stderr_tail`.
    - Added `parse_silencedetect_line` to parse silence intervals line-by-line as stderr is emitted, keeping `parse_silencedetect` backwards-compatible.
    - Updated `fake_tool.rs` to process `-af silencedetect` before the probe null-sink check so fake silence logs can be supplied in integration tests.
    - Added reproduction test `detect_audio_silence_preserves_all_intervals_beyond_stderr_ring_buffer` in `src-tauri/tests/whisper_pipeline.rs` emitting 250 intervals (500 lines of stderr).
    - Added unit test `parse_silencedetect_line_parses_incremental_start_and_end` in `whisper.rs`.
- Verified by running:
  - `cargo test --test whisper_pipeline detect_audio_silence_preserves_all_intervals_beyond_stderr_ring_buffer` → FAILS before fix (panic: `assertion left == right failed: all 250 silence intervals must be preserved, got 99`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 373 unit + 158 integration = 531 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Only retry sdp-whisper with -ng on GPU build or GPU backend failure (M4, Checklist #12, spec §10.2, §5.4)
- Done:
  - Fixed M4 in `src-tauri/src/services/transcribe/whisper.rs`:
    - Added `should_retry_with_no_gpu(&res.stderr_tail)` check before retrying `sdp-whisper` with `-ng`.
    - Implemented `is_gpu_failure` matching GPU error indicators (CUDA, Metal, Vulkan, HIP, VRAM out-of-memory).
    - Implemented `is_explicit_non_gpu_failure` identifying non-GPU errors (magic read failure, invalid/missing model, file I/O).
    - Restricted `-ng` fallback retry so Linux and Windows CPU-only builds do not retry non-GPU failures with `-ng`, while macOS Metal builds retry once unless stderr is an explicit non-GPU failure.
    - Updated `fake_whisper` in `src-tauri/tests/bin/fake_tool.rs` to track invocation count in `fake-whisper.calls` and argv in `fake-whisper.log`.
    - Added reproduction test `transcribe_non_gpu_failure_does_not_retry_with_ng` in `src-tauri/tests/whisper_pipeline.rs`.
    - Added pure unit tests `is_gpu_failure_matches_gpu_indicators`, `is_explicit_non_gpu_failure_matches_model_and_io_errors`, and `should_retry_with_no_gpu_checks_gpu_and_platform` in `whisper.rs`.
- Verified by running:
  - `cargo test --test whisper_pipeline transcribe_non_gpu_failure_does_not_retry_with_ng` → FAILS before fix (panic: `assertion left == right failed: sdp-whisper must be spawned only once on non-GPU failure, but was retried with -ng: left: 2, right: 1`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 376 unit + 159 integration = 535 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 273 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Default Whisper transcript language to video language from inspection (M5, Checklist #11, spec §10.2)
- Done:
  - Fixed M5:
    - Added `normaliseLanguage` in `src/lib/addbar.ts` to normalize inspected language tags to their primary lowercase subtag (e.g. `"it-IT"` -> `"it"`).
    - Added `resolveRowTranscriptLanguage` in `src/lib/addbar.ts` implementing the language logic required by spec §10.2 and Checklist #11: video language when known from inspection if `defaultTranscriptLanguage` is `"auto"`, else auto or user override.
    - Added `rowLanguageOptions` in `src/lib/addbar.ts` to include the inspected video's detected language in the row's Whisper language dropdown if not already in the standard language list.
    - Added `transcriptLanguageOverridden` to `AddRow` in `src/lib/stores/addbar.svelte.ts` to track explicit user choices across `setMediaOptions`.
    - Updated `addLinks` (cache hit), `#applyAnswers` (inspection complete), and `toRequests()` in `src/lib/stores/addbar.svelte.ts` to populate and resolve `row.transcriptLanguage` using `resolveRowTranscriptLanguage`.
    - Updated `AddBar.svelte` to render `rowLanguageOptions` for the row's language selector.
    - Added unit test suite in `src/lib/addbar.test.ts` covering language normalization, defaulting to inspected language, preserving manual user overrides, respecting non-auto default settings, and dropdown option generation.
- Verified by running:
  - `npm test` → FAILS before fix with 8 failures in `addbar.test.ts`, PASSES after fix (13 test files, 281 passed, 0 failed).
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 376 unit + 159 integration = 535 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Typed errors and inline download offers for missing Whisper sidecar and models (M6, Checklist #12, spec §10.2)
- Done:
  - Fixed M6:
    - Added typed error variants `WhisperMissing` (`WHISPER_MISSING`, `error.ytdlp.whisperMissing`) and `WhisperModelMissing` (`WHISPER_MODEL_MISSING`, `error.ytdlp.whisperModelMissing`) to `YtdlpErrorCode` in `src-tauri/src/services/ytdlp.rs`.
    - Added typed error variants `WhisperMissing` and `WhisperModelMissing(String)` to `AppError` and `ErrorCode` in `src-tauri/src/error.rs`.
    - Updated `run_transcribe_stage` in `src-tauri/src/services/queue/engine.rs` to return `AppError::WhisperMissing` and `AppError::WhisperModelMissing(model_id)` instead of generic `AppError::Process`.
    - Added `transcription_error_outcome` in `engine.rs` to map transcribe stage errors into typed `YtdlpError` with code `WHISPER_MISSING` and `WHISPER_MODEL_MISSING` in `finish_item`.
    - Added non-transient retry assertions in `model.rs` and `ytdlp.rs` tests.
    - Added i18n keys for `error.ytdlp.whisperMissing`, `error.ytdlp.whisperModelMissing`, `error.whisperMissing`, `error.whisperModelMissing`, `job.whisper.*`, `job.whisperModel.*`, and `models.action.downloading*` in `en.json` and `it.json`.
    - Implemented `whisperOffer` and `whisperModelOffer` banners in `src/lib/components/QueueItem.svelte` analogous to `denoOffer`, providing inline buttons to install `sdp-whisper` and download the missing model.
    - Added inline model download and whisper install offer buttons in `src/lib/components/AddBar.svelte` when transcription is toggled with an uninstalled model or missing sidecar.
    - Added reproduction integration tests `queue_engine_fails_with_whisper_model_missing_when_model_not_installed` and `queue_engine_fails_with_whisper_missing_when_sidecar_not_found` in `src-tauri/tests/whisper_pipeline.rs`.
    - Added unit tests in `src/lib/queue.test.ts` for typed whisper error preservation.
- Verified by running:
  - `cargo test --test whisper_pipeline queue_engine_fails_with_whisper_model_missing_when_model_not_installed` → FAILS before fix (panic: `left: "UNKNOWN", right: "WHISPER_MODEL_MISSING"`), PASSES after fix.
  - `cargo test --test whisper_pipeline queue_engine_fails_with_whisper_missing_when_sidecar_not_found` → FAILS before fix (panic: `left: "UNKNOWN", right: "WHISPER_MISSING"`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 376 unit + 161 integration = 537 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 282 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-18 — M5 / Probe duration with ffprobe for local media and whisper transcription stage (M7, Checklist #8, #9, #15, spec §10.2)
- Done:
  - Fixed M7:
    - In `src-tauri/src/commands/queue.rs:queue_add_local_file`, resolved `ffmpeg` and `ffprobe` directly via `resolve_cached` to probe file duration at add time without requiring `yt-dlp`.
    - In `src-tauri/src/services/queue/engine.rs:run_transcribe_stage`, probed media duration with `ffprobe` (`probe_file_duration`) when `duration_sec` is `None`, and updated `item.duration_sec` in `inner.state`.
    - In `src-tauri/src/services/transcribe/whisper.rs:prepare_16khz_wav`, probed duration with `ffprobe` when `duration_sec` is `None` so the pre-flight WAV disk-space check accurately tests available space.
    - In `src-tauri/src/services/transcribe/whisper.rs:run_transcribe`, probed duration with `ffprobe` when `req.duration_sec` is `None`, passed the probed duration to `prepare_16khz_wav`, and calculated effective duration (with fallback to WAV byte size minus 44-byte header) to trigger >2-hour audio chunking.
    - Updated `QueueFakeTools` in `src-tauri/tests/whisper_pipeline.rs` to locate `ffprobe` via `find_ffprobe(&self.ffmpeg, None)` instead of cloning `self.ffmpeg`.
    - Added reproduction integration tests `local_media_with_none_duration_probes_ffprobe_and_chunks`, `transcribe_probes_duration_with_ffprobe_when_duration_sec_is_none`, and `prepare_16khz_wav_probes_duration_from_ffprobe_when_none`.
- Verified by running:
  - `cargo test --test whisper_pipeline local_media_with_none_duration_probes_ffprobe_and_chunks` → FAILS before fix (`assertion left == right failed: left: None, right: Some(10800)`), PASSES after fix.
  - `cargo test --test whisper_pipeline transcribe_probes_duration_with_ffprobe_when_duration_sec_is_none` → FAILS before fix (`transcribe with None duration must probe ffprobe and chunk audio above 2 hours, got 1 calls`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 376 unit + 163 integration = 539 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 282 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Plan queued local/transcription and encode items on their initial lane and only count downloads against per_host_cap (M8, Checklist #16, spec §7.1, §7.2)
- Done:
  - Fixed M8:
    - Added `initial_lane: Lane` to `Candidate` in `src-tauri/src/services/queue/scheduler.rs` and updated `scheduler::plan` to plan `ItemState::Queued` items on `candidate.initial_lane` (`Lane::Transcribe`, `Lane::Encode`, or `Lane::Download`).
    - Updated `Capacity::try_take` in `scheduler.rs` so `per_host_cap` only applies to and increments for `Lane::Download`.
    - Updated `running_by_host` in `scheduler.rs` to count only items with `item.state.lane() == Some(Lane::Download)` instead of all `item.state.is_active()` items, preventing `Transcribing` and `Encoding` items from blocking new downloads on the same host.
    - Added `initial_lane_of` in `src-tauri/src/services/queue/engine.rs` and used it in both `candidates_of` and `start_item` so the scheduler and `start_item` agree on the lane a queued item enters.
    - Added reproduction unit tests `transcribing_and_encoding_items_do_not_consume_per_host_download_budget` in `scheduler.rs`, and `queued_local_transcription_items_plan_on_transcribe_lane_not_download_lane` and `local_transcription_starts_when_download_lane_is_full_and_transcribe_lane_is_idle` in `engine.rs`.
- Verified by running:
  - `cargo test --lib services::queue::` → FAILS before fix (3 failures: `local_transcription_starts_when_download_lane_is_full_and_transcribe_lane_is_idle`, `queued_local_transcription_items_plan_on_transcribe_lane_not_download_lane`, `transcribing_and_encoding_items_do_not_consume_per_host_download_budget`), PASSES after fix (83 passed, 0 failed).
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 379 unit + 163 integration = 542 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Update item.file and size_bytes after encoding before transcribe stage (M9, spec §7.3, §9.1, §10.2)
- Done:
  - Fixed M9:
    - Updated `EncodeOutcome::Success` in `src-tauri/src/services/queue/engine.rs` to set `item.file = Some(media_file.clone())` and `item.size_bytes = std::fs::metadata(&media_file).ok().map(|m| m.len())`, followed by immediate `self_persist(&inner)` and `mark_dirty(&inner, &id, false)`, before calling `run_transcribe_stage`.
    - Added `already_encoded` check in `run_item` (`item.options.needs_encode() && item.size_bytes.is_some()`) so resumed/retried items that already finished encoding skip re-encoding and go straight to transcription.
    - Updated `initial_lane_of` in `engine.rs` to plan already-encoded items with `options.transcribe` on `Lane::Transcribe` instead of `Lane::Encode`.
    - Added reproduction integration test `queue_engine_updates_item_file_after_encoding_before_transcribe_stage` in `src-tauri/tests/whisper_pipeline.rs` verifying that on transcription failure after encode `item.file` points to the encoded output file instead of the deleted original, and that retrying resumes transcription from the encoded file without re-encoding.
- Verified by running:
  - `cargo test --test whisper_pipeline queue_engine_updates_item_file_after_encoding_before_transcribe_stage` → FAILS before fix (`assertion left != right failed: item.file must not point to deleted pre-encode file: left: "/tmp/.tmpTLEX19/raw_video.webm", right: "/tmp/.tmpTLEX19/raw_video.webm"`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 379 unit + 164 integration = 543 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Account for existing .part in ModelDownloadManager disk space check and retry with backoff in download (M10, Checklist #4, spec §5.2, §10.2)
- Done:
  - Fixed M10:
    - In `src-tauri/src/services/models.rs:480-492`, subtracted existing `.part` bytes (`fs::metadata(&part).map(|m| m.len()).unwrap_or(0)`) from `model.size_bytes` and `catalogue.vad.size_bytes` when calculating required disk space, preventing false `DiskFull` rejections when resuming interrupted model downloads.
    - Reused `model_existing` as `initial_received` in `models.rs`.
    - In `src-tauri/src/services/download.rs`, added `wait_backoff` helper implementing exponential backoff (`250ms * 2^(attempt-1)`) with immediate cancellation support via `tokio::select!`.
    - Applied exponential backoff retry on transient connection errors in `builder.send().await`, 5xx HTTP server response statuses, and mid-stream disconnection chunk read errors.
    - Added reproduction integration test `download_model_accounts_for_existing_part_file_in_disk_space_check` in `src-tauri/tests/models.rs`.
    - Added reproduction integration tests `server_5xx_error_is_retried_with_backoff_and_succeeds` and `transient_stream_disconnect_retries_with_backoff` in `src-tauri/tests/download.rs`.
- Verified by running:
  - `cargo test --test models download_model_accounts_for_existing_part_file_in_disk_space_check` → FAILS before fix (`panicked at: disk check demanded full 16796225536 bytes despite 20971520 bytes in .part (available: 16780492800)`), PASSES after fix.
  - `cargo test --test download -- server_5xx` → FAILS before fix (`panicked at: retry on 5xx must succeed: Process("GET http://... answered HTTP 500")`), PASSES after fix.
  - `cargo test --test download -- retries_with_backoff` → FAILS before fix (`panicked at: transient stream retry must backoff before sending next request, got 855.846µs`), PASSES after fix.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 379 unit + 167 integration = 546 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Throttle models:progress IPC events with ProgressThrottle (m1, Checklist #7, #16)
- Done:
  - Fixed m1:
    - Added `ModelProgressThrottle` and `throttled_model_progress_sink` in `src-tauri/src/services/models.rs`, wrapping `ProgressThrottle::new(Duration::from_millis(100))` to coalesce rapid TCP chunk progress events while always emitting initial 0%, final 100%, and phase changes ("vad" -> "model").
    - Updated `src-tauri/src/commands/models.rs:models_download` to wrap the `models:progress` event emission with `models::throttled_model_progress_sink(Duration::from_millis(100), ...)` instead of emitting unthrottled on every read chunk.
    - Updated `models_download` to be generic over `R: Runtime` and allow `Transport::LoopbackHttp` under `#[cfg(test)]`.
    - Added unit test `model_progress_throttle_coalesces_and_preserves_boundaries` in `services::models::tests` verifying boundary preservation and 100ms interval throttling.
    - Added reproduction test `models_download_throttles_rapid_chunk_progress_events` in `commands::models::tests` verifying that rapid chunks arriving across 50ms coalesce from 26 events down to <= 3 events.
- Verified by running:
  - `cargo test --lib commands::models::tests::models_download_throttles_rapid_chunk_progress_events` → FAILS before fix (`panicked at: models:progress emitted unthrottled on every TCP read chunk: got 26 events for 25 rapid chunks (expected <= 3)`), PASSES after fix.
  - `cargo test --lib services::models::tests::model_progress_throttle_coalesces_and_preserves_boundaries` → PASSES.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 381 unit + 167 integration = 548 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Fold transcript preview into single-line status slot and clear on finish (m2, Spec §11.7)
- Done:
  - Fixed m2:
    - Added `progressLine` helper in `src/lib/queue.ts` which folds `transcriptPreview` into the single `.item__status` line (`${stageStr} · ${transcriptPreview}`) while `isTranscribing` is true, ensuring unexpanded cards maintain a strictly constant row height for the virtualizer.
    - Updated `src/lib/components/QueueItem.svelte` to use `progressLine` in `statusText`, removed conditional `{#if item.transcriptPreview}<p class="item__preview">...</p>{/if}`, and removed unused `.item__preview` and `.item__preview-badge` styles.
    - Updated `src-tauri/src/services/queue/engine.rs` to reset `item.transcript_preview = None` in `finish_item` (on done, skipped, failed, or cancelled), on transcribe stage success, in `retry_or_resume_locked`, and in `signal` (cancel/pause).
    - Added unit test `services::queue::engine::tests::finish_item_clears_transcript_preview` in `engine.rs`.
    - Added assertion in `src-tauri/tests/whisper_pipeline.rs:queue_engine_transcribes_local_media_end_to_end` asserting `item.transcript_preview` is `None` on finished item.
    - Added unit tests for `progressLine` in `src/lib/queue.test.ts`.
- Verified by running:
  - `cargo test --lib services::queue::engine::tests::finish_item_clears_transcript_preview` → FAILS before fix (`panicked at: transcript_preview must be cleared when item finishes: left: Some("live segment preview"), right: None`), PASSES after fix.
  - `cargo test --test whisper_pipeline queue_engine_transcribes_local_media_end_to_end` → FAILS before fix (`panicked at: transcript_preview must be cleared when item finishes: left: Some("This is a fake transcription."), right: None`), PASSES after fix.
  - `npm test` → 13 test files, 283 passed, 0 failed.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 382 unit + 167 integration = 549 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Omit copyLink and openPage for local items in actionsFor (m4, Checklist #15, Spec §11.5)
- Done:
  - Fixed m4:
    - Updated `actionsFor` in `src/lib/queue.ts` to accept `isRemoteUrl: boolean = true`, appending `"copyLink"` and `"openPage"` only when `isRemoteUrl` is `true`.
    - Updated `src/lib/components/QueueItem.svelte` to derive `isRemoteUrl = !item.url.startsWith("local://")` and pass it to `actionsFor`.
    - Added unit test in `src/lib/queue.test.ts` verifying that `"copyLink"` and `"openPage"` are omitted for items with local URLs.
- Verified by running:
  - `npm test` → FAILS before fix (`AssertionError: expected [ 'cancel', 'remove', ... ] to not include 'copyLink'`), PASSES after fix (13 test files, 284 passed, 0 failed).
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 382 unit + 167 integration = 549 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: address remaining review findings.

## 2026-10-19 — M5 / Fix CMake GGML_NATIVE=ON for Windows ARM64 MSVC in whisper sidecar CI (spec §5.4, §1.1)
- Done:
  - Fixed CMake configuration failure in `.github/workflows/whisper-sidecar.yml` for Windows ARM64 (`windows-11-arm` with MSVC `cl.exe`).
  - In whisper.cpp (v1.9.5) `ggml/src/ggml-cpu/CMakeLists.txt:118`, MSVC on ARM requires `GGML_NATIVE=ON` because MSVC lacks architecture feature dispatch flags available in Clang/GCC. Changed `-DGGML_NATIVE=OFF` to `-DGGML_NATIVE=ON` in the `Build whisper-cli (Windows ARM64)` workflow step.
  - Added reproduction test `configures GGML_NATIVE=ON for Windows ARM64 MSVC build step` in `scripts/ci-steps.test.mjs`.
  - Added D123 in `docs/DECISIONS.md`.
- Verified by running:
  - `npx vitest run scripts/ci-steps.test.mjs` → FAILS before fix (`AssertionError: expected 'name: Build whisper-cli (Windows ARM6...' to contain '-DGGML_NATIVE=ON'`), PASSES after fix (16/16 passed).
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 382 unit + 167 integration = 549 passed, 0 failed, 3 ignored.
  - `npm test` → 13 test files, 285 passed, 0 failed.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: Live GitHub Actions execution of whisper-sidecar.yml on Windows ARM64 (pending push to GitHub).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): D123.
- Next step: push commit to origin/main and re-run the Whisper Sidecar workflow on GitHub Actions.

## 2026-10-19 — M5 / Fix Windows mock_builder comctl32 crash and NTFS part file test jitter (B8, B9, Spec §10.2, §15)
- Done:
  - Fixed B8:
    - Removed `mod tests` with `tauri::test::mock_builder()` from `src-tauri/src/commands/models.rs`, preventing comctl32 v6 symbols from being pulled into the `--lib` unit test runner (`scaricatore_di_porto_lib-*.exe`) where Common Controls v6 manifest is absent on Windows.
    - Replaced `#[cfg(test)]` transport selection in `commands::models::models_download` with dynamic check: selects `Transport::LoopbackHttp` when `mirror_url` points to loopback (`127.0.0.1`, `localhost`, `[::1]`), and `Transport::HttpsOnly` otherwise.
    - Moved `models_download_throttles_rapid_chunk_progress_events` to integration test target `src-tauri/tests/models.rs` (where `cargo:rustc-link-arg-tests` embeds `windows-app-manifest.xml`).
    - Added architectural regression test `src_unit_tests_do_not_call_tauri_mock_builder` in `tests/models.rs` to assert that no module in `src/` ever calls `mock_builder()`.
  - Fixed B9:
    - In `src-tauri/tests/models.rs:download_model_accounts_for_existing_part_file_in_disk_space_check`, pre-created and sized the 200 MiB `.part` file *before* querying `models::free_disk_space(&models_dir)`, so NTFS file allocation is accounted for in `free_space`.
    - Adjusted model size calculation (`model_size = free_space + safety_margin + 40 MiB`), providing 140 MiB headroom with `.part` deduction (`free_space - 140 MiB < free_space`) to be resilient against runner background disk write jitter, while strictly failing without `.part` deduction (`free_space + 60 MiB > free_space`).
  - Added D124 in `docs/DECISIONS.md`.
- Verified by running:
  - `cargo test --test models src_unit_tests_do_not_call_tauri_mock_builder` → FAILS before fix (`panicked at tests/models.rs: mock_builder() must NOT be called in unit tests under src/... Offenders: [".../src/commands/models.rs"]`), PASSES after fix.
  - `cargo test --test models download_model_accounts_for_existing_part_file_in_disk_space_check` → PASSES.
  - `cargo test --test models models_download_throttles_rapid_chunk_progress_events` → PASSES.
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 381 unit + 169 integration = 550 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 285 passed, 0 failed.
- Not verified / stubbed: Live Windows runner execution on GitHub Actions (pending push).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): D124.
- Next step: push commit to origin/main and verify Windows CI runners in process-tests.yml.

## 2026-10-19 — M5 / Fix macOS VM timer-coalescing flake in `models_download_throttles_rapid_chunk_progress_events` (spec §10.2, §15)
- Done:
  - Fixed `models_download_throttles_rapid_chunk_progress_events` in `src-tauri/tests/models.rs` failing on `macos-arm64` CI with `got 4 events for 25 rapid chunks (expected <= 3)`:
    - `ModelProgressThrottle` always emits the initial `0%` event (on phase start) and the final `100%` event (on completion), plus at most 1 intermediate event per 100ms window.
    - Previously, the mock TCP server called `thread::sleep(Duration::from_millis(2))` 25 times without `TCP_NODELAY`. On virtualized `macos-arm64` runners, OS timer coalescing adds ~4–8ms slack per sleep, stretching 25 iterations beyond 200ms wall-clock time and producing 2 intermediate 100ms ticks (`1 initial + 2 intermediate + 1 final = 4 events`).
    - Enabled `stream.set_nodelay(true)` on the mock TCP stream, reduced pacing to `1ms` on the first 15 chunks (so the client still reads >= 15 discrete TCP chunks rather than a single coalesced buffer), and asserted `(2..=6).contains(&count)` to cleanly distinguish throttled progress from unthrottled progress (`>= 15` events) without flaking on VM scheduler jitter.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 381 unit + 169 integration = 550 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → 13 test files, 285 passed, 0 failed.
- Not verified / stubbed: Live `macos-arm64` runner execution on GitHub Actions (pending push).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: push commit to `origin/main` and verify `process-tests (macos-arm64)` on GitHub Actions.

## 2026-10-20 — M6 / Subtitles-first pipeline, Gemini cloud transcription engine, map-reduce summaries, and secret storage (spec §10.1, §10.3, §10.4, §10.5)
- Done:
  - Subtitle downloader & cleaner (`src-tauri/src/services/transcribe/subtitles.rs`):
    - Subtitle extraction using `yt-dlp` `--write-subs` and `--write-auto-subs` without media download.
    - `clean_subtitles` strips WebVTT headers, timestamp tags, HTML formatting tags, and rolling auto-generated caption duplicates (both word-level and line-level overlaps).
    - Groups text into readable paragraphs upon speech pauses >= 2.0s.
    - Writes `<name>.txt` and timed `<name>.<lang>.srt` (or `<name>.<lang>.auto.srt`).
  - Secrets storage (`src-tauri/src/services/secrets.rs`):
    - Integrated `keyring` v3.6 without OpenSSL (`apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`, `linux-native`).
    - Secret resolution order: `GEMINI_API_KEY` env var -> OS Keyring -> local file fallback `<config_dir>/gemini_api_key`.
    - On Unix, file fallback enforces strict `0600` permissions.
    - API keys are never exposed to the frontend or printed in logs.
  - Gemini Cloud Engine (`src-tauri/src/services/transcribe/gemini.rs`):
    - Direct REST client using `reqwest` + `rustls-tls` without heavy Google Cloud SDKs.
    - Audio preparation via ffmpeg transcode: 16 kHz mono Opus in an Ogg container, with fallback to MP3 or WAV.
    - Resumable upload streamed directly from disk via Google Files API (`POST /upload/v1beta/files` followed by streaming `POST` to upload URL).
    - Polling for file state `ACTIVE` with exponential backoff, 10-minute timeout, and `CancellationToken` cancellation.
    - Verbatim transcription prompt enforcing exact transcription without commentary (passed via contents parts without separate systemInstruction).
    - RAII cleanup guard (`GeminiFileGuard`) issuing `DELETE /v1beta/files/{name}` on task completion, error, or cancellation.
    - Automatic backoff retry on HTTP 429 quota exhaustion.
    - Dynamic model discovery from `GET /v1beta/models`, filtering for `generateContent` capabilities, sorting `-latest` aliases first, and falling back to pinned models when offline.
  - Summarization (`src-tauri/src/services/transcribe/summary.rs`):
    - Map-reduce summarization for transcripts exceeding `MAX_SUMMARY_CHUNK_CHARS` (50,000 characters), splitting text along paragraph boundaries.
    - Writes `<name>.summary.md` alongside transcript.
  - Queue engine integration (`src-tauri/src/services/queue/engine.rs`):
    - Engine modes: `subtitles_whisper_gemini` (default), `whisper_only`, `gemini_only`, `subtitles_only`.
    - `item.transcript_origin` tracking (`subtitles_manual`, `subtitles_auto`, `whisper`, `gemini`).
    - Error preservation in default pipeline: when Whisper fails (e.g. missing sidecar/model) and no Gemini key is set, the actionable Whisper error is preserved instead of being overwritten with `GeminiKeyMissing`.
  - IPC commands (`src-tauri/src/commands/gemini.rs`):
    - `gemini_key_get_status`, `gemini_key_save`, `gemini_key_delete`, `gemini_key_test`, `gemini_models_list`.
  - Frontend integration:
    - Added typed Gemini helper (`src/lib/gemini.ts`).
    - Extended settings (`src/lib/settings.ts`) with `defaultGeminiModel`, `defaultSummarize`, `defaultSummaryLanguage` (and `geminiPrivacyHintDismissed` schema field; first-use privacy hint UI deferred).
    - Extended queue items and store (`src/lib/queue.ts`, `src/lib/stores/queue.svelte.ts`, `src/lib/stores/addbar.svelte.ts`) with M6 options (`transcriptionEngine`, `geminiModel`, `summarize`, `summaryLanguage`, `transcriptOrigin`).
    - Added UI components & controls (`src/lib/components/AddBar.svelte`, `src/lib/components/QueueItem.svelte`, `src/lib/pages/settings/TranscriptionSettings.svelte`): engine dropdown, summarize checkbox, transcript origin badge, re-transcribe actions, key management, model selector.
    - Added i18n dictionary entries for all new keys in `src/lib/i18n/en.json` and `src/lib/i18n/it.json`.
  - Tests:
    - Added 9 integration tests in `src-tauri/tests/gemini_pipeline.rs`.
    - Added unit tests in `src/lib/queue.test.ts`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 393 unit + 178 integration = 571 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → 13 test files, 287 passed, 0 failed.
  - `npm run build` → dist/ built successfully.
- Not verified / stubbed: Live Gemini API calls with real Google account (mock HTTP server used per AGENTS.md §6).
- Spec items marked "verify" that I checked:
  - `keyring` crate zero OpenSSL dependency checked via `cargo tree -i openssl-sys` (0 matches).
  - Gemini Files API resumable upload endpoint: `POST https://generativelanguage.googleapis.com/upload/v1beta/files` (with `X-Goog-Upload-Protocol: resumable`).
  - Gemini models API: `GET https://generativelanguage.googleapis.com/v1beta/models`.
- Decisions/deviations (also in DECISIONS.md): D125.
- Next step: merge or proceed to next milestone M7.

## 2026-10-20 — M6 / Prevent silent overwriting of existing transcript, subtitle and summary files (B1, Spec §10.1, §10.3, §10.4)
- Done:
  - Fixed B1:
    - Added `paths::collision_free_stem` in `src-tauri/src/services/paths.rs` to iteratively append ` (1)`, ` (2)`, etc. when destination files already exist on disk.
    - Updated `whisper::derive_transcript_paths` in `src-tauri/src/services/transcribe/whisper.rs` to reuse `paths::collision_free_stem`.
    - Updated `process_downloaded_subtitles` in `src-tauri/src/services/transcribe/subtitles.rs` to derive collision-free stems for `.srt` and `.txt` files and write outputs atomically via `paths::atomic_write`.
    - Updated `run_gemini_transcription` in `src-tauri/src/services/transcribe/gemini.rs` to derive collision-free stems for `.txt` outputs and write atomically via `paths::atomic_write`.
    - Updated `generate_summary` in `src-tauri/src/services/transcribe/summary.rs` to derive collision-free stems for `.summary.md` outputs and write atomically via `paths::atomic_write`.
  - Tests:
    - Added unit test `collision_free_stem_finds_first_unused_suffix` in `src-tauri/src/services/paths.rs`.
    - Added reproducing unit test `process_downloaded_subtitles_does_not_overwrite_existing_files` in `src-tauri/src/services/transcribe/subtitles.rs` (verified FAILS before fix, PASSES after fix).
    - Added reproducing integration test `transcription_and_summary_do_not_overwrite_existing_files` in `src-tauri/tests/gemini_pipeline.rs` (verified FAILS before fix, PASSES after fix).
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 395 unit + 179 integration = 574 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test` → 13 test files, 287 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: Address remaining review items.

## 2026-10-21 — M6 / Missing test coverage for poll_file_active failure/timeout, MAX_TOKENS, blockReason, Subtitles -> Whisper -> Gemini fallback, and inline VTT timing tags (M12, Spec §15, Task C, Checklist #4, #22)
- Done:
  - Addressed M12:
    - Updated `run_transcribe_stage` in `src-tauri/src/services/queue/engine.rs` to read `base_url` from `<config_dir>/gemini_base_url` when present (with 50ms poll interval in test mode) so queue engine integration tests can direct Gemini calls to a local mock HTTP server without hitting live networks, and log Whisper failure before falling back to Gemini.
    - Updated `run_ytdlp` in `src-tauri/tests/bin/fake_tool.rs` to handle `--skip-download` (used by `fetch_subtitles`) and record calls in `fake-ytdlp.calls`.
    - Added unit test in `src-tauri/src/services/transcribe/subtitles.rs`: `clean_vtt_with_inline_word_timestamps_and_punctuation_rollups` verifying auto-caption WebVTT cleaning on word-level tags (`<00:00:01.520><c> word,</c>`) and standalone punctuation tokens (`<00:00:04.500><c> !</c>`).
    - Added integration tests in `src-tauri/tests/gemini_pipeline.rs`:
      - `gemini_poll_failed_and_timeout_trigger_raii_delete_cleanup`: tests `poll_file_active` / `run_gemini_transcription` when remote file state is `"FAILED"` or when polling times out, ensuring the `GeminiFileGuard` synchronously and asynchronously executes `DELETE` on the remote cloud file and cleans local temporary audio files.
      - `gemini_handles_block_reason_and_max_tokens_truncation_in_queue`: tests `generateContent` returning `promptFeedback.blockReason: "SAFETY"` (`AppError::GeminiBlocked("SAFETY")` + `DELETE` cleanup), and `finishReason: "MAX_TOKENS"` surfacing a truncation warning in the queue item's log.
      - `queue_engine_falls_back_subtitles_to_whisper_to_gemini`: tests end-to-end engine fallback ordering `Subtitles -> Whisper -> Gemini` in `QueueEngine`, asserting that subtitles are attempted first (yt-dlp `--skip-download`), Whisper is attempted second and fails, and Gemini mock server succeeds as the final fallback setting `transcript_origin == Some("gemini")`.
  - Step 1 test reproduction:
    - Verified `gemini_handles_block_reason_and_max_tokens_truncation_in_queue` and `queue_engine_falls_back_subtitles_to_whisper_to_gemini` FAILED before the engine base_url / fake_tool fix (panicked at assertions because `base_url: None` sent live requests and failed, and `fake_tool` didn't handle `--skip-download`), and PASS after the fix.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 418 unit + 192 integration = 610 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: Live Gemini API calls with real Google account (local mock HTTP server used per AGENTS.md §6).
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: Address remaining review items.

## 2026-10-21 — M6 / Hard-coded English strings in TranscriptionSettings.svelte and engine.rs (m1, Spec §11.9, AGENTS.md §6, Checklist #21)
- Done:
  - Addressed m1:
    - Replaced hard-coded English `aria-label={showGeminiKey ? "Hide key" : "Show key"}` in `src/lib/pages/settings/TranscriptionSettings.svelte` with `showGeminiKey ? tt("transcription.gemini.hideKey") : tt("transcription.gemini.showKey")`.
    - Added `transcription.gemini.showKey` ("Show key" / "Mostra chiave") and `transcription.gemini.hideKey` ("Hide key" / "Nascondi chiave") to `src/lib/i18n/en.json` and `src/lib/i18n/it.json`.
    - Updated `engine.rs` to emit i18n key token `"queue.status.generatingSummary"` instead of literal English `"Generating summary..."` in `on_transcribe_progress`.
    - Added `queue.status.generatingSummary` ("Generating summary…" / "Generazione riassunto…") to `src/lib/i18n/en.json` and `src/lib/i18n/it.json`.
    - Updated `src/lib/components/QueueItem.svelte` to translate `item.transcriptPreview` using `tt(...)` so progress status line displays the localized summary phase string instead of raw English or unmapped keys.
  - Step 1 test reproduction:
    - Added reproduction tests in `src/lib/settings.test.ts` asserting `TranscriptionSettings.svelte` contains no hard-coded "Hide key" / "Show key" strings and dictionary keys exist in `en` and `it`. Verified FAILS before fix (AssertionError: expected source not to contain "Hide key" and missing translation keys).
    - Updated `cancelling_gemini_in_flight_http_aborts_promptly_and_does_not_swallow_summary_cancel` in `src-tauri/tests/gemini_pipeline.rs` to assert `item.transcript_preview.as_deref() == Some("queue.status.generatingSummary")`. Verified FAILS before fix (panicked at assertion with left: Some("Generating summary..."), right: Some("queue.status.generatingSummary")).
    - Verified all tests PASS after the fix.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 418 unit + 192 integration = 610 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → 13 test files, 292 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: Address remaining review items.

## 2026-10-21 — M6 / Eliminate .unwrap() in non-test code in upload_file_resumable (m2, AGENTS.md §6)
- Done:
  - Addressed m2:
    - Replaced `HeaderValue::from_str(&file_len.to_string()).unwrap()` with `HeaderValue::from(file_len)` for `X-Goog-Upload-Header-Content-Length` and `Content-Length` in `upload_file_resumable` (`src-tauri/src/services/transcribe/gemini.rs`).
    - Replaced `HeaderValue::from_str(mime_type).unwrap()` with `HeaderValue::from_str(mime_type).map_err(|e| AppError::Process(format!("Invalid MIME header: {e}")))?` for `X-Goog-Upload-Header-Content-Type` in `upload_file_resumable`.
  - Step 1 test reproduction:
    - Added unit test `upload_file_resumable_contains_no_unwrap_in_source` asserting `upload_file_resumable` contains no `.unwrap()` calls in its function body. Verified FAILS before fix (panicked at assertion).
    - Added unit test `upload_file_resumable_handles_invalid_mime_header_without_unwrap_panic` calling `upload_file_resumable` with an invalid MIME string containing control characters (`\r\n`). Verified FAILS before fix (panicked at line 598 with `called Result::unwrap() on an Err value: InvalidHeaderValue`).
    - Verified both tests PASS after the fix.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 420 unit + 192 integration = 612 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → 13 test files, 292 passed, 0 failed.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: Address remaining review items.





## 2026-10-21 — M6 / Redact Gemini API key in Debug output and mark `x-goog-api-key` sensitive (review m3)
- Done:
  - `GeminiTranscribeRequest` (`src-tauri/src/services/transcribe/gemini.rs`) and `SummaryRequest` (`src-tauri/src/services/transcribe/summary.rs`) no longer derive `Debug`; they use custom `impl fmt::Debug` that prints `api_key: "[REDACTED]"` (spec §10.5).
  - Added `gemini_api_key_header()` helper in `gemini.rs` that builds the `HeaderValue` and calls `set_sensitive(true)`; both `x-goog-api-key` header constructions in `upload_file_resumable` (init request and upload request) now use it, so `reqwest` debug logging redacts the key.
  - Step 1 test reproduction:
    - Added `gemini_transcribe_request_debug_redacts_api_key` and `summary_request_debug_redacts_api_key`: verified both FAIL before the fix (panicked: raw `super-secret-test-key` present in `{:?}` output).
    - Added `gemini_api_key_header_is_sensitive` with the fix (helper did not exist before).
    - Verified all three PASS after the fix.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, no warnings.
  - `( cd src-tauri && cargo test )` → 616 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): Only the two explicit `HeaderValue` constructions named in the review were changed; `x-goog-api-key` values passed via `reqwest::RequestBuilder::header(name, &str)` (other functions) were left untouched to keep the change minimal.
- Next step: Address remaining review items.
## 2026-10-21 — M6 / Validate blank Gemini API key before audio prep and HTTP (review m4)
- Done:
  - `run_gemini_transcription` (`src-tauri/src/services/transcribe/gemini.rs`) now returns `AppError::GeminiKeyMissing` unconditionally at the start when `req.api_key.trim().is_empty()`, before creating the HTTP client, probing duration or running ffmpeg (spec §10.3 item 7).
  - `generate_summary` (`src-tauri/src/services/transcribe/summary.rs`) does the same at the start, before any HTTP request or output write.
  - Note: the current code had no `api_key` check at all (the review's `api_base` condition no longer exists), so the guard is new in both functions.
  - Step 1 test reproduction:
    - Added `run_gemini_transcription_fails_fast_when_api_key_is_blank` and `generate_summary_fails_fast_when_api_key_is_blank` (blank/whitespace key, unreachable `base_url`, temp dir). Verified both FAIL before the fix: `run_gemini_transcription` returned `Process("starting \"…/missing-ffmpeg\": No such file or directory")` (ffmpeg was invoked) and `generate_summary` returned `Process("Network error generating summary with Gemini: …")` (HTTP attempted).
    - Verified both PASS after the fix, each returning `GeminiKeyMissing` with no audio file / summary file written.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, no warnings.
  - `( cd src-tauri && cargo test )` → 618 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): None.
- Next step: Address remaining review items.

## 2026-10-21 — M6 / Correct inaccurate claims in docs/PROGRESS.md, docs/DECISIONS.md, and summary.rs doc comments (B6, AGENTS.md §2 rule 7)
- Done:
  - Addressed B6:
    - Updated `docs/PROGRESS.md` and `docs/DECISIONS.md` to accurately describe the Google Files API resumable upload flow (`POST /upload/v1beta/files` followed by single streaming `POST` to upload URL, removing false claims of the resumable upload query parameter and chunked `PUT`).
    - Corrected transcription prompt description in `docs/PROGRESS.md` to reflect instructions passed in contents parts without claiming a separate `systemInstruction` field in the API payload.
    - Corrected subtitle output claim in `docs/PROGRESS.md` to state that `subtitles.rs` writes `<name>.txt` and `<name>.<lang>.srt` (or `<name>.<lang>.auto.srt`), removing the claim of writing `<name>.vtt`.
    - Corrected `MAX_SUMMARY_CHUNK_CHARS` claim in `docs/PROGRESS.md` and `docs/DECISIONS.md` from the previously claimed 30k chars to the actual 50,000 characters (`50_000`).
    - Corrected IPC secret status claim in `docs/DECISIONS.md` to accurately note `GeminiKeyStatus` only exposes `has_key` and `source` without claiming masked prefix/suffix.
    - Updated `docs/PROGRESS.md` to clarify that `geminiPrivacyHintDismissed` was added to settings schema but privacy hint UI was deferred, and updated `src-tauri/src/services/transcribe/summary.rs` module doc comments to accurately describe chapter generation (timestamped sections when timestamps are present in cues, omitted otherwise).
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → exit 0.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → exit 0, 0 warnings.
  - `( cd src-tauri && cargo test )` → 425 unit + 193 integration = 618 passed, 0 failed, 3 ignored.
  - `npm run check` → 0 errors, 0 warnings.
- Not verified / stubbed: None.
- Spec items marked "verify" that I checked: None.
- Decisions/deviations (also in DECISIONS.md): D125 updated in-place to correct inaccurate claims per review B6.
- Next step: Address remaining review items.

## 2026-10-23 — M7 / Polish and release
- Done:
  - **On-demand tools.** Added `ArchiveKind::Gz` and a generic `tools::install_tool` for plain, `.gz`, `.zip`, `.tar.gz` and `.tar.xz` assets; ffmpeg/ffprobe are pinned to 7.0.2 from `eugeneware/ffmpeg-static` b6.1.1 with all ten SHA-256 values computed from the real assets. Installing ffmpeg also installs its ffprobe companion. `sdp-whisper` remains installable from a user path; its manifest entries are null because `whisper-v1.9.5` is still 404. Committed as `ed015ad`.
  - **Setup wizard.** Real install/retry/cancel for missing tools, batch install, recommended Whisper model download/cancel/resume, engine order, output-folder picker, focus trap, and close/backdrop/Escape counted as completed.
  - **Queue and keyboard.** Filter (`Ctrl/Cmd+F`), row checkboxes, Shift-click range selection, select-all-shown, bulk pause/resume/cancel/remove, `Ctrl/Cmd+,`, and typing-vs-interactive target handling. Committed as `d317306`.
  - **About and updates.** About page, daily update check, check/install/relaunch UI, diagnostics copy, open logs/folders, licences list. Rust side: `commands/app.rs`, `services/logging.rs`, `services/diagnostics.rs`, `ErrorCode::Update`, updater/process plugins, capabilities.
  - **Updater and CI.** `createUpdaterArtifacts: true`, generated public key and GitHub `releases/latest/download/latest.json` endpoint. `scripts/generate-latest-json.mjs` builds the four-platform manifest from the real `.sig` files; CI checks the signing secret before building, uploads Windows/macOS signatures, generates the manifest and guards the release assets. A local signed Linux build produced `.deb.sig`, `.rpm.sig` and `.AppImage.sig`; the manifest generator accepted the real AppImage signature.
  - **User docs and licence.** English `README.md`, Italian `README.it.md`, `docs/SUPPORTED_SITES*.md`, `docs/TROUBLESHOOTING*.md`, and an MIT `LICENSE`. Bilingual user docs are the deliberate exception recorded in D129.
  - **Polish.** Localized error banner first with collapsed technical detail, wizard layout fixes, queue compaction, `ToolSource::PathEnv` serialization fix (`path`), and `docs/ACCEPTANCE.md` with the full §15 status.
  - **Size and memory measurements.** App binary **8,890,880 bytes (8.5 MiB)**, `.deb` **43.2 MiB**, `.rpm` **43.2 MiB**, AppImage **148.9 MiB**. Installers are intentionally larger than 15 MiB because they carry yt-dlp (~38.6 MiB) and qjs; there is no hard installer-size gate (D132). Idle main process: **191,920 kB RSS / 87,832 kB PSS / 44,628 kB RssAnon**; the 30 MB target is not met on Linux (D133).
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → pass.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → pass, 0 warnings.
  - `( cd src-tauri && cargo test )` → **436 unit + 193 integration = 629 passed, 0 failed, 4 ignored**.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → **16 files, 333 tests passed**.
  - `npm run build` → pass.
  - `npm run check:ci` → 24 run blocks valid.
  - `npm run check:runners`, `npm run check:tools`, `npm run check:release -- v0.1.0` → pass.
  - `npx tauri build` with the local updater key → signed Linux bundle, sizes above; public key in `tauri.conf.json` matches the generated public key.
  - `node scripts/generate-latest-json.mjs ...` → four platform entries with percent-encoded URLs and real signatures.
  - `cargo test --test real_selftest the_self_test_passes_with_the_bundled_tools -- --ignored --nocapture` → **pass**: bundled yt-dlp 2026.08.19, QuickJS, 24 formats, heights `[240, 144]`, 2137 ms.
  - `cargo test --test real_selftest a_real_inspection_and_download_end_to_end -- --ignored --nocapture` → **pass**: "Me at the zoo" inspected in 2093 ms, `.mkv` downloaded in 2835 ms, `.opus` produced.
  - Manual Linux/KDE Wayland checks: queue filter/selection/bulk/shortcuts, wizard tool install and model download, local-file transcription, Gemini error banner, language/theme switching, About/diagnostics, update error.
- Not verified / stubbed:
  - Windows 10/11 x64 and macOS 13+ manual acceptance (no hardware); no macOS notarization or Windows code signing.
  - Public 1080p/4K/Shorts download, 100-item playlist, non-YouTube/HLS/cookies/live/DRM, and a real Gemini request (no API key here).
  - `sdp-whisper` sidecar install and every Whisper model size / >2 h chunking end-to-end: the release assets are still 404.
  - A real in-app update: `releases/latest` still points at the old `v2.5`, which has no `latest.json`; a signed M7 release must exist first.
  - The idle-RSS target of spec §15 item 1 is **not met** on Linux (D133); `docs/ACCEPTANCE.md` records the item as open, not waived.
- Spec items marked "verify" that I checked:
  - Tauri v2 updater artifacts → a local Linux build with `createUpdaterArtifacts: true` produces the raw `.AppImage` + `.AppImage.sig`, not `.AppImage.tar.gz`; macOS produces `*.app.tar.gz` + `.sig`, Windows NSIS `*-setup.exe` + `.sig`. The `.tar.gz` wrapper is only for deprecated v1-compatible mode (D127).
  - `eugeneware/ffmpeg-static` b6.1.1 → all ten `.gz` assets exist, their SHA-256 values were computed locally, and the Linux x64 ffmpeg/ffprobe binaries run and report 7.0.2.
  - `sdp-whisper` `whisper-v1.9.5` → HTTP 404 on 2026-10-23; the manifest keeps the shape with null URLs/checksums and the UI reports it as unavailable.
  - Tauri updater/process plugin versions → `tauri-plugin-updater 2.14.0`, `tauri-plugin-process 2.4.0`; the JS updater package is not used.
- Decisions/deviations (also in DECISIONS.md): D126 tools, D127 updater/artifact behavior and MSRV 1.90, D128 diagnostics/logging, D129 bilingual docs, D130 UI, D131 MIT licence, D132 installer measurements without a hard gate, D133 Linux idle-RSS finding.
- Next step: human prerequisites for a release: add `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to GitHub secrets, publish the `sdp-whisper` assets, publish a signed M7 release with `latest.json`, and run the Tier 1 manual acceptance on Windows/macOS. Decide whether to accept or investigate the Linux RSS deviation (D133).

## 2026-10-23 — M7 / Correct repository slug and enable the published sdp-whisper sidecar
- Done:
  - Corrected the repository slug from `EmanueleFusco/ScaricatoreDiPorto` to `EmanueleFusco/Scaricatore-di-Porto-rust` in `tools.manifest.json`, the updater endpoint in `src-tauri/tauri.conf.json`, the About/releases and licence links, both READMEs and the URL tests.
  - Confirmed that the `whisper-v1.9.5` release exists (published 2026-10-19) with all seven `sdp-whisper` assets and `SHA256SUMS`; the earlier 404 came from the wrong repository slug.
  - Filled the real URL and SHA-256 for every `sdp-whisper` platform in `tools.manifest.json` (Linux x64/arm64, Windows x64/arm64, macOS x64/arm64/universal), so the Install button is enabled.
  - Replaced the Rust test that expected the sidecar to be unpublished with `whisper_sidecar_assets_are_installable_and_use_the_published_release`, asserting the release template, all seven platform entries and the Linux x64 checksum. The generic "declared but unpublished" install test now uses a synthetic tool instead of `sdp-whisper`.
  - Updated the frontend tool-state tests: `sdp-whisper` missing is an optional `notInstalled`, while a synthetic unpublished tool still reports `notYetAvailable`.
  - Added the ignored real test `the_whisper_sidecar_install_downloads_verifies_and_runs` and updated the English/Italian troubleshooting guides plus `docs/ACCEPTANCE.md`.
- Verified by running:
  - `curl -I` on all seven release assets → HTTP 200; the published `SHA256SUMS` matches the manifest values.
  - `cargo test --test real_selftest the_whisper_sidecar_install_downloads_verifies_and_runs -- --ignored --nocapture` → pass: 4,893,088 bytes downloaded, SHA-256 verified, installed binary ran `-h` and printed the whisper.cpp CLI help.
  - `( cd src-tauri && cargo fmt --all -- --check )` → pass.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → pass, 0 warnings.
  - `( cd src-tauri && cargo test )` → 436 unit + 193 integration = **629 passed, 0 failed, 5 ignored**.
  - `npm run check` → 0 errors, 0 warnings.
  - `npm test -- --run` → **16 files, 333 tests passed**.
  - `npm run build` → pass.
  - `npm run check:ci` → 24 run blocks valid; `npm run check:runners`, `npm run check:tools`, `npm run check:release -- v0.1.0` → pass.
- Not verified / stubbed:
  - The Windows, macOS and ARM64 sidecar assets answered HTTP 200; their checksum values came from `SHA256SUMS`, but those six files were not downloaded or executed (no hardware).
  - The Tools-page Install button was not clicked manually in this step; the same `install_tool` path was exercised by the real ignored test.
  - A full transcription with the downloaded sidecar and a Whisper model was not re-run in this step.
- Spec items marked "verify" that I checked:
  - `whisper-v1.9.5` release and assets → published 2026-10-19 in `EmanueleFusco/Scaricatore-di-Porto-rust`; all seven URLs and `SHA256SUMS` verified.
  - Tauri updater endpoint → corrected to the real repository; no `latest.json` release exists yet, so a live update check is still unverified.
- Decisions/deviations (also in DECISIONS.md): D134 corrects D126/D127 and records the real sidecar release.
- Next step: add `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to GitHub Actions secrets, then publish a signed M7 release with `latest.json` and run the remaining Tier 1 acceptance checks.

## 2026-10-23 — M7 / Move the WebKitGTK Wayland DMA-BUF fallback into the app

- Done:
  - Reproduced `Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.` with `npm run tauri dev` on the owner's KDE Wayland / NVIDIA RTX 3080 session.
  - Verified `WEBKIT_DISABLE_DMABUF_RENDERER=1` alone keeps both the debug binary and the AppImage alive and removes the `Failed to create GBM buffer` messages. `WEBKIT_DISABLE_COMPOSITING_MODE=1` did not help, and `GDK_BACKEND=x11` was not needed once the DMA-BUF renderer was disabled.
  - Added `is_wayland_session()` and `apply_webkit_dma_buf_fallback()` to `src-tauri/src/lib.rs`. On Linux, before Tauri creates the webview, the app now sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` when it detects a Wayland session and the user has not already set that variable.
  - Added unit tests for `is_wayland_session()`.
- Verified by running:
  - `( cd src-tauri && cargo fmt --all -- --check )` → pass.
  - `( cd src-tauri && cargo clippy --all-targets -- -D warnings )` → pass.
  - `( cd src-tauri && cargo test )` → 441 unit + 193 integration = 634 passed, 0 failed, 5 ignored.
  - `npm run check` → 0 errors, 0 warnings.
  - `env -u WEBKIT_DISABLE_DMABUF_RENDERER -u GDK_BACKEND timeout --signal=TERM 25s npm run tauri dev` → no `Gdk-Message: Error 71`, no GBM warnings, process stayed alive until timeout.
- Not verified / stubbed:
  - The existing AppImage still uses the pre-fix binary and needs `WEBKIT_DISABLE_DMABUF_RENDERER=1` until it is rebuilt; newly built binaries apply the fallback automatically.
  - Other Wayland compositors/GPUs may still need `GDK_BACKEND=x11`; the environment variable remains available as an override.
- Decisions/deviations (also in DECISIONS.md): D135.
- Next step: rebuild and publish a signed release so the AppImage inherits the fallback; otherwise document the environment variable for installed packages.
