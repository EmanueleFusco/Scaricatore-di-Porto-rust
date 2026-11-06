# Version 0.1.0 audit — 2026-11-06

## Scope and verdict

Audited checkout `ad1f67feb7767bd0fe378852cf56afd74217bfb1` on Windows x64.
The `v0.1.0` tag resolves locally to `89a40523824e0a3db39491b10b8f3cae426f3476`;
the intervening commit changes only PROGRESS, DECISIONS and SPIKES. Production
code therefore matches that tag. No application fixes were applied in this audit.

### Follow-up implementation status

- F01: corrected after the audit; unique staging files and no-clobber numbered
  outputs are covered by unit and fake-tool integration regression tests (D145).
- F02: corrected request ownership, stale replies and row/batch cancellation
  after the audit (D146). Progressive per-row results and the shared three-process
  inspection budget were subsequently completed and tested (D150).
- F03/F04: corrected duplicate reconciliation using original request indexes,
  retained confirmations, and explicit reverse enqueue order (D147).
- F05: implemented persistent AppImage paths and preservation of managed desktop
  entries (D148); native Linux acceptance still required.
- F06–F10: corrected inspection details/copy, FFmpeg version parsing, low-speed
  feedback, pause/interruption wording and shell-free launcher paths (D149).
- The findings and reproduction notes below describe the audited baseline.

The core download pipeline works in the exercised cases, and the ordinary test
suite is green. That does not make the whole release accepted: there are two
high-priority defects, several reproducible queue/UI defects, and important
unexercised platform and service combinations. Fix output collisions and
inspection lifecycle first, then queue/playlist behavior, before cosmetic work.

Evidence labels:

- **UI reproduced:** observed in the running native Windows app built from this checkout.
- **Service reproduced:** exercised the real service or compiled frontend store in a diagnostic harness.
- **Source confirmed:** the implementation shows the behavior, but no complete UI reproduction was performed.
- **Platform inference:** source evidence plus platform documentation; requires a native acceptance test.

## Executed checks

| Check | Result |
| --- | --- |
| `npm.cmd run check` | 0 errors, 0 warnings |
| `npm.cmd test -- --run` | 327 passed, 12 skipped, 16 files |
| `npm.cmd run build` | Passed |
| `cargo fmt --all -- --check` | Passed on the audited application sources |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test` | 637 passed, 5 ignored |
| `npm.cmd run check:ci` | All 24 shell blocks valid |
| `npm.cmd run check:release -- v0.1.0` | All three version manifests agree |
| `npm.cmd run tauri build -- --no-bundle` | Production executable built successfully; installer not built/tested |
| Bundled QuickJS live self-test | Passed; 25 formats, heights 240/144, 8,766 ms |
| Real Whisper sidecar install self-test | Windows asset downloaded, checksum verified, `-h` succeeded |
| Temporary frontend diagnostic harness | Reproduced stale inspection, playlist order and mixed-batch resend defects |
| Temporary ignored Rust diagnostic probes | Real trim/encode, existing-output replacement, real banner parsing and real Tiny transcription exercised |

The first sandboxed Rust/Vitest/CI attempts failed on Windows temporary-file
renames or Git Bash named-object permissions. Rerunning outside that sandbox
passed. Those initial environment errors are not application regressions.
A first attempt to launch the live Rust test through Cargo also encountered the
open Windows executable lock; running the existing test executable succeeded.

## Interactive and real-tool coverage

| Scenario | Observed result |
| --- | --- |
| Native startup and existing queue loading | Worked; pre-existing history loaded |
| Public YouTube `jNQXAC9IVRw` | Inspected and downloaded to MKV, about 463 kB |
| In-app YouTube self-test | Reported QuickJS success, 25 formats, about 9 s |
| `https://example.com` | Correct no-downloadable-content error |
| Unavailable YouTube ID `00000000000` | Correct unavailable-video error |
| Video URL with a playlist parameter | Inline video/playlist choice appeared |
| Mixed HTTP, `file://` and invalid text | Only HTTP(S) links became inspection rows |
| Local HTTP direct MP4 | Inspected and downloaded successfully |
| Local HTTP WAV, audio-only option | Produced Opus successfully |
| Local static HLS manifest | Inspected and downloaded successfully |
| Local HTML page containing a video element | HTML5 media detected and downloaded |
| Parallel slow local transfers | Both ran concurrently |
| Pause/resume | Paused state persisted; resume returned the item to downloading |
| Cancel | Item became cancelled; did not create a completed output |
| Close during an active transfer | App exited; subsequent host process inspection found no app or app-data tool processes |
| Queue filtering | Isolated the expected test rows |
| Window widths | Default layout, about 640 px with icon sidebar, and maximized layout inspected |
| About/update check | Displayed 0.1.0 and reported already current; no update installed |
| Real CPU H.264/AAC encode with 2–5 s trim | Succeeded; ffprobe measured 3.0 s; original hash unchanged with keep-original enabled |
| Real Tiny Whisper | 32,152,673-byte model downloaded and checksum verified; real speech produced nonempty TXT/SRT/VTT |

