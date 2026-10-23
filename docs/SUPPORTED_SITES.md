# Supported sites

Scaricatore Di Porto does not carry a site list of its own. The download engine
is [yt-dlp](https://github.com/yt-dlp/yt-dlp), so the real list is the one yt-dlp
maintains:

- [yt-dlp supported sites](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)
- On your machine: `yt-dlp --list-extractors` (the copy inside the app is in the
  folder shown in **Settings → Tools → Folders**).

If a site is on that list, the app can usually download it. The list changes
with every yt-dlp release; when a site stops working, update yt-dlp first
(**Settings → Tools → Update**).

## YouTube

- Videos, Shorts, playlists and channels.
- Manual subtitles and automatic captions.
- Live streams: from the beginning or from the current moment, depending on
  what the stream exposes.
- Age-restricted, private and members-only videos with cookies from a browser
  or a Netscape cookies file (**Settings → Network & Accounts**).
- "Video + playlist" URLs (`watch?v=…&list=…`) ask whether you want the single
  video or the whole playlist.
- DRM-protected content is not supported.

## Other sites

These are common examples, not a promise that every page works today:

| Site | Typical use |
| :-- | :-- |
| Vimeo | Video pages, some showcases |
| SoundCloud | Tracks and sets |
| Bandcamp | Tracks and albums |
| Twitch | VODs and clips (live support depends on the stream) |
| TikTok | Public videos |
| Instagram | Public posts, Reels, stories where yt-dlp can access them |
| Facebook | Public videos |
| X / Twitter | Public videos |
| Reddit | Videos hosted by Reddit and external players |
| Direct links | Plain HTTP(S) files, HLS (`.m3u8`) and DASH manifests |

The generic extractor also handles many ordinary pages that embed a video. When
it cannot, the app shows the error from yt-dlp instead of guessing.

## Formats

The add bar offers **Best available**, a maximum height (1080p, 720p, …) or
**Audio only**. For anything else, open the advanced format table, pick a row,
or paste an exact yt-dlp format ID. The app merges the video and audio streams
with ffmpeg when needed.

## When a site needs a login

Some sites only show the media to a logged-in session. Two options:

1. **Cookies from a browser** — choose the browser in **Settings → Network &
   Accounts**. Reading the cookie database can fail while the browser is open;
   close it and try again.
2. **Cookies file** — export a Netscape-format `cookies.txt` and choose it in
   the same page.

Cookies are secrets. They are passed to yt-dlp, not written to the app log, and
they are not included in **Copy diagnostics**.

## When a site blocks the app

Sites sometimes answer with an anti-bot page or HTTP 403. Two settings help:

- **Retry blocked requests with Chrome impersonation** (Settings → Network &
  Accounts) retries once with `--impersonate chrome`.
- **JavaScript runtime** — yt-dlp needs a JavaScript interpreter for YouTube's
  challenges. QuickJS is bundled; if it is not enough, the app can install Deno
  with your consent.

## Live streams, DRM and geo-blocking

- **Live** — a live stream may download from the start or from "now"; the app
  follows what the extractor reports. A stream that has just ended may need a
  retry.
- **DRM** — encrypted media (Netflix, Disney+, many paid platforms) is not
  downloadable. The app reports the DRM error rather than saving a broken file.
- **Geo-blocking** — the site refuses based on your network location. A proxy
  can be configured in Settings, but respect the site's terms.
- **Rate limits** — lower **Parallel downloads** or **Max per site** if a site
  starts refusing requests.

## Reporting a site that does not work

Include:

1. the page URL (or a similar public one, if the original is private);
2. the exact error from the item's **Details** panel;
3. the yt-dlp version from **Settings → Tools**;
4. **Settings → About → Copy diagnostics**.

Never paste cookies or API keys into an issue.
