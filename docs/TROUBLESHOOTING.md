# Troubleshooting

Most problems leave a readable message in the item's **Details** panel. Start
there, then open **Settings → About**:

- **Copy diagnostics** gives you a plain-text report with the version, OS, tool
  paths and the last log lines.
- **Open logs** reveals the log file in your file manager.
- **Folders** shows where settings, tools, models and logs live.

The report contains local paths but no API keys or cookies. Read it before
posting it publicly.

---

## Tools

### "ffmpeg is missing" / merging or audio conversion fails

ffmpeg is not bundled because of its size and licence. Install it from
**Settings → Tools → ffmpeg → Install**. The app also installs **ffprobe**,
which is needed to read durations.

If the download fails, use **Use my own path** and point the app at an existing
`ffmpeg` binary. A system package works too: on Debian/Ubuntu
`sudo apt install ffmpeg`, on Fedora `sudo dnf install ffmpeg`, on macOS
`brew install ffmpeg`, on Windows `winget install Gyan.FFmpeg`.

### "sdp-whisper sidecar is not installed" / Tools says "Not available yet"

Offline transcription needs the `sdp-whisper` sidecar and a Whisper model. The
sidecar is published separately from the app on the project's Releases page;
the Tier 1 builds are available for Linux x64, Windows x64 and macOS. Install it
from **Settings → Tools → sdp-whisper → Install**.

If the download fails, check the network or proxy and retry. If the Tools page
says **Not available yet**, that build has no published asset for your platform
or CPU architecture: use **Use my own path** with a compatible `sdp-whisper`
binary, or use Gemini for transcription.

A missing **model** is a different problem: download one from **Settings →
Transcription & Models**.

### yt-dlp fails, shows no formats, or returns HTTP 403

1. **Update yt-dlp** in **Settings → Tools**. Sites change constantly, and most
   extraction failures are fixed by an update.
2. Try the **nightly** channel if the stable release is behind.
3. If the message mentions a **JavaScript challenge**, keep the runtime on
   **Automatic** or install **Deno** when the app offers it.
4. If the site asks for a login, configure **cookies** (see below).
5. If the site blocks automated requests, enable **Retry blocked requests with
   Chrome impersonation**.

### "The browser cookies could not be read"

Close the browser completely and try again: browsers lock their cookie database
while they run. If it still fails, export a Netscape-format `cookies.txt` and
choose it under **Settings → Network & Accounts → Cookies file**.

Cookies expire. If a private video suddenly fails, export them again.

### A download is stuck at "Waiting for data"

This usually means the network stalled or the site is throttling the request.
Cancel and retry the item. If it repeats, lower **Parallel downloads** or
**Max per site** in Settings, or set a proxy.

### The queue says "Interrupted"

The app closed or crashed while the item was running. Press **Resume**. If the
partial file is unusable, remove the item and add it again; finished files are
never deleted.

---

## Transcription

### "No subtitles were found"

The video has no manual or automatic subtitles in the requested languages.
Choose an engine order that includes Whisper or Gemini, or change the default
language. **Subtitles only** intentionally fails when there are no subtitles.

### Whisper is slow or runs out of memory

Whisper runs on the CPU. A smaller model is faster and uses less RAM; **Small**
is a reasonable choice on 4-core machines, **Turbo** is the best quality/size
trade-off on modern hardware. Close other memory-heavy applications, and lower
**CPU threads** only if the rest of the system becomes unresponsive.

### The transcript has repeated lines or hallucinations

Enable **Skip silence with Silero VAD** (it is on by default when the VAD model
is installed). Long silences are the usual cause of repeated text. Very noisy
audio can still produce errors; try a larger model.

### Gemini: "API key is invalid or unauthorized"

- Check that the key is a Gemini API key from Google AI Studio, not an OAuth
  client secret.
- Press **Test key** in **Settings → Transcription & Models**.
- If the key worked before, it may have been revoked or the project may have no
  quota left.

### Gemini: "Model … is no longer available"

Google retires model names. Open **Settings → Transcription & Models**, press
**Refresh models**, and choose a current `-latest` alias (for example
`gemini-flash-latest`).

### Gemini: rate limit or quota exceeded

Wait and retry, or check the quota of the Google Cloud project behind the key.
The app retries HTTP 429 with backoff, but it cannot create quota.