The non-YouTube HTTP cases used a loopback fixture server with small media made
from the public test video. They validate extractor/pipeline behavior, not
compatibility with four independent public providers. The Tiny English transcript
contained recognition errors; this is a pipeline smoke test, not a quality score
for the recommended model, Italian, or long recordings.

## Findings, ordered by priority

### F01 — P1: encoding replaces an existing result without preserving it

**Service reproduced on Windows; relevant to all platforms.**

`services/ffmpeg.rs:370` always derives `<stem>-encoded.<ext>` from the input.
`run_encode` writes a deterministic partial name and then renames it over the
final name (`:762`). There is no collision-free name or overwrite decision.

Reproduction: create an existing file at the derived encoded path, run the real
H.264/AAC encode with a 2–5 s trim and `keep_original: true`. Encoding succeeds,
the input hash stays unchanged, and the previous output is replaced. Consequently,
two different clips/codec settings of the same source can destroy the earlier
result even when the user chose to keep the source. Shared partial names also
deserve a concurrent-encode test if encode concurrency is enabled.

Suggested correction: reserve a unique destination for each output and a unique
partial path per job. Test two different trims and a pre-existing output.

### F02 — P1: adding a link during inspection can strand earlier rows forever

**UI and compiled-store reproduced; relevant to all platforms.**

`stores/addbar.svelte.ts:527` increments one global `#request` for each check.
Starting batch B makes every result from batch A stale (`:553`), although A's
rows still exist. A's rows remain `checking`; they have no Retry button and can
never become downloadable. Playlist pagination also increments that counter.

UI reproduction: inspect a direct MP4 whose server delays the response by 15 s;
before it finishes, add a fast HTML video link. The second link becomes ready,
while the first stays on “Controllo…” after its real request has completed.
The diagnostic harness independently resolves both requests and confirms
`[checking, ready]` with `checking === true`.

Related lifecycle gap: Clear/removing a row does not invoke `inspect_cancel`,
and `commands/inspect.rs` creates batch cancellation tokens without registering
them with the cancellable inspection slot. Clearing the UI therefore does not
stop those network/tool requests. Batch results also return only after the entire
batch finishes, so one slow URL delays feedback for its successful neighbors.

Suggested correction: track requests/cancellation per row or batch, without
invalidating unrelated rows; emit results incrementally and cancel abandoned work.

### F03 — P2: mixed duplicate/new batches are handled incorrectly

**Compiled-store reproduced and source confirmed; all platforms.**

In `components/AddBar.svelte:200`, any successful addition calls `clearAdded()`
for every ready row and `dismissDuplicates()`. A mixed result therefore loses
the duplicate rows and the question needed to add them intentionally.

The complementary path is also wrong: `stores/queue.svelte.ts:187` resends the
entire `#lastRequest` with `force: true`, rather than only the refused requests.
The diagnostic harness confirms that a mixed two-item request resubmits both
items. The playlist flow can retain such a mixed duplicate result and reach this
store method, potentially adding an accepted item a second time.

