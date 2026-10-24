import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { t } from "$lib/i18n";
import {
  DEFAULT_SETTINGS,
  isLangPreference,
  isThemePreference,
  normaliseSettings,
  sanitizePlayerClients,
  type Settings,
} from "$lib/settings";

describe("normaliseSettings()", () => {
  it("returns the defaults for a missing document", () => {
    expect(normaliseSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(normaliseSettings(undefined)).toEqual(DEFAULT_SETTINGS);
    expect(normaliseSettings({})).toEqual(DEFAULT_SETTINGS);
  });

  it("keeps valid values", () => {
    const raw: Settings = {
      ...DEFAULT_SETTINGS,
      schemaVersion: 2,
      language: "en",
      theme: "dark",
      defaultOutputDir: "/out",
      toolPaths: { "yt-dlp": "/opt/mine/yt-dlp" },
      ytdlpChannel: "nightly",
      setupWizardCompleted: true,
    };
    expect(normaliseSettings(raw)).toEqual(raw);
  });

  it("drops a tool override that is not a usable path", () => {
    // Mirrors the Rust-side normalise(): an override that is not a path would be
    // refused by the process layer (spec §4.8), so showing it as if it worked is
    // worse than not showing it.
    expect(
      normaliseSettings({
        toolPaths: { "yt-dlp": "/opt/mine/yt-dlp", ffmpeg: "", "": "/x" },
      }).toolPaths,
    ).toEqual({ "yt-dlp": "/opt/mine/yt-dlp" });
  });

  it("falls back to the stable channel and to an unconfigured wizard", () => {
    const out = normaliseSettings({
      ytdlpChannel: "weekly" as never,
      setupWizardCompleted: undefined,
    });
    expect(out.ytdlpChannel).toBe("stable");
    expect(out.setupWizardCompleted).toBe(false);
  });

  it("replaces an unknown language or theme with the default", () => {
    const out = normaliseSettings({
      language: "klingon" as never,
      theme: "ultraviolet" as never,
    });
    expect(out.language).toBe("system");
    expect(out.theme).toBe("system");
  });

  it("treats an empty output folder as unset", () => {
    expect(normaliseSettings({ defaultOutputDir: "" }).defaultOutputDir).toBeNull();
  });

  it("treats an empty expert client override as unset (D74)", () => {
    // Empty is the default and the recommended state: yt-dlp's own client list.
    expect(normaliseSettings({ youtubePlayerClients: "" }).youtubePlayerClients).toBeNull();
    expect(
      normaliseSettings({ youtubePlayerClients: "web_embedded,web,tv" }).youtubePlayerClients,
    ).toBe("web_embedded,web,tv");
  });
});

describe("M3 settings", () => {
  it("defaults the playlist behaviour to ask and the archive on for playlists", () => {
    const out = normaliseSettings({ playlistUrlBehavior: "bogus" as never, customHeaders: [" X: 1 ", "", 3 as never] });
    expect(out.playlistUrlBehavior).toBe("ask");
    expect(out.useDownloadArchive).toBe(true);
    expect(normaliseSettings({ useDownloadArchive: false }).useDownloadArchive).toBe(false);
    expect(out.includeIdInFilename).toBe(true);
    expect(out.impersonateRetry).toBe(true);
    expect(out.customHeaders).toEqual(["X: 1"]);
    expect(normaliseSettings({ proxyUrl: "  " }).proxyUrl).toBeNull();
  });
});

describe("M4 settings", () => {
  it("defaults defaultAudioCodec to aac and preserves it in normalisation", () => {
    expect(DEFAULT_SETTINGS.defaultAudioCodec).toBe("aac");
    expect(normaliseSettings({}).defaultAudioCodec).toBe("aac");
    expect(normaliseSettings({ defaultAudioCodec: "mp3" }).defaultAudioCodec).toBe("mp3");
    expect(normaliseSettings({ defaultAudioCodec: "  " }).defaultAudioCodec).toBe("aac");
  });
});

describe("M5-A settings", () => {
  it("defaults transcription settings and preserves them in normalisation", () => {
    expect(DEFAULT_SETTINGS.activeWhisperModel).toBe("large-v3-turbo-q5_0");
    expect(DEFAULT_SETTINGS.skipSilenceVad).toBe(true);
    expect(DEFAULT_SETTINGS.whisperThreads).toBe(0);
    expect(DEFAULT_SETTINGS.whisperTranslateToEnglish).toBe(false);
    expect(DEFAULT_SETTINGS.defaultTranscriptLanguage).toBe("auto");
    expect(DEFAULT_SETTINGS.transcriptionEngineOrder).toBe("subtitles_whisper_gemini");
    expect(DEFAULT_SETTINGS.transcriptsSubfolder).toBe(false);

    const normalised = normaliseSettings({
      activeWhisperModel: "small-q5_1",
      whisperModelsMirrorUrl: "https://hf-mirror.com/models",
      skipSilenceVad: false,
      whisperThreads: 4,
      whisperTranslateToEnglish: true,
      defaultTranscriptLanguage: "IT",
      transcriptionEngineOrder: "whisper_only",
      transcriptsSubfolder: true,
    });
    expect(normalised.activeWhisperModel).toBe("small-q5_1");
    expect(normalised.whisperModelsMirrorUrl).toBe("https://hf-mirror.com/models");
    expect(normalised.skipSilenceVad).toBe(false);
    expect(normalised.whisperThreads).toBe(4);
    expect(normalised.whisperTranslateToEnglish).toBe(true);
    expect(normalised.defaultTranscriptLanguage).toBe("it");
    expect(normalised.transcriptionEngineOrder).toBe("whisper_only");
    expect(normalised.transcriptsSubfolder).toBe(true);
  });
});

describe("sanitizePlayerClients() (spec §5.3.7, D74)", () => {
  it("normalises a real client list", () => {
    expect(sanitizePlayerClients("  WEB_EMBEDDED,web,Tv ")).toBe("web_embedded,web,tv");
    expect(sanitizePlayerClients("-tv,web")).toBe("-tv,web");
  });

  it("refuses anything yt-dlp would not accept; empty means default", () => {
    expect(sanitizePlayerClients("")).toBeNull();
    expect(sanitizePlayerClients("   ")).toBeNull();
    expect(sanitizePlayerClients("web --exec")).toBeNull();
    expect(sanitizePlayerClients("web;rm")).toBeNull();
  });
});

describe("type guards", () => {
  it("recognise exactly the supported values", () => {
    expect(isLangPreference("system")).toBe(true);
    expect(isLangPreference("it")).toBe(true);
    expect(isLangPreference("en")).toBe(true);
    expect(isLangPreference("fr")).toBe(false);

    expect(isThemePreference("system")).toBe(true);
    expect(isThemePreference("dark")).toBe(true);
    expect(isThemePreference("light")).toBe(true);
    expect(isThemePreference("sepia")).toBe(false);
  });
});

describe("Gemini model selector and 404 handling (spec §10.3 item 6, §11.6, M7)", () => {
  it("formats actionable error message with retired model name and suggestion", () => {
    expect(t("en", "error.ytdlp.geminiModelNotFound", { model: "gemini-2.5-flash" })).toBe(
      "Model gemini-2.5-flash is no longer available on Gemini. Select another model (for example gemini-flash-latest) in Settings → Transcription.",
    );
  });

  it("verifies TranscriptionSettings uses an editable combobox and QueueItem has settings action", () => {
    const transcriptionSrc = readFileSync(
      join(__dirname, "pages/settings/TranscriptionSettings.svelte"),
      "utf-8",
    );
    expect(transcriptionSrc).not.toContain('<select id="setting-gemini-model"');
    expect(transcriptionSrc).toContain('list="gemini-models-list"');
    expect(transcriptionSrc).toContain('<datalist id="gemini-models-list"');

    const queueItemSrc = readFileSync(
      join(__dirname, "components/QueueItem.svelte"),
      "utf-8",
    );
    expect(queueItemSrc).toContain("openTranscriptionSettings");
  });
});

describe("Gemini privacy and cost notice (spec §10.3 item 8, §10.4, M11)", () => {
  it("renders summary privacy hint in TranscriptionSettings and gates first-use Gemini/summary notice on geminiPrivacyHintDismissed", () => {
    expect(t("en", "settings.transcription.geminiSummaryPrivacyHint")).toContain("Gemini API");
    expect(t("it", "settings.transcription.geminiSummaryPrivacyHint")).toContain("API Gemini");

    const transcriptionSrc = readFileSync(
      join(__dirname, "pages/settings/TranscriptionSettings.svelte"),
      "utf-8",
    );
    expect(transcriptionSrc).toContain("settings.transcription.geminiSummaryPrivacyHint");

    const addBarSrc = readFileSync(
      join(__dirname, "components/AddBar.svelte"),
      "utf-8",
    );
    expect(addBarSrc).toContain("geminiPrivacyHintDismissed");
    expect(addBarSrc).toContain("settings.transcription.geminiSummaryPrivacyHint");

    const queueItemSrc = readFileSync(
      join(__dirname, "components/QueueItem.svelte"),
      "utf-8",
    );
    expect(queueItemSrc).toContain("geminiPrivacyHintDismissed");
  });
});

describe("No hard-coded English strings in TranscriptionSettings and engine (m1)", () => {
  it("uses i18n keys for show/hide key aria-labels in TranscriptionSettings", () => {
    const transcriptionSrc = readFileSync(
      join(__dirname, "pages/settings/TranscriptionSettings.svelte"),
      "utf-8",
    );
    expect(transcriptionSrc).not.toContain('"Hide key"');
    expect(transcriptionSrc).not.toContain('"Show key"');
    expect(transcriptionSrc).toContain("transcription.gemini.hideKey");
    expect(transcriptionSrc).toContain("transcription.gemini.showKey");
  });

  it("provides translations for queue.status.generatingSummary and gemini key aria-labels in en and it", () => {
    expect(t("en", "transcription.gemini.showKey")).toBe("Show key");
    expect(t("en", "transcription.gemini.hideKey")).toBe("Hide key");
    expect(t("it", "transcription.gemini.showKey")).toBe("Mostra chiave");
    expect(t("it", "transcription.gemini.hideKey")).toBe("Nascondi chiave");
    expect(t("en", "queue.status.generatingSummary")).toBe("Generating summary…");
    expect(t("it", "queue.status.generatingSummary")).toBe("Generazione riassunto…");
  });
});

