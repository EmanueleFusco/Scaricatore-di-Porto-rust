# AGENTS.md — Scaricatore Di Porto (Rust + Tauri v2)

You are a coding agent working on a rewrite of **Scaricatore Di Porto** (a Python/CustomTkinter downloader + transcriber) as a light, cross-platform **Rust + Tauri v2 + Svelte (TypeScript)** desktop app for Windows, macOS and Linux. Read this file at the start of **every** session.

## 1. Source of truth

| File | Role | Rule |
| :-- | :-- | :-- |
| `docs/RUST_PORT_GUIDELINES.md` | The specification (rev. 3). Product decisions, architecture, flags, phases, acceptance tests. | **Read-only.** Never edit it. If it is wrong or ambiguous, write the problem in `docs/DECISIONS.md` and continue with the safest interpretation. |
| `reference/ScaricatoreDiPorto.py` | Original app. Shows the real behaviour to keep (parity) and the bugs not to port (spec §12). | Read-only. |
| `docs/PROJECT_REPORT.md` | High-level description of the original app. | Read-only. |
| `docs/PROGRESS.md` | Your running log (you maintain it). | Update at the end of every step. |
| `docs/DECISIONS.md` | Questions, deviations, things marked "verify" that you could not verify (you maintain it). | Append only. |

Spec section map (read **only** the sections your task needs, to save context):
`§0` decisions · `§1/1.1` targets and support tiers · `§2` scope · `§3` architecture · `§4` process handling · `§5` tools (5.1 manifest, 5.2 lookup/provisioning, 5.2.1 bundled yt-dlp+qjs, 5.3 JS runtime, 5.4 Whisper sidecar CI, 5.5 yt-dlp updater) · `§6` link inspection, non-YouTube, playlists · `§7` queue · `§8` yt-dlp execution · `§9` ffmpeg · `§10` transcription (10.1 subtitles, 10.2 Whisper, 10.3 Gemini, 10.4 summary, 10.5 secrets) · `§11` UI · `§12` do-not-port list · `§13` packaging/updater/CI · `§14` phases · `§15` tests/acceptance.

## 2. Working agreement

1. **One step at a time.** Work on the task you were given, in the phase you were given (M0…M7, spec §14). Do not start the next phase. Do not refactor unrelated code.
2. **Small, verifiable steps.** After every meaningful change run the checks in §4. Never leave the build broken at the end of a step.
3. **Do not invent.** Before using any yt-dlp / ffmpeg / whisper-cli flag, GitHub asset name, URL, or crate API, verify it: run `<tool> --help`, read `docs.rs` or the crate source in `~/.cargo/registry`, or read the official docs. If you cannot verify, write `// TODO(verify): …` in the code and add a line to `docs/DECISIONS.md`. Spec text marked "verify" must be verified, not assumed.
4. **Never invent checksums, hashes, versions or URLs.** Fill them with scripts that download and compute them (see M0), or leave a clearly marked placeholder that makes CI fail until filled.
5. **Ask only when blocked.** If the spec leaves a real choice open, pick the simplest option that matches the spec, record it in `docs/DECISIONS.md`, and go on. Stop and report only for things only a human can do (e.g. macOS notarization, Windows antivirus tests, signing keys, accounts).
6. **Keep changes reviewable:** one logical change per commit (§5), no giant rewrites, no reformatting of untouched files.
7. **Tell the truth in `docs/PROGRESS.md`:** what works, what is only compiled, what is untested, what is stubbed. Do not claim a test passed unless you ran it.

## 3. Repository layout (create it in M0)

```
AGENTS.md
docs/            RUST_PORT_GUIDELINES.md  PROJECT_REPORT.md  PROGRESS.md  DECISIONS.md  SPIKES.md
reference/       ScaricatoreDiPorto.py
scripts/         fetch_bundled_tools.mjs  (and other helper scripts, Node, cross-platform)
src/             Svelte + TypeScript frontend
src-tauri/       Rust backend (modules as in spec §3), tauri.conf.json, capabilities/, binaries/ (git-ignored)
tools.manifest.json   pinned tools (spec §5.1) — tracked in git
whisper.models.json   Whisper model catalogue (spec §10.2) — tracked in git
.github/workflows/    CI (spec §13.5)
```

## 4. Commands (run from the repo root unless noted)

