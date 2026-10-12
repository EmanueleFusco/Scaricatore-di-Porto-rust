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

### Open questions (owner)

* Tier 1 assumption of spec §1.1 (Windows x64, Linux x64, macOS Apple Silicon) is still a
  *guess* — nobody has confirmed it yet. It should be stated explicitly in the README.