### Where is my API key stored?

In the OS keyring. On Linux without a Secret Service daemon, the app falls back
to a file with mode `0600` in the app config folder. The key is never sent to
the webview and never written to the log. The `GEMINI_API_KEY` environment
variable also works and takes priority.

### Summaries fail

Summaries use Gemini too. A summary needs a configured key and a transcript;
if the transcript came from subtitles, that is enough. For very long
transcripts the app summarizes in chunks and then combines them, which can take
several requests.

---

## Updates

### "Check for updates" fails

The check contacts the GitHub release endpoint. A proxy, a firewall or a GitHub
outage can break it. Downloads and the rest of the app keep working. Try again
later.

### Linux: the app says an update is available but cannot install it

Only the **AppImage** can replace itself. If you installed the `.deb` or
`.rpm`, update it with your package manager, or download the new release from
the Releases page. The About page shows the releases link in that case.

### The update downloads but the version does not change

On Windows the installer may need the app to exit; the app asks you to restart.
On Linux, make sure you are running the AppImage from a writable location and
that the file was not replaced by a package manager. If the signature check
fails, the update is rejected on purpose: download the release manually.

### Daily checks

The app checks at most once every 24 hours and never installs anything by
itself. The timestamp is local UI state, not a setting.

---

## Platform issues

### Linux: the window is black or crashes on Wayland

Some WebKitGTK/GPU combinations cannot allocate DMA-BUF buffers. From this
version the app disables WebKitGTK's DMA-BUF renderer itself when it detects a
Wayland session on Linux, so the common case no longer needs environment
variables. If it still crashes, launch with:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 ./Scaricatore*.AppImage
```

If the window still crashes, force XWayland:

```bash
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 ./Scaricatore*.AppImage
```

For older builds, `npm run app` applies both workarounds on Wayland. Installed
packages may still need the variables in the launcher or desktop entry.

### Linux: the folder picker or "Open file" does nothing

These use the desktop portal / `xdg-open`. Install `xdg-utils` and a portal
implementation for your desktop (for example `xdg-desktop-portal-gtk` or
`xdg-desktop-portal-kde`).

### Linux: the keyring is unavailable

The app stores the Gemini key in a `0600` file in the app config folder and says
so in Settings. This is expected on minimal window managers, headless sessions
and some containers.

### macOS: "The app is damaged" or Gatekeeper refuses to open it

The current builds are not notarized. First try right-click → **Open**. If macOS
still refuses, verify the download and remove the quarantine attribute:

```bash
xattr -dr com.apple.quarantine "/Applications/Scaricatore Di Porto.app"
```

Only do this for a file you downloaded from the official Releases page.

### Windows: SmartScreen or antivirus warns about the installer

The installer is not code-signed. SmartScreen shows **More info → Run anyway**.
Some antivirus products also flag the bundled `yt-dlp.exe`; it is the official
yt-dlp binary, verified against the release checksum before it is bundled.

### No notification when the queue finishes

Enable **Notify me when the queue finishes** in Settings, and allow
notifications for the app in the OS settings. On Linux, a notification daemon
must be running.

---

## Files and disk

### Where did my file go?

Open the item's **Details** panel: it shows the final path. The default folder
is the system Downloads folder unless you chose another one in Settings or in
the add bar. Playlists can use a folder per playlist.

### "Insufficient disk space"

Free some space and retry. Whisper models are large (the recommended Turbo
model is about 547 MB), and a transcription may temporarily need room for a WAV
and chunks. The models table shows the free space before a download.

### I want to start over

- Settings: **Settings → About → Folders** shows the settings path. Close the
  app before editing or deleting it; a corrupt file is backed up and replaced
  with defaults on the next start.
- Downloads archive: **Settings → General → Clear archive**.
- Models and tools: delete them from their rows in **Settings → Transcription**
  and **Settings → Tools**, or remove the folders shown in About.
- Logs: **Open logs**, then delete the file while the app is closed.

---

## Reporting a bug

Please include:

1. what you did, what you expected, what happened;
2. the exact error message from the item's **Details** panel;
3. the app version and OS from **Settings → About**;
4. **Copy diagnostics**;
5. a few relevant lines from **Open logs**.

Do not include API keys, cookies or private URLs. If a URL is needed to
reproduce the problem, use a public one or describe the site.
