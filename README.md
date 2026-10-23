# Scaricatore Di Porto

A calm, queue-based downloader and transcriber for Windows, macOS and Linux.
It downloads video and audio with [yt-dlp](https://github.com/yt-dlp/yt-dlp),
processes media with [ffmpeg](https://ffmpeg.org), and turns speech into text
with offline [Whisper](https://github.com/ggml-org/whisper.cpp) or Google Gemini.

The app is built with Rust, Tauri v2 and Svelte. It has no telemetry, no
account, and no background service: it does something only when you ask.

[Italiano](README.it.md) · [Supported sites](docs/SUPPORTED_SITES.md) ·
[Troubleshooting](docs/TROUBLESHOOTING.md)

---

## What it does

- **Queue, not one job at a time.** Add many links, run 1–4 downloads in
  parallel (2 by default), pause, resume, retry, reorder and restart where you
  left off.
- **Playlists and channels.** Pick the entries you want, keep a folder per
  playlist, number the files, and skip what you already downloaded.
- **More than YouTube.** Any site yt-dlp supports: Vimeo, SoundCloud, Twitch,
  TikTok, Instagram, direct HLS/HTTP links and many more.
- **Formats you choose.** Best available, a maximum height (1080p, 720p, …),
  audio only, or an exact yt-dlp format ID from the advanced table.
- **Media processing.** Encode to MP4/MKV/WebM or audio (AAC, MP3, Opus), trim
  without re-encoding when possible, embed metadata, thumbnail and chapters,
  and remove SponsorBlock segments.
- **Transcripts.** Use existing subtitles first, then offline Whisper, then
  Gemini, in the order you choose. Outputs `.txt` and `.srt` (Whisper also
  writes `.vtt`) plus an optional `.summary.md`.
- **Local files too.** Drop a media file on the app to transcribe it without
  downloading it again.
- **Everything downloadable from inside the app.** yt-dlp and QuickJS ship in
  the installer; ffmpeg, ffprobe, Deno, Whisper models and updates are fetched
  on demand, with checksums and progress.
- **Updates.** The About page checks a signed release feed at most once a day.
  On Linux only the AppImage can update itself; `.deb` and `.rpm` users get a
  link to the release page.

## Install

Download the latest release from the
[Releases page](https://github.com/EmanueleFusco/ScaricatoreDiPorto/releases).

| System | File | Notes |
| :-- | :-- | :-- |
| Windows 10/11 x64 | `*-setup.exe` | NSIS installer. WebView2 is already present on Windows 11 and most updated Windows 10. |
| macOS (universal) | `*_universal.dmg` or `*.app.zip` | Universal build (arm64 + x86_64). The current builds are not notarized; see [Troubleshooting](docs/TROUBLESHOOTING.md). |
| Linux x64 | `*.AppImage`, `*.deb`, `*.rpm` | The AppImage is self-contained and supports in-app updates. |

The installers are not code-signed yet. On Windows, SmartScreen may ask you to
confirm; on macOS, use **Open** from the context menu the first time. These are
expected warnings for an unsigned build, not a sign that the file is damaged.

### First run

The setup wizard opens once. Every step is optional and can be skipped:

1. **Welcome** — choose language and theme.
2. **Tools** — yt-dlp and QuickJS are already inside the app. If ffmpeg is
   missing, **Install missing tools** downloads it (and ffprobe) with a
   checksum. Deno is optional and only offered when it is needed.
3. **Transcription** — choose the engine order and, if you want offline
   transcription, download the recommended Whisper model. Gemini can be
   configured later with an API key.
4. **Output folder** — use the system Downloads folder or pick another one.

You can reopen the wizard at any time from **Settings → Tools → Reopen the
setup**. The app remains usable if you skip everything: missing tools are shown
with an **Install** button in **Settings → Tools**.

## Using the app

### Add links

Paste one or more links in the box at the top of the queue. You can also drop a
`.txt` file containing one link per line. Each link is inspected on its own and
shows its title, site, duration and quality before it enters the queue.

Choose a quality (or **Audio only**), then press **Add to queue**. **Download**
starts the links immediately. If a link is already queued with the same options,
the app asks before adding it twice.

### The queue

- **Filter** with `Ctrl/Cmd+F`; **select all shown** with the toolbar button.
- **Select rows** with the checkbox, `Shift`-click for a range, then pause,
  resume, cancel or remove the selection from the bulk bar.
- **Reorder** by dragging a row, or use **Move to top** from its menu.
- **Expand details** to see the chosen options, output path, error details and
  the last messages from the tool.
- Keyboard: `Space` pauses/resumes the selection, `Del` removes it, `Ctrl/Cmd+,`
  opens Settings. Shortcuts are ignored while you are typing in a field.
- **Pause all**, **Resume all**, **Retry failed** and **Clear finished** are at
  the bottom of the queue.
- If the app closes mid-download, the item comes back as **Interrupted** and can
  be resumed.

### Playlists and other sites

A playlist or channel link opens a selection panel with a virtualized list.
Filter it, select a subset, choose a folder and numbering, and press **Add N
items**. The download archive remembers finished video IDs so a later run skips
them; clear it from **Settings → General**.

For a URL that contains both a video and a playlist, the app asks whether you
want **This video only** or **Whole playlist**. You can make that choice the
default in Settings.

Non-YouTube links behave like YouTube ones. The advanced format table lets you
paste an exact yt-dlp format ID. Live streams, DRM-protected pages and login
walls show a plain-language message with the action to take (usually cookies).

### Transcripts and summaries

Set the engine order in **Settings → Transcription & Models**:

- **Subtitles → Whisper → Gemini** (default): use manual subtitles, then auto
  captions, then offline Whisper, then Gemini.
- **Whisper only**, **Gemini only**, or **Subtitles only**.

Whisper runs locally. Download a model from the models table; **Turbo** is the
recommended quality/size trade-off. Gemini needs an API key, which is stored in
the OS keyring (or a `0600` file where no keyring is available) and never sent
to the webview.

Finished items can be transcribed later with **Transcribe…**. A summary is
written as `<name>.summary.md` when enabled. Gemini uploads the audio to Google
only when you choose that engine; the remote file is deleted after success,
error or cancellation.

### Settings

- **General** — language, theme, default download folder, parallel downloads,
  per-site limit, paste-anywhere, notifications, archive, naming and codec
  preferences.
- **Transcription & Models** — engine order, language, VAD, threads, Whisper
  model manager, Gemini key and model list, summary defaults.
- **Tools** — every external tool with source, version and status; install,
  update, test, use your own path, reopen the wizard, yt-dlp channel
  (stable/nightly), and the YouTube extraction self-test.
- **Network & Accounts** — cookies from a browser or a Netscape cookies file,
  proxy, custom headers, User-Agent, Referer, impersonation retry, JavaScript
  runtime.
- **About** — version, update check, licences, **Copy diagnostics**, **Open
  logs**, and the folders the app uses.

## Privacy

- No telemetry, analytics or crash reporting.
- Network access happens only for downloads, on-demand tool/model downloads,
  the daily update check, and Gemini when you configure it.
- The Gemini API key is stored in the OS keyring; on Linux without a Secret
  Service it falls back to a file readable only by your user (`0600`). The key
  is never sent to the frontend and never written to logs.
- **Copy diagnostics** contains versions, OS, tool paths and the last log lines.
  It contains no API keys or cookies, but it does contain local folder paths:
  read it before posting it in a public issue.

## Troubleshooting

Start with **Settings → About → Copy diagnostics** and **Open logs**. The
[Troubleshooting guide](docs/TROUBLESHOOTING.md) covers the common cases:
missing ffmpeg, a failing yt-dlp extraction, cookies, Whisper models, Gemini,
updates on Linux, a blank window on Wayland, and unsigned installers.

## Supported sites

YouTube is the main target, but the downloader is yt-dlp: if yt-dlp supports a
site, the app can usually download it. See
[Supported sites](docs/SUPPORTED_SITES.md) for the details, examples and
caveats (DRM, live streams, login walls).

## Build from source

Requirements: Node.js LTS, a stable Rust toolchain, and the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your
system.

```bash
npm install
npm run fetch:tools     # yt-dlp + qjs for this platform (~43 MB, checksummed)
npm run app             # development, with hot reload
```

Checks:

```bash
npm run check
npm test
npm run check:ci
npm run check:runners
npm run check:tools
( cd src-tauri && cargo fmt --all -- --check )
( cd src-tauri && cargo clippy --all-targets -- -D warnings )
( cd src-tauri && cargo test )
```

Bundle:

```bash
npx tauri build
```

Updater artifacts are signed at build time. To build them locally, export
`TAURI_SIGNING_PRIVATE_KEY` (the key content or its path) and, if the key has
one, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The public key is in
`src-tauri/tauri.conf.json`; the private key must never enter the repository.

### Release

The version lives in three manifests (`package.json`,
`src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`). Bump all three, check the
tag, then push the tag and run the **CI** workflow manually from the Actions
page:

```bash
npm run check:release -- v0.1.0
git tag v0.1.0
git push origin v0.1.0
```

The release job builds the installers, generates `latest.json`, refuses to
continue if a platform or a `.sig` is missing, and creates a **draft** release.
Publish it from the Releases page after testing the binaries. The updater
private key and password must be present in the repository secrets as
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

## Project documents

- [`docs/RUST_PORT_GUIDELINES.md`](docs/RUST_PORT_GUIDELINES.md) — the
  specification.
- [`docs/PROGRESS.md`](docs/PROGRESS.md) — what is done, tested or still
  stubbed.
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — decisions and deviations.
- [`docs/SPIKES.md`](docs/SPIKES.md) — platform spikes (notarization,
  antivirus, sizes).
- [`docs/ACCEPTANCE.md`](docs/ACCEPTANCE.md) — the spec §15 checklist and what
  is still open.

## Licence

MIT. Third-party components and their licences are listed in **Settings →
About → Licences** and in `tools.manifest.json`. ffmpeg is distributed under
GPL-3.0-or-later; yt-dlp is Unlicense; QuickJS-NG, whisper.cpp and Deno are MIT.
