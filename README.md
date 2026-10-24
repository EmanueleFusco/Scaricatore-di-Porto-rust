# Scaricatore Di Porto

Desktop app for downloading videos and audio with [yt-dlp](https://github.com/yt-dlp/yt-dlp)
and transcribing them. Runs on Windows, macOS and Linux.

It's a rewrite of the original Python/CustomTkinter app in Rust + Tauri v2 + Svelte.

## Features

- Download queue with parallel jobs, pause/resume/retry, persisted across restarts
- Playlists and channels, with per-entry selection
- YouTube and the other sites supported by yt-dlp ([details](docs/SUPPORTED_SITES.md))
- Video or audio only, quality/height selection, MP4 or MKV output
- Embedded metadata, thumbnail and chapters, SponsorBlock
- Re-encode and trim with ffmpeg (CPU or GPU codecs)
- Transcripts from existing subtitles, offline with Whisper, or with Gemini (API key required)
- `.txt`, `.srt` and `.vtt` output, optional Gemini summary
- Browser cookies, proxy, rate limit
- Italian and English UI, auto-update

## Install

Get the latest build from [Releases](https://github.com/EmanueleFusco/Scaricatore-di-Porto-rust/releases).

| OS | File |
| :-- | :-- |
| Windows 10/11 x64 | `*-setup.exe` |
| macOS 12+ (universal) | `*_universal.dmg` or `*.app.zip` |
| Linux x64 | `*.AppImage`, `*.deb`, `*.rpm` |

Builds are not code-signed or notarized yet, so Windows SmartScreen and macOS
Gatekeeper will warn you the first time. On Windows, click **More info → Run
anyway**. On macOS, try to open the app once, then go to **System Settings →
Privacy & Security** and click **Open Anyway** (on macOS 14 and older,
right-click the app and choose **Open** also works). If macOS says the app is
damaged, see [Troubleshooting](docs/TROUBLESHOOTING.md).
In-app updates on Linux only work with the AppImage; `.deb` and `.rpm` are
updated through your package manager or by downloading the new release.

yt-dlp and QuickJS ship with the app. ffmpeg, Deno and Whisper models are
downloaded on demand when a feature needs them (or picked up from your system
if already installed).

macOS Intel, Windows ARM64 and Linux arm64 builds are built in CI but not
tested by hand.

## Building from source

Requirements:

- Node.js (current LTS)
- Rust stable
- The [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
  (on Linux: WebKitGTK 4.1 and friends)

```sh
npm install
npm run fetch:tools     # downloads the bundled yt-dlp and qjs into src-tauri/binaries/
npm run app             # dev mode (wraps `tauri dev`)
npm run tauri build     # release bundle
```

`npm run app` applies a couple of Linux workarounds for WebKitGTK on Wayland/NVIDIA
(`GDK_BACKEND=x11`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`) only when needed. Plain
`npm run tauri dev` works too.

### Checks

```sh
npm run check
npm test
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Project layout

```
src/                  Svelte + TypeScript frontend
src-tauri/            Rust backend
scripts/              Node helper scripts (tool fetching, release, CI checks)
tools.manifest.json   pinned versions and checksums of external tools
whisper.models.json   Whisper model catalogue
docs/                 spec, progress log, decisions, troubleshooting
```

## Troubleshooting

See [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md). When opening an issue,
attach the output of **Settings → About → Copy diagnostics**.

## Disclaimer

This app was developed with the help of AI tools.

The software is provided "as is", without warranty of any kind. The authors
take no responsibility for how it is used. You are responsible for respecting
the terms of service of the sites you download from and the copyright of the
content.

## License

[MIT](LICENSE)
