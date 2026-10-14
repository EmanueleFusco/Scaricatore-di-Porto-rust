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
- **Verified by running**
  - `cargo test`: **199** lib + 3 bundled_tools + 9 download + 6 m1_ipc +
    16 process_tree + 6 settings_ipc + 13 tools + 15 ytdlp_job (3 `#[ignore]`,
    vedi sotto); `cargo clippy -D warnings` e `cargo fmt --check` puliti.
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
