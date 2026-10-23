# DECISIONS

Questions, deviations, spec ambiguities and everything marked `TODO(verify)` that could not be
verified. Append only.

---

## 2026-10-12 — M0-A (skeleton, settings, i18n, theme, shell UI)

### D1 — Frontend: plain Vite + Svelte, **not** SvelteKit

`PHASE_PROMPTS.md` M0-A step 1 says to scaffold with "the official Tauri v2 Svelte-TS template",
which currently means SvelteKit + `adapter-static`. The production build of that template contains
an **inline `<script>`** (SvelteKit's SPA bootstrap) in `index.html`, which the strict CSP of
`tauri.conf.json` (`script-src 'self'`) blocks — the shipped app would show a blank window.

The spec itself (spec §1) asks for "TypeScript + Vite + **Svelte**", never for SvelteKit, and
AGENTS.md §6 asks for a *light* frontend with no framework runtime. So the frontend is plain
Vite + Svelte 5 with a ~40-line path router (`src/lib/router.svelte.ts`). Consequences:

* strict `script-src 'self'` works as-is (verified: `dist/index.html` has no inline `<script>`),
* production payload drops from ~130 KB of SvelteKit runtime to **70 KB JS + 7.7 KB CSS**,
* one less dependency; `@sveltejs/kit` and `@sveltejs/adapter-static` were removed.

Cost: routing is ours, so it only supports the paths we actually add. That is fine for an app
shell with two screens and it grows with the code that needs it.

### D2 — Plugins added

`dialog`, `opener` and `single-instance` are registered (all on the spec §1 allow-list).
`dialog` is not used by any M0-A screen yet but is needed by M0-C (folder picker) and M1, and it
is zero extra binary weight. `single-instance` implements spec §3 "a second launch focuses the
first window". **No `tauri-plugin-shell`** (spec §1 forbids it). The `deep-link` feature of
`single-instance` is deliberately **not** enabled: it pulls in `tauri-plugin-deep-link` and the
command-line/protocol URL forwarding it provides belongs to a later phase.

### D3 — Devtools are off in release builds without a config flag

Spec §11.1 asks for "devtools off in release". `app.windows[].devtools` in `tauri.conf.json` would
also disable them in `tauri dev`, which would hurt development. Instead the **`devtools` cargo
feature is not enabled** on the `tauri` crate, so wry's devtools API is not compiled into release
builds at all. Nothing was set in `tauri.conf.json`.

### D4 — Lenient enum deserialisation in `Settings`

A strict `#[derive(Deserialize)]` on `language`/`theme` turns a single bad value
(`"theme": "ultraviolet"`) into "corrupt file", which moves the whole document to `.bak` and
resets *every* setting. Spec §2.8 asks for resilience, so unknown values degrade to the default
instead (`de_theme` / `de_language` in `state.rs`), and values are matched
case-insensitively after trimming.

### D5 — Wire format is camelCase

`Settings`, `SettingsView`, `Recovery` and `ErrorInfo` all serialise with
`#[serde(rename_all = "camelCase")]` so the TypeScript side can use the names as-is. The
persisted `settings.json` therefore also uses camelCase keys.

### D6 — Settings file is only created on the first write

Loading a missing file returns defaults **without** creating the file. Writing always creates it.
This keeps the first launch free of disk writes; the trade-off is that a fresh install has no
`settings.json` until the user changes something.

### D7 — `npm run dev` outside Tauri degrades gracefully

`src/lib/stores/settings.svelte.ts` detects the absence of the Tauri IPC bridge
(`__TAURI_INTERNALS__`) and falls back to in-memory defaults instead of showing an error banner.
This makes UI work possible in a normal browser (`http://127.0.0.1:1420`), which is how the
screenshots in PROGRESS.md were taken. Inside the real app the backend stays the only source of
truth.

### D8 — CI, `tools.manifest.json` and the wizard are not part of M0-A

`PHASE_PROMPTS.md` M0-A does not ask for them; `.github/workflows/`, `tools.manifest.json` and the
first-run wizard belong to M0-C. `scripts/` is therefore still empty.

### D9 — `original_python/` is kept as-is

The layout in `AGENTS.md` §3 only lists `reference/ScaricatoreDiPorto.py`. The rest of the upstream
repository (`LICENSE`, `README.md`, `requirements.txt`, `images/`) was left at the root under
`original_python/` instead of being deleted, because it is the user's source material and the
Python file's licence may matter for the About screen (§11.8).

**Chiarimento aggiunto in D17:** questi file **esistono e servono** al lavoro degli agenti, ma non
sono tracciati in git e quindi non finiscono su GitHub. Restano sul disco.

### D10 — Wayland: worked around in the dev launcher, not "fixed" in the app

On the owner's machine (CachyOS, KDE Plasma 6.7.5, Wayland, NVIDIA RTX 3080, `webkit2gtk-4.1`
2.54.1, `gtk4` 4.24.1) any GTK window dies immediately with
`Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display`.

**Verified it is not our code**: the pristine `create-tauri-app` Tauri template, built and run on
the same session, dies with the identical message.

There are in fact **two** separate problems here, and the second one only showed up when the owner
ran the app for real:

| symptom | cause | fix |
| :-- | :-- | :-- |
| `Error 71`, process dies, no window | GTK4 against this compositor | `GDK_BACKEND=x11` |
| window opens but is **black on screen**, log shows `Failed to create GBM buffer` | WebKitGTK cannot allocate the NVIDIA GPU buffers via GBM | `WEBKIT_DISABLE_DMABUF_RENDERER=1` (spec §13.3) |

With both set the app stays alive, the GBM messages disappear entirely and the log is empty.
**Whether the window is actually visible on screen still has to be confirmed by the owner** — the
screenshot tool available here (`magick import -window`) reads the X11 backing buffer and bypasses
the compositor, so it reports correct pixels even when the compositor shows black.

So `scripts/run-dev.mjs` (exposed as `npm run app`) injects **both** variables only when all of
these hold: Linux, a Wayland session, XWayland available, and the user has not already set
`GDK_BACKEND`. It prints what it is doing. This is deliberately not hidden inside the Rust code:
if a future GTK fixes the crash, the workaround disappears on its own instead of masking a real
problem, and on machines where Wayland works nothing changes.

Known follow-up (not M0 work): if other Wayland desktops hit this, the right fix is in the app —
detect the failure and tell the user to retry with `GDK_BACKEND=x11` — not in the user's shell.

### D11 — Icons: Lucide (`lucide-svelte`), not hand-written SVG

M0-A initially drew the two sidebar icons as hand-written inline `<svg>` path data written from
memory. The gear rendered badly — and that was the correct outcome of the method, not bad luck:
AGENTS.md §2.3 says *"Do not invent […] verify them"*, and invented SVG path data is invented like
any other value. From M2 on there will be dozens of icons (play, pause, folder, trash, retry, …),
so the trap would have repeated every time.

Replaced with `lucide-svelte` (v1.0.1):

* **Still inline SVG.** Spec §1 requires "inline SVG icons", "no remote assets" and (spec §11.9) no
  external fonts or images. Lucide renders `<svg>` elements in the DOM — no icon font, no network.
* **Tree-shaking verified, not assumed.** The package is `sideEffects: false` and icons are
  imported one by one (`lucide-svelte/icons/settings`). Measured: the bundle grew from 70.28 kB to
  81.97 kB for three icons, and spot checks confirm unused icons (`airplay`, `airplane`, `banana`,
  `trophy`) are absent while the three used paths are present.
* **Licence ISC** (the Feather-derived icons are MIT). Both are permissive. It has to be added to
  the About screen in M7 alongside yt-dlp/ffmpeg/whisper.cpp/QuickJS/Deno (spec §11.8).
* Weight impact: **+11.7 kB JS** (≈ +4.4 kB gzip), build-time only — nothing is added to the Rust
  binary.

Rule for the rest of the project: **never hand-write SVG path data. Take it from Lucide, or copy a
published path verbatim from the icon set's source, and say where it came from.**

### D12 — CI designed around the **private** repository's minute budget

The owner created the repository **private**. Verified from GitHub's billing docs:

* included minutes: **2 000/month** on Free, 3 000 on Pro/Team; **public repos are free and
  unlimited**;
* multipliers: **Linux 1x, Windows 2x, macOS 10x** — a 6-minute macOS job costs 60 Linux-equivalent
  minutes. Effective free macOS time on a Free private repo: roughly **200 minutes/month**.

So `.github/workflows/ci.yml` does not run everything on every push:

* `check` — lint + tests on **Linux only** (1x), on every push and PR;
* `bundle` — the 3-OS matrix (linux-x64, windows-x64, macos-arm64) only when an artifact is
  actually wanted: push to `main`, a tag, or `workflow_dispatch`. Skipped on pull requests;
* `bundle-macos-universal` — separate job so a universal failure cannot block the native `.app`
  used for manual testing;
* `tier2` — macOS Intel / Windows ARM64 / Linux ARM64 (spec §1.1) **only on `workflow_dispatch`**:
  those targets must build and pass the fake-tool tests in CI, but must not burn the quota on
  every push.

Third-party actions used (pinned to major tags, all widely used): `dtolnay/rust-toolchain`,
`Swatinem/rust-cache`. First-party: `actions/checkout`, `actions/setup-node`,
`actions/upload-artifact`.

Not verified yet: the workflow has never been executed — the first run is the test. The parts most
likely to need a fix are the Linux system packages and the `universal-apple-darwin` target.

### D13 — bundle sizes measured, not summed

`scripts/report-bundle-sizes.mjs` counts only the installable artifacts (`.deb`, `.rpm`, `.AppImage`,
`.exe`, `.msi`, `.dmg`, `.snap`, and `.app` directories). My first version walked the whole
`bundle/<format>/` tree and also summed the staging files, which reported **424 MiB** for an AppImage
that is really **108 MiB**. The numbers that reach the job summary must be the ones a user
downloads.

### D14 — Why CI builds on `ubuntu-22.04`, and what was actually wrong

The owner noticed the workflow says `ubuntu-22.04` while the desktop is "Ubuntu 26". Three different
things were being conflated, so here is the audit:

1. **The owner's machine is not Ubuntu** — it is **CachyOS** (Arch-based, rolling), running
   `webkit2gtk-4.1 2.54.1`, `gtk4 4.24.1`, `glib 2.90.1`, `mesa 26.2.4`. The distro version in the
   workflow names a **CI runner**, not the dev machine.
2. **`ubuntu-22.04` is deliberate, not stale.** Spec §13.5 names it, and spec §5.4 states the
   principle ("build on the *oldest supported* distro"). Building against the oldest glibc makes the
   `.deb` run on newer systems too. Kept.
3. **The modern end of the range is already covered**: the `check` job runs on `ubuntu-latest`, so
   every push is validated on a current distro while `bundle` ships the oldest-compatible build.

Verified against the real apt indices rather than from memory:

| fact | value |
| :-- | :-- |
| `ubuntu-22.04` (jammy) amd64 webkit2gtk | **2.50.4** |
| `ubuntu-22.04` (jammy) **arm64** webkit2gtk | **2.36.0** — Canonical backported only amd64/i386 |
| Ubuntu 26.04 codename | `resolute` |
| `ubuntu-latest` | 24.04 |
| owner's machine (CachyOS) | 2.54.1 |

**What was genuinely wrong and is fixed:** the CI installed a hand-written package list that did
**not** include `libayatana-appindicator3-dev`, which appears in Tauri v2's documented prerequisites
(page last updated 20 Aug 2026). It happens not to be required here — `tray-icon` is not among
Tauri's default features and we never enable it — but the list now matches the official source
instead of my recollection. `patchelf`, which was not in the official list either, was dropped.

While checking I first reported that `libayatana-appindicator3-dev` does not exist on 22.04: that was
wrong, I had searched only the `universe` index. It is in `main`. Corrected here.

All eight packages were verified present in the jammy `main`/`universe` indices.

**Open for the owner:** when 22.04 leaves standard support, the `bundle` job label must move up.
Deciding between "newest LTS" and "oldest still supported" is a product call, not mine.

### D15 — Windows and macOS verified against current sources (two real fixes)

Owner asked for an audit of the Windows x64 and macOS side. Checked against GitHub's runner
README, Tauri's prerequisites page (updated 20 Aug 2026) and the Tauri config schema, not memory.

**Runner landscape today:**

| label | resolves to |
| :-- | :-- |
| `windows-latest` | Windows Server 2025 + **Visual Studio 2026** (migrated June 2026) |
| `windows-2022` | Windows Server 2022 + VS 2022 — still available |
| `macos-latest` | **macOS 26 arm64** (Apple Silicon) |
| `macos-15-intel` / `macos-26-intel` | macOS Intel, for the Tier 2 job |
| `ubuntu-latest` | 24.04; `ubuntu-26.04` exists |

**Fix 1 — `bundle.targets`: dropped `msi`.** It was `"all"`, which on Windows also builds an MSI.
Tauri's docs state MSI packaging needs the **VBSCRIPT optional feature** on the build machine and
that it may be disabled on some Windows images, in which case `light.exe` fails. The runner would
need the feature enabled plus a restart. Spec §13.1 says the **NSIS installer is preferred** and
never mentions MSI, so the list is now explicit:
`["deb", "rpm", "appimage", "nsis", "app", "dmg"]` (values confirmed against the published config
schema, which allows exactly these plus `msi`). Adding MSI back is one word if the Microsoft Store
ever matters.

**Fix 2 — CI uses `windows-2022`, not `windows-latest`.** Since June 2026 `windows-latest` carries
**Visual Studio 2026**, which is new enough that the Tauri ecosystem is not yet proven against it.
`windows-2022` is both the more conservative choice and the one consistent with the owner's policy
("oldest product = widest compatibility": an older SDK means the binary runs on more Windows
versions). Switching back is one line.

**Verified as correct, no change needed:**

* `PathResolver::download_dir()` (used by `services/paths.rs` for the default output folder) works
  on all three: `C:\Users\<u>\Downloads`, `/Users/<u>/Downloads`, `$XDG_DOWNLOAD_DIR`.
* `tauri-plugin-single-instance` uses a Win32 **named mutex + window class** on Windows and a
  **Unix socket under `/tmp`** on macOS (path kept under 100 chars) — no platform-specific code
  needed from us.
* `main.rs` already carries `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`,
  so release builds on Windows show no console window (spec §4.1).
* Tauri's stated minimum is macOS 10.15; the spec asks for macOS 12+. Our floor is therefore
  stricter, which is safe.

**Still unverified, and only CI can verify it:** `std::fs::rename` replacing the destination on
Windows. I asserted it from the std docs (`MoveFileExW` + `MOVEFILE_REPLACE_EXISTING`), but the test
`settings_survive_a_restart` only ever ran on Linux — and it is **still** unverified after the first
green CI run, because `cargo test` only runs in the `check` job (ubuntu). The `bundle` jobs compile
but do not execute the suite. Running the tests on Windows is the obvious next improvement.

**First CI run outcome** (30m 17s): every job green — `check` 7m08s, `bundle` linux-x64 7m30s,
windows-x64 9m12s, macos-arm64 2m35s, macos universal 3m52s. Green means *it compiles, links and
bundles*, not *it works*: no runner ever opened a window. Two corrections to my own predictions: the
universal macOS build, which I called the most likely failure, passed first time; and the Tier 2 job
showing green in 0s means it was **skipped**, not passed.

### D16 — Pre-push audit of what actually gets committed

Before the first push the owner asked for a secrets/`.gitignore` audit. Checked with commands that
look at **what is really tracked**, not at what `.gitignore` claims:

* `git grep` for API-key/token/private-key patterns and for real key formats
  (`AIza…`, `sk-…`, `ghp_…`, `AKIA…`, `-----BEGIN`) → **no hits**. The matches for the words
  "secret"/"token" are all documentation (`CancellationToken`, spec §10.5).
* no `/home/<user>`, no e-mail addresses, no LAN IP addresses in any tracked file;
* commit author is the placeholder `Scaricatore Dev <dev@example.invalid>` — **no real identity**;
* heaviest tracked files are 264 kB (`original_python/images/icona.ico`) and 114 kB
  (`Cargo.lock`) — nothing that should not be in a repository;
* nothing that is ignored today was ever committed (`--diff-filter=A` cross-check);
* the only ignored directories present are `src-tauri/target` (16 194 files), `node_modules`
  (9 469), `src-tauri/gen` and `dist`.

**Two real gaps found and closed.** The old `.gitignore` had no rule for signing material, so a
private key generated locally for a test would have been committed — spec §13.4 requires the Tauri
updater private key to exist **only** in CI secrets. Added `*.key *.pem *.p12 *.pfx *.keystore
*.jks`, plus `*.bak` and WebKitGTK crash dumps (`core`, `core.*`). Verified both directions: the
new rules catch key files, and none of the files that *must* be tracked (`tools.manifest.json`,
`whisper.models.json`, `Cargo.lock`, `package-lock.json`, `.github/workflows/ci.yml`) are caught by
accident.

### D18 — The operational guide is personal and stays out of git

The owner asked for the command/CI walkthrough to be exported to a file, then clarified that it is
**their** working document, not a README for visitors: the real README is written later, when the
app has something to show (spec §11.8 About/licences).

So it lives in **`GUIDA_OPERATIVA.md`**, is listed in `.gitignore`, and is removed from git's index
while staying on disk. Verified both ways: `git check-ignore` matches it, and `git status` does not
offer it for `git add`.

Deliberately **not** done: erasing it from history. It was committed once, as `README.md`, in commit
`0eb35a9`, so someone reading the repository history finds one commit mentioning a README that no
longer exists at that path. That costs nothing, and rewriting the history a second time for
cosmetics is not worth it.

### Open questions (owner)

* Tier 1 assumption of spec §1.1 (Windows x64, Linux x64, macOS Apple Silicon) is still a
  *guess* — nobody has confirmed it yet. It should be stated explicitly in the README.

---

## 2026-10-12 — D17: workflow files and the Python original removed, history rewritten

Owner asked to drop three things that are not part of the programme, then extended it to the Python
source itself:

* **`COME_INIZIARE.md`**, **`PHASE_PROMPTS.md`** — notes and prompts for driving the agent, not
  programme code;
* **`original_python/`** — leftovers from the upstream repository (`LICENSE`, `README.md`,
  `requirements.txt`, `images/`); the Python file itself had already been moved to `reference/`;
* **`reference/ScaricatoreDiPorto.py`** — the owner states the Python programme is their own work,
  they keep a copy elsewhere and do not need it here, and the licence is not a constraint because
  they can relicense it at will.

**Licence note, for the record:** the original app is **GPL v3** while this project declares
`license = "MIT"`. That mixture was the original reason to tidy up; the owner has since confirmed it
is a non-issue. If the repository ever becomes public, decide again deliberately.

**History was rewritten**, not merely deleted, because the files were already on GitHub — a plain
`git rm` would have left them readable from any earlier commit. Done with `git filter-repo
--invert-paths`; all 13 commits were preserved (new SHAs) and a mirror backup was taken first.
`filter-repo` removes the `origin` remote on purpose, so it had to be re-added. The remote branches
`main` and `m0-skeleton` both need a `--force` push.

**Correzione (stessa giornata):** what the owner actually wanted was *off GitHub*, not *off the
disk. `git filter-repo` rewrites the working tree as well as the history, so the five paths above
were removed from the local checkout too — which broke the very thing they were kept for:
`AGENTS.md` §1/§3 and the phase prompts both rely on `reference/ScaricatoreDiPorto.py`.

All five were restored from the mirror backup (byte-identical, verified path by path) and are now
held out of git with `.gitignore` instead. That is the correct mechanism: it keeps them available
to the agents on this machine while never pushing them to GitHub. History rewriting was still the
right call for the already-pushed copies, and stays.

**Lesson:** before running a history rewrite, copy the affected paths *out of the repository*. The
filter removes them from the checkout, and a backup is only a backup if you go and use it.
---

## 2026-10-13 — M0-B (process layer, spec §4)

### D19 — `fake_tool` is a Cargo **example**, not a `[[bin]]`

The first version declared the fake tool as a `[[bin]]` of the app package, because only bin targets
get `CARGO_BIN_EXE_<name>` and so only bin targets are built by `cargo test --test <name>`.

**That shipped the test helper to users.** Measured, not guessed: `dpkg-deb -c` on the `.deb` produced
by `npx tauri build --bundles deb` listed

```
usr/bin/fake_tool
usr/bin/scaricatore-di-porto
```

— the Tauri Linux bundler installs *every* `[[bin]]` of the package into `/usr/bin`, not just the main
one. A 344 KB test helper has no business in a user installer.

A second problem showed up on the way: with two bins, `npx tauri build` fails with

```
failed to find main binary, make sure you have a `package > default-run` in the Cargo.toml file
```

so `default-run` had to be added as well.

Now declared as `[[example]]` instead:

| | `[[bin]]` | `[[example]]` |
| :-- | :-- | :-- |
| built by `cargo test` | yes | yes |
| built by `tauri build` | yes → **installed in `/usr/bin`** | no |
| in the installer | yes | no |

Re-measured after the change: the same `.deb` now contains only `usr/bin/scaricatore-di-porto`, and
`target/release/` contains no `fake_tool` at all. Bundle size is unchanged (1.94 MiB).

**Cost, accepted and documented:** Cargo only builds examples for the *default* target set, so
`cargo test --test process_tree` does not build the helper and the suite fails with an explicit
message naming the fix ("run it as `cargo test`"). The supported commands in `AGENTS.md` §4 are
`cargo test` without filters, and CI uses that. `SDP_FAKE_TOOL=<path>` overrides the lookup.

`.sh` cannot run on Windows and `.cmd` cannot run on Unix, so the helper is ~200 lines of Rust with no
dependencies at all; that part was never in doubt.

### D20 — the Job Object is attached *after* `CreateProcess`, and the race is real