Suggested correction: reconcile results per request and retain only refused
requests for “Add anyway”. Test mixed results, not only all-new/all-duplicate.

### F04 — P2: playlist reverse changes the display but not download order

**Service reproduced and source confirmed; all platforms.**

`PlaylistPanel.svelte:62` reverses the visible entries. Its `add()` does not pass
the reverse choice to `buildPlaylistRequests`, which unconditionally sorts by
ascending index (`playlist.ts:290`). The harness shows visual order `[2, 1]`
but queue request order `[1, 2]`.

Suggested correction: pass the chosen enqueue order explicitly while retaining
original playlist positions for numbering. Verify subset selection and reverse
together.

### F05 — P2: Linux desktop shortcuts can point into a vanished AppImage mount

**Platform inference; not executed on Linux in this audit.**

`lib.rs:100` uses `std::env::current_exe()` for `Exec=` and overwrites two user
desktop entries on every launch (`:126`). In an AppImage this can name the payload
inside its temporary mount, which disappears after exit, instead of the persistent
AppImage. An entry created by an installer/integration tool can also be overwritten.

The AppImage documentation distinguishes the temporary mounted AppDir from
`APPIMAGE`, which identifies the actual image file:
[environment variables](https://docs.appimage.org/packaging-guide/environment-variables.html),
[runtime architecture](https://docs.appimage.org/reference/architecture.html?highlight=read+only).

Suggested correction: use the persistent image path when appropriate, correctly
escape desktop-entry values, and avoid replacing managed shortcuts at every start.
Acceptance: launch an AppImage, exit, then launch its menu shortcut after the mount
has disappeared.

### F06 — P2: inspection errors discard useful technical details in the UI

**Source confirmed; all platforms.**

The Add bar store retains each row's `errorDetail`, but the row renders
`errorText(row.errorKey, null)` and exposes no error Details/Copy details control.
“More options” exposes request settings, not the underlying failure. A TLS,
extractor or authentication failure can therefore leave the user with a generic
message and no useful diagnostic path before the item reaches the queue.

Suggested correction: collapsed technical details plus a copy action, preserving
the concise localized message by default. Verify no secrets are copied.

### F07 — P3: FFmpeg/ffprobe can display the compiler's version

**UI and service reproduced on Windows.**

Tools displayed **16.2.0** for a real Gyan FFmpeg git build. The raw banner starts
with the date/git build name; `tools.rs:505` scans later tokens until it accepts
the GCC version. The diagnostic probe confirms `Some("16.2.0")` from that actual
banner. This also makes minimum-version comparisons unreliable for such builds.

Suggested correction: parse the tool's version field specifically; show an
uncomparable build identifier honestly instead of borrowing a dependency version.

### F08 — P3: slow but advancing downloads are displayed as 0 Mbit/s

**UI reproduced; all platforms.**

The slow fixture advanced and had a changing ETA while both the row and footer
showed 0 Mbit/s. `queue.ts:787` rounds positive rates below 0.05 Mbit/s to zero.
This looks like a stalled transfer.

Suggested correction: display `<0.1 Mbit/s`, more precision, or kbit/s at low rates.

### F09 — P3: manual pause is described as recovery after a restart

**UI reproduced; all platforms.**

`QueueItem.svelte:313` shows `queue.item.resumed` for both `paused` and
`interrupted`. Immediately after manually pausing, the Italian UI said
“Ripreso dopo un riavvio”, despite no restart or resume having occurred.

Suggested correction: show distinct pause, interrupted, and resumed states.

### F10 — P3: the Windows release launcher script cannot find an existing binary

**Command reproduced on Windows; developer workflow, not the installed app.**

`npm.cmd run app:binary` reports that the release executable does not exist,
although it is present. `scripts/run.mjs:27` uses `URL.pathname`, producing a
`/C:/...` path on Windows; `fileURLToPath` is imported but not used for this root.
URL-escaped spaces/non-ASCII paths also need coverage on other platforms.

Suggested correction: use `fileURLToPath` and path joins; test a checkout path
with spaces and Unicode.

## UI assessment and improvements that preserve the style

The dark palette, system typography, restrained accent color and consistent
settings cards are a good basis. The narrow sidebar correctly becomes icons;
settings retain a readable maximum width at a large window size. Inline source
errors, format selection, queue filtering and the tool status table are useful.

The main usability weakness is space allocation. At the default window size,
the Add bar and instructions consume much of the height. With a few inspected
links, the queue and global actions move largely below the fold. The page,
pending-link list and queue each have their own scroll area, making it hard to
tell which one is moving. Completed rows contain up to eight equally weighted
buttons, while the full-width progress bar remains visually dominant at 100%.

Recommended order of UI changes:

1. Keep active queue work visible; collapse the Add bar after submission and
   cap its expanded area. Preserve a predictable queue viewport.
2. Give each state one primary action and one secondary action; place the
   remaining actions in an accessible overflow menu. Keep destructive actions
   visually and spatially distinct.
3. Reduce completed-row height and de-emphasize the completed progress bar;
   emphasize the file/output and the next useful action.
4. Show how many links are checking, ready and failed. “No links to add” while
   an inspection spinner is active is misleading.
5. Make detailed errors available (F06), then fix the version/speed/status
   feedback defects (F07–F09).
6. Test keyboard focus, expanded details and mixed row heights with thousands
   of entries. Queue virtualization currently assumes one measured row pitch,
   although expanded details and wrapped actions have different heights.

Additional specification gaps found by source review: playlist requests expose
quality/folder/numbering/archive but do not expose or carry the per-playlist
transcription options described in §6.4; reusable user presets described in §11.3
also have no completed save/select workflow. These are missing product workflows,
separate from the reproduced defects above.

## Limits and follow-up acceptance

- **Windows:** native development UI and current production build checked.
  The new production executable was compiled, not independently exercised as an
  installed NSIS package. A truly fresh machine, installer/signing/AV behavior,
  read-only/full disks and Windows ARM64 were not tested.
- **macOS/Linux:** source and packaging review only. No native runtime claims.
  macOS Keychain, notarization, Apple Silicon/Intel sidecars, VideoToolbox and
  clipboard shortcuts still require hardware. Linux AppImage integration,
  GNOME/KDE/Wayland, Secret Service fallback and VAAPI need native checks.
- **Sources:** no live 100-item playlist/channel/Mix, 1080p/4K/Shorts, private
  cookies-authenticated media, public DRM/live stream or real anti-bot retry was
  exercised here. Existing fake-tool tests are not substitutes for those cases.
- **Transcription:** real short English Tiny run only. Recommended large model,
  Italian accuracy, every model, VAD and >2-hour chunking were not exercised live.
- **Gemini:** existing local mock API tests passed. No real credential, paid API
  call, cloud-file cleanup failure/retry or real summary was tested.
- **Updates:** endpoint check succeeded; no old-to-new signed update installation.
- **Native dialogs:** the folder picker opened, but the automation helper could
  not reliably target its modal controls. Folder selection/import was therefore
  not accepted or classified as an app defect.
- **Performance/accessibility:** no 5,000-row runtime measurement, idle-memory
  benchmark, screen-reader pass or complete light-theme/keyboard acceptance.

## Proposed implementation sequence

1. Output safety and inspection lifecycle (F01–F02), each with regression tests.
2. Duplicate reconciliation and playlist ordering (F03–F04).
3. Linux launcher and useful inspection diagnostics (F05–F06).
4. Small feedback fixes (F07–F10), then the queue layout changes above.
5. Finish missing playlist/preset workflows and run the remaining native and
   real-service acceptance matrix before calling the release fully verified.

This report is a triage proposal. No automatic corrections, release publication,
tag changes, dependency upgrades or GitHub issue creation were performed.
