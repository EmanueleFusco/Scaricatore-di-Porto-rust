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