Spec §4.2 wants every child in a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. `tokio`'s
`Command` (and `std`'s) offers no hook to assign a job at creation time, so the sequence is:

```
CreateProcess (CREATE_NO_WINDOW)  ->  CreateJobObjectW  ->  SetInformationJobObject
                                  ->  AssignProcessToJobObject
```

Between `CreateProcess` and `AssignProcessToJobObject` there is a window — microseconds, but not zero —
in which the child could itself spawn a grandchild that would then land outside the job. The fully
correct version is `PROC_THREAD_ATTRIBUTE_JOB_LIST` via
`CreateProcessW(EXTENDED_STARTUPINFO_PRESENT)`, which assigns atomically at creation; that means
bypassing `tokio::process` for Windows and re-implementing async process handling. Not worth it here.

If the assignment fails, `spawn()` kills the child and returns an error — it **fails closed**: it
never silently runs an unmanaged process tree.

**What this means in practice:** for yt-dlp (a Python interpreter that has to boot, import and parse
its arguments before it spawns anything) the window is closed by many orders of magnitude. The test
that would catch a real regression — "cancel leaves no `ffmpeg`, no `qjs`, no `deno`" (spec §15 item
12) — runs on Windows in the new CI job from D23.

### D21 — on Unix, a reaped pid must never be signalled again

`killpg(pgid, SIGKILL)` on a pid the OS has already given to somebody else would kill an unrelated
process group. That can happen as soon as the child is reaped.

Two mitigations, both in the code:

* `TreeHandle::forget_group()` zeroes the pgid the moment the child has been reaped; `kill_tree()`
  then does nothing instead of signalling a stranger.
* the grace loop polls `child.try_wait()` and stops as soon as the direct child is gone, so the window
  between "reaped" and "the final `killpg`" is microseconds rather than the whole 2 s.

The alternative — `waitid(P_PID, …, WNOWAIT)` to look at the exit status without reaping — was
rejected: tokio's own reaper can reap the child concurrently on Linux (`pidfd`), so it would not
actually close the window.

`RunningTool::drop` therefore kills the tree, and after a clean `wait()` that kill is a guaranteed
no-op.

### D22 — `TerminateJobObject` on an empty job: not documented, so we do not rely on it

The Microsoft documentation for `TerminateJobObject` says nothing about what it returns when the job
has no processes, and spec §4 / M0-B asks for a `kill_tree` that "works if the child has already
exited (no panic, no error)". Guessing is not an option (AGENTS.md §2.3), so the Windows
implementation reads `JobObjectBasicAccountingInformation` first and returns success immediately when
`ActiveProcesses == 0`. Only a non-empty job reaches `TerminateJobObject`.

`QueryInformationJobObject` / `JobObjectBasicAccountingInformation` / `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`
were all read out of the `windows` 0.62.2 sources in `~/.cargo/registry`, not from memory.

### D23 — new CI job `process-tests` (Windows + macOS, tag or manual only)

The handoff at the end of M0-A said it plainly: *"i test girano solo su Linux […] se M0-B aggiunge
test che dipendono dal sistema (spawn, Job Object, process group), vanno eseguiti esplicitamente
anche su Windows e macOS"*. Spec §4 is precisely that code, so a linux-only run would be a
multi-OS-shaped single-OS test.

`process-tests` runs `cargo test` on `windows-2022` and `macos-latest`. Same gate as `bundle` (tag or
`workflow_dispatch`), **not** on every push, because of the private-repository minute budget of D12:
a dev-profile build of the Tauri tree on macOS costs about 6 minutes of wall clock and 60
Linux-equivalent minutes of quota. It is in the `needs` of the `release` job, so a broken process
layer blocks a release while still costing nothing on an ordinary push.

It will also answer, for the first time, the `std::fs::rename`-over-the-destination question that
D15 has been carrying as unverified since M0-A.

### D24 — `tokio` needs the `macros` feature in normal dependencies

`tokio::select!` is **production** code here (spec §4.4), not a test helper. The first version put
`macros` in `[dev-dependencies]` and it happened to work on Linux only because Cargo unified the
features; `cargo build --target x86_64-pc-windows-msvc` failed with "cannot find `select` in `tokio`".
`macros` pulls in `tokio-macros`, a proc-macro crate used at compile time: nothing is added to the
shipped binary.

### D25 — `windows::HANDLE` is not `Send`/`Sync`, so `TreeHandle` declares it

`windows` 0.62.2 `Win32::Foundation::HANDLE` is a `*mut c_void` newtype with no `Send`/`Sync` impl
(checked by grepping the whole `windows`, `windows-core` and `windows-result` sources). Without the
declaration `RunningTool` would be `!Send` and `tokio::spawn` of the registry, and the `Arc` sharing
between the tool and the registry, would not compile.

The `unsafe impl Send`/`Sync` is justified in the code: every method takes `&self` and only *uses* the
handle, never replaces or moves it, and the handle is closed exactly once in `Drop`, which needs
`&mut self` and therefore cannot race with another `&self` call.

### Open questions / things only a human or the CI can answer

* `assign_process_to_job_object` and the whole Windows cancellation path are **compiled** (clippy on
  `x86_64-pc-windows-msvc` is clean) but have **never been executed**. Only the new CI job can say
  whether the Job Object really kills an ffmpeg grandchild.
* macOS is **compiled nowhere**: cross-compiling from Linux needs an Objective-C toolchain for
  `objc2-exception-helper`, which does not exist here. The Unix-specific API surface was checked
  against `aarch64-apple-darwin` and `x86_64-apple-darwin` in a scratch crate that mirrors exactly
  the `#[cfg(unix)]` lines of `process.rs` (`killpg`, `SIGTERM`, `SIGKILL`, `ESRCH`,
  `Command::process_group`, `CancellationToken`) — that is a real check of the symbols, not of the
  whole crate.
* The bounded reader drain (`READER_DRAIN_TIMEOUT`) is defensive code with no test: exercising it
  needs a grandchild that *escapes* the process group (`setsid`) on Unix and a breakaway-flag process
  on Windows. It is documented as unverified rather than dressed up.

---

## 2026-10-13 — D26: Tier 2 spostato in un workflow tutto suo

**Cosa è successo.** Un avvio manuale di `ci.yml` chiesto per una cosa sola — verificare i
test del livello processi su Windows e macOS — ha fatto partire anche `tier2`, perché la sua
condizione era `if: github.event_name == 'workflow_dispatch'`: stessa soglia, quindi stesso
pulsante.

Costo reale di quella run, calcolato con i moltiplicatori di D12 (Linux 1x, Windows 2x, macOS 10x):

| runner | moltiplicatore | ~minuti di parete | minuti-equivalente Linux |
| :-- | --: | --: | --: |
| `macos-15-intel` | 10x | ~6 | **~60** |
| `windows-11-arm` | 2x | ~8 | ~16 |
| `ubuntu-24.04-arm` | 1x | ~7 | ~7 |
| | | | **~83** |

Tre target "sperimentali, solo CI" (spec §1.1) che in quel momento non servivano a nessuno, e il
43% della quota mensile gratuita privata (2 000 min) bruciata in un colpo.

**Il difetto di fondo non è la condizione, è il raggruppamento.** Con un solo pulsante e più job
condizionati, *qualsiasi* avvio manuale innesca *tutti* quelli che ammettono
`workflow_dispatch`. Scrivere `if: github.event_name == 'workflow_dispatch'` sembra
"solo avvio manuale" ma significa "tutti gli avvii manuali", che è diverso da "solo quando lo
voglio esplicitamente".

**Soluzione: file separato.** `tier2` ora vive in `.github/workflows/tier2.yml`, con un solo
trigger e un solo pulsante ("Actions → Tier 2 → Run workflow"). In `ci.yml` non esiste più la
condizione: è impossibile che venga trascinato da un dispatch di questo workflow.

La regola generale per le prossime fasi: **un workflow, un pulsante, un motivo per premere.**
Il job `process-tests` è rimasto in `ci.yml` perché ha uno scopo diverso dagli altri (verificare
il livello processi sui due target Tier 1) e va spinto nelle stesse occasioni di `check` e
`bundle`; quando non serve, non si preme nessun pulsante.

`scripts/check-ci-syntax.mjs` legge già **tutti** i `.yml` della cartella (verificato: ora
controlla 8 blocchi, 7 in `ci.yml` e 1 in `tier2.yml`), quindi il file nuovo è coperto dal
controllo che aveva salvato la prima CI.

**Costo residuo e quando usarlo:** ~83 minuti-equivalente Linux per run, cioè ~24 run all'anno
con la quota gratuita. È una verifica occasionale, non continua — che è esattamente quello che
la spec §1.1 chiede per i target sperimentali.

**Nota operativa:** `workflow_dispatch` funziona solo se il file esiste sul branch di default.
`tier2.yml` diventa quindi avviabile dalla pagina Actions **dopo** che questo commit arriva su
`main`.

---

## 2026-10-13 — D27: un albero terminato ha due exit code diversi, e va bene così

La prima esecuzione dei test su Windows ha mostrato che un tool terminato dal nostro
`kill_tree()` riporta `exit_code == Some(1)`, mentre lo stesso tool terminato con `SIGKILL` su Unix
riporta `None`. Non è un bug: sono le due implementazioni di spec §4.2 che la specifica chiede
esplicitamente, e hanno semantiche diverse.

* Unix: `killpg(SIGKILL)` uccide per segnale, e `ExitStatus::code()` restituisce `None` perché un
  processo ucciso da un segnale non *ha* un codice di uscita.
* Windows: `TerminateJobObject(hJob, uExitCode)`. Il secondo parametro è documentato da Microsoft
  come *«The exit code to be used by all processes and threads in the job object»*: **assegna** un
  codice a tutti i membri. Noi passiamo `TREE_TERMINATE_EXIT_CODE = 1`.

**Non normalizziamo**, e il perché è una verifica, non una scelta di gusto:

1. il classificatore di errori della spec §8.5 è una tabella di **pattern sul testo di stderr**
   (`HTTP Error 403`, `Sign in to confirm`, `No space left`, …): non legge il codice di uscita, quindi
   la differenza non la tocca;
2. `ProcessResult::success()` è `!cancelled && exit_code == Some(0)` ed è `false` su entrambe le
   piattaforme: la semantica che l'app usa è già portabile;
3. aggiungere un flag «l'ho ucciso io» significherebbe propagare stato attraverso
   `kill_all_running()`, che è una funzione che non restituisce nulla e che viene chiamata anche da
   un `Drop` e da un hook di uscita: si aggiungerebbe plumbing per un caso che oggi non produce
   nessun messaggio visibile.

Quindi la differenza è **documentata sul campo** `ProcessResult::exit_code`, con l'istruzione esplicita
di non leggerlo come «è stato ucciso?», e il test la verifica esplicitamente per piattaforma.

**Nota di processo:** questo è il secondo test scritto a forma di Linux in due run di fila (prima i
percorsi `/usr/bin/...` non assoluti su Windows, poi questo). La regola emersa è in `PROGRESS.md`: in
un test, mai un valore che cambia da una piattaforma all'altra scritto alla cieca — o un
`cfg!(windows)`, o una sua funzione dedicata che lo dichiari.

---

## 2026-10-13 — D28: i binari di test su Windows non avevano il manifest Common Controls v6

**Sintomo** (prima volta che i test di IPC giravano su Windows):

```
Running tests\settings_ipc.rs
Caused by: process didn't exit successfully: `settings_ipc-....exe`
  (exit code: 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND)
```

Non è un test che fallisce: il **processo non parte**, prima di `main()`. E solo
`settings_ipc` è affetto, mentre `process_tree` sulla stessa macchina passa 15/15.

**Causa, verificata nelle fonti, non dedotta.**

1. `tauri-build` passa il proprio manifest al **binario applicativo** usando `embed-resource`, che
   emette `cargo:rustc-link-arg-bins` (letto in `embed-resource-3.0.12/src/lib.rs:443`). **`-bins`, non
   `-tests`**: i binari prodotti da `cargo test` non lo ricevono.
2. Un eseguibile senza quel manifest, che importa i simboli di `comctl32` v6, carica la `comctl32`
   "legacy" (v5), che non li esporta → `STATUS_ENTRYPOINT_NOT_FOUND`.
3. `tests/settings_ipc.rs` usa `tauri::test::mock_builder()` e quindi importa la catena della
   finestra/webview → comctl32 v6. `tests/process_tree.rs` non tocca Tauri → non importa quei
   simboli → parte senza problemi. **Ecco perché solo uno dei due muore.**

Bug noto di Tauri, non nostro, con tre riferimenti che si triangolano:

* [tauri#13419](https://github.com/tauri-apps/tauri/issues/13419) — «STATUS_ENTRYPOINT_NOT_FOUND error
  when running … if you use the mock function in `tauri::test` on Windows»;
* [tauri#14580](https://github.com/tauri-apps/tauri/issues/14580) — ancora **open**: basta che
  `tauri::Window` compaia nella firma di una funzione di una libreria perché `cargo test` non parta;
* [discussione tauri#11179](https://github.com/orgs/tauri-apps/discussions/11179) — con la soluzione.

**Soluzione adottata**: `src-tauri/windows-app-manifest.xml` (identico a
`tauri-build/src/windows-app-manifest.xml`, MIT/Apache-2.0) più, in `build.rs`, due righe:

```rust
println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
println!("cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}", manifest.display());
```

Tre dettagli che non sono ovvi e che sono il punto di questa decisione:

* **`-tests`, non `rustc-link-arg` globale.** I workaround che circolano online usano il flag
  globale, che è applicato anche al binario applicativo: lì il manifest c'è già (via
  `tauri-build`) e si finisce con risorse duplicate ed errore di link. `-tests` è chirurgico.
* **`CARGO_CFG_TARGET_OS` / `CARGO_CFG_TARGET_ENV`, non `#[cfg(windows)]`.** Un build script è
  compilato per l'**host**, non per il target: con `#[cfg(windows)]` il flag non verrebbe emesso
  durante `cargo clippy --target x86_64-pc-windows-msvc` su questa macchina Linux, e proprio la
  verifica che uso per controllare il codice Windows would be silently blind. Verificato
  esplicitamente: cross-compilando per Windows da Linux i due flag **escono**; compilando per
  Linux **non escono** (0 occorrenze).
* **Niente `/WX`.** I workaround lo aggiungono per trasformare i warning del linker in errori; qui
  non serve a nulla e amplierebbe il raggio del fallimento.

**Non verificato:** l'esito su runner Windows arriva dalla prossima run. Quello che ho potuto
verificare qui è (a) XML valido, (b) i flag emessi per il target Windows e non per Linux,
(c) `cargo test` e `clippy` verdi su entrambe le piattaforme.

---

## 2026-10-13 — D29: chiusura di M0-B, e cosa "verificato" significa qui

Il proprietario ha confermato che `macos-arm64` e Linux sono verdi e che il fallimento era solo
Windows. Vale la pena fissare cosa questo significa, perché "verde" è una parola che si usa male.

**Non significa:** che l'app funziona. Significa che `services/process.rs` è stato **eseguito** —
non solo compilato — su tutti e tre i target Tier 1, e che i suoi test passano. È il salto che M0-B
chiedeva: la sessione precedente poteva solo dire "compila su Windows e macOS", perché i test vivevano
solo su Linux.

**Significa in particolare**, perché è la parte che la spec §4.2 considera più fragile:

* la cancellazione uccide **l'albero**, non solo il figlio. Il test non lo deduce: il fake tool
  stampa il pid del suo nipote e il test lo interroga finché non sparisce. Su Windows questo
  significa che `AssignProcessToJobObject` + `TerminateJobObject` funzionano su un runner vero; su
  Linux e macOS che `process_group(0)` + `killpg` funzionano su Darwin, dove `killpg` ha anche il
  diverso `link_name = killpg$UNIX2003` su x86_64;
* `CREATE_NO_WINDOW` non apre finestre di console — verificato solo per compilazione (non c'è modo di
  accorgersene da un test headless), quindi **quel punto specifico resta unverificato a mano**;
* nessuna finestra di dialogo su Windows è comparsa durante le run.

**Quello che resta fuori, detto chiaramente:** nessun processo reale è stato coinvolto. Il fake tool
è un programma Rust di 200 righe, non `yt-dlp`. La prova che `yt-dlp` che avvia `ffmpeg` che avvia un
runtime JavaScript muore tutto insieme arriva da M1, non da qui.

**La lezione che si porta dietro, per tutte le fasi seguenti:** tre difetti di M0-B sono emersi solo
quando i test hanno girato davvero su un sistema diverso da quello su cui erano stati scritti — due
test con presupposti impliciti da Linux e un binario di test che non partiva. Nessuno dei tre era
visibile in `cargo clippy`, che passava pulito su entrambe le piattaforme. **"Compila su tre sistemi" e
"funziona su tre sistemi" sono due affermazioni diverse, e il passaggio in mezzo costa una CI.**

---

## 2026-10-13 — D30: "funziona offline dopo l'installazione" che cosa vuol dire (e che cosa non)

La spec §14, criterio di uscita di M0, dice: *«yt-dlp + qjs work **offline right after install**»*. Il
proprietario ha segnalato che la frase sembra un contrasenso per un'app che scarica da internet. **Ha
ragione che la frase è scritta male**, e va chiarita una volta per tutte.

**Non significa** che l'app scarichi video senza rete. Significa che **al primo avvio non esiste uno
step di rete per diventare funzionanti**: i due tool sono già lì, risolvibili ed eseguibili.

Il criterio verificabile è `yt-dlp --version` e `qjs --version` con la rete staccata, ed è esattamente
come `PHASE_PROMPTS.md` M0-C lo formula: *«with no internet, the bundled yt-dlp and qjs run
(`--version`)»*.

**Il punto reale a favore del bundling non è la rete.** Se l'app ha bisogno di internet, il vantaggio
è che **il primo avvio non può fallire**:

* **quarantena macOS** — spec §5.2 riga 161: un binario appena scaricato viene ucciso al primo lancio
  e va riparato con `xattr -d com.apple.quarantine` + `codesign`. Dentro l'app, firmato e notarizzato
  insieme a lei, non succede;
* **antivirus Windows** — un PyInstaller appena scaricato in una cartella utente è il caso classico
  che attiva SmartScreen/Defender; dentro l'installer no (spec §5.2.1 riga 170);
* **versione nota** — nell'installer c'è quella con cui l'app è stata costruita e testata;
* **nessun vicolo cieco** — se il download fallisce (proxy, CDN, disco pieno, antivirus) l'utente ha
  un'app che sembra rotta e non può fare nulla.

Nessuna di queste cose cambia l'aggiornamento automatico: la copia che si aggiorna sta in app data ed è
già prevista da spec §5.2.1 riga 165 e §5.5.

Nota di processo: la frase era stata riportata così com'è nelle note precedenti senza chiederne il
significato. Le note di M0-C useranno "nessun bootstrap di rete al primo avvio", che è non ambiguo.

---

## 2026-10-13 — M0-C (tools manifest, bundled yt-dlp + qjs, provisioning, UI)

### D31 — Il manifest è **compilato dentro il binario**, non è una risorsa

`tools.manifest.json` sta nella root del repository ed è letto con `include_str!`.

L'alternativa (un file di risorse accanto all'eseguibile) sembra più ovvia e
sarebbe sbagliata: la sua posizione cambia fra le quattro destinazioni
(`Contents/Resources` su macOS, `/usr/share` nel `.deb`, la mount di sola
lettura dell'AppImage, `%LOCALAPPDATA%` sotto NSIS) e una risoluzione sbagliata
non dà un errore — dà "gli strumenti non ci sono" su una sola piattaforma.

Con `include_str!` l'app non deve cercare nulla, e il manifest viaggia
nell'aggiornamento dell'app, che è esattamente ciò che spec §5.1 chiede ("the
manifest can be refreshed by the app's own update, no code change needed").

Un test (`the_manifest_shipped_in_the_repository_is_valid`) lo parsa a ogni
`cargo test`: un manifesto malformato è un errore di compilazione, non una
sorpresa al primo avvio.

### D32 — Nomi degli asset e checksum: misurati, non ricordati

Tutto quello che riguarda "il nome del file nella release" è stato preso dalle
API GitHub il 2026-10-13 e non dalla memoria. La ragione è che la spec lo dice
esplicitamente ("names changed between tags") e che il costo dell'errore qui è
silenzioso: un nome sbagliato non fa fallire la compilazione, fa fallire la CI
dopo il download, o peggio fa mancare un tool dall'installer.

| cosa | valore verificato |
| :-- | :-- |
| yt-dlp, release stabile più recente | `2026.08.19` |
| asset yt-dlp per target | `yt-dlp_linux`, `yt-dlp_linux_aarch64`, `yt-dlp.exe`, `yt-dlp_macos` (universale) |
| checksum yt-dlp | dal `SHA2-256SUMS` della stessa release (formato `<sha256>␠␠<nome>`) |
| QuickJS-NG, tag scelto | `v0.17.0` (≥ 0.12.0 richiesto da spec §5.3) |
| asset qjs per target | `qjs-linux-x86_64`, `qjs-linux-aarch64`, `qjs-windows-x86_64.exe`, `qjs-darwin-arm64`, `qjs-darwin-x86_64` |
| sha256 qjs v0.17.0 (linux x86_64) | `0bfc02511a9f549c28b53880d988fc7cd5d361e90c5e8afdfcd7dc6774ceace5` |
| sha256 qjs v0.17.0 (linux aarch64) | `3372133484edf50a69f3c67903af41206d22a061e930e3cfb63269272ef56d2e` |
| sha256 qjs v0.17.0 (windows x86_64) | `2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77` |
| sha256 qjs v0.17.0 (darwin arm64) | `8be3ddfe3397d2e692e4e1e8972ee9d032a0a580505d2f8b4ea528cf1b651c11` |
| sha256 qjs v0.17.0 (darwin x86_64) | `9e5e101b4fd13cda3204222ca9f8be35412c41dcdef3745829633b7a67245412` |
| licenze | yt-dlp **Unlicense**, QuickJS-NG **MIT**: entrambe lette dai file LICENSE dei repository |

**QuickJS-NG non pubblica checksum.** Lo script li calcola al primo download e
li scrive nel manifest; da quel momento sono verificati come quelli di yt-dlp. È
il massimo che si può fare senza fidarsi di qualcun altro, ed è detto qui.

**Il bundle universale macOS non è un asset.** QuickJS-NG pubblica due slice;
l'entry `universal-apple-darwin` del manifest dichiara `builtFrom` e lo script le
unisce con `lipo` sul runner macOS. Per questo quell'entry non ha `sha256`: non
è scaricata. Un test lo verifica.

### D33 — `releases/latest/download/` è risultato inaffidabile: si risolve il tag via API

Misurato il 2026-10-13: l'URL `…/releases/latest/download/yt-dlp_linux` ha
risposto **404 per minuti**, sia con curl sia con Node, su HTTP/1.1 e HTTP/2,
mentre lo stesso file con il **tag esplicito** nella stessa finestra rispondeva
200. Non è stato trovato un pattern (header, user agent, accept-encoding,
connessioni riusate): è un comportamento del bordo di GitHub.

Lo script quindi chiama `https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest`,
prende `tag_name` e gli URL esatti degli asset, e **registra la versione** che sta
per costruire — che è anche ciò che spec §5.2.1 chiede ("recording its version").
Il template `latest` resta nel manifest come ripiego se l'API non risponde, e i
tentativi sono ripetuti con attese crescenti.

Costo: una richiesta all'API per build (il limite anonimo è 60/ora/IP e una build
ne fa una). Beneficio: una build non fallisce per un alias che si è raffreddato.

### D34 — `deno` **non** è nel manifest, per ora

La prima stesura del manifest lo dichiarava con `versioning: "pinned"` e
`version: null`. La validazione l'ha rifiutata — che è esattamente il suo
compito: un'entry che non si può installare è uno stato a metà che il backend
dovrebbe gestire in modo speciale.

Le opzioni erano riempirlo (inventando un tag) o toglierlo. L'abbiamo tolto: il
suo versionamento è una decisione di M1 (spec §5.3.2, Deno lazy con consenso) e
finché non c'è, l'app lo **rileva** (è nella lookup order, quindi Homebrew e
`~/.deno/bin` lo trovano) ma non lo scarica. Nel manifest entrerà con versione e
checksum veritiere in M1. `tools::extra_tools()` elenca `ffmpeg` e `ffprobe`
per lo stesso motivo: spec §5 dice di *preferire* l'installazione di sistema.

Conseguenza onesta: la pagina Strumenti oggi **non può** installare niente, e il
bottone non è mostrato. Non è un vicolo cieco (D30): ogni riga offre "Usa il mio
percorso" e `tools_install` risponde con un errore localizzato che spiega il
motivo, invece di una promessa non mantenuta.

### D35 — `reqwest` 0.12, non 0.13: i nomi delle feature di spec §1 esistono solo sulla 0.12

Spec §1 riga 49 prescrive `default-features = false, features =
["rustls-tls", "stream", "json"]`. Quelle feature **non esistono più** in
reqwest 0.13, che le ha rinominate (`rustls`, `rustls-no-provider`,
`native-tls`, …). Con 0.13 si sarebbe potuto scrivere `features = ["rustls-tls"]`
solo scoprendo, dal messaggio di Cargo, che il nome è sbagliato.

Nella 0.12 `rustls-tls` = `rustls-tls-webpki-roots` = `__rustls-ring`: si usa
**ring**, non aws-lc-rs, quindi nessun assembler e nessun `cmake` su Windows. È
la stessa combinazione di `tauri-plugin-http`, il plugin ufficiale.

Radici: le `webpki-roots` sono incorporate nel binario e identiche sui tre
sistemi. Il prezzo è che una CA aziendale installata nel sistema non viene
usata; per un'app che scarica da GitHub e Hugging Face è la scelta più
prevedibile. Nota per il futuro: se un utente segnalerà un proxy aziendale, il
passaggio è `rustls-tls-native-roots` (verificare i requisiti di build).

Dipendenze aggiunte, con l'impatto che impatta (AGENTS.md §8):
`reqwest`+`rustls` (~3 MB, obbligata da spec §1), `sha2` (~50 kB),
`zip`+`tar`+`flate2`+`lzma-rs` (~400 kB, tutte Rust puro). Nessuna di queste
richiede un toolchain C: `zip` ha `default-features = false` perché il suo
default include `xz`, che lega liblzma.

### D36 — Le feature di versione sono per-tool, e il parser accetta il prefisso `n` di ffmpeg

`--version` non è universale: spec §5.2 dice "`--version` **(or equivalent)**" e
l'eccezione è reale. Verificato su un `ffmpeg` vero il 2026-10-13: il flag
documentato è `-version`.

E il banner reale è `ffmpeg version n9.0.2 Copyright (c) 2000-2026 …`: il
prefisso `n` fa parte del token. Un parser che accetta solo token interamente
numerici avrebbe letto **"ffmpeg non ha un numero di versione"** e mostrato
"versione sconosciuta" su ogni macchina con ffmpeg 7 o successivo.

Correzione collegata: una versione notturna di yt-dlp è `YYYY.MM.DD.NNNNNN`,
che **è** numerica a punti e si confronta normalmente. Una nota precedente di
questo file diceva il contrario; la misura sul nome reale ha corretto la nota.

### D37 — L'unica porta aperta verso http è `loopback_http()`, e rifiuta ogni host non-loopback

I test del downloader hanno bisogno di un server HTTP in chiaro. La prima
versione aveva un flag `allow_insecure`, che chiunque avrebbe potuto puntare
sull'internet: un'eccezione "solo per i test" che si può puntare ovunque non è
un'eccezione.

Ora `Transport::loopback_http()` accetta `localhost`, `127.x.y.z` e `[::1]` e
nient'altro. I test hanno trovato due buchi nella versione iniziale:
`http://127.0.0.1.evil.com` passava (`starts_with("127.")`, che è un host
internet come un altro) e `http://127.0.0.1@evil.com` passava perché il
userinfo veniva letto al posto dell'host.

### D38 — Un solo guard di path traversal per zip e tar, e rifiuta la `\` su ogni sistema

Le specifiche dei due formati descrivono nomi in modo diverso, quindi la tentazione
è di scrivere due guard. Una sola funzione (`safe_extract_path`) copre
entrambi, e in più rifiuta la barra rovesciata **anche su Linux**: lì
`a\..\..\etc` è un nome di file innocuo, sulla stessa app spacchettata su Windows
è un traversal. Una guardia che regge solo sulla macchina che ha scritto il
codice non è una guardia.

I link simbolici dentro un `.tar` non vengono mai materializzati: un archivio di
tool non ha motivo di contenerne e onorarli è il modo tipico di uscire dalla
cartella di destinazione. C'è un test che lo afferma.

### D39 — In un build di sviluppo la ricerca del tool incluso guarda anche il nome col triple

Il bundler mette i tool inclusi **accanto all'eseguibile principale, senza il
suffisso `-<triple>`** (verificato in `tauri-bundler`
`bundle/settings.rs::copy_binaries`, che toglie `-<target>` dal nome; `bin_dir` è
`Contents/MacOS` su macOS, `/usr/bin` nel `.deb`, `$INSTDIR` sotto NSIS, la
squashfs nell'AppImage).

Ma `tauri dev` non copia nulla, e quello che lo script di fetch lascia in
`src-tauri/binaries/` ha **ancora** il suffisso. Senza il candidato in più, la
prima volta che si preme F5 dopo `npm run fetch:tools` si sarebbe vista una pagina
Strumenti che dice che yt-dlp non esiste, subito dopo averlo scaricato. Il
candidato extra esiste solo in build di debug e non cambia l'ordine di
un'installazione vera.

### D40 — Il `lookup` non usa più funzioni "da" che ricavano l'ambiente

`lookup_from` / `lookup_for` costruivano il `Lookup` duplicando le tre righe di
`ToolContext::lookup`. Sono state rimosse: un solo posto legge `PATH`, la home e
gli override, ed è quello che i test possono raggiungere.

### D41 — `tools_install` esiste e risponde "non ancora", invece di non esistere

La spec §5.2.1 dice che la pagina Strumenti deve offrire un Install per ogni
composizione scaricabile e che "features whose tool is missing are visible but
disabled … never a dead end". Oggi non c'è **niente** di scaricabile nel manifest,
quindi il comando esiste, accetta il nome e risponde `UNSUPPORTED` con un dettaglio
che dice perché e quando arriverà. Un bottone che non fa niente è peggio di
un'assenza.

### D42 — `minVersion` di yt-dlp: `2025.11.12`, con le prove che ho e quelle che non ho

Spec §5.3 (QuickJS → Deno) richiede `--js-runtimes`, che nel 2026 esiste
sicuramente. **Non** ho pinning la release esatta che lo ha introdotto: la ricerca
nel changelog di yt-dlp non l'ha trovata (il changelog non nomina mai
`--js-runtimes`).

Quello che **ho** verificato è la release `2025.11.12`, il cui testo dice
«**An external JavaScript runtime is now required for full YouTube support**».
È un pavimento conservativo e ragionevole, non un fatto esatto sul flag.

→ **TODO(verify) in M1**, quando si implementa la strategia QuickJS: verificare
con `--help` del binario fissato quale sia la release minima reale, e correggere
`minVersion` nel manifest se è più bassa. Il codice non dipende dal valore: legge
il minimo dal manifest, quindi la correzione è una riga di JSON.

### D43 — Il `-U` non viene mai eseguito su un tool che non è nostro

Spec §5.5 dice di aggiornare "the binary's own updater on the **app-data copy**".
Se la risoluzione trova un yt-dlp da Homebrew o da un gestore di pacchetti,
aggiornarlo non è affare nostro: il comando risponde `updated: false` con un
messaggio che dice da dove viene e di aggiornarlo lì. Aggiornare il solo file
proprio è l'unica interpretazione che non rompe il sistema dell'utente.

### D44 — Il classificatore dell'aggiornamento usa il codice di uscita **prima** del testo

Prima versione: se il testo non conteneva "up to date", l'aggiornamento era
riportato come riuscito. Con `ERROR: unable to download` e uscita non-zero
l'aggiornamento risultava **riuscito**. Il test l'ha trovato. Ora un'uscita non
zero non è mai un aggiornamento, e il testo serve solo quando l'uscita è 0 ma
non è stato possibile confrontare le versioni.

Nota sul perché è difficile stabilirlo dal solo testo: `-U` scrive su **stderr**
e "already up to date" è un'uscita 0 normale. Per questo esiste una funzione
dedicata con i casi esatti coperti dai test.

### D45 — La CI scarica i tool prima di costruire, e li **esegue**

`bundle.externalBin` senza i file fallisce con un errore poco leggibile, quindi i
job `bundle` e `bundle-macos-universal` ora chiamano `npm run fetch:tools` prima
di `tauri build`, e stampano `ls -l src-tauri/binaries` nel log.

In più, prima di costruire girano `cargo test --test bundled_tools`: il vero
yt-dlp e il vero qjs, presi da quanto appena scaricato, devono rispondere a
`--version`. Questo è il criterio di uscita di spec §14 per M0 ("yt-dlp + qjs
work offline right after install") verificato **davvero** e non dichiarato.

Il job `check`, che gira a ogni push, aggiunge `npm run check:tools`: validazione
del manifest **offline**, senza rete, in circa un secondo.

### D46 — Il test del falso tool è multi-call: decide da `argv[0]`

`fake_tool` copiato come `yt-dlp`, `qjs` o `ffmpeg` si comporta come quel
programma, `-U` compreso (cambia la versione che risponderà alla chiamata
successiva). È così che la lookup di spec §5.2 si prova offline su tre sistemi
senza dipendere da quale yt-dlp è installato sulla macchina che esegue i test.

Tre cose imparate scrivendo i test, che vale la pena non dimenticare:

1. **`ffprobe`/`ffmpeg` rispondono a `-version`**, non a `--version` (D36);
2. **uno script di shell finto non è "un file che non gira"**: su Linux e macOS
   gira, e il test passa per il motivo sbagliato. I byte non eseguibili sono
   l'unico modo onesto di simularlo su tre sistemi;
3. **chiudere una socket con dati non letti manda RST**, e l'RST fa perdere al
   client il corpo **già ricevuto**: sembrava un bug del downloader, era il
   server di test.

### D47 — `ETXTBSY`: Linux diceva "yt-dlp non c'è" per colpa del filesystem

Il test della copia al primo avvio falliva **una volta su tre**, sempre su un
test diverso, e solo quando i test girano in parallelo. Il sintomo era
`resolve()` che restituiva `None`: la copia appena creata in app data non era
eseguibile.

Il motivo, stampato: `spawn error starting "…/tools/yt-dlp/yt-dlp": Text file
busy (os error 26)`.

`ETXTBSY` è ciò che Linux risponde a `execve` quando **qualcosa** tiene ancora
il binario aperto in scrittura. Ed è esattamente la situazione che questo
progetto crea: spec §5.2.1 copia yt-dlp in app data e lo esegue subito dopo.

Una scansione di `/proc/*/fd` fatta al momento del fallimento **non trova
nessun titolare**: chi scriveva aveva già chiuso. È la firma di un errore
transitorio, non di un bug stabile.

Cosa è stato fatto, due pezzi:

1. **`process.rs` ritenta lo spawn** su `ETXTBSY`: 5 tentativi, 20 ms di
   attesa, complessivi 100 ms — ben sotto la sonda più corta dell'app (5 s), e
   c'è un test che lo vincola. Un `Command` non si può riusare, quindi
   l'argv si calcola una volta e il `Command` viene ricreato a ogni tentativo.
2. **`install_app_data_copy` fa `fsync`** del file prima del rename e della
   cartella dopo. Non è la causa dell'`ETXTBSY`, ma è la correttezza che
   comunque mancava: senza, un'interruzione di corrente può lasciare un
   eseguibile che parte e poi muore.

Il punto da tenere: **senza il ritento, il sintomo per l'utente sarebbe stato
"yt-dlp non è installato"**, che è la diagnosi più sbagliata possibile: il
file c'era, era giusto, e sarebbe bastato aspettare 20 ms. Non è una difesa
"per il test": è la difesa per il caso reale in cui un antivirus, un indicizzatore
o un backup tiene aperto per un istante il file appena scritto.

Cosa **non** ho determinato: chi teneva il file aperto in quel momento. Due
ipotesi restano aperte — un altro processo della suite che leggeva la sorgente
condivisa `target/debug/examples/fake_tool`, o una finestra del kernel sulla
scrittura. Il ritento copre entrambe, quindi la domanda non blocca il lavoro, ma
è registrata qui perché qualcuno potrebbe volerla chiudere.

Nota di processo: i due fix sono entrambi ragionevoli **separatamente**, ma solo
la ripetizione del test li ha distinti. Senza il `fsync` il ritento da solo
avrebbe potuto sembrare sufficiente; senza il ritento il `fsync` da solo
avrebbe potuto sembrare sufficiente. La lezione è la stessa di D29: «compila» e
«funziona» sono due affermazioni diverse, e qui anche «passa una volta su tre» e
«passa» lo erano.

### D48 — La CI era rotta: `tauri_build::build()` gira a **ogni** cargo

La prima run della branch è fallita così:

```
resource path `binaries/yt-dlp-x86_64-unknown-linux-gnu` doesn't exist
error: failed to run custom build command for `scaricatore-di-porto v0.1.0`
```

È un errore mio, non un problema di macchina. Avevo dichiarato
`bundle.externalBin` in `tauri.conf.json` e avevo messo il fetch dei tool **solo
nei job `bundle`**. Ma `tauri_build::build()` sta dentro `build.rs`, e `build.rs`
 gira a **ogni invocazione di cargo**: `check`, `clippy`, `test`, `build`. Non
solo a `tauri build`.

Riprodotto qui prima di correggere, con `src-tauri/binaries/` spostato via:

| comando | binaries assenti |
| :-- | :-- |
| `cargo check` | **fallisce** |
| `cargo clippy --all-targets` | **fallisce** |
| `cargo test` | **fallisce** |

Quindi non era solo il job `check`: erano rotti anche `process-tests` (Windows e
macOS) e `tier2`. Nessuno se n'è accorto perché **in locale la cartella c'era
sempre**: l'avevo scaricata prima di ogni run. Un errore che si nasconde
esattamente dove lo hai costruito.

**La correzione**, e la scelta che contiene:

Nei job che **non impacchettano** (`check`, `process-tests`, `tier2`):

```yaml
env:
  TAURI_CONFIG: '{"bundle":{"externalBin":[]}}'
```

Nei job che **impacchettano** (`bundle`, `bundle-macos-universal`): nessuna
variabile, i tool vengono scaricati davvero, **eseguiti**, e solo poi
impacchettati.

Il perché della divisione, e perché non è una scorciatoia:

* il percorso che produce l'installer resta verificato **per intero** — anzi più
  di prima, perché i tool vengono eseguiti prima di entrare nel pacchetto;
* il job più frequente non dipende più dalla rete. Oggi è una scelta motivata da
  un fatto, non da una preferenza: **l'alias `releases/latest` di GitHub ha
  risposto 404 per minuti** mentre lo stesso file con il tag esplicito
  rispondeva 200 (D33). Un `check` che diventa rosso perché un CDN ha avuto un
  colpo di calmo isterizza chi lo guarda;
* 43 MB di download risparmiati a ogni push, su un repository privato dove i
  minuti sono contati (D12).

**Il controllo che mancava**, adesso c'è: `npm run check:tools` verifica anche che
`bundle.externalBin` in `tauri.conf.json` e `bundled: true` nel manifesto
dicano le stesse cose. Un tool in uno solo dei due file romperebbe la build con
un messaggio che non dice quale delle due metà sia sbagliata; il controllo costa
zero e chiude il ciclo.

Una nota su quel controllo, perché è la parte instructive. La prima versione
diceva che le voci di `externalBin` dovessero essere **nomi nudi** (`qjs`, non
`binaries/qjs`). Ho scritto il test, l'ho lanciato, e **il test ha detto che il
repository è sbagliato** — perché avevo scritto male io la regola. La fonte
(`tauri-utils/src/resources.rs`, `external_binaries`) dice che la voce viene
usata così com'è e le viene appesi `-<triple>` ed `.exe`:

```rust
paths.push(format!("{curr_path}-{target_triple}{extension}"));
```

Quindi `binaries/yt-dlp` è corretto (ed è la forma documentata), e ciò che è
davvero vietato è portare un'estensione di sistema: `binaries/qjs.exe`
chiederebbe `binaries/qjs.exe-x86_64-pc-windows-msvc.exe`, che nessuno produce.
Regola corretta, coperta da test che verificano entrambi i casi.

Vale la pena tenere a mente il verso: **un controllo che aggiungi per impedire un
errore futuro può essere lui stesso l'errore**, e l'unico modo che lo scopra è
guardare la fonte e far girare il controllo. Esattamente come per `ETXTBSY`
(D47) e per il parser delle versioni (D36): nessuno dei tre si vede leggendo il
codice in piedi.

### D49 — Cinque test che sapevano una sola risposta (solo-Linux)

La prima run Windows ha fallito con 5 test rossi, tutti sulla gestione dei path.
Non era un bug di produzione: **era il test a essere scritto male**, e il
codice aveva ragione in tutti e cinque i casi.

| test | cosa si aspettava | cosa rispondeva Windows | chi aveva ragione |
| :-- | :-- | :-- | :-- |
| `an_override_wins_and_nothing_else_is_listed` | `/opt/mine/qjs` | `/opt/mine/qjs.exe` | il codice: `.exe` è il nome giusto su Windows |
| `path_dirs_are_split_read_and_never_made_relative` | 2 voci | `[]` | il codice: `/usr/bin` **non** è assoluto su Windows |
| `well_known_locations_include_the_ones_the_spec_lists` | `/opt/homebrew/bin` | lista Windows | il codice: Homebrew non esiste lì |
| `state::a_tool_override_survives_a_round_trip_…` | `Some("/opt/mine/qjs")` | `None` | il codice: relativa, quindi scartata (spec §4.8) |
| `settings_ipc::tool_settings_travel_through_the_ipc` | il valore | `Null` | il codice: idem |

La radice è una sola e la verifica è nella sorgente di std, non nella memoria
(`library/std/src/sys/path/windows.rs`, Rust 1.95.0):

```rust
pub(crate) fn is_absolute(path: &Path) -> bool {
    path.has_root() && path.prefix().is_some()
}
```

`/opt/mine/qjs` ha una root ma **nessun prefisso**: su Windows non è assoluta.
Il codice che la scarta fa esattamente quello che dice la regola, e il test lo
ha scambiato per un difetto.

**La correzione non è allentare le asserzioni.** Sarebbe stato il modo più
veloce di farle passare e il modo peggiore di tenere il segreto: nessuno
avrebbe più saputo che quel comportamento è intenzionale. I test sono stati
riscritti perché chiedano **di questa macchina**:

* `host_dir(name)` / `host_exe(dir, tool)` costruiscono un path assoluto nella
  sintassi dell'host (`C:\opt\mine\qjs.exe` su Windows, `/opt/mine/qjs` su Unix).
  Non è una comodità: è il punto dell'intervento.
* `well_known_locations_…` ora asserisce **entrambi** i rami, e la funzione è
  stata separata in `well_known_dirs_for(home, windows, user_profile,
  local_app_data)`, cioè dagli stessi argomenti di `override_candidate`, che
  già esisteva per lo stesso motivo (D27).

**Perché separare la funzione invece di splittare il test per piattaforma.**
Senza, la lista Windows sarebbe asserita solo dal runner Windows — cioè per la
prima volta dal runner che aveva gli altri cinque test rossi. Un controllo che
gira solo quando ilgioco è già rotto non è un controllo. Qui il ramo Windows
viene eseguito da **Linux, a ogni push**, ed è per questo che una delle
asserzioni nuove è volutamente negativity: la lista Unix non deve contenere
`WinGet`, e quella Windows non deve contenere Homebrew.

**Una cosa che il test nuovo ha trovato per conto suo.** Chiedendo a
`well_known_dirs_for` la lista Windows da Linux, il risultato è stato
`C:\Users\tester/.deno\bin`: `Path::join` sceglie il separatore dell'**host**,
non quello della piattaforma richiesta. Non è un difetto di produzione (in
produzione i due coincidono sempre), ma rende la funzione non determinata dai
suoi soli argomenti, cioè non testabile. La funzione ora compone i path Windows
come testo con `format!`, con il motivo scritto accanto.

Come ogni correzione di questa sessione, è stata **verificata revertendola**:
invertendo l'ordine delle due directory Windows il test va rosso per la
ragione giusta, e tornando indietro va verde.

**Il limite onesto:** questi cinque test non sono mai stati **eseguiti** su
Windows. Le risposte di `std` sono verificate dalla sorgente, ma il giudice è il
runner Windows della CI. E il `cargo check --target x86_64-pc-windows-msvc` non
è disponibile qui: si ferma su `ring`, che vuole `lib.exe` di MSVC.

### D50 — Un bundle universale vuole **tre** nomi per tool, non uno

`bundle (macos universal)` è fallito così:

```
cargo:rustc-env=TAURI_ENV_TARGET_TRIPLE=aarch64-apple-darwin
resource path `binaries/yt-dlp-aarch64-apple-darwin` doesn't exist
```

La causa, verificata in `crates/tauri-build/src/lib.rs` e
`crates/tauri-bundler/src/bundle/settings.rs`:

```rust
// tauri-build: dentro build.rs, quindi una volta per ogni cargo
let target_triple = env::var("TARGET").unwrap();              // aarch64-apple-darwin
copy_binaries(ResourcePaths::new(
  &external_binaries(paths, &target_triple, &target), true), &target_triple, target_dir, ...)

// tauri-bundler: una volta sola, alla fine
external_bin: self.bundle_settings.external_bin
  .map(|bins| external_binaries(bins, &target, &target_platform)),   // target = universal-apple-darwin
```

`tauri build --target universal-apple-darwin` **lancia cargo una volta per
architettura**. A ogni passaggio `build.rs` chiede i nomi **per-architettazione**;
solo alla fine il bundler chiede il nome **universale**. Occorrono tutti e trei:

```
binaries/<tool>-aarch64-apple-darwin       <- build script, passaggio 1
binaries/<tool>-x86_64-apple-darwin       <- build script, passaggio 2
binaries/<tool>-universal-apple-darwin    <- bundler, impacchettamento
```

Noi ne producevamo uno. Da qui il fallimento, su `yt-dlp`, al primo dei due
passaggi di compilazione.

**Come si ottengono le due slice, e perché i due tool sono casi diversi.**

* `qjs` ha `builtFrom` nell'universale: le due slice sono due asset separati,
  gia' con checksum, che scarichiamo e lipo-iamo. Le slice sono gia' in mano.
* `yt-dlp` non ha `builtFrom`: l'asset universale **e' gia' un fat binary**, e le
  slice si ricavano con `lipo -thin`. Verificato leggendo l'intestazione
  Mach-O **su Linux, senza macOS**: magic `0xcafebabe`, `nfat_arch` 2,
  `x86_64` (73 808 B) + `arm64` (37 047 744 B), asset `yt-dlp_macos` (37 146 048 B).

Quindi `lipo -thin` è la strada giusta, e se un asset futuro fosse a
architettura singola `lipo -thin` fallisce qui, adesso, invece di produrre un
`.app` che parte su una CPU e non sull'altra.

`UNIVERSAL_SLICES` e `lipoArchName` sono i due fatti che il codice deve sapere:
le slice sono di Tauri (non nostre), e `lipo` vuole `arm64` dove Rust dice
`aarch64`. Entrambi sono derivati, non indovinati, e hanno test propri.

**Un controllo che è anche una difesa.** `validateManifest` pretende ora che
`builtFrom` elenchi esattamente le slice che Tauri assembla: una terza slice
verrebbe scaricata e buttata via, una sola produrrebbe un bundle che si dichiara
universale e non lo e'. E come ogni correzione di questa sessione, e' stata
verificata revertendola.

**Il passo di CI e' stato eseguito, e aveva un bug.** Il vecchio passo era
`lipo -info` su **un** file, scritto il giorno prima e mai eseguito. Sostituito
da `Every bundled file is the architecture its name promises`, che controlla tutti
e sei e puo' fallire su sei modi diversi. Eseguito qui con un `lipo` finto, e ha
trovato subito due difetti miei:

1. **`Non-fat file` contiene la sottostringa `fat file`.** Il controllo di rifiuto
   scattava quindi sui file *giusti*. Il passo era rotto per i 6/6 file e avrebbe
   reso rosso il job che doveva accertare che il bundle fosse a posto. Risolto
   usando le due forme esatte (`Architectures in the fat file` /
   `Non-fat file`). Nessun `lipo` reale era servito a trovarlo: `Non-fat file`
   vuol dire *non* grasso, e la parola contiene lo spezzone che cercavo.
2. **Un ramo morto.** `check_single` aveva anche un controllo "e' un fat binary?".
   Irraggiungibile: `lipo -info` su un fat binary stampa
   `Architectures in the fat file:`, che non contiene mai `is architecture:`, quindi
   il primo controllo lo ha gia' respinto. Tolto: un ramo che nessun test puo'
   raggiungere e' un'affermazione che nessuno puo' verificare.

Il passo ora e' coperto da `scripts/ci-steps.test.mjs`: 7 test che lo eseguono
davvero, su Linux, a ogni push, contro un `lipo` finto che risponde in base al
**contenuto** del file e non al suo nome — l'unico modo per provare la cosa che
conta, cioe' un file che mente sul proprio nome. Il primo tentativo di questi
test era sbagliato (il finto decideva dal nome, quindi non poteva rappresentare
il caso) ed e' stato rifatto.

**Il limite onesto:** `lipo -create`, `lipo -thin` e `tauri build` universale non
sono mai girati qui. Non esistono su Linux. Le uniche cose verificate su questa
macchina sono la logica di fetch (nomi, checksum, ordinamento), il passo di CI
contro il `lipo` finto, e l'intestazione Mach-O dell'asset yt-dlp. Il resto lo
vedrà il runner macOS.

### D51 — La CI Windows come seconda opinione, e il bug che ha trovato subito

Domanda di comodo: «si possono fare i test Windows su Linux?». Risposta: sì,
via `cargo-xwin` + wine, e su questa macchina funziona. Ma il valore vero non è
la comodità, è che **una delle cinque risposte che avevo dato con supposizioni
era sbagliata**, e me la sono lasciata sfuggire due volte.

**La supposizione.** Avevo scritto che wine implementa i Job Object «in modo
parziale» e che `process_tree` sotto wine sarebbe stato inutile. Non l'avevo
verificato. Quando ho cercato di verificarlo non sono riuscito a trovarne
l'implementazione nella sorgente di Wine (`dlls/ntdll` e `dlls/kernel32` non
hanno file con `job` nel nome; `dlls/kernelbase/process.c`,
`dlls/ntdll/process.c`, `server/process.c` non contengono **zero** occorrenze di
`JobObject`), e ho detto «non lo so» invece di lasciare l'assunzione in piedi.
Già così sarebbe stato un miglioramento.

**La misura, tramite un proxy.** Ho scritto una finestrella che chiama
`CreateJobObjectW`, crea un figlio, lo assegna al job e termina il job.
Risultato: **il figlio muore**. La mia supposizione era sbagliata proprio nel
verso in cui l'avevo formulata.

*Nota sul proxy, che è metodo e non aneddoto.* La prima versione si uccideva
da sola: avevo messo **il processo stesso** dentro il job, quindi
`TerminateJobObject` terminava anche noi, e l'output finiva lì. Quello non era
un difetto di wine — è comportamento Windows corretto — era il mio probe
mal scritto.

**Il test vero, che vale più del proxy.** Compilata la suite per
`x86_64-pc-windows-msvc` e lanciata sotto wine:

```
test cancelling_kills_the_child_and_the_grandchild ... ok
test dropping_a_running_tool_kills_the_tree ... ok
test the_app_exit_hook_kills_every_running_tool ... ok
test result: ok. 15 passed; 0 failed
```

Vale la pena tenere a mente perché i due potevano contraddirsi: se mi fossi
fermato al proxy avrei scritto un'altra assunzione al posto di un fatto, e
sarebbe stata sbagliata. **Il proxy testa la mia idea del problema, il test
vero testa il problema.** Quando i due discordano, il test vero ha ragione.

Prima compilazione fallita 15/15, per un motivo che non c'entrava: avevo
filtrato con `--test process_tree`, quindi Cargo non aveva compilato l'esempio
`fake_tool`. La suite lo diceva da sola, per nome di file e con l'istruzione
giusta. Un test che sa distinguere «i Job Object non funzionano» da «non hai
eseguito niente» è un test che ti protegge; se non lo facesse, ti
m(and)erebbe.

**Ed ecco il bug: era mio, e del tipo esatto che D49 aveva appena corretto.**
I 133 test della libreria sotto wine ne hanno trovato uno che Linux non poteva
vedere:

```
assertion failed: unix.contains(&"/home/tester/.deno/bin".to_owned())
```

`Path::join` usa il separatore dell'host, quindi su Windows
`well_known_dirs_for` produceva `/home/tester\.deno\bin` — un path che non
appartiene a nessun sistema. Peggio: il commento scritto sopra la funzione
diceva che «la risposta dipende solo dagli argomenti», e per il ramo Unix era
**falso**. Ora anche il ramo Unix è composto con `format!`. Su Unix il risultato
è identico a prima (il separatore è `/`), quindi **il comportamento di
produzione non cambia**: cambia la promessa della funzione, che ora è vera.

Il punto non è il bug. Il punto è che **un test scritto la sera prima e dichiarato
«cross-platform» non lo era**, e a scoprirlo non è stato un runner Windows ma
un controllo locale di novanta secondi. È la sesta volta in due giorni che la
stessa classe di errore viene fuori da qualcosa che non era mai stato eseguito
dove dice di funzionare.

**Il 133 contro 132.** Sotto Windows la libreria ha un test in meno:
`create_private_dir_uses_mode_0700`, `#[cfg(unix)]`. Su Windows i permessi Unix
non esistono e la privacy della temp dir (spec §4.9, «mode 0700») viene dal
fatto che la cartella sta sotto la app data dir dell'utente, che porta già la
sua ACL. Non è un buco: è una differenza di piattaforma che ora è scritta nel
codice, accanto alla funzione, invece di essere una differenza di conteggio che
qualcuno avrebbe notato fra un anno chiedendosi perché manca un test.

**Cosa ci si compra, e cosa no.** Si compra una seconda opinione Windows a costo
zero di quota: 35 secondi il check di compilazione, 8 secondi i test della
libreria, 10 secondi `process_tree`. Non si compra la validazione del bundle:
`npx tauri build` su Windows, l'installer e `bundled_tools` restano da provare su
un runner vero. E non sostituisce la CI: wine non è Windows, e su questo
l'hardware non lo dimostreremo meglio di così.

Comandi (la `TAURI_CONFIG` è la stessa del job `check`, e serve):

```bash
export CARGO_TARGET_DIR=/home/arturo/xwin-target
export TAURI_CONFIG='{"bundle":{"externalBin":[]}}'
export WINEDEBUG=-all
cargo xwin check --all-targets --target x86_64-pc-windows-msvc
cargo xwin build   --example fake_tool     --target x86_64-pc-windows-msvc
cargo xwin test    --lib                   --target x86_64-pc-windows-msvc
cargo xwin test    --test process_tree     --target x86_64-pc-windows-msvc
```

Prerequisiti verificati qui: `clang` 23.1.1 (serve `clang-cl`), `llvm-tools` di
rustup (`ring` usa assembly, si installa con `rustup component add llvm-tools`),
`wine` 11.19. **L'esempio `fake_tool` va compilato a parte**: con
`--test <nome>` Cargo non lo costruisce, e serve a `process_tree`.

### D52 — Un path scritto al posto dei contenuti, e la guardia che lo rende impossibile

Seconda run di `bundle (macos universal)`. La prima si era fermata su
`resource path ... doesn't exist` (D50); questa è arrivata fino al passo di CI,
che ha fatto il suo lavoro:

```
Architectures in the fat file: …/yt-dlp-universal-apple-darwin are: x86_64 arm64
Non-fat file: …/yt-dlp-aarch64-apple-darwin is architecture: arm64
Non-fat file: …/yt-dlp-x86_64-apple-darwin is architecture: x86_64
Architectures in the fat file: …/qjs-universal-apple-darwin are: x86_64 arm64
fatal error: …/lipo: can't figure out the architecture type of:
  …/binaries/qjs-aarch64-apple-darwin
```

Cinque file su sei corretti, e il sesto non è un Mach-O.

**Prima la pista sbagliata, poi quella giusta.** La prima lettura del messaggio
suggerisce un asset sbagliato, e infatti sono andato a verificare gli asset
prima di guardare il codice: `qjs-darwin-arm64` e `qjs-darwin-x86_64` scaricati
e letti su Linux, sono entrambi Mach-O `MH_EXECUTE` (`filetype 2`, `cpu 0x100000c`
e `0x1000007`). Asset e manifesto sono **corretti**. Il difetto è come li
scriviamo.

**La causa, in tre righe di codice.**

```js
async function downloadSlice(…) { …; return path; }              // restituiva un PATH
…
if ((entry.builtFrom ?? []).length > 0) {
  return await downloadSlice(manifest, name, slice, scratch);     // passato come payload
}
…
await writeFile(temporary, bytes);                               // scriveva il PATH come contenuto
```

`binaries/qjs-aarch64-apple-darwin` conteneva la **stringa**
`/tmp/sdp-tools-XXXXXX/qjs-aarch64-apple-darwin`: una trentina di byte di testo.
Il checksum era corretto — combaciava con quello scaricato, che era quello
giusto. Il fetch si dichiarava **riuscito**. Solo due passi dopo `lipo` si
accorgeva che il file non era un eseguibile, e lo diceva in un modo che non
nomina la causa: «can't figure out the architecture type», una frase
sull'architettura quando il problema è che non è un Mach-O.

**Perché solo qjs.** I due strumenti prendono due rami diversi di
`perArchBytes`: qjs ha `builtFrom` e passa da `downloadSlice` (il path), yt-dlp
no e usa `lipo -thin` con `readFile` (i byte). Solo il primo era rotto. Un ramo
che si esercita solo su una delle due piattaforme è un ramo che può essere
sbagliato a metà senza che nulla se ne accorga.

**Il rimedio, in due parti.** La prima è far scegliere al chiamante:
`downloadSlice` restituisce `{ bytes, path }` e i due chiamanti dichiarano
esplicitamente quale dei due gli serve (`(await …).path` per `lipo -create`,
`.bytes` per la scrittura). In JavaScript non c'è il compilatore che ti
obbliga, ma la scelta è scritta e si vede.

La seconda è una guardia in `install()`, che rifiuta un payload di tipo stringa
**al momento dell'errore**:

```js
if (typeof bytes === "string") {
  fail(`internal error: asked to write a string to ${destination}\n  …`);
}
```

Una riga che trasforma un mistero in una frase, sul posto giusto. È il tipo di
controllo che in un linguaggio con i tipi non serve e in JavaScript serve, perché
`writeFile` accetta una stringa e la scrive senza protestare. Ha **quattro
test**, incluso quello che verifica che il rifiuto non lasci un file a metà.

**Il passo di CI ha fatto il suo lavoro, e questo è il punto.** `lipo -info` non
è stato quello che ha trovato il difetto: il difetto era a monte e non gli
arrivava. Quello che ha fatto è stato **non lasciarlo passare in silenzio**, e
farlo in un modo che ha permesso di capire in quattro secondi che 5 file su 6
erano giusti. Un controllo che dice solo «fallito» avrebbe obbligato a
ricostruire la catena da capo; uno che dice «questo file non è un eseguibile»
mette il dito sul file.

**Il ciclo è chiuso su due livelli.** La guardia prende lo sbaglio alla fonte,
il passo di CI prende qualunque cosa altro sia sbagliata al confine. Se uno dei
due manca, l'altro c'è.

**L'ennesima volta la stessa cosa.** È il settimo difetto di questa classe, e
sono tutti nati da codice che non era mai stato eseguito. La volta prima (D51)
ero stato fortunato: il proxy che avevo scritto per misurare wine diceva la
stessa cosa del test vero, e avrei potuto fermarmi lì con un fatto sbagliato.
Questa volta la catena si è rotta e non me ne sono accorto, perché **il
comportamento esiste solo su macOS**: su Linux il ramo `lipo -thin` funziona,
quindi il codice sembrava a posto. È il limite vero di questo approccio, e va
detto: `cargo-xwin` copre Windows, ma per macOS non esiste niente di
paragonabile, e D50 l'aveva già scritto che `lipo` e `tauri build` universale
non girano su Linux.

**Non verificato, ancora:** il fetch universale vero e `tauri build --target
universal-apple-darwin` restano senza esecuzione. La prossima run dirà se la
catena regge fino in fondo.

### D53 — `.exe` dopo il triple, non prima: il bundle Windows non trovava i suoi stessi tool

Ultimo rosso, e il più economico da spiegare e il più costoso da trovare.

```
---- the_bundled_ytdlp_and_qjs_run_without_a_network stdout ----
panicked at tests\bundled_tools.rs:92:32:
yt-dlp: the bundled binary did not answer --version
```

`resolve()` non trova nulla. Ma il file c'era: nella stessa suite,
`the_bundled_ytdlp_is_a_real_yt_dlp_and_not_a_stub` passa e controlla che il
file esista e superi i 5 MB. Il test che esegue qjs passa, e spawma
l'eseguibile **direttamente**. Solo `resolve` fallisce: il problema non è
eseguire i binari, è **non trovarli**.

**La causa, in una riga.**

```rust
let file_name = executable_name(tool, triple);            // Windows: "yt-dlp.exe"
…
lookup.bundled_dir.join(format!("{file_name}-{triple}")) // "yt-dlp.exe-x86_64-pc-windows-msvc"
```

Il file si chiama `yt-dlp-x86_64-pc-windows-msvc.exe`. Due nomi diversi, quindi
il candidato non esiste, `probe_version` restituisce `None` subito — coerente
con i 0,23 s della suite: **non è un timeout, non è PyInstaller, non è
`%TEMP%`**. Non ho perso tempo a inseguire nessuna delle tre ipotesi buone,
perché il tempo diceva già che il processo non era mai partito.

**Due fonti indipendenti dicono dove va `.exe`**, e non sono una mia scelta:

* `tauri_utils::external_binaries` costruisce `format!("{curr_path}-{target_triple}{extension}")`,
  con `extension = ".exe"` solo su Windows;
* `fetch_bundled_tools.mjs` (`destinationPath`) scrive `<nome>-<triple>.exe`.

Quindi `executable_name` e il nome del file in `binaries/` sono **due cose
diverse** e vanno costruite in due modi diversi. La funzione nuova
`fetched_name(tool, triple)` le tiene separate a verbale.

**Perché Linux non lo ha mai visto.** Su Linux non c'è estensione da collocare:
`executable_name` restituisce `yt-dlp`, e `yt-dlp-<triple>` è già giusto. Il
codice era corretto per due piattaforme su tre e sbagliato solo dove la
convenzione portava dentro un dettaglio.

**Il test che avrebbe dovuto accorgersene rispecchiava l'implementazione.**

```rust
let suffixed = PathBuf::from("/app").join(format!("yt-dlp-{LINUX}"));   // LINUX
```

Una sola triple, e costruita con lo stesso criterio del codice. Su Linux le due
costruzioni coincidevano **per costruzione**, quindi l'asserzione era vera senza
dire niente: passava su Windows, passava su Linux, e non avrebbe notato un nome
sbagliato in nessuno dei due. Riscritto: le tre triple Tier 1 più
`universal-apple-darwin`, con i nomi esatti scritti a lettere dentro il test, e
una guardia esplicita che `.exe-` non compaia mai in un nome.

Verificato revertendo la correzione, il test rosso stampa il candidato:

```
x86_64-pc-windows-msvc: candidates must include "/app/yt-dlp-x86_64-pc-windows-msvc.exe",
got [… (Bundled, "/app/yt-dlp.exe-x86_64-pc-windows-msvc") …]
```

**Un difetto che la CI poteva-dirci subito.** `explain_probe` esiste per
esattamente questo — dice perché ogni candidata è stata scartata — ma è spento
per default dietro `SDP_DEBUG_PROBE`, e il passo di CI non lo impostava. Il
messaggio che abbiamo ricevuto («did not answer --version») nomina la
conclusione e non la causa. Con la variabile accesa, la stessa run avrebbe detto
`could not be started` o `exited 0 with nothing on stdout` e il nome del file
provato. **Un diagnostico che esiste ma non è acceso non è un diagnostico**, e
questa è la seconda volta in due giorni che mi costa un round-trip: la prima è
stata `externalBin`, che non diceva quale dei due file fosse sbagliato.

**L'ottava volta stessa classe.** `externalBin`, `lipo -info`, la regola di
`externalBin`, i cinque test only-Unix, `format!` al posto di `join`, un path
scritto al posto dei contenuti, `well_known_dirs_for`, e questo. Tutti nati da
codice mai eseguito sull'ambiente per cui era scritto. Quello che cambia,
rispetto alla prima volta, è che **due di questi li ha trovati un controllo
locale in meno di due minuti** invece di una run da 95 minuti.

### D54 — Il fallback puntava a qualcosa di rotto: tag via redirect, e `GITHUB_TOKEN`

Ultimo rosso, e l'unico che non era un difetto di logica: era **GitHub che
rispondeva in due modi sfavorevoli nella stessa run**.

```
the releases API answered HTTP 403; falling back to the manifest URL
yt-dlp: downloading …/releases/download/latest/yt-dlp_macos
  retrying in 1s (attempt 2) … 30s (attempt 6)
  HTTP 404 Not Found
```

**Le due cause, misurate.**

1. **`HTTP 403` dalla API.** Le chiamate senza autenticazione sono limitate a
   60/ora per IP, e i runner ospitati da GitHub condividono intervalli di IP.
   Quindi un intervallo condiviso esaurisce la quota e un build perfettamente
   legittimo si presenta come «rate limit exceeded».
2. **L'alias `latest/download/` rispondeva `404`.** Non «a tratti»: **tre volte
   su tre**, mentre l'URL con il tag esplicito rispondeva `206` per lo stesso
   asset. Verificato lo stesso giorno, con `curl` e con Node.

Il secondo è la parte che brucia. Il fallback che D33 aveva messo — «se la API
non c'è, usa l'alias `latest`» — puntava, nel momento in cui serviva, a
qualcosa di rotto. Quindi: API satura → 404 → sei tentativi in 57 secondi →
fallimento. **Un fallback che si sa rotto costa un minuto prima di arrendersi,
ed è peggio di nessun fallback**, perché il costo è certo e il beneficio è
zero.

**La via che funziona, misurata prima di scriverla:**

```
$ curl -sI https://github.com/yt-dlp/yt-dlp/releases/latest
HTTP/2 302
location: https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19
```

Nessuna API, nessuna quota, nessun rate limit: il tag è nell'header `Location`
del redirect. Il template del manifesto è diventato
`…/releases/download/{tag}/{asset}`, così il tag c'è dove metterlo, e
`{tag}` viene sostituito non appena è noto. Con l'alias `latest` non si poteva
fare: era letteralmente scritto nel pezzo da sostituire.

Quindi l'ordine è: **API col token** (solo GitHub, mai stampato), **API senza
token** (macchina di sviluppo), **redirect della pagina release** (nessuna
quota), e basta.

**Il fallback è stato eseguito, non solo scritto.** Il primo tentativo di
provarlo è stato una proxy locale con `HTTPS_PROXY`, e **non ha funzionato**:
il `fetch` di Node non onora quella variabile, quindi l'API è andata a buon
fine e non avevo simulato niente. Non l'ho contato come verifica. Rifatto
disabilitando la chiamata all'API nel codice ed eseguendo il fetch vero:

```
the releases API is unreachable (…); asking the releases page for the tag
the releases page points at 2026.08.19
yt-dlp: downloading …/releases/download/2026.08.19/yt-dlp_macos
yt-dlp: checksum verified against SHA2-256SUMS (0f192b7ec147ab62…)
yt-dlp: installed -> … (37146048 bytes)
```

Il digest `0f192b7ec147ab62…` è **identico** a quello del percorso con l'API:
non è «ha scaricato qualcosa», è lo stesso identico file verificato.

**Il tag viene trattato come dati, non come testo.** `tagFromLocation()` non è
una regex abbastanza rigorosa, e il test lo ha dimostrato: accettava `..`, che
passa ogni controllo sui caratteri e poi finirebbe nell'URL come
`/releases/download/../{asset}` — un risalimento di directory. `.` e `..` sono
ora rifiutati esplicitamente, insieme a slash e spazi. Questo è l'unico punto in
cui il mio codice cercava di essere furbo e ci riusciva nel modo sbagliato.

**E un test che ho scritto io era sbagliato**, non il codice: mi aspettavo
`null` per `…/tag/2026.08.19/../../etc`, ma la funzione estrae `2026.08.19` e
scarta il resto, che è il comportamento giusto — la proprietà che conta è che
ciò che torna non contiene mai uno slash. Ottavo caso in due giorni di «un
controllo che aggiungi per impedire un errore futuro può essere lui stesso
l'errore» (D48): la correzione è stata nel test.

**Il token è di prima parte e non viene stampato.** `GITHUB_TOKEN` finisce solo
nell'header `authorization` delle chiamate a `api.github.com`; nessun messaggio
lo contiene, e `grep` sul sorgente lo conferma (due occorrenze in lettura, zero
in scrittura). AGENTS.md §7.

### D55 — Allowlist invece di commenti: la prossima migrazione la decidiamo noi

Richiesta: sistemare i due warning per non avere più il problema su GitHub.
Fatto, ma non con le due righe che ci si aspetta: con un **controllo che gira a
ogni push**, perché le due righe da sole risolvono il 2026 e non il 2028.

**Le due correzioni.**

* `runs-on: ubuntu-latest` → `ubuntu-24.04` (job `check` e `release`), e
  `- os: ubuntu-22.04` → `ubuntu-24.04` (gamba Linux di `bundle`).
* Le azioni alla prima versione che gira su Node 24: `checkout@v7`,
  `setup-node@v7`, `upload-artifact@v7`, **`download-artifact@v8`**,
  `rust-cache@v2.9.2`.

16 righe in tutto, più i 3 commenti che nominavano le immagini vecchie e
sarebbero diventati falsi. Il commento in testa a `ci.yml` spiega perché le
versioni sono quelle, e non è decorazione: senza, la prossima persona che vede
`@v7` e `@v8` nella stessa lista pensa a un refuso e la "sistema".

**Perché una riga non basta, e un commento non basta per niente.** I due warning
non erano difetti: erano **preannunci**. Il problema non è che non si sapesse, è
che nel file non c'era nulla che rendesse visibile il momento in cui la previsione
si avvera. `ubuntu-latest` è il caso peggiore: è un'etichetta che **migra da
sola**, in una finestra (19 ottobre – 19 novembre 2026) in cui la stessa run
poteva capitare su 24.04 o su 26.04. Non si può rimediare a posteriori: o lo
prevedi, o lo scopri da un build rosso.

**Quindi `scripts/check-ci-runners.mjs`, un allowlist.** Ogni immagine e ogni
azione deve essere scritta là dentro; quello che non c'è fa fallire il job
`check`, a ogni push, in meno di un secondo. Tre regole:

1. **nessuna etichetta fluttuante** (`ubuntu-latest`, `macos-latest`,
   `windows-latest`): migrano da sole;
2. **nessuna immagine in deprecazione**, con la scadenza nel messaggio, perché
   la data è l'informazione che serve per decidere;
3. **nessuna azione sotto la prima versione Node 24**, e un'azione non
   registrata blocca: deve essere qualcuno a verificarla, non a indovinarla.

Vantaggio dell'allowlist sulla denylist: se GitHub pubblica Ubuntu 28.04, la CI
non ci migra. `ubuntu-28.04` nell'allowlist lo decidiamo noi, misurando.

**I confini Node 24 non sono uno solo, e non si indovinano.** Verificato leggendo
`action.yml` a ogni tag il 2026-10-13:

| azione | v4 | v5 | v6 | v7 | v8 |
| :-- | :-: | :-: | :-: | :-: | :-: |
| `actions/checkout` | 20 | **24** | **24** | **24** | — |
| `actions/setup-node` | 20 | **24** | **24** | **24** | — |
| `actions/upload-artifact` | 20 | 20 | **24** | **24** | — |
| `actions/download-artifact` | 20 | 20 | 20 | **24** | **24** |

Cioè `@v5` va bene per due azioni ed è su Node 20 per le altre due, e
`download-artifact` è a **v8** mentre le altre stanno a v7. «Alzo tutto a v7»
sbaglia due volte. Un test verifica proprio questa asimmetria, perché è la
trappola della migrazione.

**Due azioni su cui la guardia stessa mi ha corretto.**

* `dtolnay/rust-toolchain@stable` l'ha bloccata, e aveva ragione da un punto di
  vista e torto da un altro: è un'azione **composite** (`runs.using: composite`),
  quindi gira negli shell e **non ha un runtime JS**: la rimozione di Node 20 non
  la tocca. Il suo tag mobile è voluto — `stable`, `nightly`, o una versione
  precisa sono la sua interfaccia. Registrata come unica esenzione dichiarata,
  con il motivo scritto accanto.
* `Swatinem/rust-cache@v2`: gira su Node 24 **da 2.9.2 in poi**. Il warning della
  CI non la citava, e ora si sa perché. L'ho pinnata a `v2.9.2`: `@v2` è un tag
  mobile che oggi punta lì, ma il controllo non può saperlo, e un numero
  verificabile è meglio di uno presunto.

**Verifiche.** La guardia è stata provata in tre modi, non uno:

* sui workflow veri: verde, «nessuna immagine fluttuante, nessuna deprecata,
  tutte le azioni su Node 24»;
* **invertendo una riga** (`ubuntu-24.04` → `ubuntu-22.04` nella matrice di
  `bundle`): rossa, con la scadenza del 17 aprile 2027 e i brownout nel
  messaggio. Senza questo passaggio, un controllo che passa non prova niente;
* **rompendo il confine Node 24** (`download-artifact` da v7 a v6): due test
  cadono. I test della guardia non sono vacui.

32 test nuovi in `scripts/check-ci-runners.test.mjs` (127 frontend in totale,
erano 104).

**Il limite, che è grosso.** Passare `bundle` da 22.04 a 24.04 **cambia il
glibc minimo richiesto dal `.deb`**, quindi le soglie di compatibilità e le
dimensioni misurate fino a ieri non valgono più. È scritto nel commento della
matrice, ed è la ragione per cui va rimisurato e non dato per buono. Non ho
eseguito nessuna build su 24.04: ho verificato che i pacchetti ci sono e che
`ubuntu-24.04` è un'etichetta valida, non che `tauri build` produca lo stesso
binario. Questo lo dice la prossima run.

## 2026-10-14 — M1 (video singolo: ispezione, download, runtime JS, cancellazione)

### D56 — `minVersion` di yt-dlp: **confermato** `2025.11.12` (chiude il TODO di D42)

D42 lasciava un `TODO(verify)`: "verificare con `--help` del binario fissato quale
sia la release minima reale". Il `--help` non lo dice — dice che il flag esiste
*oggi* — quindi la verifica è stata fatta sulla sorgente delle opzioni ai tag:

```
2025.10.14  yt_dlp/options.py → nessuna occorrenza di --js-runtimes
2025.10.22  yt_dlp/options.py → nessuna occorrenza
2025.11.12  yt_dlp/options.py → righe 460-478: --js-runtimes, --no-js-runtimes
```

Quindi `2025.11.12` è **esattamente** la prima release con i flag, non un
pavimento conservativo. Il valore nel manifest resta e il TODO si chiude. Nota
utile trovata nella stessa lettura: la help di quella opzione dice
`--no-js-runtimes needs to be passed before enabling other runtimes`, cioè
l'ordine di spec §5.3.1 è documentato da monte, non dedotto da noi.

### D57 — ffmpeg è un pre-flight, non una scoperta a download finito

`run_job` rifiuta di partire se ffmpeg non è risolto, con codice
`FFMPEG_MISSING`. Non è una preferenza: è il risultato di una misura. Con
`--ffmpeg-location /nonexistent`, yt-dlp **esce 0** e lascia due file separati
(`x.f278.webm` + `x.f251.webm`): "due file splittati" non è un risultato, e
distinguerlo a posteriori richiederebbe di capire dallo stderr un esito che il
codice di uscita dichiara riuscito.

Il costo: su un sito il cui selettore sceglierebbe un singolo stream muxed, il
job viene comunque rifiutato. È accettato per M1 (YouTube-centric) e va rivisto
in M3, quando i siti generici arrivano davvero — con la stessa nota nel codice.

### D58 — Anche lo stderr ha una callback (misurato, non simmetrico per gusto)

Misurato sul binario fissato: i template `download:` finiscono su **stdout**,
quelli `postprocess:` su **stderr**. "Merging…" e la conversione audio si
possono seguire solo leggendo stderr, quindi `process.rs` ha ora `LineSinks`
con due callback e il ring buffer che continua a riempirsi in ogni caso (il
classificatore di §8.5 legge quello). Un test d'integrazione lo pinna.

### D59 — Deno nel manifest: pinnato 2.9.7, checksum verificati

D34 diceva "deno non è nel manifest, per ora: M1 è dove si pinna". Ora c'è, con
`bundled: false` (spec §0.7: solo yt-dlp e qjs nell'installer), `versioning:
pinned`, sei asset. I checksum vengono dai file `<asset>.sha256sum` pubblicati
dalla release; quello per `x86_64-unknown-linux-gnu` è stato **verificato byte
per byte** scaricando lo zip (41,6 MB) il 2026-10-14 e confrontando lo sha256.
Gli altri cinque non sono stati scaricati: sono i valori pubblicati, e questo è
scritto nella `note` del manifest, non solo qui.

Deno **non** entra in `wizardToolNames` come problema: `toolState` lo tratta
come opzionale, altrimenti il pallino rosso del footer si accende su ogni
installazione nuova per uno strumento che serve solo come ripiego.

### D60 — `--playlist-end` non esiste più: fuori dall'M1, nota per M3

Spec §6.2 e §6.4 nominano `--playlist-end 1000`. Sul binario fissato
(2026.08.19) il flag **non c'è più**: l'equivalente è `-I :1000`
(`--playlist-items`, con `[START]:[STOP][:STEP]`). M1 non enumera playlist,
quindi nessun codice lo usa; M3 deve usare `-I`. Verificato con `--help` e con
la tabella delle opzioni, non con la memoria.

### D61 — La memoria QuickJS→Deno vive in `runtime-memory.json`, ed è best effort

`<app data>/runtime-memory.json`, scritto atomico, letto con tolleranza: un file
corrotto vale come "nessuna memoria" (costa un tentativo lento, non un errore).
La chiave è `(yt-dlp version, qjs version)` con TTL 24 h; un orologio che torna
indietro non rende eterna una decisione vecchia (`now >= decided_at` è parte del
test). Un aggiornamento di yt-dlp la invalida da solo, che è la ragione per cui
la chiave esiste.

### D62 — Eventi temporanei `job:progress` / `job:done`

Spec §3 elenca `queue:update`; la coda è M2. Fino a lì il job singolo emette
`job:progress` (payload di spec §3 più `jobId`) e `job:done` (esito terminale,
una volta sola). M2 li sostituirà: il payload è già quello giusto perché lo
spostamento sia una rinomina, non una riscrittura.

### D63 — Il video di prova del self-test: `jNQXAC9IVRw`

"Me at the zoo" (2005, il primo video di YouTube). Il candidato ovvio — 
`BaW_jenozKc`, il video di prova storico di youtube-dl — **oggi risponde "This
video is unavailable"**: verificato il 2026-10-14 con il binario fissato. Un id
verificato non resta verificato per sempre, quindi il self-test riporta runtime,
secondi e numero di formati invece di fidarsi di un nome.

### D64 — Timeout di ispezione: 60 s, 120 s se l'URL sembra una playlist

Euristica pura su `list=`, `/playlist`, `/videos`, `/shorts`, `/@`, …: serve a
scegliere il timeout *prima* dell'ispezione che lo direbbe. Il costo di una
risposta sbagliata è solo "quale dei due timeout si applica".

### D65 — `playlistKind`, non `kind`: una chiave doppia trovata dal compilatore

`Inspected` è un enum con tag `kind`; `PlaylistStub` aveva un campo `kind`
proprio. Serde lo avrebbe serializzato **due volte** con la stessa chiave — JSON
che il nostro scrittore accetta e ogni lettore risolve diversamente. Non l'ha
trovato un test Rust: l'ha trovato TypeScript dall'altra parte del confine
(`"error" | "playlist" | "video"` non assegnabile). Il campo ora è
`playlistKind`, e un test Rust verifica che la chiave `"kind"` compaia una volta
sola nel JSON serializzato.

### D66 — `-U` sul vero yt-dlp: provato, e il cambio di canale non è simmetrico

Il follow-up di M0-C chiedeva di provare l'aggiornatore sul binario vero, che
riscrive il proprio file mentre è in esecuzione. Fatto su una copia in `/tmp`
(come D43 vuole: mai su un tool non nostro):

* `--update-to nightly` da 2026.08.19: **aggiornato** a
  `nightly@2026.10.10.233141`, il file cambia dimensione, il binario riparte.
* `--update-to stable` da nightly: **no-op con uscita 0**, "yt-dlp is up to
  date (nightly@…)": yt-dlp non considera un downgrade di data un aggiornamento.

Il secondo caso è esattamente ciò per cui `classify_update` di D44 confronta le
versioni prima del testo: uscita 0 e nessun cambiamento → `updated: false`,
messaggio dell'updater. Non serve altro codice.

### D67 — Gli extractor args di YouTube riducono i formati; restano perché la spec li vuole

Misurato il 2026-10-14 sul binario fissato, video 4K: senza `--extractor-args`
49 formati, con `youtube:player_client=web_embedded,web,tv` (il valore
dell'originale, spec §5.3.7) **31**. La scala delle risoluzioni è identica
(2160…144). La spec chiede di tenerli, "user-overridable in an expert field":
restano, in una sola costante (`YOUTUBE_PLAYER_CLIENTS`), con la misura scritta
accanto, così la prossima decisione è una riga. Il campo esperto arriva con
§11.8.

### D68 — Il test vero è `#[ignore]`, e si lancia a mano

`tests/real_selftest.rs` usa il vero yt-dlp e il vero qjs, con rete: ispezione,
download a 144p con merge, audio-only in `.opus`, self-test. È ignorato di
default perché AGENTS.md §6 vieta test che dipendono da YouTube vivo nella suite
normale; il comando per eseguirlo sta in testa al file. La suite normale lo
conta come "ignored" — due test — e non come "passed", che è il punto.

### D69 — `browserNamesForTriple`: Safari solo dove esiste

Il dropdown dei cookie si costruisce dal **triple del backend**, non dallo user
agent (stessa fonte della normalizzazione Rust, che scarta un browser che su
questo OS non esiste). Un triplo vuoto — `npm run dev` nel browser — offre tutto
tranne Safari: indovinare "mac" da una UA è la classe di rilevamento inaffidabile
che questa separazione evita.

### D70 — Il self-test in CI esiste, ma non gira da solo

Spec §5.3.6 dice di usarlo in CI sulle tre piattaforme, §13.5 lo mette in un
"nightly job". `selftest.yml` lo fa, ma **solo su `workflow_dispatch`**: un run
tocca YouTube e ffmpeg di sistema, sui tre sistemi costa ~45 minuti di quota
(il repository è privato e i minuti sono limitati), e può fallire per cause che
non sono nostre. La differenza fra "un pulsante che risponde la verità quando lo
premi" e "una notifica notturna che si impara a ignorare" è tutta qui. Renderlo
notturno è una decisione del proprietario: il file è pronto, la pianificazione
no.

I test veri sono anche `#[ignore]` in locale (D68): la suite normale non tocca
mai la rete, e la CI a ogni push non paga niente per questo workflow.

### D71 — "1440p scelto, 360p scaricato": rilevato con `SDP_FORMAT`, e non più silenzioso

Segnalato dal proprietario: su un video aveva scelto 1440p e il file era 360p;
riprovando due volte è venuto 1440p come previsto. Non era casuale, ed è la
conseguenza di due cose che si sommano:

1. il selettore di spec §8.2 finisce con un ripiego muxed:
   `bv[height=N]+ba/bv[height<=N]+ba/b[height<=N]/b`. Se in *quella* esecuzione
   l'estrazione è ridotta (flussi separati assenti — il sintomo di
   §5.3.2), `bv…` non trova nulla e vince `b[height<=N]`, che su YouTube è il
   muxed **360p**;
2. yt-dlp in quel caso **esce 0**: nessun errore, nessuna riga da classificare,
   quindi né il fallback né l'avviso si attivavano. Il risultato era "riuscito"
   e sbagliato.

La correzione usa una stampa in più, misurata sul binario fissato:

```
--print "before_dl:SDP_FORMAT|%(format_id)s|%(height)s|%(vcodec)s|%(acodec)s"
SDP_FORMAT|278+251|144|vp9|opus     ← un video con merge
SDP_FORMAT|251|NA|none|opus         ← audio only
```

Una riga per job con il formato **davvero scelto**, e da lì due decisioni:

* **annullamento anticipato** (aggiunto in D74): alla riga `SDP_FORMAT` il job
  sa cosa sta per scaricare. Se è un muxed più basso del richiesto, l'errore è
  già commesso e il download viene **ucciso lì**, prima che il file sbagliato
  venga finalizzato: si eliminano i parziali e si ritenta una volta (Deno se
  c'è, altrimenti lo stesso runtime, perché la challenge fallita è transitoria).
  Se anche il secondo tentativo è degradato, il job **fallisce** con
  `YT_CHALLENGE` e un messaggio chiaro: meglio nessun file che un 360p al posto
  di un 1440p.
* **merge più basso = esito legittimo**: `137+140` a 720p quando ne erano
  chiesti 1080p significa che i flussi separati c'erano e il video non ha di
  più. Nessun ritentativo; il risultato porta `requestedHeight`/`actualHeight` e
  la UI avvisa.

**Non basta "più bassa del richiesto".** Un'altezza inferiore può essere
legittima: il video non ha quella risoluzione, e ritentare scaricherebbe di
nuovo la stessa cosa. Il discriminante è il formato scelto:

* `137+140` (con `+`) = un **merge**: i flussi separati c'erano, quindi
  l'altezza più bassa è una proprietà del video. Nessun ritentativo; nel
  payload restano i due numeri e la UI avvisa.
* `18` (senza `+`) = un **muxed**, cioè il ripiego `b[height<=N]`: è il sintomo
  dell'estrazione ridotta. Rimosso il file sbagliato, un ritentativo con Deno.

Senza riga `SDP_FORMAT` si sceglie la strada cauta (ritentare). La regola non
guarda l'audio only (`height = NA`) né `Best` (nessuna altezza richiesta).

### D72 — Il footer di stato dice la verità, e non ripete la pagina

`FooterStatus.svelte` era il segnaposto di M0: stringhe fisse, mai collegate.
Andava bene finché non c'era niente da contare, ma durante un download
affermava "Nessun processo in esecuzione" mentre la pagina mostrava "Video 1/2 ·
73 Mbit/s": non un segnaposto, una bugia. Segnalato dal proprietario.

Ora legge il job: stadio, flusso, percentuale; a fine job il nome del file, in
errore il messaggio, e la voce degli strumenti con problemi porta a Strumenti
(anche `footer.toolsPending` era definito e mai usato). I contatori
`N in esecuzione · M in coda` di §7.5 restano a M2.

**Velocità ed ETA non sono più nel footer** (seconda segnalazione): erano gli
stessi due numeri già sotto il nome del video, e ripeterli non aggiungeva
niente. Il footer è un riassunto a colpo d'occhio, il dettaglio sta dove c'è la
barra.

### D73 — L'ispezione ridotta si riconosce dal warning di yt-dlp, e si ritenta

Seconda segnalazione del proprietario, e più precisa della prima: **a volte,
incollando il link, la lista delle risoluzioni contiene solo 360p**; pulendo e
reincollando compaiono tutte. Il download non c'entra — è l'ispezione.

**La causa, e la prova che era già scritta.** Quando la challenge JS non viene
risolta, yt-dlp continua con quello che il player response ancora offre: i
formati muxed, e su YouTube l'unico muxed è il **360p**. Non lo nasconde: lo
dice, e il testo è nel suo sorgente al tag fissato
(`yt_dlp/extractor/youtube/_video.py`):

```
WARNING: [youtube] n challenge solving failed: Some formats may be missing. …
WARNING: [youtube] Signature solving failed: Some formats may be missing. …
```

Sono su **stderr**; il JSON è su **stdout**. Peccato che il comando di
ispezione passasse `--no-warnings`, cioè **cancellava l'unica prova**. Da qui la
deviazione, deliberata, da spec §6.2: l'ispezione **non** passa più
`--no-warnings`. Il download continua a passarlo (lì il segnale è `SDP_FORMAT`,
D71), e il parsing non cambia perché i warning non toccano stdout.

**Cosa fa l'app adesso**, in quest'ordine:
1. un tentativo con il runtime scelto;
2. se il risultato è *degradato* — il warning qui sopra, o un video senza
   nessuna altezza utile (la condizione che spec §5.3.2 già nomina), **un solo**
   tentativo in più: con Deno se il ripiego è disponibile (e allora si ricorda
   per 24 h), altrimenti **con lo stesso runtime**, perché una challenge fallita
   è transitoria — è esattamente ciò che l'utente ha risolto ricopiando il link;
3. si restituisce il **migliore** dei risultati visti (più altezze, poi la più
   alta: `choose_next`, funzione pura con test); se è ancora degradato, il
   risultato lo dice (`video.degraded = true`).

**La lista parziale non si mette in cache.** Il frontend non memorizza un
risultato degradato, quindi reincollare lo stesso link richiede davvero di
nuovo, e accanto alla lista compare un avviso con **Riprova** (che salta il
debounce e la cache). Prima, "Pulisci + reincolla" funzionava per caso: la cache
veniva svuotata dal pulsante.

*Nessuna delle prove è contro un caso reale, ed è giusto dirlo:* sulla macchina
di sviluppo la degradazione **non** è riproducibile (6 run con e 6 senza la
lista di client forzata: sempre 8 altezze, zero warning). Per questo la regola
non tenta di *prevenire* la challenge — non ho prove su come farlo — ma di
**accorgersene e riprovare**, che è ciò che mancava. Il segnale è quello che
yt-dlp stampa; non un'euristica sulle altezze.

**Il limite.** Se anche il secondo tentativo è degradato, l'app mostra la lista
corta **con l'avviso** e il pulsante Riprova: non può inventare risoluzioni che
in quella run non esistono. Un video che davvero ha solo il muxed 360p non
produce il warning, quindi non viene ritentato inutilmente.

### D74 — Il bug ereditato dall'originale: la lista di client YouTube forzata

Terza segnalazione del proprietario, che chiedeva di cercare il bug nel codice
ripreso dall'app Python. Era lì, ed è un bug di **obsolescenza**.

**Il sintomo.** A intermittenza l'estrazione restituisce solo il muxed 360p:
prima in ispezione (lista corta), poi anche in download (file a 360p chiesto
1440p). Due run su quattro, sulla stessa macchina, sullo stesso video.

**La causa, in tre passi, tutti verificabili nel sorgente di yt-dlp al tag
fissato (2026.08.19).**

1. Noi passavamo `--extractor-args youtube:player_client=web_embedded,web,tv`,
   il valore dell'app originale (spec §5.3.7). Ma i client **predefiniti** di
   questa versione per chi non è loggato e ha un runtime JS sono
   `_DEFAULT_CLIENTS = ('visionos', 'web')` (`_video.py`).
2. `_base.py` dichiara, per `web` (e `web_safari`, `mweb`):
   ```python
   'GVS_PO_TOKEN_POLICY': {
       StreamingProtocol.HTTPS: GvsPoTokenPolicy(required=True, recommended=True, …),
   ```
   mentre `visionos` non ha nessuna policy PO token e ha
   `REQUIRE_JS_PLAYER: False`.
3. Quindi forzare `web_embedded,web,tv` **butta via l'unico client che non
   richiede un PO token**, e si affida a client che lo richiedono: quando
   yt-dlp non riesce a coniarlo (o la challenge JS fallisce per quel client),
   restano solo i formati muxed — su YouTube, il 360p. L'intermittenza è cioè la
   *varianza dell'enforcement* di YouTube, non un caso. `web_embedded`, per
   giunta, yt-dlp lo aggiunge **da sé** quando il video è age-gated
   (`_video.py`, riga ~3153): forzarlo era ridondante.

**La correzione.** Di default non si passa più nessun extractor arg: yt-dlp usa
la sua lista, quella che i manutentori tengono aggiornata. La costante resta e
alimenta un campo esperto in Impostazioni → Rete e account ("Client player di
YouTube"), vuoto = automatico, validato su entrambi i lati (solo nomi, varianti
con punto, virgole, esclusioni con `-`: il valore finisce in un elemento argv).

**Le misure che sostengono la decisione**, non solo il ragionamento:

| | lista forzata | default |
| :-- | --: | --: |
| formati (video 4K) | 31 | **49** |
| self-test "Me at the zoo" | 7,4 s, 15 formati | **1,95 s, 24 formati** |
| warning di challenge in 6 run | 0 | 0 |

Nelle mie prove la degradazione non si era mai riprodotta (per questo non l'avevo
toccata in D73); il caso del proprietario sì, e la spiegazione dei PO token è la
sola che rende conto della *stanza* del sintomo (solo muxed) e della sua
intermittenza. Il cambiamento è una deviazione da spec §5.3.7, che chiedeva di
passare la lista dell'originale: la spec invita a verificare ciò che invecchia,
e questa lista era invecchiata.


### D75 — La risoluzione dei tool si ricorda per la sessione

**Il problema, misurato.** Ogni ispezione e ogni download chiamavano
`resolve_job_tools`, che *esegue* `--version` su yt-dlp, qjs, Deno e ffmpeg per
verificare che i candidati rispondano (spec §5.2). Il binario yt-dlp ufficiale è
un pacchetto PyInstaller one-file che **si spacchetta a ogni avvio**: misurato
qui, `yt-dlp --version` costa **0,46 s** (qjs ~0 s, ffmpeg 0,036 s). Erano
~0,5 s di attesa aggiunti a **ogni** incolla di link, e di più su Windows, dove
all'unpacking si somma l'antivirus.

**La soluzione.** `AppState` tiene una mappa `(tool, percorso di override) →
ResolvedTool` per la sessione. Si memorizzano **solo i successi**: un tool
mancante è economico da ricontrollare (esistenza dei file, non esecuzione) e
mettere in cache la sua assenza sopravviverebbe all'installazione che la risolve.
La chiave contiene l'override, quindi "Usa il mio percorso" è una mancata
corrispondenza automatica; i comandi che possono cambiare un binario
(`tools_set_path`, `tools_update_ytdlp`, `tools_install`) azzerano la cache, e
`tools_status`/`tools_test` la azzerano perché **sono** la superficie di
verifica: quello che mostrano è la verità del momento, non deve invecchiare per
colpa di una scorciatoia.

**Verificato da un test, non da un cronometro.** `m1_ipc.rs` risolve i tool
finti, poi **cancella i binari dal disco**: la seconda risoluzione deve
comunque rispondere (solo la cache può), e dopo `clear_tool_cache()` deve
fallire. Un test di velocità sarebbe stato instabile; questo dimostra la
proprietà che conta.

### D76 — Il debounce resta 500 ms per la digitazione, sparisce per l'incolla

Valutato, non cambiato di default: **500 ms mentre si digita** restano. Servono
esattamente a non lanciare un processo yt-dlp (0,46 s) e una richiesta di rete a
ogni tasto, sono quello che fa l'originale e quello che spec §6.2 e §2.9
prescrivono. Toglierli sarebbe un peggioramento mascherato da reattività.

**Un incolla però è un evento unico**, non digitazione: dopo il `paste` non
arriva altro, e i 500 ms sono latenza pura. Ora l'input ascolta `paste` e in quel
caso parte subito (`inspectDelay(true) === 0`). È una deviazione piccola e
deliberata dalla parità con l'originale, che debounceva anche l'incolla perché
aveva un solo percorso di input; qui l'incolla è l'interazione dominante
(spec §2.3), e con la cache di D75 il tempo che resta è tutto estrazione.

### D77 — The Italian text still to translate, and how to do it (recorded, not done)

The language rule at the top of `AGENTS.md` (2026-10-14) makes English mandatory
for everything published, and says the historical Italian part is translated in
one deliberate, separate task. **The owner asked for the inventory to be
recorded and left for later**, so this is the list; nothing here has been
translated.

**Inventory** (counts are matching lines, found 2026-10-14 with a word-list
grep over `git ls-files`):

| file | what is Italian inside | lines |
| :-- | :-- | --: |
| `docs/DECISIONS.md` | the whole file, D1–D76 | 664 |
| `docs/PROGRESS.md` | the whole file, every step block | 484 |
| `.github/workflows/ci.yml` | header comments, step names, comments inside `run:` blocks | 123 |
| `docs/SPIKES.md` | the whole file (human checklists) | 50 |
| `scripts/check-ci-runners.mjs` | comments and console messages | 46 |
| `scripts/check-ci-runners.test.mjs` | test names (`describe`/`it`) and assertions on those messages | 32 |
| `scripts/report-bundle-sizes.mjs` | comments and console messages | 24 |
| `.github/workflows/tier2.yml` | comments and step names | 15 |
| `scripts/run.mjs` | comments and console messages | 12 |
| `scripts/check-ci-syntax.mjs` | comments and console messages | 12 |
| `.gitignore` | comments only | 10 |
| `src-tauri/windows-app-manifest.xml` | comments only | 9 |
| `scripts/fetch_bundled_tools.mjs` | comments and console messages | 7 |
| `scripts/check-release-version.mjs` | comments and console messages | 5 |
| `scripts/ci-steps.test.mjs` | assertions on `check-ci-syntax.mjs` messages | 2 |

**Traps, i.e. why this cannot be a search-and-replace:**

* **Two pairs must change together.** `scripts/check-ci-runners.test.mjs`
  asserts the Italian messages printed by `scripts/check-ci-runners.mjs`, and
  `scripts/ci-steps.test.mjs` asserts those of `scripts/check-ci-syntax.mjs`
  (e.g. `expect(output).toContain("doveva essere solo arm64")`). Translating one
  side alone turns a green suite red.
* **The two documents are read by every session.** They are append-only logs of
  what happened; the translation must not change a single fact, number or
  conclusion while changing the language.
* **`docs/PROGRESS.md` and `docs/DECISIONS.md` are currently bilingual**: the
  rule took effect on 2026-10-14, so entries after D77 are English and the
  earlier ones Italian until this task runs.

**Correctly Italian and never to be translated** (excluded on purpose):

* `src/lib/i18n/it.json` — product strings, the rule itself lists them as never
  translated;
* `docs/RUST_PORT_GUIDELINES.md` — read-only specification, in English already;
* `src-tauri/src/services/paths.rs:286` — a test fixture string (`"Perché no"`)
  that exists to prove accented text survives sanitising;
* binaries (`src-tauri/icons/icon.icns` and friends) — grep matches bytes.

**The six commit messages already pushed** (`3a01559`…`29ddd09`, subjects in
Italian) stay as they are. Rewriting them means a force-push on a branch that is
already published, and the rule is about what we publish from now on: a
rewritten history is a larger problem than a few Italian subjects. New commits
are English (`c24f0c6`, `9e0be32`).

**How to do the pass when it is done** (the rule's own conditions, made
concrete):

1. one commit per group — `.github/` + `scripts/` first (small, and they are the
   files agents edit most), then `docs/SPIKES.md`, then the two logs;
2. prose only: no code, no behaviour, no number, no conclusion changed;
3. run `npm run check:ci`, `npm run check:runners`, `npm test`, `cargo test`
   after each group — those are exactly the checks that catch trap #1;
4. say in `PROGRESS.md` that the pass happened, with the commit range.

### D78 — `docs/PROJECT_REPORT.md` is listed by AGENTS.md but does not exist

Found while building the D77 inventory: `AGENTS.md` §1 lists
`docs/PROJECT_REPORT.md` as a read-only source of truth, but the file is neither
in the repository nor on disk (`git ls-files docs/` lists only `DECISIONS.md`,
`PROGRESS.md`, `RUST_PORT_GUIDELINES.md`, `SPIKES.md`).

Two possible explanations, and I cannot choose: it was never added, or it is kept
out of the repository on purpose. What is certain is that a pointer to a missing
file costs every session that follows it, so the table row now says it is not
part of this repository, and this decision records why. If the file exists
somewhere, adding it back is a one-line change to that row.

---

## M2 (English from here on, per the language rule)

### D79 — The queue notification is a plugin, and here is what it costs

Spec §7.7 calls the "system notification when the queue becomes idle"
**optional**, and spec §14 row M2 lists "notifications" among the phase's
deliverables. Both are true, so the feature exists — behind a setting that is
**off by default**, because a notification that always fires is one nobody reads.

`tauri-plugin-notification` 2.4 is therefore a new dependency, and AGENTS.md
asks for the reason and the binary-size impact. Measured on this machine, same
release profile, back to back:

| build | app binary |
| :-- | --: |
| without the plugin | **6,71 MiB** |
| with the plugin | **6,88 MiB** |

**+0,17 MiB**, against a budget of 15 MB (spec §1) and a binary that grew from
4,70 MiB in M0-C to 6,88 MiB now. The gap between those two numbers is the queue
engine, not the plugin: the whole M2 backend, the persisted model and the
scheduler, for 2 MiB.

Two things about the dependency that are worth knowing:

* **Pinned to 2.4 on purpose.** `cargo add` picks the newest release whose
  `rust-version` fits this package's `rust-version = 1.85`, which is 2.4.0; the
  2.7 line needs 1.90. A dependency the declared toolchain cannot build is not a
  dependency.
* **It pulls a second copy of the `windows` crate** (0.61, through
  `tauri-winrt-notification`, next to the 0.62 that spec §4.2 needs for Job
  Objects). They are independent crates and coexist; the cost is inside the
  +0,17 MiB above.

The wording of the notification comes from the **same dictionaries the UI uses**,
embedded with `include_str!`. Two alternatives were rejected: building the
sentence in the backend would create a second place where user-visible strings
live (spec §11.9 forbids it), and letting the webview send it would miss the
moment the queue goes idle while the window is in the background — which is
exactly when a notification is worth having.

### D80 — `queue.json` has no separate `order` array

Spec §7.4 lists what the file holds: "schema version, items, groups, **order**".
The document has the first three; the order is **the order of the `items`
vector**.

A second array is a second thing that can disagree with the first, and nothing
needs it: reordering is moving elements inside the vector, and the frontend
receives the order as a list of ids in the `queue:update` event (only when it
changed — re-sending 5 000 ids ten times a second would cost more than the items
themselves). Two sources of truth for one fact is the kind of bug that only
shows up on someone else's machine.

### D81 — `queue:update` carries whole items, not field-level patches

Spec §7.6 asks for two things: "coalesce progress per item and emit at most
~10 updates/s" (implemented, in `services/queue/events.rs`) and "send only changed
fields".

The second is implemented at **item granularity**: a changed item is sent whole,
an unchanged one is not sent at all. Field-level patches would need
`Option<Option<T>>` on every field of the payload and a merge step in the
frontend store, and the win — not sending three unchanged strings — is smaller
than the risk that the store drifts from the backend after a partial patch that
nobody tests. A queue item is a few hundred bytes; the coalescing that actually
matters is the one that decides **whether** it is sent.

The frontend patches single rows either way (a keyed map plus an explicit order,
`$lib/queue.ts`), which is what spec §11.7 asks for.

### D82 — The lanes are counters in the scheduler, not semaphores

Spec §7.2 says "Resource-aware scheduling with separate semaphores". This engine
keeps the equivalent as **counters inside the scheduler task**, because the
scheduler is the only thing that ever starts an item: a permit could not be
acquired anywhere else, so a semaphore would only duplicate a count that already
exists.

The argument that decided it is the mid-flight settings change. With a
semaphore, "2 → 1 parallel downloads" while two items are running has to either
revoke a live permit (cannot) or be reconciled against the items that hold one
(a second source of truth). With counters, the next planning pass simply reads
`capacity - busy` and the truth is always the items themselves.

The observable behaviour spec §15 item 7 asks for — parallel downloads,
per-host cap — is unchanged and is tested as a **pure function**
(`services/queue/scheduler.rs`): lanes, the per-host cap of §6.4, retry timers
and "first runnable first" are all unit tests with no process and no clock.

### D83 — The Add bar inspects; the engine does not

Spec §3's pipeline starts `inspect → download → …`, and spec §2.2 says a pasted
link is inspected **as it is added**, with the row showing a spinner and then the
title and the qualities. So in M2 the inspection happens in the Add bar
(`inspect_urls`, three in parallel, each with its own token and answers in
order), and the item stores the result in its **options snapshot** together with
the title, the site and the duration.

The engine therefore starts at the download stage. The state machine still
declares `Inspecting` (and `Encoding`, `Transcribing`, `Summarizing`, which are
M4/M5/M6): the persisted documents, the wire values and the i18n keys are already
the ones those phases need, and an item whose state is `interrupted` in
`paused_from: inspecting` can be resumed correctly when M3 adds inspection
inside the queue.

The reason is not laziness but cost: a yt-dlp process costs ~0,46 s to start
(measured, D75) plus a network round trip. Re-inspecting at enqueue time would
pay it twice for every link, to learn what the Add bar already knows.

### D84 — A playlist link is refused, not silently added

Spec §6.4 (playlists) is M3. In M2 the Add bar inspects a pasted link and, when
the answer is a **playlist**, the row says so (`queue.add.playlist.unsupported`)
instead of adding it as if it were a video. yt-dlp with `--no-playlist` on a
playlist URL returns the playlist's own metadata, and the app would then
"download" something the user did not ask for — the behaviour spec §12.12
records as a bug in the original.

### D85 — The host key strips `www.` and nothing else

The per-host cap of spec §6.4 needs one key per site. The rule is
**lowercase, minus a leading `www.`**: `www.youtube.com` and `youtube.com` share
a budget, `m.youtube.com` does not (it is a different host string, and pretending
otherwise would be a guess about what "the same site" means).

The alternative — folding `m.`, `mobile.` and friends — was considered and
rejected: it is a per-site mapping that would be wrong somewhere, and a cap that
is slightly too generous is much less harmful than one that blocks a download the
user asked for. The rule is a pure function with a test
(`queue::model::host_key`).

### D86 — The paused item goes back to `Queued`, not to the stage it was in

A first version resumed an interrupted or paused item into the stage recorded in
`paused_from` (e.g. `downloading` → `downloading`). It looked right and was
wrong: **only a `Queued` item is eligible for the scheduler**, so a resumed item
in any other state sat there forever, "downloading" with nothing running. Caught by
`pause_keeps_the_item_and_resume_finishes_it`, which is exactly the test that
should exist.

`paused_from` is kept — it is what the lane is taken from, and what the UI shows
("resumed after a restart") — but the state a resume produces is `Queued`.

### D87 — Interrupted items wait for the Resume button (and why it was wrong first)

Spec §7.4: "On startup items in an active stage become **Interrupted** and are
shown with a 'Resume' button (or auto-resume if the setting is on)".

The first version treated `Interrupted` as **eligible** for the scheduler, on
the reading that the two alternatives are "Resume button" and "auto-resume
setting" and that a queue which stops dead after a restart is worse. It is the
wrong reading: the spec's default is the button, and the setting is the
alternative, so without the setting the item must not move.

It was caught by a flaky test rather than by reading — `resume_all()` came back
empty, because the scheduler had already started the item — which is exactly the
kind of bug a test that asserts the *outcome* misses and one that asserts the
*decision* finds. The planner now takes only `Queued` items
(`scheduler::is_eligible`), and `Interrupted` and `Paused` wait for
`queue_resume` / `queue_resume_all`.

Auto-resume on start is **not** implemented: it needs the setting of spec §7.4,
and one setting that only exists to switch off the default behaviour is not
worth its place until somebody asks for it. Noted in `PROGRESS.md`.

### D88 — The engine's tasks are spawned on Tauri's runtime, found by running the app

`QueueEngine::start()` is called from `setup`, on the **main thread**, where no
Tokio context is entered. A bare `tokio::spawn` there panics at once:

```
thread 'main' panicked at src/services/queue/engine.rs:271:9:
there is no reactor running, must be called from the context of a Tokio 1.x runtime
```

Found by actually starting the release binary (`npm run app:binary`) — no test
would have found it, because every test calls `start()` from inside a
`#[tokio::test]`, where a bare `tokio::spawn` works. The spawns now go through
`tauri::async_runtime::spawn`, which is the documented way and works from both
places; the tasks spawned *inside* a running task still use `tokio::spawn`.

Recorded because it is the second time in this project that "the tests pass"
and "the app starts" are different claims (the first was the Windows CI run that
found the `Path::is_absolute` difference, D20-ish). Starting the app is cheap
and is worth doing before declaring a phase done.

### D89 — A cancelled item's temp directory goes *before* the row says so

Spec §7.3: "Cancel: kill the tree, **delete the item's temp dir**, never touch
finished outputs." The first implementation deleted the directory a few lines
*after* setting `state = Cancelled`, which is a real (if narrow) hole: anything
that sees "Cancelled" and then looks for the directory can find it still there.

Two things depend on the two going together. The startup sweep of spec §4.10
lists the temp dirs the queue still **owns** and deletes what is left over, so a
directory that outlives its row is invisible to it. And the frontend's own
"the item is cancelled, so nothing is being written any more" reading is only
true if the writer has really stopped.

The order is now, on the path that matters — a **running** item that is
cancelled or removed: kill the tree → delete the temp dir (with the queue's
lock released, because deleting a folder is not something to hold the whole
queue still for) → record the terminal state. There is no time-of-check /
time-of-use window there: the item is already finished when `finish_item` runs,
so the scheduler will not start it again, and its temp path does not depend on
the state being written.

The other path — cancelling an item that is not running (nothing to kill) —
deletes the directory immediately after the mutation instead, again outside the
lock. It cannot be reordered without a window of its own: the scheduler could
start that item in between, and deleting the temp directory of a job that is
about to run would be worse than the one it closes.

Found by a **flaky** integration test — `assert!(!temp.exists())` right after the
poll saw `Cancelled` — which failed roughly one run in ten. The test was not
weakened: the code was made to keep the promise the test was making.

### D90 — Pausing an item that is not running parks it, it does not cancel it

`queue_pause` on a `Queued` item used to set it to `Cancelled`: the
"nothing is running" branch of `QueueEngine::signal` treated every signal that
was not `Remove` as a cancel. The same branch also hit a **finished** item —
`pause` on a row that had just completed turned `Done` into `Cancelled`, with
its progress wiped and its file pointer kept. The UI never offers Pause on
either (only on rows with a lane), and `pause_all` only touches items with a
lane, so it was unreachable — but the command is public IPC, and a stale view of
a row is exactly the case where the user clicks Pause on something that has
meanwhile finished or not started yet.

Cancelling is a lie the user then has to undo: a queued item shows "Cancelled"
(Retry) for something that was never downloaded, and its partial files — if any
— were deleted. Pausing now parks the item as `Paused` with `paused_from` set,
which is the state the spec already has for "stopped, waits for the user" and
which `queue_resume` returns to `Queued` (D86). An item that is already parked
is left exactly as it is.

### D91 — An item is written to `queue.json` as *running* **before** the tool is spawned

Found by a flaky integration test, one run in about ten:
`the_queue_survives_a_restart_and_the_interrupted_item_can_resume` came back
from its simulated restart with the item in state **`Queued`** — and, in another
run, `Downloading`. Both are the same bug seen at two moments.

Spec §7.4 says: "On startup items in an active stage become **Interrupted**
and are shown with a 'Resume' button". That recovery can only see what the file
says, and the file said `Queued`, because `QueueEngine::start_item` set the
state to `Downloading` in memory, marked the item dirty for the flusher and
spawned the task — and **never wrote the file**. The only writes were the ones
`after_mutation` and `finish_item` do. So for the whole life of a download the
document claimed the item had not started yet.

A clean exit hid it: `finish_item` persisted `Cancelled` at the end, and the
test reopened the file after the item had settled. A crash — `kill -9`, a power
cut, an OOM kill, the window being closed with the process killed — leaves the
file as it was, and the next launch **downloads the item again by itself**, with
no Resume, no question asked and no sign that anything had happened. The `.part`
files it starts from are still there, so it mostly resumes, but the user was
never asked and the row never said `Interrupted`.

The fix is one line and one place: `start_item` persists **after** taking the
state lock and **before** `tokio::spawn`. The invariant now holds by
construction:

> a `queue.json` that says `Queued` never has a tool behind it.

The only thing the new order can get wrong is dying between the write and the
spawn: the file says `Downloading` while nothing ran, so the item comes back
`Interrupted` and waits for a Resume it did not need. That is the cheap
direction — one unnecessary click instead of a silent re-download — and it is
the direction spec §7.4 chooses.

**Cost:** one extra write of the queue document per item, on top of the one at
the end. The §7.4 bound of "≤ 1 write/s while running" is about *progress*, and
progress still never reaches the file; a start is a state change, not a
progress tick, and it is the state change the crash case depends on.

Covered by two tests that assert it: a unit test that reads `queue.json` (not
the engine) while an item runs, and an assertion inside the restart integration
test that reads the file while the fake yt-dlp is running. The unit test was
checked against the old code and fails there, printing `Queued`.

### D92 — The temp file of an atomic write is per **write**, not per process

`paths::atomic_write` named its temp file `.<file>.tmp-<pid>`. Inside one
process that name is **shared by every concurrent write to the same
destination**, which makes "temp + rename" stop being atomic:

1. writer A creates the file (truncating it) and starts writing;
2. writer B creates the same file — a second file descriptor, its own offset —
   and writes over A's bytes;
3. A renames it over the destination: `queue.json` now holds whatever mixture
   the two writes left, possibly truncated mid-document;
4. B renames: `ENOENT`, and `settings.json`/`queue.json` silently kept the
   broken version.

Step 3 is the data loss: the next reader sees unparseable JSON, and both
`Settings::load` and the queue's `store::load` answer a corrupt document the way
the spec asks (§7.4) — move it to `.bak` and carry on with **defaults**. So the
user's whole queue, or their settings, is replaced by an empty document because
two threads wrote at the same moment.

The M2 queue is the first thing that writes concurrently by design: its
scheduler task, its flusher and each running item all persist, and the
`queue:update` events, the actions and the item transitions can land together.
The flaky test that led here printed it plainly in stderr —
`cannot write the queue file: No such file or directory (os error 2)`, which is
step 4, and the empty queue that followed is step 3.

The name is now `.<file>.tmp-<pid>-<counter>` with a process-wide `AtomicU64`,
so two writes can never meet. Cost: one static and one `fetch_add`. The test
that proves it runs eight threads × twelve rounds over ~40 KB documents and
fails immediately with the old name.

### D93 — `save_settings` moves the file and the in-memory copy under one lock

`settings_save` replaces the **whole** document, and two saves can overlap: the
settings page saves on every change and the app saves again on the way out. The
old order was "write the file, then take the lock and update the copy", so a
second save could land its file between the two steps — and the engine, which
reads the lanes and the notification switch from the in-memory copy, would then
be scheduling with settings that were never on disk.

Now the file write and the in-memory update happen under the same lock, file
first: a failed write still leaves the engine on the last saved settings.

### D94 — Every wait in `queue_engine.rs` is a poll with a deadline, never a fixed sleep

The soak (30 runs of `cargo test --test queue_engine`, three times) found five
separate flakes, all of the same shape: a test that looked at what a **task**
produces the instant the **state** said it should be there.

- the state is updated under the lock; the file write and the notifier call
  happen after it, with no lock held;
- the flusher is a task: "the last event has been sent" is a wait;
- a `shutdown()`-ed engine's running task still writes the file once the
  process is really gone — a real app cannot hit this, because exiting kills
  the tasks, but a test that returns early can.

Each is now a poll with a deadline that fails with the reason, which is what
`wait_for` has done for states since the first version. Two claims needed care
because they are claims about **absence**:

- "exactly one idle notification": wait for the first, then wait again, or the
  test proves nothing about the second through the third;
- "the file never said `queued` while running": wait for the file to leave
  `queued`, and assert `!= "queued"` rather than `== "downloading"`, because
  the fake's download is 400 ms and a loaded machine can be slow enough that the
  read lands after it finished. Every state except `queued` is one the item can
  only reach **by having started**, and that is the invariant. The exact value
  is pinned by a unit test whose tool lookup never returns.

Recorded because the tempting alternative — "the test was flaky, let me just add
a sleep" — is how a suite ends up with a 500 ms wait that hides a real
interleaving. Five of the six flakes in this phase were the tests, and the two
that were not (D91, D92) were the worst bugs of the phase.

### D95 — The same lock discipline applies to the runtime memory, not only to the settings

D93 was about `AppState::save_settings`; the same read-then-write pair exists in
`RuntimeMemoryStore::remember` (spec §5.3.3: which runtime to start with, per
pair of versions), so it is fixed the same way rather than left as the one place
where the pattern still disagrees with itself.

Two yt-dlp jobs can find the same degraded runtime at the same moment — two
downloads of a YouTube link is the normal case, not an edge case — and each
carries a whole document. The window between "the file is written" and "the
copy is updated" is short, but the consequence is that the file and the memory
disagree about which runtime to start with, and only one of them is read back
after a restart.

Both still write the file **first**: a failed write must leave the engine on the
last settings that were actually saved, not on ones that exist only in memory.

### D96 — Nothing starts after `shutdown()`, and the check is in `start_item`

Found by the soak, through the corruption test: it shuts an engine down, waits
for the item to settle, corrupts `queue.json` and expects the next launch to
start empty. It intermittently saw items instead.

The cause is a real window, not a test artefact. `scheduler_loop` checks
`stopped` at the top of each pass, but a pass already in flight when
`shutdown()` lands goes on to `start_item` for everything it planned. So an
item the scheduler had chosen **before** the app started tearing down could be
spawned **after** it, and then write the queue file — over the document the test
had just corrupted, and over whatever the next launch is about to read.

Two problems in one, both real:

- a yt-dlp process that the exit hook has already walked past, so spec §4.2's
  "on app exit cancel all running items" does not hold for it;
- a `queue.json` written during teardown, which is the file the next launch
  trusts.

`start_item` now returns immediately when `stopped` is set, before it takes the
state lock. The check belongs there rather than only in the loop because the
loop is exactly the caller that can be mid-plan; a caller that arrives *after*
the exit is not a scheduler decision any more. Covered by a unit test that calls
`start_item` directly after `shutdown()` and asserts nothing moved — which
fails on the old code, though not for the reason one would guess: without the
guard the test reaches `tokio::spawn` from a synchronous `#[test]` and panics
with "there is no reactor running". The panic *is* the bug, seen from a test
that has no runtime to spawn into.

### D97 — A `queue_list` that was overtaken by events never walks a row back

The frontend's `load()` (spec §3's initial read) replaces the whole queue with
the snapshot's contents. That is right when nothing else has happened, and
**wrong** in one real case: the read is in flight, a `queue:update` arrives
describing an item that has started downloading, and *then* the read lands
with a snapshot taken a moment earlier. The row goes back to `queued` and the
footer counts it again.

It is not a flicker. Once the queue goes idle the next event is the one that
would have corrected it, and an idle queue does not emit one: the row would say
"queued" for a download that is really running, until the user does something
else. The window is the time between `invoke("queue_list")` being sent and its
answer being reduced — tens of milliseconds on a loaded machine, and the
restored-queue case (an item that starts the instant the app opens) is exactly
when it is most likely to be hit.

The store now remembers the `seq` it had when the request went out. If the
answer comes back with events already applied (`seq` has moved on), the
snapshot is **not** applied wholesale: only the rows the UI is missing are
taken from it, and the newer summary and sequence are kept. `groups` are still
taken from the snapshot — they travel nowhere else.

The same class of problem, one layer down, decided the shape of the fix: the
reducer moved out of the store into `$lib/queue` as a pure function, because
the store's own comment claimed it was "testable without a Tauri event bus" and
it was not — a `$state` field needs the Svelte compiler, and the vitest config
deliberately does not use it. `applyUpdate` and `applySnapshot` are now nine
tests, including "drops an event that is not newer" and "never walks a row back
when events overtook the read".

The store keeps **per-field** reactivity on purpose: the state is one value,
but `#write` assigns only the fields the reducer changed, because replacing the
whole object would invalidate every reader and repaint the footer on every
item tick — the thing spec §11.7 asks the UI not to do.

### D98 — The queue extends the standard download; it does not replace it

The first hands-on feedback on the M2 build (verbatim): *"L'app si avvia ma non
capisco onestamente le modifiche effettuate, il download non parte e il tasto
non riesco ad avviarlo"*, *"Questa modalità coda non ha senso, ti fa scegliere
solo una risoluzione di picco e non è granulare"*, *"La modalità di download di
coda deve essere un'estensione del download standard, non deve sostituirla.
L'utilità principale è mettere in coda altri download e ha senso più che altro
per le playlist, salvo altri casi."*

Two defects were behind it, both in the Add bar, none in the engine:

1. **The button could not work.** `toRequests()` is built from the *rows* of the
   Add bar; rows were created only by `submit()`, which ran only on
   Ctrl/Cmd+Enter or a drop. Typing or pasting — what the original's URL entry
   did — left the rows empty, the button disabled (`readyCount === 0`), and the
   click a silent no-op. Spec §11.3 asks for inspection "debounced 500 ms as in
   the original" (`on_url_change` → `after(500, init_submit)`); it had simply
   never been implemented.
2. **The only action was "Aggiungi alla coda"**, and the only quality control
   visible without rows was the coarse preset: the standard download was gone
   from the app, and the queue's real utility (many links, playlists later) was
   unreachable.

What changed:

- Inspection starts on its own, debounced 500 ms, as the original; the pasted
  text becomes rows with spinner → title, site, duration.
- Two actions, and **both check whatever is still in the box and wait for it
  before adding**, so a click is never a no-op: **Scarica** (primary) and
  **Aggiungi alla coda** (secondary).
- **Scarica** puts the new items at the top of the queue — the "Start
  immediately" side of §11.3; **Aggiungi alla coda** leaves them at the end.
  With nothing ahead of them the two are the same by construction.
- Both go through `queue_add`. Reviving the M1 `download_start` path for the
  primary button would have been a second engine with no lanes, no per-host
  cap, no retry policy and no persistence — the point of M2 is one path.
- Quality: the preset is a **cap** (`MaxHeight`) over the six common heights,
  Best and Audio only; the **per-row** picker lists the resolutions that video
  actually reports as *exact* heights (the original's `1080p` list and its
  exact-with-fallback selector, spec §8.2). The separate "audio only" checkbox
  is gone — it was the missing `queue.add.audio` label (D99) and a second way
  to say what the preset already says.

Spec §11.2 (Queue as home) and §11.3 ("Add to queue") stay as written; what
changed is which verb is primary, on the product owner's explicit instruction.

### D99 — i18n keys are guarded by a test on both sides of the IPC

`t()` falls back to the key itself, deliberately: a missing label is neither a
crash nor a compiler error, it is raw text on screen. That is how
`queue.add.audio`, `settings.general.behaviour` and
`settings.general.notifyIdle` shipped. A fourth, `queue.add.playlist.unsupported`,
was found by machine, not by eye: the Add bar sets it as a row's `errorKey` and
it existed in no dictionary.

Three changes:

- **Heights are parameters.** `queue.item.quality.exact.{144…2160}` and
  `queue.item.quality.max.{144…2160}` (16 keys per language) became
  `queue.item.quality.exact` / `queue.item.quality.max` + `{height}`.
  `qualityLabel()` returns `{ key, params }` and `qualityKey(item)` is the same
  function now; a video reporting 576p or 4320p gets a label instead of a raw
  key, and no future height can reopen the hole.
- **Frontend guard**: `src/lib/i18n/usage.test.ts` scans every `.ts`/`.svelte`
  under `src/` (tests excluded — some of them ask for missing keys on purpose)
  for literal keys in `tt`/`ttCount`/`errorText`/`t`, for keys *assigned* to
  `key:`/`errorKey:`/… fields, for all 12 item states and 10 item actions, and
  for quality labels at heights no list covers.
- **Rust guard**: `error::message_key_in_dictionaries` (test-only, reads both
  dictionaries with `include_str!`), used by `error.rs` for every `ErrorCode`,
  by `services/ytdlp.rs` for every `YtdlpErrorCode`, and by
  `services/notify.rs` for **every `toast.` literal of the three files that
  build those keys** (`notify.rs`, `queue/engine.rs`, `queue/store.rs`) —
  scanned rather than listed, so a new toast fails the test.

Both guards were verified by breaking them on purpose (a key renamed in the
store, a toast renamed in the engine) and watching the test fail, then
restoring. Small wording fixes came with them: "Qualità predefinita", "Svuota
la lista", the list counter is the whole queue (`tCount`, not the rows in the
viewport), the notification checkbox moved to a new "Comportamento" section
(the heading it borrowed did not exist), and the stale first-run text ("La coda
arriva con M2") became a plain hint.

### D100 — Priority uses the commands that exist; no new flag on `queue_add`

"Scarica" needs the new items ahead of the queue, and there were two ways: a
`priority: bool` on `queue_add` (engine, model, IPC and its 12 tests), or the
frontend calling `queue_move_to_top` per new id. The loop won: it changes no
IPC surface, and it is **skipped entirely** when no *other* item is `queued` —
the common case of an idle queue — so the usual single download costs one
`queue_add` and nothing else. The calls go back to front, so a batch keeps the
order it was pasted in. A backend flag remains an option if a future UI ever
needs priority without the queue being non-empty.

### D101 — The restart test's download margin: 3 s → 8 s

`the_queue_survives_a_restart_and_the_interrupted_item_can_resume` reads
`queue.json` *while the fake tool runs* and asserts it says `downloading`, the
value a crash would leave behind (D91). One run out of the ~130 so far failed
with `done` on the right: the test task was starved past the fake's 3 000 ms
window, so the read landed after the tool had finished — a race in the *test*,
not in the engine, and the same class of thing the `veryslow-<ms>` URL was
introduced for. The margin is 8 000 ms now; the resume phase at the end waits
it out. Four further full runs of the binary and one full `cargo test` are
green, and the failures that would matter (a file still saying `queued`) fail
immediately rather than waiting.

### D102 — The queue page never paints over its own actions

Second hands-on report: *"Quando si aggiungono i video la sezione della coda
diventa più grande e si compenetra con le scritte e i pulsanti di sotto."*

The list viewport carried `min-height: 240px` and
`max-height: calc(100vh - 320px)`, inside a page pinned to `height: 100%` with
no overflow policy of its own. As long as everything fit, the flex column
distributed the space and the magic numbers did nothing. The moment it did not
fit — the Add bar grows by 260px of pasted rows, or the window is small — the
viewport's floor won: the list kept its 240px and the overflow painted *over*
the row below it (`Sospendi tutto`, `Riprova i falliti`), which stayed at its
flex position because nothing had moved it.

The fix is the shape the layout should have had:

- the page itself scrolls (`overflow: auto`) when its minimum does not fit, so
  nothing can be painted over;
- the list keeps a floor (104px) but the *viewport* is `flex: 1 1 auto;
  min-height: 0` with no `max-height`, so the list scrolls internally whenever
  there is room;
- the Add bar, the page actions and the status footer are `flex: none`: on a
  small window the page scrolls, and none of them is squeezed.

The magic `calc(100vh - 320px)` is gone: it encoded the add bar's height at a
moment in time and was wrong as soon as the bar changed.

### D103 — One status line per card, and a virtual list that measures its rows

Same report, second half: *"i valori di velocità e di tempo stimato … ci sono
dei momenti in cui spariscono (forse perché yt-dlp riporta 0 o lavora a pezzi)
e … il rettangolo della UI … continua a rimpicciolirsi e ingrandirsi … lo trovo
distrattivo."* yt-dlp does print `NA` between fragments, and the card rendered
speed and ETA only when they were there: every gap removed a line and moved
everything below it.

A card now has one shape for every state — head, bar, **one** status line,
actions — and nothing about it depends on whether a number arrived:

- the bar is always drawn: empty for `queued`, the last reported width for
  `paused`/`interrupted`/`failed`/`cancelled`, full for `done`, animated only
  while it runs and no percentage is known;
- the status line shows, in order of priority: the problem (the error message,
  or the short "Salvato a 360p invece di 1440p"), the progress
  (`Velocità: … Mbit/s · Tempo rimasto …`, and `In attesa dei dati…` while
  yt-dlp has not reported either), or the quality while the item waits. It is
  clamped to one line with a `title`, and the full error and the full warning
  moved into the details panel, where there is room for them;
- `speedMbits` keeps one decimal below 1 Mbit/s: rounding a slow download to `0`
  reads as "stalled" and it is the number the user is staring at.

The same report exposed a latent bug the fix had to close: the list's
virtualiser used a 92px constant, commented "the cards are fixed-height by
construction" — they are not. A card is ~130px and its height depends on the
window width, because the action buttons wrap. The pads that stand in for the
off-screen rows were computed from the wrong number, so the window scrolled to
the wrong place. The list now measures the real pitch between two rendered rows
(height + flex gap) and re-measures when the window or the rendered slice
changes; `visibleRange` keeps its pure signature and its tests.

The status footer also learned the difference between "all done" and "not
finished": with parked or failed items and nothing running it says `1 in pausa`
or `2 falliti` instead of `Fatto`, and it is `flex: none` so it cannot be
squeezed.

### D104 — New items keep FIFO, and the list shows you where they went

The user asked whether the newest links should go on top instead of at the end.
The answer is no, and the reason is the batch: with "newest first", a playlist
of forty (M3) would be starved by every single link added while it waits — the
classic LIFO starvation, on exactly the case the queue exists for. Order stays
the order items were added (spec §7.3: the scheduler picks the first runnable
item), and the two explicit escapes stay what they are: **Scarica** moves the
new items ahead of everything *waiting* (§11.3 "Start immediately") and
**Sposta in cima** does it for one card later. Nothing already running is
overtaken: that would mean cancelling it.

What was really wrong was the feedback: with a long list, an appended row lands
below the visible ones and looks like it went nowhere. So the queue store
remembers the ids it just added (`flash`), and the list:

- scrolls to the first of them, **only if it is not already in view**, and to
  the nearest edge — the offset comes from `scrollOffsetFor()`, the same pure
  arithmetic the pads use, because a virtualised row that is off-screen has no
  element to ask;
- keeps them highlighted for 2.5 s (`.item--new`, an accent ring pulsing twice,
  off under `prefers-reduced-motion`), so the eye finds the row it just pasted;
- the count line is a `role="status"` region, so a screen reader hears the
  queue grow even when the new row is out of sight.

Smooth scrolling is skipped under `prefers-reduced-motion`; an id that rolled
off the history cap is "nowhere" rather than row zero, which `scrollOffsetFor`
pins with a test.

### D105 — The Deno offer and the stream detail live on the card

The M1 screen carried two things the queue UI did not, and both belong to a
card:

- **The Deno install offer** (spec §5.3.2 "ask once"). With the screen gone, a
  download that failed with `DENO_MISSING` left one message and a trip to
  Settings → Tools: the only hole in the audit that could *block* a real
  download. The failed card now carries the offer — Install (the same
  `tools_install` path as the Tools page, with progress and Cancel), Not now
  (persisted as `denoOfferDismissed`), and the tools error if the install
  fails. It disappears as soon as Deno is found; retrying the item stays the
  user's click, because the item's options froze the runtime mode it was added
  with.
- **The stream detail** (spec §11.5, "stage detail" next to the stage chip):
  "Video 1/2", "Merging…". The backend has sent `progress.label` / `subIndex` /
  `subTotal` since M1, the store read them, and nothing rendered them. They now
  open the card's status line while the item runs. `progressLabelKey` maps the
  stable tokens to the `job.label.*` strings of both dictionaries, and the i18n
  guard walks those tokens too: no literal in the source can name a key that
  arrives as data.

### D106 — The URL metadata cache and the paste rule moved to the Add bar

Spec §2.9 lists three things together on the URL input: the **metadata cache**,
the **500 ms debounce**, and "cleared on Clear". M1 implemented them in its
inspect store; M2 replaced that screen, and the store became unreachable — so
the cache was dead code holding a required behaviour.

It moved to the add bar, which is where the paste happens now:

- a URL inspected in this session answers from the cache: no yt-dlp process, no
  network, no spinner. Only complete, non-degraded videos are remembered (the
  `MetadataCache` refuses playlists, errors and degraded answers on its own, so
  a short resolution list can never stick), the cap is 50 URLs, "Ricontrolla"
  bypasses the cache on purpose, "Svuota la lista" clears it, a restart starts
  empty.
- **D76 still holds**: a paste is one atomic event, so the inspection starts at
  once; typing keeps the 500 ms. The component flags the paste, the store asks
  `inspectDelay()` — the same function M1 tested, instead of a hard-coded
  literal.

### D107 — The dead M1 frontend is gone; the M1 backend stays

Deleted because nothing could reach them any more: `stores/job.svelte.ts`,
`stores/inspect.svelte.ts`, `job.ts` and its tests, `App.svelte`'s start/stop of
that store, the helpers only they used (`resolutionOptions`, `defaultResolution`,
`speedMbits`, `isDeterminate`, `summarise`/`UrlSummary`), and the six
`job.stage.*` strings. The behaviour they carried is in D105/D106; the rest was
duplicate of `$lib/queue`.

The backend M1 commands (`download_start`, `download_cancel`,
`inspect_url`, `inspect_cancel`, `download_dismiss_deno_offer`) and their eight
IPC tests stay. They are the M1 path that spec §14 asks to keep working, they
cost one module and 0.5 s of tests, and the queue calls into the same
`services/job.rs` underneath. `services/job.rs`'s own header said "M2 replaces
this module" — it is the module the queue runs on, and the comment now says so.

### D108 — The fake tools never write into the repository

The pre-push review found `src-tauri/Fake title [fakeid].mkv` **tracked** — sixteen
bytes of `fake media bytes`, committed in the M1 branch (`91d469d`) and
regenerated on every `cargo test` since, including one run during the review
itself.

The cause was a probe being read as a download. The fixtures copy the single
`fake_ytdlp` binary under three names — `yt-dlp`, `qjs` **and** `ffmpeg` — and
the tool resolution asks each with its own documented flag (`--version`,
`--version`, `-version`). The fake only recognised `--version`, so the ffmpeg
probe fell through to the download path, found no URL after `--` and no
`--paths home:`, and wrote its media file where its `home` fell back to: `.`,
the process's working directory, which is `src-tauri/` when cargo runs the
tests.

Two changes close the class, not the instance:

- the fake answers `-version` with an ffmpeg banner, exactly as it answers
  `--version` with its own version;
- a download invocation **without a URL after `--`** or **without
  `--paths home:`** prints an error and exits 2 instead of guessing.
  `home_dir()` replaces the five `unwrap_or_else(|| PathBuf::from("."))`
  fallbacks inside the branches, so the trap cannot come back behind the check
  in `main`.

The committed file is out of the index (it stays in the history of the M1
branch: sixteen harmless bytes, not worth rewriting history for), and
`.gitignore` gained a narrow net (`src-tauri/*.mkv`, `*.opus`, `fake-ytdlp*`)
under the fix.

### D109 — One artifact per Linux format, not one per platform

A manual run (`workflow_dispatch`) is how installers are obtained to try by
hand, and GitHub hands an artifact back as a single zip. With `bundle/**` in
one artifact, the 5 MB `.deb` could only be downloaded together with the
~110 MB AppImage: the three Linux formats were one heavy archive.

The `bundle` job now uploads one artifact per Linux format —
`scaricatore-di-porto-linux-x64-deb`, `-rpm`, `-appimage` — each with
`if-no-files-found: error`, so a format that silently stops being produced
still fails the run. Windows ships one format (`nsis`) and macOS two files of
the same product (the `.dmg` and the `.app.zip`), so a single artifact stays
right there.

The release path is untouched: the `release` job collects every
`scaricatore-di-porto-*` artifact, flattens the folders and attaches the files
one by one, which is where a user of a published release already picks the
format they want.

Numbered D109 on purpose: this branch is cut from `main`, which still ends at
D76, while `m2-queue` carries D77–D108. When the two meet, this entry belongs
*after* D108.

### D110 — The stall test's margin has to outlast the process launch, not just the stall

The `process tests (windows x64)` job failed on
`a_stalled_challenge_switches_runtime_instead_of_failing`:

```
["invoke download runtime=deno format=yes url=https://youtu.be/stall"]
left: 1   right: 2
```

The test shortened the production stall watchdog (spec §5.3.2, "no progress for
90 s") to **300 ms**. On the Windows runner the *first* execution of the freshly
copied fake is delayed by the loader and by Defender's real-time scan, so the
watchdog killed the process **before it could write its first log line**: the
run was correct (it fell back to Deno, which the outcome asserts), and the log
simply had one line instead of two.

Two things make that the diagnosis and not a guess:

- every other test that asserts on the `runtime=quickjs` line leaves the
  watchdog at its production 90 s, and all of them passed on the same runner —
  the log itself is fine on Windows;
- a `Done { runtime: Deno }` outcome is only reachable by **abandoning a stalled
  QuickJS attempt**: a spawn failure would have failed the job instead of
  falling back. The behaviour under test was proven; only the fake's logging
  raced the kill.

The margin is 5 s now, which costs the test binary about two seconds. The
assertion stays the strong one (two lines, quickjs then deno). If a future
runner is slower still, the honest next step is to assert the Deno line and the
outcome — already asserted — and drop the line count, because the count depends
on the fake logging before the kill, which is a property of the fake, not of the
app. Production waits 90 s, so nothing here was a bug in the app.

### D111 — The artifacts, and the release, carried the bundlers' staging trees

The per-format split (D109) put two numbers in front of us: the `linux-x64-deb`
artifact weighed **127 MB** for a **42 MB** `.deb`, and the AppImage **251 MB**
for a **148 MB** file. A local build says why — next to the installer the
bundlers leave the *unpacked* package:

    bundle/deb/<name>/data/usr/…         88 MB
    bundle/appimage/<name>.AppDir/      359 MB
    bundle/squashfs-root/                46 MB

and the upload glob was `bundle/**`, so all of it travelled.

The `release` job would have been worse. Its flatten step moves **every** file it
finds into `assets/`, so a tag would have attached those trees — the application
binary, the icons, hundreds of files — to the draft release beside the
installers. Nobody had seen it because no `v*` tag has been cut yet.

Two changes:

- the uploads name **files**, never `**`: `deb/*.deb`, `rpm/*.rpm`,
  `appimage/*.AppImage*` (which also covers the updater's `.AppImage.tar.gz` and
  its `.sig` when M7 turns them on), `nsis/*.exe`, `dmg/*.dmg` and the
  `.app.zip`;
- the `release` job gained a guard, **"Only installers reached the release"**,
  which walks `assets/` after the flatten and fails the job when anything is
  there whose name is not an installer, `SHA256SUMS` or `latest.json`. It has its
  own tests in `scripts/ci-steps.test.mjs` — run by `npm test` — including the
  case that matters, the flattened application binary, because a shell step
  nobody executes is not a check.

What the artifacts should weigh now: `.deb` 42 MB, `.rpm` 42 MB, `.AppImage`
148 MB (the spec expects the AppImage to be the heavy one: it embeds WebKitGTK
and the size targets do not apply to it), macOS universal ≈ `.dmg` + `.app.zip`,
Windows ≈ the NSIS `.exe`.

### D112 — An invariant is asserted once, in the test that can make it deterministic

Two `ci` runs failed `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`
on the same assertion — "the file on disk says `downloading` while the tool
runs" — first with a 3 s margin, then with an 8 s one, and finally with a marker
that proved a tool was running but not **which** of the two items it was (the
next read then found `queued`, i.e. it was looking at the wrong item). A test
that reads the disk in the middle of a run is racing the machine: no margin
fixes that, and a marker that does not name its item does not either.

The invariant is not lost, because it already had a better owner:
`services::queue::engine::tests::the_file_says_an_item_is_running_before_the_tool_is_spawned`
asserts exactly it — it calls `start_item` and reads the file with no scheduler
and no clock in between — and it passed in **both** of those CI runs. The
integration test now keeps only what it can prove deterministically: the fake
reports while it is alive (a marker named after the URL,
`.fake-started-<url tail>`), the app is closed **while a tool runs**, and the
next launch parks the item and lets a Resume finish it — which is also the half
a user actually meets.

The rule this leaves behind: when an assertion turns out to need a margin, the
question is not "how big a margin" but "which test can assert this without
one". Then the margin goes away instead of growing. (`read_state_on_disk` went
with the read: with the assertion gone it had no users, and dead test helpers
are still dead code.)

### D113 — A test that breaks the queue file must be the only writer of it

The third `ci` run failed `the_queue_file_survives_a_corruption_with_a_backup`
with *"the queue starts empty"*: the reopened engine found a **valid** document
with items in it. The file had been rewritten **after** the test corrupted it.

The mechanism is in the code, not in a timing guess. `wait_until_quiet` waits
for the item to settle in the **snapshot** (memory), and `finish_item` mutates
the state under the lock, releases it, and only *then* calls `self_persist` —
so the state is visible a moment before the write that records it (the same
window D94 documents, from the other side). A test that corrupts the file right
after `wait_until_quiet` corrupts a document the item task is about to
overwrite.

The fix is not a longer wait: it is to remove the other writer. The document is
now written by `fx.idle_engine()` — an engine whose tasks were **never started**
— so `add()` is the only thing that touches the file, and the test owns the
corruption. The fixture already had that constructor, and it is what an earlier
soak used to keep an engine out of the way.

The rule, which is D112's rule applied to a file instead of an assertion: when
a test has to make something happen *at* a moment, remove the other actors
rather than wait longer for them. A real app does not have this problem —
exiting kills the process before anything writes again.

### D114 — The tool is told to write UTF-8, because a piped stdout is not UTF-8

The first Windows run of the built installer reported, for some videos:

> Download non riuscito: yt-dlp reported C:\Users\…\GTA 6 … [sVunlQ_mhbQ].mkv
> but the file is not there (spec §8.3: the path is never guessed)

and the file **is** there, correct and playable. The same links never failed on
Linux, and the report was the same every time for the same video, not random.

**The cause is the encoding of the tool's stdout, and only titles decide.**

1. The path the app checks is the one yt-dlp *prints*
   (`--print after_move:SDP_FILE|%(filepath)s`), and it is printed through
   `YoutubeDL._write_string` → `write_string(..., encoding=self.params['encoding'])`.
   With no `--encoding`, `write_string` falls back to
   `out.encoding or preferredencoding()` — **the encoding of the pipe**.
2. On Windows a *pipe* gets the ANSI code page (cp1252 on the western systems;
   yt-dlp's own debug line shows `locale cp1252, pref cp1252`), not UTF-8: only
   a *console* gets UTF-8 I/O. `write_string` then encodes with
   `.encode(enc, 'ignore')`, so every character outside the code page is
   **dropped** and single-byte accents become mojibake. (yt-dlp issue #13853 is
   exactly this: redirecting the output loses Unicode "and spaces appear
   instead" — and the reporter found the remedy in `--help`, which is what this
   entry applies.)
3. The *file*, on the other hand, is created with Python's wide-char APIs under
   its **real** name, so the folder is right and the printed path is wrong.
   Which is why the report quoted a clean, plausible-looking ASCII path: the
   characters that could not survive the code page were simply gone.
4. So the failure is **deterministic per title** (accented or non-Latin
   characters, emoji) and invisible for pure-ASCII titles — "some downloads,
   often", the same links on the same machine, and never on Linux, where the
   pipe is UTF-8 already.

Why the environment cannot fix it from outside: the standalone Linux/macOS and
Windows builds are PyInstaller bundles whose interpreter runs an **isolated**
configuration, where `PYTHON*` variables are not read. Measured on the bundled
binary: `PYTHONIOENCODING=cp1252` and `PYTHONIOENCODING=ascii` change nothing,
and `LC_ALL=C` is coerced to UTF-8 (PEP 538) — which is also why this cannot be
reproduced on this machine. The only lever is the tool's own flag, which is what
spec §4.5 prescribes: **pass `--encoding utf-8`**.

What changed: `ytdlp::ENCODING_ARGS` is part of the **base** arguments of every
download (so it survives the retry without `-f`, the audio-only path and every
future branch) and of the inspection. It is passed on **every** platform: the
same bug waits on any machine whose locale is not UTF-8, and making the tool's
output UTF-8 is what our reader already assumes (`String::from_utf8_lossy`,
spec §4.5). The inspection's JSON is safe either way (`json.dumps` escapes
non-ASCII), but its stderr carries titles into error messages, and the flag has
to be a property of "how we call yt-dlp", not of one call site.

The rule this leaves behind: **every tool we parse must be told which encoding
to write**, because the encoding of a pipe is a platform (and locale) decision,
never ours. A tool that has no such flag (ffmpeg is byte-preserving) still gets
its output decoded as UTF-8, and its behaviour has to be measured, not assumed.

Reproduction in the suite, on every OS: the fake yt-dlp now writes the final
path the way the real tool does — through the console code page unless
`--encoding utf-8` was passed — so
`ytdlp_job::an_accented_final_path_survives_the_tools_console_code_page` fails
with the user's exact message if the flag is ever removed (verified by removing
it).

### D115 — The restart test waits for the file to catch up, because the memory settles first

The fourth `check` failure on Windows was `the_queue_survives_a_restart_and_the_interrupted_item_can_resume`:

> an item that was running comes back parked, never running again on its own: Queued

It passed on Linux and macOS, on every run, and it passed on the Windows runner
before the marker handshake was introduced (the earlier failures of this test
were about *reading the file mid-run*, which the marker solved).

**The mechanism, which is the same window D94 and D113 document from the other
sides:** `finish_item` mutates the state under the lock, releases it, and only
then writes `queue.json`. `snapshot()` — what the test was waiting on — sees the
settled state a moment **before** the write that records it. The test dropped the
engine and reopened it in that window, so the "second launch" read the document
the app had written *before* it closed: for the item whose tool had just been
killed, that document still said `queued`. It is not a timing margin that fixes
this, because the gap is not a duration, it is an ordering.

The fix is to wait for the **file**, not for the memory: `wait_for_parked_on_disk`
polls `queue.json` until it records the item as parked, and only then is the
engine dropped. The invariant is still guarded, and guarding it is now visible:
with the closing write removed the test fails with *"timed out … waiting for the
closed item to reach queue.json"* instead of passing quietly (verified by
breaking it on purpose).

**The second thing this test was doing wrong, which the fix also removes:** it
waited for *any* marker from the fake and then asserted on `ids[0]`, while its own
comment said "which of the two items the single download lane picked is not the
test's business". On a starved machine the lane and the queue order say nothing
about the thread running the test. The fake now writes its marker named after the
**host** it is downloading, and the test follows that to the item it added — so
the assertion is about the item that really had a tool behind it.

The rule this leaves behind, which is D113's rule applied to a *reopen*: when a
test closes and reopens something, it must first wait for the artifact the reopen
reads to agree with what the test has already seen. Waiting longer for the same
moment does not close an ordering gap; waiting for the *other* side of it does.

### D116 — Windows: a spawn that fails because the file is held is retried too

`is_text_file_busy` recognised only Unix's `ETXTBSY` (26) — "the executable is
still open for writing" — and retried the spawn for up to 100 ms. On Windows the
same situation has different names: a freshly created or copied executable is read
by a scanner (antivirus, indexer), and `CreateProcessW` answers
`ERROR_SHARING_VIOLATION` (32) or `ERROR_ACCESS_DENIED` (5) until it releases the
file. This project creates that situation on purpose: the first launch copies
yt-dlp and qjs into the app data dir and runs them a moment later (spec §5.2.1).

This is **precaution, not a measurement**: unlike the Unix case — where the suite
observed the retry firing — there is no recorded Windows failure of this shape,
and the CI failure of D115 turned out to be the reopen race, not the spawn. So
the amount (1 s of budget, 50 ms apart) is a judgement written down as such, and
the `TODO(verify)` below is what closes it.

What is verified: the two error codes are stable Win32 values; they are compared
only on Windows (32 and 5 are `EPIPE` and `EIO` on Linux and macOS, where the
predicate now covers exactly one errno, as its unit test still asserts); the
budget stays inside a second so a genuinely unstartable tool is still reported
inside the shortest user-visible timeout.

`TODO(verify)`: run a first launch on Windows with the bundled tools and confirm
whether `CreateProcess` ever answers 32 or 5 for them; if it does not, the budget
can go back to the Unix amount and this becomes a one-line comment instead of two
constants.

### D117 — The spawn-busy predicate, and how a `cfg(windows)`-only failure is verified on Linux

Two things came out of the D116 change landing on the Windows runner. The first is
a bug I introduced, the second is a way to stop paying CI runs for that class of
bug.

**1. `ETXTBSY` and the sharing errors are compared per family, and so is the
test.** The first Windows run of D116 failed `check` with:

> assertion failed: `!is_text_file_busy(&io::Error::from_raw_os_error(other))`
> at `src\services\process.rs:1576`

That was my own unit test, and it failed *correctly*: the test asserted that the
codes `[2, 5, 13, 22, 27, 32]` are never retried, which is true on Unix and false
on Windows, and I had left it ungated. The mistake is not the assertion, it is
that D116 gave the predicate a per-family meaning while leaving one half of it
described as unconditional.

The correction goes one step further than adding a `#[cfg(not(windows))]`: the
predicate compares **only** the codes that mean "not runnable yet" on the family
it is running on. `raw_os_error()` is an errno on Unix and a Win32 code on
Windows, and the same number means two different things — 26 is `ETXTBSY` on one
and `ERROR_NOT_DOS_DISK` on the other, 32 and 5 are `EPIPE` and `EIO` on one and
the sharing errors on the other. So `ETXTBSY` is now `#[cfg(not(windows))]` too,
and the Windows test asserts that 26 is *not* retried there: reading a Win32 code
with the Unix table would turn a real failure into the same real failure one
second later. Each half is a `#[cfg(...)]` test, and neither can run on the
family it does not describe.

**2. A `cfg`-only failure can be reproduced and fixed on Linux, without a
Windows runner.** The reason this bug reached the runner at all is that the
offending function does not exist on the machine where I run `cargo test`. It can
be made to exist: take the item's text out of the file, let `rustc` apply the
`cfg` expansion by rewriting the *other* family's attributes to `#[cfg(any())]`
(never true) and dropping the kept family's attributes, then compile that as a
test with `--test` and run it natively. The technique is worth the paragraph
because it verified both halves of this decision:

* *as Windows*, the pre-fix source has **two** tests and
  `text_file_busy_is_recognised_and_nothing_else_is` **fails** with the CI's exact
  assertion — the Windows failure, reproduced on Linux;
* *as Windows*, the fixed source has **one** test and it passes;
* *as Unix*, the fixed source has the other one and it passes.

It is not a substitute for the runner (it cannot cover `CreateProcess`, the Job
Object, or real antivirus behaviour, and it is only as faithful as the extraction
is), but for the specific question "does this item exist and pass on the other
family?" it answers in seconds instead of in a CI round trip. The extraction is
throwaway code in `/tmp`, not something the repository carries.

### D118 — Nothing starts by itself: every workflow is a button

Until today `ci.yml` started on `push` (`main` and `v*` tags) and on
`pull_request`, so **every** commit pushed to `main` bought a Linux `check` (7
minutes of the private repository's minutes, D12) that nobody had asked for.
Waiting for a result that was going to be looked at later is the same as not
having it, and the budget is the reason the push trigger was ever narrowed
(D19) — narrowing it twice was the sign that the trigger itself was wrong.

**The decision: no workflow starts on its own.** All four are
`workflow_dispatch` only (`ci.yml`, `process-tests.yml`, `selftest.yml`,
`tier2.yml`). The `push` and `pull_request` triggers are gone; the tag trigger is
gone too, because "only manual" that keeps one automatic path is not only
manual.

**Two buttons instead of one.** They answer two different questions, and the
second is the fast one:

* **`Checks and process tests`** (`process-tests.yml`) — "is the code healthy?":
  the same `check` job as `ci.yml` (Linux: fmt, clippy, `cargo test`, frontend
  type-check/test/build, tools manifest, runner-image guard) plus the whole
  suite on **Linux, Windows and macOS** (`linux-x64` was added: the process
  layer is the part whose behaviour really changes from one OS to the next, and
  the two failures of D115 and D117 are exactly what a matrix missing a row
  hides). It builds no installer.
* **`CI`** (`ci.yml`) — everything: the same `check`, the process tests on
  Windows and macOS, the three bundles, the universal macOS bundle and, if the
  dispatch is on a tag `v*`, the draft release. Unchanged in what it does.

**What this costs.** A green run is now a deliberate act, which is the point,
but it also means a broken commit can sit on `main` until someone presses the
button; there is no longer a background check. And the release is no longer "push
the tag and wait": with the tag trigger gone, the flow is *tag → push the tag →
Actions → `CI` → Run workflow → choose the tag in the dropdown*, because
`release` is still conditioned on `startsWith(github.ref, 'refs/tags/v')` and a
dispatch on a branch would silently skip it. That step is the price of the
choice, and it is written here so the v0.1.0 release does not go looking for it.

**Mechanics worth knowing.**

* The jobs of `ci.yml` keep their `if: github.event_name == 'workflow_dispatch' ||
  startsWith(github.ref, 'refs/tags/v')`. The first clause is now always true and
  the second is unreachable; they are left as the written statement of "these are
  not jobs for a push" and as the only thing that would have to be true again if
  a trigger ever came back.
* The `check` job exists twice, in `ci.yml` and in `process-tests.yml`, because
  Actions cannot share a job between workflows (`workflow_call` would mean
  restructuring `ci.yml` into a reusable workflow, which is a bigger change than
  the one asked for). The two blocks are **byte-identical** — they were generated
  from the same text — and both files say so; the check that they have not
  drifted is one `diff` of the two blocks, and it is a candidate for
  `scripts/check-ci-syntax.mjs` if it ever drifts in practice.
* The `process-tests` matrix of `process-tests.yml` runs `cargo test` on Linux
  too, which the `check` job already does. That is deliberate: the three rows are
  the same command on three systems, which is what makes a comparison possible,
  and it costs a cached minute.
* `permissions: contents: read` and the `concurrency` groups stay: cancellation
  is still useful when a button is pressed twice.

**Verified** by `npm run check:ci` (13 `run: |` blocks), `npm run
check:runners` (4 workflows, no floating or deprecated image, every action on
Node 24), a YAML parse of all four files, and the `diff` above (empty).

### D119 — Whisper sidecar (sdp-whisper) build workflow and model download manager (M5-A)

Spec §5.4, §10.2, §11.6, `whisper.models.json`.

1. **Pinned whisper.cpp sidecar (`sdp-whisper`) version is `v1.9.5`** (ggml-org/whisper.cpp).
   The workflow `.github/workflows/whisper-sidecar.yml` builds static binaries:
   - Linux (x86_64, aarch64): static linkage with `-static-libgcc -static-libstdc++`.
   - Windows (x64, arm64): static C++ runtime (`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`).
   - macOS (arm64, x86_64): Metal enabled (`GGML_METAL=ON`).
   Asset release naming adheres to `sdp-whisper-{target}`.
2. **Model catalogue (`whisper.models.json`) pinned via Hugging Face Git LFS query**:
   Contains 11 Whisper ggml models (Tiny, Base, Small, Medium, Large-v3, Large-v3-Turbo with quantized variants) and Silero VAD (`silero-v5.1.2` from `ggml-org/whisper-vad`, `ggml-silero-v5.1.2.bin`, 885,098 bytes, sha256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`).
   Every entry has exact byte size and sha256 checksum. `scripts/fetch_models_manifest.mjs` provides offline/online validation (`npm run check:models`).
3. **Automatic VAD co-download**:
   As specified in §10.2 ("Silero VAD model fetched together with the first model"), `download_model` checks if `ggml-silero-v5.1.2.bin` exists; if not, it transparently downloads and verifies the VAD model first before initiating the target Whisper model download.
4. **Resumable HTTP Range download and disk space check**:
   Downloader writes to `<model>.part` using HTTP `Range: bytes=N-`, calculates in-progress received bytes, guards against low disk space prior to start, and deletes corrupted `.part` files on sha256 mismatch.
5. **State and IPC**:
   `SETTINGS_SCHEMA_VERSION` bumped to 7 with transcription preferences (`active_whisper_model`, `whisper_models_mirror_url`, `skip_silence_vad`, `whisper_threads`, `whisper_translate_to_english`, `default_transcript_language`, `transcription_engine_order`, `transcripts_subfolder`).
   IPC commands `models_list`, `models_download`, `models_cancel_download`, `models_delete`, `models_import`, `models_set_active`, `models_open_folder` wired to Svelte 5 frontend.

### D120 — Whisper offline transcription pipeline, chunking, and local media action (M5-B)

Spec §10.2, §3, §11.5, §14 row M5, §15 item 9.

1. **Audio preparation**:
   - 16 kHz mono WAV conversion via ffmpeg: `-vn -ac 1 -ar 16000 -c:a pcm_s16le`.
   - Pre-flight free disk space check requires minimum estimated WAV size (`32,000 bytes/sec * duration + 200 MiB safety margin`).
2. **Chunking for audio > 2 hours (7200 s)**:
   - Evaluates silence points within ±20 s of 30-minute boundaries (1800 s) using ffmpeg `silencedetect=noise=-30dB:d=0.5`.
   - If no silence interval is detected, falls back to a hard cut with 2.0 s overlap.
   - Individual chunk transcripts (.txt, .srt, .vtt) are generated in temporary storage and stitched together. SRT and VTT cues are offset by the chunk starting millisecond and cue sequence indices are renumbered continuously.
3. **Whisper CLI flags pinned**:
   - Spawns `sdp-whisper` with flags: `-m <model>`, `-f <wav>`, `-l <lang>`, `-tr` (if translate requested), `-pp` (progress callback), `-otxt`, `-osrt`, `-ovtt`, `-of <out_base>`, `--vad` and `-vm <silero_vad>` (if VAD enabled), `-t <threads>`, and `-ng` (if GPU off / fallback).
4. **Stderr progress parsing & stdout live preview**:
   - Parses stderr progress callbacks: `whisper_print_progress_callback: progress = X%` and maps them cleanly into item stage progress `transcribing (X%)`.
   - Streams live segment text from whisper stdout to item state and UI preview.
5. **Automatic retry fallback with `-ng`**:
   - If `sdp-whisper` terminates with an error and was launched without `-ng`, the pipeline automatically retries once on CPU by appending `-ng`.
   - Emits a warning notification informing the user that transcription failed and CPU fallback was engaged.
6. **Hallucination detection loop guard**:
   - Detects repetitive segment loops (>= 5 identical consecutive segments) or suspicious silence markers emitted by Whisper (spec §10.2 item 7).
   - Emits a warning toast to notify the user ("try VAD, a larger model, or set the language") without modifying the transcript text.
7. **Cancellation & process tree cleanup**:
   - Graceful cancellation via `CancellationToken` terminates the process tree (whisper or ffmpeg) and removes temporary WAV and partial transcript files.
8. **Queue engine integration**:
   - Added dedicated transcription concurrency semaphore (`max_parallel_transcriptions`).
   - Supports transcription after media download, standalone "Transcribe a local file…" action in the Add bar, and contextual "Transcribe…" action on completed queue items.

### D121 — Errata for D119 and D120: Silero VAD metadata, fallback retry, and hallucination guard (B5)

Review finding B5 identified discrepancies between code/manifest reality and descriptions in D119/D120:

1. **Silero VAD metadata (D119 item 2 & 3)**:
   The Silero VAD asset in `whisper.models.json` is `ggml-silero-v5.1.2.bin` (885,098 bytes, SHA-256 `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`) sourced from `ggml-org/whisper-vad`. D119 previously cited an outdated size and hash from `snakers4/silero-vad` and referenced `ggml-silero.bin`.
2. **CPU fallback on non-zero exit (D120 item 5)**:
   `whisper.rs` retries with `-ng` upon non-zero exit if `-ng` was not passed. It does not inspect stderr for GPU-specific error strings.
3. **Hallucination guard behavior (D120 item 6)**:
   Per spec §10.2 item 7 ("after a run, detect obvious loops (the same segment repeated >= 5 times) or suspicious silence-only output and warn the user"), `detect_hallucination` inspects the transcript and returns a `HallucinationWarning` to display a toast notification. It does not alter, trim, or mutate the written transcript text.

### D122 — Whisper sidecar CI: macOS Intel deduplication and Ubuntu 24.04 runner standard (M1)

Spec §5.4, §1.1. Review M1.

1. **macOS Intel binary deduplication**:
   In `.github/workflows/whisper-sidecar.yml`, `build-tier1` on `macos-26` compiles the macOS universal binary (`arm64;x86_64`) with Metal enabled and embedded library (`-DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON`), producing `sdp-whisper-universal-apple-darwin`, `sdp-whisper-aarch64-apple-darwin` (`lipo -thin arm64`), and `sdp-whisper-x86_64-apple-darwin` (`lipo -thin x86_64`).
   This matches spec §1.1 ("macOS Intel (the x64 slice of the universal binary)").
   `build-tier2` previously also built `sdp-whisper-x86_64-apple-darwin` on `macos-15-intel` without Metal. This resulted in an artifact name collision when downloading all job artifacts into `incoming/` during the packaging step, where `find ... -exec cp` arbitrarily overwrote one with the other.
   `macos-15-intel` has been removed from `build-tier2`. `sdp-whisper-x86_64-apple-darwin` is built exclusively by `build-tier1` with Metal support embedded.

2. **Artifact packaging collision guard**:
   The `Collect and compute SHA256SUMS` step in `whisper-sidecar.yml` now inspects every incoming artifact filename and aborts with `exit 1` if a duplicate filename is encountered across job artifact folders, preventing silent overwriting of assets.

3. **Ubuntu 24.04 as CI runner standard**:
   Spec §5.4 originally suggested building the Linux sidecar binary on Ubuntu 22.04 ("oldest supported distro"). However, GitHub Actions has announced the deprecation and phased brownouts of `ubuntu-22.04` (actions/runner-images#14254). Per D55 and `scripts/check-ci-runners.mjs`, all repository workflows use `ubuntu-24.04` as the fixed Linux runner. The sidecar build in `whisper-sidecar.yml` uses `runs-on: ubuntu-24.04` with static linking (`-static-libgcc -static-libstdc++`) as standard.

### D123 — Whisper sidecar CI: GGML_NATIVE=ON for Windows ARM64 MSVC build (spec §5.4, §1.1)

In `whisper.cpp` (v1.9.5), `ggml/src/ggml-cpu/CMakeLists.txt:118` enforces:
```cmake
if (MSVC)
  message(FATAL_ERROR "MSVC on ARM requires GGML_NATIVE=ON, use clang for GGML_CPU_ARM_ARCH / GGML_CPU_ALL_VARIANTS")
endif()
```
MSVC `cl.exe` targeting ARM64 does not support the runtime CPU architecture feature dispatch that Clang/GCC use when `GGML_NATIVE=OFF`. Building with `-DGGML_NATIVE=OFF` on the `windows-11-arm` GitHub Actions runner causes CMake configuration to fail immediately.

In `.github/workflows/whisper-sidecar.yml`, the `Build whisper-cli (Windows ARM64)` step now uses `-DGGML_NATIVE=ON`. Windows ARM64 is a Tier 2 target (spec §1.1, CI-only / best-effort). This allows MSVC to compile `whisper-cli.exe` targeting ARM64 natively on the runner.

### D124 — Restriction of mock_builder() to integration tests and NTFS-safe free space testing (spec §10.2, §15)

1. **`mock_builder()` restricted to integration tests (`tests/*.rs`)**:
   In Cargo, `cargo:rustc-link-arg-tests` in `src-tauri/build.rs` embeds `windows-app-manifest.xml` (Common Controls v6) only into integration test binaries under `tests/*.rs`. The `--lib` unit test runner (`src/lib.rs`, `scaricatore_di_porto_lib-*.exe`) does not receive `rustc-link-arg-tests` and cannot use global `rustc-link-arg` without corrupting the cdylib target or duplicating resources (see `build.rs:15-20` and D16). Calling `tauri::test::mock_builder()` in any module inside `src/` pulls in `comctl32.dll` v6 symbols without the manifest, causing `scaricatore_di_porto_lib-*.exe` to crash before `main` on Windows with `0xc0000139 (STATUS_ENTRYPOINT_NOT_FOUND)`.
   Therefore, all tests calling `tauri::test::mock_builder()` must live in integration test binaries (`tests/*.rs`). `models_download_throttles_rapid_chunk_progress_events` was moved to `tests/models.rs`. An architectural test `src_unit_tests_do_not_call_tauri_mock_builder` verifies that `src/` never calls `mock_builder()`.
   `commands::models::models_download` selects `Transport::LoopbackHttp` whenever `mirror_url` targets a loopback interface (`http://127.0.0.1:`, `http://localhost:`, `http://[::1]:`), allowing integration tests to exercise `models_download` without `#[cfg(test)]`.

2. **NTFS-safe `.part` file free space accounting in tests**:
   In `tests/models.rs:download_model_accounts_for_existing_part_file_in_disk_space_check`, querying `free_disk_space` before creating the `.part` file causes false `DiskFull` failures on Windows NTFS, because `File::set_len` physically reserves/allocates clusters and immediately decreases `GetDiskFreeSpaceExW` by the file size. Creating and sizing the 200 MiB `.part` file *before* querying `free_disk_space` ensures the initial allocation is already accounted for in `free_space`, and using a 140 MiB safety headroom (`model_size = free_space + safety_margin + 40 MiB`) protects against runner background disk jitter (OS paging, Defender, logs).

### D125 — Transcription Phase M6: Subtitles-first pipeline, Gemini REST engine, summaries, and secret storage (spec §10.1, §10.3, §10.4, §10.5)

Phase M6 implements subtitles-first transcript extraction, the Gemini cloud transcription engine, map-reduce summarization, and secure secret storage:

1. **`keyring 3.6` dependency with zero OpenSSL**:
   - `keyring` v3.6 is added with `default-features = false` and native backends: `apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`, `linux-native`.
   - Verified that `cargo tree -i openssl-sys` produces 0 matches; the entire build remains strictly free of OpenSSL in adherence to spec §1.

2. **Secret storage hierarchy & Linux fallback (spec §10.5)**:
   - Priority: `GEMINI_API_KEY` environment variable -> OS Keyring (`service = "scaricatore-di-porto"`, `user = "gemini_api_key"`) -> local file fallback.
   - On Linux systems where the D-Bus Secret Service daemon is unavailable (minimal window managers, headless CI, or containerized environments), falls back to `<app_config_dir>/gemini_api_key`.
   - On Unix, the fallback file is created with strict `0600` permissions (`libc::S_IRUSR | libc::S_IWUSR`) using `OpenOptionsExt::mode(0o600)`.
   - The key is never transmitted to the frontend or written to logs; IPC only exposes key presence and storage target (`has_key`, `source`).

3. **Subtitle downloader and cleaner (spec §10.1)**:
   - Uses `yt-dlp --write-subs --write-auto-subs --skip-download` to retrieve manual or automatic subtitles.
   - `clean_subtitles` strips WebVTT headers, timestamp tags (`<00:00:00.000>`), styling/positioning metadata, and cue tags (`<c>`, `<b>`, `<i>`).
   - Deduplicates rolling automatic caption loops where lines or word sequences repeat across sliding time windows.
   - Automatically breaks continuous captions into readable paragraphs whenever speech pauses exceed 2.0 seconds.

4. **Gemini Cloud Engine REST client (spec §10.3)**:
   - Lightweight implementation over `reqwest` + `rustls-tls` without heavy Google Cloud SDKs.
   - Audio preparation via ffmpeg transcode: 16 kHz mono Opus in an Ogg container (`-c:a libopus -b:a 24k -ar 16000 -ac 1`), with fallback to MP3 or WAV if Opus encoding is unavailable.
   - Audio upload uses Google Files API Resumable Upload protocol streamed directly from disk (`POST /upload/v1beta/files` followed by streaming `POST` to upload URL), avoiding high RAM usage on large audio files.
   - Polls for file state `ACTIVE` with exponential backoff, 10-minute timeout, and `CancellationToken` cancellation.
   - RAII cleanup guard (`GeminiFileGuard`): issues `DELETE /v1beta/files/{name}` on task completion, error, or cancellation to avoid leaving orphaned media files on user quotas.
   - Automatic backoff retry on HTTP 429 quota exhaustion.
   - Dynamic model discovery from `GET /v1beta/models`, filtering for `generateContent` capabilities, sorting `-latest` aliases first, and falling back to pinned models when offline.

5. **Map-reduce summarization (spec §10.4)**:
   - For long transcripts exceeding `MAX_SUMMARY_CHUNK_CHARS` (50,000 characters), splits text along paragraph boundaries and summarizes individual sections before combining them in a final reduction pass.
   - Produces `<name>.summary.md` alongside `<name>.txt` transcript.

6. **Pipeline orchestration and error preservation (spec §10, §7)**:
   - Configurable engine modes: `subtitles_whisper_gemini` (default), `whisper_only`, `gemini_only`, `subtitles_only`.
   - Sets `QueueItem.transcript_origin` (`subtitles_manual`, `subtitles_auto`, `whisper`, `gemini`) upon completion.
   - In fallback pipeline (`subtitles_whisper_gemini`), when offline Whisper fails (e.g. missing sidecar or model) and no Gemini key is configured, the actionable Whisper error is preserved instead of being overwritten with `GeminiKeyMissing`.







### D126 — M7: on-demand ffmpeg/ffprobe provisioning and the unavailable Whisper sidecar (spec §5.1, §5.2)

M7 closes the "everything downloadable from inside the app" gap without bundling anything beyond yt-dlp and qjs (spec §0.7, §5.2.1):

1. **Source and version.** ffmpeg and ffprobe are pinned to **7.0.2** from `eugeneware/ffmpeg-static` release **b6.1.1**, which republishes the johnvansickle (Linux), evermeet (macOS) and Gyan/BtbN (Windows) builds. The release ships one `.gz` per executable and platform; all ten SHA-256 values in `tools.manifest.json` were computed locally from the downloaded assets. The Linux x64 binaries were executed: `ffmpeg -version` reports 7.0.2 and `-encoders` lists `libopus`, `libmp3lame` and `libx264`. The other nine binaries were checked for the expected ELF/PE/Mach-O format but not executed (no hardware for the other platforms).
2. **No new archive dependency.** A single gzip stream is not a `.tar.gz`, so `ArchiveKind::Gz` and `extract_gz` were added. `tools::install_tool` is now generic over plain assets, `.gz`, `.zip`, `.tar.gz` and `.tar.xz`: it downloads to `.<exe>.download` (plain) or `unpack-<version>` (archive), verifies SHA-256, extracts with the existing path-traversal guard, renames atomically and sets the executable bit on Unix. `extract_archive` deliberately refuses `Gz`; `download()` handles that case before it.
3. **ffprobe is a companion, not a second button.** Installing `ffmpeg` also installs `ffprobe`, because duration probing needs both. `tools_status` no longer lists ffprobe twice, and the wizard's "Install missing tools" only asks for ffmpeg.
4. **Platform gaps are explicit.** `eugeneware/ffmpeg-static` has no `win32-arm64` asset, so that platform is absent from the manifest and the UI reports "not available on this platform" instead of offering a 404. The same rule applies to `sdp-whisper`: on 2026-10-23 the expected `whisper-v1.9.5` release answered 404, so every `url`/`sha256` is `null`, the Tools row explains that no release is published yet, and **Use my own path** still works. The manifest shape is ready for the day `whisper-sidecar.yml` publishes the assets.
5. **Verification.** The ignored integration test `the_ffmpeg_install_downloads_verifies_and_runs` downloads both binaries, verifies their checksums, runs `ffmpeg -version` and removes the temp tree. It passed locally but is not part of `cargo test` because it needs the network; the normal suite uses the existing fake tools.

### D127 — M7: in-app updates, signed updater manifest and the MSRV bump (spec §13.4)

1. **Plugins and key handling.** `tauri-plugin-updater` **2.14.0** and `tauri-plugin-process` **2.4.0** are Rust-side dependencies. The JS updater package was installed during the spike and then removed: the frontend only calls `app_update_check` / `app_update_install`, and `@tauri-apps/plugin-process` is used solely for `relaunch()`. The public key is in `tauri.conf.json`; the private key was generated outside the repository (`/tmp/opencode/updater.key`, password empty) and only the public half was copied into the config. A human must add `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to the repository secrets before a signed release can be built.
2. **Endpoint and artifacts.** `bundle.createUpdaterArtifacts = true`; the endpoint is `https://github.com/EmanueleFusco/ScaricatoreDiPorto/releases/latest/download/latest.json`. The local Linux bundle build on 2026-10-23 produced `*.deb.sig`, `*.rpm.sig` and **`*.AppImage.sig`**, with no `.AppImage.tar.gz`: with the v2 (`true`) mode the updater installs the raw AppImage, and the `.tar.gz` wrapper only exists in the deprecated v1-compatible mode. The local `tauri-bundler` 2.10.4 source confirms that macOS always archives the `.app` as `*.app.tar.gz` and signs it, and that Windows NSIS produces `*-setup.exe` + `.sig`. Platform keys are `linux-x86_64`, `windows-x86_64`, `darwin-aarch64` and `darwin-x86_64`. `scripts/generate-latest-json.mjs` prefers an `.AppImage.tar.gz` if a legacy build ever supplies one, otherwise it points Linux at the raw `.AppImage`.
3. **Linux scope.** Only the AppImage can replace itself. `.deb` and `.rpm` installs show "new version available" plus a link to the releases page (`canSelfUpdate()` in `src/lib/about.ts`), matching spec §13.4.
4. **Static `latest.json`.** `scripts/generate-latest-json.mjs` builds the manifest from the files actually collected in the release job: URLs are percent-encoded, signatures are inlined, and the script fails when a platform, a signature or an expected artifact is missing. The release job runs it after flattening the artifacts, and the release guard rejects anything that is not an installer/updater file. On macOS the universal `.app.tar.gz` is used for both `darwin-*` keys; the native macOS bundle deliberately does not upload its own updater archive, because two files with the same name would collide in the merged artifact directory.
5. **UX.** The About page checks at most once every 24 hours, stores the timestamp in `localStorage` (no settings schema bump), and never installs without a click. Installing is refused while the queue is active. The update progress arrives through the `update:progress` event; after installation the page offers a relaunch.
6. **MSRV and Clippy.** `rust-version` moved from 1.85 to **1.90** because the updater requires it (local toolchain 1.95.0). The higher floor enables let-chain rewrites in code unrelated to M7, so `clippy::collapsible_if` is allowed at crate level and in the four affected integration-test binaries. This is a deliberate, reviewable exception, not a blanket lint disable.
7. **Size.** The binary measured by `scripts/report-bundle-sizes.mjs` after a full local `npx tauri build` is **8,890,880 bytes (8.5 MiB)**, against 6.9 MiB before M7 (about +1.6 MiB, +23%). `cargo tree -d` shows why: the app keeps `reqwest 0.12.28` and the updater brings `reqwest 0.13.5`, both rustls-based. The binary remains well under the 15 MB target of spec §15. The same build produced a **43.2 MiB `.deb`**, a **43.2 MiB `.rpm`** and a **148.9 MiB AppImage** (excluded from the budget by spec §1). These are local Linux numbers; the CI runner may differ and reports its own.
8. **Verified locally.** `npx tauri build` with the generated private key produced `.deb.sig`, `.rpm.sig` and `.AppImage.sig`, and `scripts/generate-latest-json.mjs` read the real `.AppImage.sig` into a four-platform `latest.json` with percent-encoded URLs. The public key in `tauri.conf.json` matches `/tmp/opencode/updater.key.pub`.
9. **Not verified.** A real update cannot be exercised until a signed M7 release with `latest.json` exists; `releases/latest` currently resolves to the old `v2.5`, which has no manifest. Windows and macOS updater behaviour is likewise unverified on hardware.

### D128 — M7: diagnostics and a minimal log without `tauri-plugin-log` (spec §11.8)

1. **Logging.** `services/logging.rs` writes one append-only file at `<app log dir>/scaricatore-di-porto.log`. Each line is `[unix-seconds] LEVEL: message`; embedded newlines are replaced so a log entry can never forge a second line. Logging is best effort and must never make an operation fail. The file is capped only when read: **Copy diagnostics** keeps the last 80 lines.
2. **Why no plugin.** `tauri-plugin-log` would add a dependency tree for a feature that is one `OpenOptions::append` away. The app needs a support file, not structured logging, rotation or a remote sink.
3. **Diagnostics.** `services/diagnostics.rs` is a pure builder: app name/version, OS/arch, target triple, bundle type, settings/tools/bundled/temp/models/log paths, one row per tool (version, source, path) and the log tail. It is explicitly tested to contain no `api_key`, `AIza` or `cookie` strings. The frontend copies the text and can reveal the log file in the system file manager.
4. **Privacy.** The report contains local paths, which is why both READMEs tell the user to read it before posting it publicly. It never contains API keys, cookies or URLs from the queue.

### D129 — M7: bilingual user documentation (deliberate exception to the English-only rule)

The repository rule says everything published to GitHub is in English. M7 adds **Italian user documentation** anyway, because the application itself ships Italian and English and a user who chooses Italian should not be sent to an English-only guide:

- `README.md` (English) and `README.it.md` (Italian);
- `docs/SUPPORTED_SITES.md` / `.it.md`;
- `docs/TROUBLESHOOTING.md` / `.it.md`.

English is the source of truth; the Italian files are maintained translations and must be updated in the same commit as their English counterpart. This exception covers **user-facing documentation only**. Code comments, commit messages, CI text, `docs/PROGRESS.md`, `docs/DECISIONS.md` and `docs/RUST_PORT_GUIDELINES.md` remain English. The historical Italian content inventoried in D77 is a separate translation task and is not changed here.

### D130 — M7: UI quality-of-life, keyboard shortcuts and honest error banners (spec §11.5, §11.9)

1. **Queue.** Added a find-as-you-type filter (`Ctrl/Cmd+F`), row checkboxes, Shift-click range selection, select-all-shown, and an inline bulk bar for pause/resume/cancel/remove. The list stays virtualised; the 5,000-item arithmetic test still renders fewer than 20 nodes.
2. **Shortcuts.** `shortcuts.ts` distinguishes `isTypingTarget` (text-consuming inputs, textareas, selects, contenteditable) from `isInteractiveTarget` (any input, button or link). `Ctrl/Cmd+F` therefore still works after clicking a checkbox, while `Delete` and `Space` do not steal keys from a button or a link. `Ctrl/Cmd+,` opens Settings. The bug was found by manual testing and covered by the 10 shortcut tests.
3. **Setup wizard.** The Tools step performs real installs with retry/cancel, can install all missing required tools in one action, and shows per-tool state. The Transcription step downloads/cancels/resumes the recommended Whisper model and chooses the engine order. The last step picks an output folder. The dialog traps focus, and closing it with the X, the backdrop or Escape counts as completed, so it does not reappear on every launch.
4. **Errors.** Error banners show the localized `message_key` first; raw provider/technical detail sits in a collapsed `<details>` (`common.technicalDetails`). A Gemini JSON error no longer fills the page.
5. **Layout.** The wizard has a fixed header/footer and a scrollable body. The queue was compacted so the add bar, filter, bulk bar, list and footer all fit the default 980×680 window; the empty state is centred.
6. **Contract fix.** `ToolSource::PathEnv` serializes as `path` (not the camelCased `pathEnv`), matching what the frontend already switched on. A regression test locks the five source names.
7. **Verification.** Manual checks on Linux/KDE Wayland covered the queue, filter, selection, bulk actions, shortcuts, wizard, tool install, model download, local-file transcription, Gemini error, language/theme switching, About, diagnostics and update error. Windows and macOS were not manually tested.

### D131 — M7: MIT licence file

Added a root `LICENSE` (MIT, Copyright (c) 2026 Emanuele Fusco) to match the `license = "MIT"` already declared in `package.json` and `src-tauri/Cargo.toml`. The Tauri bundle metadata now also says MIT. Third-party components keep their own licences, listed in **Settings → About → Licences** and in `tools.manifest.json`; the GPL-3.0-or-later status of ffmpeg is called out explicitly.

### D132 — M7: installer size measurements, without a hard gate (spec §1, §15 item 1)

Spec §1 fixes the **application binary** at ≤ 15 MB and asks for the installer size to be measured and reported; the AppImage is excluded because it embeds WebKitGTK. M7 records the Linux numbers so a future regression is visible:

| artifact | local Linux build, 2026-10-23 | note |
| :-- | --: | :-- |
| application binary | 8.5 MiB | under the spec §1 binary target of ≤ 15 MB |
| `.deb` | 43.2 MiB | includes the bundled yt-dlp (~38.6 MiB) and qjs |
| `.rpm` | 43.2 MiB | same contents as the `.deb` |
| AppImage | 148.9 MiB | excluded by spec §1 (embeds WebKitGTK) |
| Windows NSIS | not measured locally | to be reported by the first CI bundle |
| macOS universal `.dmg` / `.app.zip` | not measured locally | to be reported by the first CI bundle |

The installers are deliberately larger than 15 MiB: yt-dlp alone is about 38.6 MiB, and shipping it is what makes the first run work offline (D30). There is **no hard installer-size gate**: `scripts/report-bundle-sizes.mjs` reports the measured numbers on every bundle job, but does not fail a release for being large. The 15 MB figure remains the spec target for the application binary only, and the measured 8.5 MiB is comfortably below it. Windows and macOS numbers are unknown until CI runs on those runners; they are not guessed here.

### D133 — M7: idle memory on Linux and the 30 MB target (spec §15 item 1)

Spec §15 item 1 asks for "idle RSS of the Rust process ≲ 30 MB (tree measured separately)". Measured on 2026-10-23 on Linux x64 / CachyOS / KDE Wayland, using the release binary after the window had been idle for 30 seconds, with no queue activity:

| process | VmRSS | PSS | RssAnon |
| :-- | --: | --: | --: |
| `scaricatore-di-porto` (Rust process) | 191,920 kB | 87,832 kB | 44,628 kB |
| `WebKitNetworkProcess` | 65,464 kB | 32,685 kB | 22,700 kB |
| `WebKitWebProcess` | 160,060 kB | 110,289 kB | 42,924 kB |

The Rust process's RSS is dominated by file-backed shared library mappings, not by the application's own heap: `libnvidia-gpucomp.so` 27.8 MB, `libLLVM.so` 26.3 MB, `libwebkit2gtk` 19.7 MB, `libjavascriptcoregtk` 12.5 MB, `[anon:WKFastMalloc]` 11.4 MB, and the application binary itself 7.7 MB. Its private anonymous memory (`RssAnon`, equal to `Private_Dirty` here) is 43.6 MiB, so the 30 MB target is **not met** even when "Rust process" is read as private memory rather than raw RSS. The tree (main + network + web process) is about 408 MiB RSS and 226 MiB PSS, measured separately as the spec asks.

No memory optimisation was attempted in M7. This is a WebKitGTK/GPU-driver baseline rather than a small allocation that a code change can remove, and the correct next step is to repeat the measurement on a clean VM without the NVIDIA/LLVM stack before changing the target or the application. The item remains **open**, not waived: M7 does not claim that spec §15 item 1 fully passes.

### D134 — M7 correction: the repository slug and the published `sdp-whisper` sidecar (spec §5.1, §5.4, §13.4)

D126 item 4 and D127 item 2 used `EmanueleFusco/ScaricatoreDiPorto` for the release and updater URLs. That is **not this repository**: `origin` is `https://github.com/EmanueleFusco/Scaricatore-di-Porto-rust.git`. The wrong slug made the `whisper-v1.9.5` API lookup, every sidecar asset URL and the updater endpoint answer 404. The sidecar release was not missing; the URL was wrong. This entry is a correction, not a rewrite of the historical records.

The correct release exists: `whisper-v1.9.5`, published 2026-10-19, with seven assets and a `SHA256SUMS` file. `tools.manifest.json` now carries the real URL and SHA-256 for every platform, so the Install button is enabled instead of reporting "not available yet". The updater endpoint, the About/releases links, the README links and the URL tests now use the correct slug too. The Linux x64 asset was downloaded through `tools::install_tool`, its checksum matched, and the installed `sdp-whisper -h` ran (real ignored test, 2026-10-23). The other six assets answered HTTP 200; their checksum values were copied from the release's `SHA256SUMS` but those six files were not downloaded or executed.

### D135 — Linux Wayland: move the WebKitGTK DMA-BUF fallback into the app (spec §13.3)

On the owner's machine (`XDG_SESSION_TYPE=wayland`, KDE + NVIDIA RTX 3080,
`webkit2gtk-4.1 2.54.1`) `npm run tauri dev` died immediately with
`Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.`
and the AppImage did the same. The existing workaround lived only in
`scripts/run.mjs`, so the direct Tauri command bypassed it.

Verified in this step:

- `WEBKIT_DISABLE_DMABUF_RENDERER=1` alone is enough on this machine: both the
  debug binary and the AppImage stay alive and emit no `Failed to create GBM
  buffer` message. `WEBKIT_DISABLE_COMPOSITING_MODE=1` does not help, and
  `GDK_BACKEND=x11` is not needed once the DMA-BUF renderer is disabled.
- The WebKit bug is upstream (WebKit bug 280210); it is not our code.

The fallback is now applied in `src-tauri/src/lib.rs` before the Tauri builder
runs, on Linux only:

- `is_wayland_session()` detects a Wayland session from `WAYLAND_DISPLAY` or
  `XDG_SESSION_TYPE=wayland`;
- `apply_webkit_dma_buf_fallback()` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`
  only when the user has not already set it, so an explicit override always wins;
- the helper is covered by unit tests in `lib.rs`.

This supersedes the "launcher only" part of D10 for newly built binaries: the
direct `npm run tauri dev` command now works, and release binaries/AppImages
apply the same fallback. `scripts/run.mjs` is kept for older binaries and for
forcing `GDK_BACKEND=x11` when a compositor still needs it. Binaries built
before this change still need the environment variable until they are rebuilt.

### D136 — Restrict the Wayland DMA-BUF fallback to NVIDIA (spec §13.3)

The fallback from D135 was applied on every Linux Wayland session, including
AMD/Intel systems where the DMA-BUF renderer works. It now also requires an
NVIDIA proprietary driver to be present (`/proc/driver/nvidia/version` or
`/dev/nvidia0`). On non-NVIDIA Wayland systems the app keeps WebKitGTK's default
renderer and its hardware-accelerated path; users on other affected GPUs can still
force the fallback with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