```
npm install
npm run check                       # svelte-check / type-check
npm run build                       # frontend build
npm run tauri dev                   # run the app
npm run tauri build                 # bundle
( cd src-tauri && cargo fmt --all -- --check )
( cd src-tauri && cargo clippy --all-targets -- -D warnings )
( cd src-tauri && cargo test )
```
**Definition of done for any step:** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` and `npm run check` all pass; `docs/PROGRESS.md` updated; changes committed. If a command cannot run in your environment (e.g. no display), say so in `docs/PROGRESS.md` and say what you did instead.

## 5. Git

* Branch per phase: `m0-skeleton`, `m1-single-video`, … Commit often.
* Commit message: `<area>: <what> (spec §x.y)` — e.g. `process: kill process tree on Windows via Job Object (spec §4.2)`.
* Never commit: downloaded tool binaries, models, API keys, `.env`, user data. `src-tauri/binaries/` and `target/` stay git-ignored.

## 6. Code rules

**Rust**
* Edition 2021+, stable toolchain. `thiserror` for typed errors (`AppError` → serializable `ErrorInfo {code, message_key, detail}`); **no `unwrap()`/`expect()` outside tests** and unavoidable static init.
* `tokio` only with the features needed; use Tauri's async runtime. `reqwest` with `rustls-tls` and **no OpenSSL** (spec §1).
* Commands in `commands/` are thin; logic lives in `services/` as pure, unit-testable functions wherever possible (arg builders, parsers, selectors, filename/time helpers, error classifier).
* Spawn tools only through `services/process.rs` (spec §4): `CREATE_NO_WINDOW` on Windows, Job Object / process group for tree-kill, drain stdout **and** stderr, URL after `--`, argv arrays only, absolute tool paths, `--ignore-config` for yt-dlp. Never build shell command strings.
* Never hold a mutex across `.await` on a child process. Use `tokio::select!` with a cancellation token.
* Settings/queue files: `serde(default)`, schema version, atomic write (temp + rename).
* Cross-platform differences go behind `#[cfg(target_os = "…")]` in small, tested functions.
* Do **not** disable TLS verification anywhere. Do **not** put secrets in logs.

**Frontend (Svelte + TypeScript)**
* Light: no UI framework runtime, no CSS framework runtime, system font stack, inline SVG, no remote assets (strict CSP).
* All user-visible strings go through the i18n dictionaries (`it`, `en`); no hard-coded text, no concatenated sentences.
* Lists that can be large are **virtualized**; progress updates arrive as events and patch single rows (spec §7.6, §11.7).
* Accessibility: keyboard navigation, focus styles, labels, ARIA roles, contrast (spec §11.9).

**Tests**
* Unit tests for every pure function you add. Integration tests use **fake tools** (small scripts or test binaries that print canned output / spawn a grandchild) so they run on all OSes without network.
* Never make tests depend on live YouTube or live Gemini. Real-tool checks go in the separate self-test / nightly job (spec §5.3.6, §13.5).

## 7. Security and privacy

* Never write real API keys, tokens or personal data into code, tests, docs, logs or prompts. Test keys are fake strings.
* Gemini key storage: OS keyring with the Linux fallback of spec §10.5. Never send the key to the frontend.
* Downloaded executables: HTTPS only, checksum verified, path-traversal-safe extraction, atomic rename (spec §5.2).

## 8. Things you must never do

* Edit `docs/RUST_PORT_GUIDELINES.md` or `reference/*`.
* Port a behaviour listed in spec §12.
* Add a heavyweight dependency (or a new plugin) without noting the reason and binary-size impact in `docs/DECISIONS.md`.
* Bundle anything besides **yt-dlp and qjs** in the installer (spec §0.7, §5.2.1).
* Claim support for macOS Intel, Windows ARM64 or Linux arm64 as "tested" — they are Tier 2 / CI-only (spec §1.1).
* Skip a failing check by deleting or weakening a test. If a test is wrong, fix it and explain why in `docs/PROGRESS.md`.

## 9. `docs/PROGRESS.md` format

Append one block per step, newest last:

```
## <date> — <phase> / <step title>
- Done: …
- Verified by running: <exact commands and results>
- Not verified / stubbed: …
- Spec items marked "verify" that I checked: <flag/URL → result>
- Decisions/deviations (also in DECISIONS.md): …
- Next step: …
```

## 10. Session protocol

1. Read this file and the last ~100 lines of `docs/PROGRESS.md`.
2. Read only the spec sections named in your task.
3. Make a short plan (≤ 10 lines) and write it at the top of your reply, then work step by step.
4. Run the checks (§4). Fix what you broke.
5. Update `docs/PROGRESS.md`, commit, and end with: what is done, what is next, what needs a human.
