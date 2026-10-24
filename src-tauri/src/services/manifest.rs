//! The tools manifest: the single source of truth for external tools
//! (spec §5.1).
//!
//! `tools.manifest.json` lives at the repository root, is **tracked in git** and
//! is compiled into the binary with [`include_str!`]. Two consequences, both
//! wanted:
//!
//! * the backend never has to guess where the file is at runtime. On Linux the
//!   bundled tree can be a read-only AppImage mount, on macOS it is inside the
//!   `.app`, on Windows under `%LOCALAPPDATA%` — three different answers, and a
//!   lookup that picks the wrong one silently breaks tool provisioning;
//! * the manifest travels with the app, so an app update can refresh it "with no
//!   code change needed for new tool versions" (spec §5.1). Downloading a new
//!   `deno` version is a manifest edit, not a release.
//!
//! [`scripts/fetch_bundled_tools.mjs`](../../../../scripts/fetch_bundled_tools.mjs)
//! reads the *same* file before the bundle is built, so the two can never
//! disagree about an asset name.
//!
//! Nothing here performs IO or spawns anything: parsing, validation and the
//! version rules are pure and unit-tested.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The manifest, compiled in. Bump the schema only together with a migration.
pub const MANIFEST_JSON: &str = include_str!("../../../tools.manifest.json");

/// Where the manifest lives in the repository, for messages and the script.
pub const MANIFEST_FILE: &str = "tools.manifest.json";

/// Only schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Tool name as the spec spells it everywhere.
pub const YTDLP: &str = "yt-dlp";
/// QuickJS-NG's interpreter. yt-dlp requires exactly this name (spec §5.2.1).
pub const QJS: &str = "qjs";
/// Deno, the lazy fallback runtime (spec §5.3).
pub const DENO: &str = "deno";
/// Whisper sidecar CLI (spec §5.4).
pub const SDP_WHISPER: &str = "sdp-whisper";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How a tool picks its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Versioning {
    /// Always the newest release. `url`/`sha256` stay `null` and the checksum
    /// comes from the release's own `SHA2-256SUMS`. **This is yt-dlp's mode**
    /// (spec §5.1: "yt-dlp is always 'latest'").
    Latest,
    /// One exact release, pinned in the manifest with its checksum.
    Pinned,
}

/// What has to be unwrapped before the tool is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveKind {
    /// The asset is the executable itself (every bundled tool).
    None,
    Zip,
    Tar,
    TarGz,
    /// What BtbN publishes for Linux ffmpeg builds.
    TarXz,
    /// A single executable compressed with gzip, as `ffmpeg-static` publishes.
    Gz,
}

impl ArchiveKind {
    pub const fn needs_extraction(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// One tool/platform pair: everything needed to fetch and install it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlatformEntry {
    /// File name of the asset inside the release, `null` when the binary is
    /// assembled locally from other platforms (the universal macOS `qjs`).
    pub asset: Option<String>,
    /// What the file must be called once installed. `yt-dlp` and `qjs` are
    /// plain names; Windows gets the `.exe`.
    pub executable_name: String,
    pub archive_kind: ArchiveKind,
    /// Absolute `https://` URL, `null` in [`Versioning::Latest`] mode.
    pub url: Option<String>,
    /// Lowercase hex sha256, `null` in [`Versioning::Latest`] mode.
    pub sha256: Option<String>,
    /// Platforms this binary is built from (the macOS universal `qjs`).
    pub built_from: Vec<String>,
}

impl Default for PlatformEntry {
    fn default() -> Self {
        Self {
            asset: None,
            executable_name: String::new(),
            archive_kind: ArchiveKind::None,
            url: None,
            sha256: None,
            built_from: Vec::new(),
        }
    }
}

impl PlatformEntry {
    /// `executableName`, or `null` when the entry is unusable. A `null` here is
    /// what makes an un-pinned tool refuse to install instead of downloading
    /// something arbitrary.
    pub fn executable(&self) -> Option<&str> {
        let name = self.executable_name.as_str();
        if name.is_empty() { None } else { Some(name) }
    }

    /// Can this entry actually be fetched right now? False means "we know the
    /// shape but have not pinned a release yet" (see `deno` in the manifest).
    pub fn is_installable(&self) -> bool {
        if !self.built_from.is_empty() {
            return false;
        }
        match (&self.url, &self.sha256) {
            (Some(url), Some(sum)) => url.starts_with("https://") && is_sha256(sum),
            _ => false,
        }
    }
}

/// One tool, across every platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolEntry {
    pub name: String,
    /// Shown in the Tools page; never parsed.
    pub role: String,
    /// Shipped inside the installer (spec §0.7: **only** yt-dlp and qjs).
    pub bundled: bool,
    pub license: String,
    pub license_url: Option<String>,
    pub homepage: Option<String>,
    pub versioning: Versioning,
    /// `https://` endpoint returning the newest release. Present only in
    /// [`Versioning::Latest`] mode; resolving the concrete tag there is what
    /// lets a build record *which* version it shipped (spec §5.2.1).
    pub release_api: Option<String>,
    /// `{asset}` is replaced with the platform's asset name. yt-dlp only.
    pub release_url: Option<String>,
    /// Where the authoritative checksums live, when they are not in the manifest.
    pub checksum_url: Option<String>,
    /// `None` in [`Versioning::Latest`] mode.
    pub version: Option<String>,
    /// Refuse anything older (spec §5.1).
    pub min_version: String,
    pub platforms: BTreeMap<String, PlatformEntry>,
}

impl Default for ToolEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            role: String::new(),
            bundled: false,
            license: String::new(),
            license_url: None,
            homepage: None,
            versioning: Versioning::Pinned,
            release_api: None,
            release_url: None,
            checksum_url: None,
            version: None,
            min_version: String::new(),
            platforms: BTreeMap::new(),
        }
    }
}

/// The whole document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolsManifest {
    pub schema_version: u32,
    pub tools: Vec<ToolEntry>,
}

impl Default for ToolsManifest {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tools: Vec::new(),
        }
    }
}

impl ToolsManifest {
    /// Parse and validate the manifest that was compiled into this binary.
    ///
    /// A malformed or inconsistent manifest is a build-time mistake, not a
    /// runtime condition, so this returns the collected problems instead of
    /// panicking and `cargo test` fails loudly (see the tests at the bottom).
    pub fn embedded() -> Result<Self, ManifestError> {
        Self::parse(MANIFEST_JSON).map_err(|mut err| {
            err.problems.push(format!("while reading {MANIFEST_FILE}"));
            err
        })
    }

    /// Parse a manifest from text.
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let manifest: ToolsManifest =
            serde_json::from_str(raw).map_err(|err| ManifestError::new(vec![err.to_string()]))?;
        let problems = manifest.problems();
        if problems.is_empty() {
            Ok(manifest)
        } else {
            Err(ManifestError::new(problems))
        }
    }

    /// Everything structurally wrong with this manifest, in reading order.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.schema_version != SCHEMA_VERSION {
            problems.push(format!(
                "schemaVersion is {}, this build understands {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.tools.is_empty() {
            problems.push("the manifest declares no tools".to_owned());
        }

        let mut seen_names = Vec::new();
        for tool in &self.tools {
            if tool.name.is_empty() {
                problems.push("a tool entry has an empty name".to_owned());
                continue;
            }
            if seen_names.contains(&tool.name.as_str()) {
                problems.push(format!("the tool {:?} is declared twice", tool.name));
            }
            seen_names.push(tool.name.as_str());

            problems.extend(tool.problems());
        }
        problems
    }

    pub fn tool(&self, name: &str) -> Option<&ToolEntry> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// The entry for `name` on `target`, if this tool exists on that platform at
    /// all. `None` is not an error: spec §1.1 asks for "not available on this
    /// platform" rather than a silent failure.
    pub fn entry(&self, name: &str, target: &str) -> Option<&PlatformEntry> {
        self.tool(name)?.platforms.get(target)
    }

    /// Every tool shipped in the installer, in manifest order.
    pub fn bundled_tools(&self) -> impl Iterator<Item = &ToolEntry> {
        self.tools.iter().filter(|tool| tool.bundled)
    }

    /// True when the tool has at least one entry for `target`.
    pub fn supports(&self, name: &str, target: &str) -> bool {
        self.entry(name, target).is_some()
    }
}

impl ToolEntry {
    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let tool = &self.name;

        if self.role.is_empty() {
            problems.push(format!("{tool}: role is empty"));
        }
        if self.license.is_empty() {
            problems.push(format!(
                "{tool}: license is empty (spec §5.2.1 ships notices)"
            ));
        }
        if parse_version(&self.min_version).is_none() {
            problems.push(format!(
                "{tool}: minVersion {:?} is not a dotted numeric version",
                self.min_version
            ));
        }

        match self.versioning {
            Versioning::Latest => {
                if self.version.is_some() {
                    problems.push(format!(
                        "{tool}: versioning is \"latest\" so version must be absent"
                    ));
                }
                if self
                    .release_url
                    .as_deref()
                    .is_none_or(|url| !url.starts_with("https://"))
                {
                    problems.push(format!(
                        "{tool}: versioning is \"latest\" so releaseUrl must be an https template"
                    ));
                }
                if self
                    .checksum_url
                    .as_deref()
                    .is_none_or(|url| !url.starts_with("https://"))
                {
                    problems.push(format!(
                        "{tool}: versioning is \"latest\" so checksumUrl must point at the \
                         release's own sum file"
                    ));
                }
                if self
                    .release_api
                    .as_deref()
                    .is_some_and(|url| !url.starts_with("https://"))
                {
                    problems.push(format!("{tool}: releaseApi must be https"));
                }
            }
            Versioning::Pinned => {
                if self.version.is_none() {
                    problems.push(format!(
                        "{tool}: versioning is \"pinned\" so version must be filled in"
                    ));
                }
                if self.checksum_url.is_some() {
                    problems.push(format!(
                        "{tool}: versioning is \"pinned\" so checksumUrl must be absent \
                         (the sha256 belongs in the manifest)"
                    ));
                }
            }
        }

        if self.platforms.is_empty() {
            problems.push(format!("{tool}: no platforms"));
        }
        for (target, entry) in &self.platforms {
            problems.extend(platform_problems(&format!("{tool}/{target}"), entry));
        }
        problems
    }
}

/// Structural rules for one platform entry, labelled `tool/target`.
fn platform_problems(label: &str, entry: &PlatformEntry) -> Vec<String> {
    let mut problems = Vec::new();

    if entry.executable().is_none() {
        problems.push(format!("{label}: executableName is empty"));
    }
    match (entry.asset.as_deref(), entry.built_from.is_empty()) {
        (None, true) => problems.push(format!("{label}: neither asset nor builtFrom")),
        (Some(_), false) => problems.push(format!(
            "{label}: builtFrom and asset are mutually exclusive"
        )),
        _ => {}
    }
    if let Some(asset) = &entry.asset {
        if !is_safe_asset_name(asset) {
            problems.push(format!(
                "{label}: asset {asset:?} is not a plain file name (no separator, no \"..\")"
            ));
        }
    }
    if entry.url.is_some() != entry.sha256.is_some() {
        problems.push(format!(
            "{label}: url and sha256 must both be set or both be absent"
        ));
    }
    if let Some(url) = &entry.url {
        if !url.starts_with("https://") {
            problems.push(format!("{label}: url must be https, got {url:?}"));
        }
    }
    if let Some(sum) = &entry.sha256 {
        if !is_sha256(sum) {
            problems.push(format!(
                "{label}: sha256 must be 64 lowercase hex characters"
            ));
        }
    }
    problems
}

/// Parse failure with every problem at once: fixing a manifest one error per
/// compile round-trip is not productive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub problems: Vec<String>,
}

impl ManifestError {
    fn new(problems: Vec<String>) -> Self {
        Self { problems }
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            formatter,
            "{} problem(s) in the tools manifest:",
            self.problems.len()
        )?;
        for problem in &self.problems {
            writeln!(formatter, "  - {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ManifestError {}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// A plain file name: no separator, no `..`, not empty, no NUL.
///
/// Checked on everything that comes out of the manifest before it can reach a
/// path join, because a manifest is data and data can eventually be remote
/// (spec §7: "the manifest can be refreshed by the app's own update").
pub fn is_safe_asset_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
        && !name.contains('\0')
        && !name.starts_with('.')
}

/// 64 lowercase hex characters.
pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Fill `{asset}` in a release URL template.
///
/// Returns `None` for a template without the placeholder: without it the URL
/// would silently point at a directory listing.
pub fn resolve_asset_url(template: &str, asset: &str) -> Option<String> {
    if !template.contains("{asset}") || !is_safe_asset_name(asset) {
        return None;
    }
    Some(template.replace("{asset}", asset))
}

/// Split a dotted version into its numeric parts.
///
/// Handles `2026.08.19` (yt-dlp's calendar versions) and `0.17.0` (qjs) with
/// the same rule, because both are dotted and numeric. Returns `None` for
/// anything else, including `1.2.3-beta`, rather than guessing: a version we
/// cannot compare must not silently pass a `minVersion` check.
pub fn parse_version(version: &str) -> Option<Vec<u64>> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                part.parse::<u64>().ok()
            }
        })
        .collect()
}

/// `Less` / `Equal` / `Greater` for dotted numeric versions.
///
/// A shorter version is the prefix of a longer one, so `1.2 < 1.2.1` — the same
/// rule `strcmp`-style version schemes use.
pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let left = parse_version(left)?;
    let right = parse_version(right)?;
    let length = left.len().max(right.len());
    for index in 0..length {
        // A missing component counts as 0: `1.2` == `1.2.0`.
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

/// Is `found` new enough for `min_version`?
///
/// An unparseable `found` (a nightly build, a wrapper script printing garbage)
/// answers **false**: refusing an unknown version is safe, accepting it would
/// hand `--js-runtimes` to a yt-dlp that predates the flag.
pub fn satisfies_min_version(found: &str, min_version: &str) -> bool {
    matches!(
        compare_versions(found, min_version),
        Some(Ordering::Greater | Ordering::Equal)
    )
}

/// The target triple this binary was compiled for.
///
/// Uses Tauri's own resolution (`tauri::utils::platform::target_triple`), the
/// same one the bundler uses to pick the `-<triple>` suffixed binaries.
pub fn current_target_triple() -> Result<String, String> {
    tauri::utils::platform::target_triple()
        .map_err(|err| format!("cannot determine the target triple of this build: {err}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_shipped_in_the_repository_is_valid() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
        assert!(!manifest.tools.is_empty());
    }

    #[test]
    fn yt_dlp_and_qjs_are_bundled_and_nothing_else_is() {
        // Spec §0.7: "the installer includes yt-dlp and qjs" and §5.2.1 "never
        // bundle anything else".
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let bundled: Vec<&str> = manifest
            .bundled_tools()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(bundled, vec![YTDLP, QJS]);
    }

    #[test]
    fn yt_dlp_is_always_latest_and_qjs_is_pinned() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));

        let ytdlp = manifest.tool(YTDLP).expect("yt-dlp is in the manifest");
        assert_eq!(ytdlp.versioning, Versioning::Latest);
        assert_eq!(ytdlp.version, None, "yt-dlp has no pinned version");
        assert!(
            ytdlp.checksum_url.is_some(),
            "checksums come from the release"
        );

        let qjs = manifest.tool(QJS).expect("qjs is in the manifest");
        assert_eq!(qjs.versioning, Versioning::Pinned);
        assert_eq!(qjs.version.as_deref(), Some("0.17.0"));
        assert_eq!(qjs.checksum_url, None);
    }

    #[test]
    fn every_tier_one_target_has_both_bundled_tools() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        for triple in [
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "universal-apple-darwin",
        ] {
            assert!(manifest.supports(YTDLP, triple), "yt-dlp on {triple}");
            assert!(manifest.supports(QJS, triple), "qjs on {triple}");
        }
    }

    #[test]
    fn qjs_is_called_qjs_on_every_platform_and_yt_dlp_exe_on_windows() {
        // yt-dlp looks the interpreter up by name (spec §5.2.1).
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        for (triple, entry) in &manifest.tool(QJS).expect("qjs").platforms {
            let name = entry.executable().expect("executable name");
            assert_eq!(
                name,
                if triple.contains("windows") {
                    "qjs.exe"
                } else {
                    "qjs"
                },
                "{triple}"
            );
        }
        for (triple, entry) in &manifest.tool(YTDLP).expect("yt-dlp").platforms {
            let name = entry.executable().expect("executable name");
            assert_eq!(
                name,
                if triple.contains("windows") {
                    "yt-dlp.exe"
                } else {
                    "yt-dlp"
                },
                "{triple}"
            );
        }
    }

    #[test]
    fn the_universal_macos_qjs_is_a_lipo_of_the_two_native_slices() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let qjs = manifest.tool(QJS).expect("qjs");
        let universal = manifest
            .entry(QJS, "universal-apple-darwin")
            .expect("universal");
        assert_eq!(
            universal.built_from,
            vec!["aarch64-apple-darwin", "x86_64-apple-darwin"]
        );
        assert_eq!(universal.asset, None);
        assert!(
            !universal.is_installable(),
            "a locally assembled binary is not downloaded by the app"
        );
        for slice in &universal.built_from {
            assert!(
                qjs.platforms.contains_key(slice),
                "{slice} must be declared"
            );
        }
    }

    #[test]
    fn a_pinned_entry_is_installable_and_an_unfinished_one_is_not() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let linux_qjs = manifest
            .entry(QJS, "x86_64-unknown-linux-gnu")
            .expect("qjs/linux");
        assert!(linux_qjs.is_installable());

        // A tool declared but not yet pinned must answer "not installable", so
        // the UI can still offer "use the one on my system" and nothing worse.
        // This is the shape `deno` will take in M1 (spec §5.3 makes it a lazy
        // download); it is not in the manifest yet precisely because it cannot
        // be installed today.
        let raw = r#"{
            "schemaVersion": 1,
            "tools": [{
                "name": "later", "role": "r", "license": "MIT", "versioning": "pinned",
                "version": "1.0.0", "minVersion": "1.0",
                "platforms": {"x86_64-unknown-linux-gnu": {
                    "asset": "later", "executableName": "later", "archiveKind": "none"}}
            }]
        }"#;
        let pending = ToolsManifest::parse(raw).expect("must load");
        let entry = pending
            .entry("later", "x86_64-unknown-linux-gnu")
            .expect("entry");
        assert!(!entry.is_installable());
    }

    #[test]
    fn the_manifest_carries_every_on_demand_tool() {
        // Spec §0.7, §5.4: bundled: only yt-dlp and qjs ship in the installer;
        // Deno, ffmpeg/ffprobe and sdp-whisper are downloaded on demand.
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let names: Vec<&str> = manifest.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec![YTDLP, DENO, QJS, "ffmpeg", "ffprobe", SDP_WHISPER]
        );

        let whisper = manifest
            .tool(SDP_WHISPER)
            .expect("sdp-whisper is in the manifest");
        assert!(
            !whisper.bundled,
            "sdp-whisper is downloaded on demand, never bundled"
        );
        assert_eq!(whisper.versioning, Versioning::Pinned);
        assert_eq!(whisper.version.as_deref(), Some("1.9.5"));

        let whisper_entry = manifest
            .entry(SDP_WHISPER, "x86_64-unknown-linux-gnu")
            .expect("a linux x64 sdp-whisper asset");
        assert_eq!(whisper_entry.archive_kind, ArchiveKind::None);
        assert!(
            whisper_entry.is_installable(),
            "the published whisper-v1.9.5 assets must be installable"
        );

        let deno = manifest.tool(DENO).expect("deno is in the manifest");
        assert!(!deno.bundled, "Deno is downloaded on demand, never bundled");
        assert_eq!(deno.versioning, Versioning::Pinned);
        assert_eq!(deno.version.as_deref(), Some("2.9.7"));

        let entry = manifest
            .entry(DENO, "x86_64-unknown-linux-gnu")
            .expect("a linux x64 deno asset");
        assert_eq!(entry.archive_kind, ArchiveKind::Zip);
        assert!(
            entry.is_installable(),
            "deno must carry url + sha256 so the lazy install works"
        );
    }

    #[test]
    fn every_deno_asset_is_marked_as_a_zip_with_a_checksum_where_pinned() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let deno = manifest.tool(DENO).expect("deno");
        assert!(!deno.platforms.is_empty());
        for (triple, entry) in &deno.platforms {
            assert_eq!(entry.archive_kind, ArchiveKind::Zip, "{triple}");
            assert!(entry.url.is_some() && entry.sha256.is_some(), "{triple}");
            assert!(
                entry.is_installable(),
                "{triple} must be installable, not a half-declared entry"
            );
        }
    }

    #[test]
    fn whisper_sidecar_assets_are_installable_and_use_the_published_release() {
        // The whisper-v1.9.5 release was published on 2026-10-19. Every
        // platform entry must point at an asset of that release and carry the
        // matching SHA-256, otherwise the Install button would be dead.
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let whisper = manifest.tool(SDP_WHISPER).expect("sdp-whisper");
        let template = whisper.release_url.as_deref().expect("releaseUrl");
        assert_eq!(
            template,
            "https://github.com/EmanueleFusco/Scaricatore-di-Porto-rust/releases/download/whisper-v1.9.5/{asset}"
        );

        let expected = [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "universal-apple-darwin",
        ];
        assert_eq!(whisper.platforms.len(), expected.len());
        for triple in expected {
            assert!(
                whisper.platforms.contains_key(triple),
                "missing sdp-whisper asset for {triple}"
            );
        }

        for (triple, entry) in &whisper.platforms {
            assert_eq!(entry.archive_kind, ArchiveKind::None, "{triple}");
            let asset = entry.asset.as_deref().expect("asset");
            let expected_url =
                resolve_asset_url(template, asset).expect("the release template must resolve");
            assert_eq!(
                entry.url.as_deref(),
                Some(expected_url.as_str()),
                "{triple}"
            );
            assert!(entry.is_installable(), "{triple}");
        }

        let linux = manifest
            .entry(SDP_WHISPER, "x86_64-unknown-linux-gnu")
            .expect("linux x64");
        assert_eq!(
            linux.sha256.as_deref(),
            Some("0addfb2aa67079fdfd74f7ff0cb11f950347625fcc12f80e4326cc81cd66f2ce")
        );
    }

    #[test]
    fn ffmpeg_and_ffprobe_are_pinned_zip_assets_with_checksums() {
        // Both tools are packaged in zip archives containing ffmpeg and ffprobe.
        // Both tools must be installable on every platform where an asset
        // exists; Windows ARM64 has no asset and is deliberately absent.
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        for name in ["ffmpeg", "ffprobe"] {
            let tool = manifest.tool(name).expect(name);
            assert!(!tool.bundled, "{name} is downloaded on demand");
            assert_eq!(tool.version.as_deref(), Some("9.0.2"), "{name}");
            for triple in [
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "x86_64-pc-windows-msvc",
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
            ] {
                let entry = manifest
                    .entry(name, triple)
                    .unwrap_or_else(|| panic!("{name} is missing its {triple} entry"));
                assert_eq!(entry.archive_kind, ArchiveKind::Zip, "{name}/{triple}");
                assert!(
                    entry.asset.as_deref().is_some_and(|a| a.ends_with(".zip")),
                    "{name}/{triple}: the asset is a zip archive"
                );
                assert!(
                    entry.is_installable(),
                    "{name}/{triple} must be installable"
                );
                assert_eq!(
                    entry.executable(),
                    Some(if triple.contains("windows") {
                        if name == "ffmpeg" {
                            "ffmpeg.exe"
                        } else {
                            "ffprobe.exe"
                        }
                    } else {
                        name
                    }),
                    "{name}/{triple}"
                );
            }
            assert!(
                !manifest.supports(name, "aarch64-pc-windows-msvc"),
                "{name}: no win32-arm64 asset exists, so the platform must be absent"
            );
        }
    }

    #[test]
    fn latest_mode_never_carries_a_per_platform_url_or_checksum() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let ytdlp = manifest.tool(YTDLP).expect("yt-dlp");
        for (triple, entry) in &ytdlp.platforms {
            assert_eq!(entry.url, None, "{triple} must resolve at download time");
            assert_eq!(
                entry.sha256, None,
                "{triple} is verified against SHA2-256SUMS"
            );
            assert_eq!(
                entry.archive_kind,
                ArchiveKind::None,
                "{triple} is a bare binary"
            );
        }
    }

    #[test]
    fn every_pinned_sha256_in_the_manifest_is_real_sha256() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        let mut checked = 0;
        for tool in &manifest.tools {
            for (triple, entry) in &tool.platforms {
                if let Some(sum) = &entry.sha256 {
                    assert!(is_sha256(sum), "{}/{triple}: {sum}", tool.name);
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "the manifest should pin at least one checksum");
    }

    #[test]
    fn a_parse_error_lists_every_problem_instead_of_only_the_first() {
        let raw = r#"{
            "schemaVersion": 99,
            "tools": [
                {"name": "a", "role": "", "bundled": true, "license": "",
                 "versioning": "pinned", "minVersion": "x",
                 "platforms": {"x86_64-unknown-linux-gnu": {"executableName": "",
                    "asset": "../escape", "url": "http://insecure", "sha256": "nope"}}},
                {"name": "a", "role": "r", "license": "MIT", "versioning": "pinned",
                 "version": "1.0", "minVersion": "1.0", "platforms": {}}
            ]
        }"#;
        let err = ToolsManifest::parse(raw).expect_err("must fail");
        let joined = err.problems.join("\n");
        for expected in [
            "schemaVersion is 99",
            "the tool \"a\" is declared twice",
            "role is empty",
            "license is empty",
            "minVersion",
            "no platforms",
            "not a plain file name",
            "url must be https",
            "sha256 must be 64 lowercase hex",
        ] {
            assert!(
                joined.contains(expected),
                "missing {expected:?} in:\n{joined}"
            );
        }
    }

    #[test]
    fn a_url_without_a_checksum_is_rejected_and_the_reverse_too() {
        // Half a pin is worse than none: without the checksum the download
        // cannot be verified, and spec §5.2 makes verification mandatory.
        let raw = r#"{
            "schemaVersion": 1,
            "tools": [{
                "name": "t", "role": "r", "license": "MIT", "versioning": "pinned",
                "version": "1.0.0", "minVersion": "1.0",
                "platforms": {"x86_64-unknown-linux-gnu": {
                    "asset": "t", "executableName": "t",
                    "url": "https://example.invalid/t"}}
            }]
        }"#;
        let err = ToolsManifest::parse(raw).expect_err("must fail");
        assert!(
            err.problems
                .iter()
                .any(|p| p.contains("both be set or both be absent")),
            "{:?}",
            err.problems
        );

        let raw = raw.replace(
            "\"url\": \"https://example.invalid/t\"",
            "\"sha256\": \"0000000000000000000000000000000000000000000000000000000000000000\"",
        );
        let err = ToolsManifest::parse(&raw).expect_err("must fail");
        assert!(
            err.problems
                .iter()
                .any(|p| p.contains("both be set or both be absent")),
            "{:?}",
            err.problems
        );
    }

    #[test]
    fn unknown_fields_are_ignored_so_a_newer_manifest_still_loads() {
        let raw = r#"{
            "schemaVersion": 1,
            "somethingFromTheFuture": {"a": 1},
            "tools": [{
                "name": "t", "role": "r", "license": "MIT", "bundled": false,
                "versioning": "pinned", "version": "1.2.3", "minVersion": "1.0",
                "futureField": true,
                "platforms": {"x86_64-unknown-linux-gnu": {
                    "asset": "t.bin", "executableName": "t", "archiveKind": "none",
                    "url": "https://example.invalid/t.bin",
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "futurePlatformField": 3}}
            }]
        }"#;
        let manifest = ToolsManifest::parse(raw).expect("must load");
        assert!(manifest.supports("t", "x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn missing_platform_fields_fall_back_to_their_defaults() {
        let raw = r#"{
            "schemaVersion": 1,
            "tools": [{
                "name": "t", "role": "r", "license": "MIT", "versioning": "pinned",
                "version": "1.2.3", "minVersion": "1.0",
                "platforms": {"x86_64-unknown-linux-gnu": {
                    "asset": "t.bin", "executableName": "t",
                    "url": "https://example.invalid/t.bin",
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"}}
            }]
        }"#;
        let manifest = ToolsManifest::parse(raw).expect("must load");
        let entry = manifest
            .entry("t", "x86_64-unknown-linux-gnu")
            .expect("entry");
        assert_eq!(entry.archive_kind, ArchiveKind::None);
        assert!(entry.built_from.is_empty());
        assert!(entry.is_installable());
    }

    #[test]
    fn is_safe_asset_name_rejects_traversal_and_separators() {
        assert!(is_safe_asset_name("yt-dlp_linux"));
        assert!(is_safe_asset_name("qjs-windows-x86_64.exe"));
        assert!(!is_safe_asset_name(""));
        assert!(!is_safe_asset_name("."));
        assert!(!is_safe_asset_name(".."));
        assert!(!is_safe_asset_name("../yt-dlp"));
        assert!(!is_safe_asset_name("a/b"));
        assert!(!is_safe_asset_name(r"a\b"));
        assert!(!is_safe_asset_name(".hidden"));
    }

    #[test]
    fn is_sha256_wants_exactly_64_lowercase_hex() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
        assert!(!is_sha256(&"a".repeat(65)));
        assert!(!is_sha256(&"A".repeat(64)), "uppercase is not canonical");
        assert!(!is_sha256(&"g".repeat(64)));
        assert!(!is_sha256(""));
    }

    #[test]
    fn resolve_asset_url_substitutes_the_asset_once() {
        assert_eq!(
            resolve_asset_url(
                "https://example.invalid/releases/latest/{asset}",
                "yt-dlp_linux"
            )
            .as_deref(),
            Some("https://example.invalid/releases/latest/yt-dlp_linux")
        );
        // A template with no placeholder would download an HTML page.
        assert_eq!(
            resolve_asset_url("https://example.invalid/latest", "yt-dlp"),
            None
        );
        // An unsafe asset name never reaches the URL.
        assert_eq!(
            resolve_asset_url("https://example.invalid/{asset}", "../etc/passwd"),
            None
        );
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        // The bug this guards: "2025.9.1" > "2025.11.12" if compared as text.
        assert_eq!(
            compare_versions("2025.9.1", "2025.11.12"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_versions("2026.08.19", "2026.8.19"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_versions("0.17.0", "0.12.0"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("2.3", "2.3.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.2", "1.2.1"), Some(Ordering::Less));
    }

    #[test]
    fn an_unparseable_version_is_never_comparable() {
        assert_eq!(parse_version("2025.01.01-beta"), None);
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1..2"), None);
        assert_eq!(compare_versions("nightly", "2025.11.12"), None);
    }

    #[test]
    fn min_version_is_inclusive_and_refuses_what_it_cannot_compare() {
        assert!(satisfies_min_version("2025.11.12", "2025.11.12"));
        assert!(satisfies_min_version("2026.08.19", "2025.11.12"));
        assert!(!satisfies_min_version("2025.11.11", "2025.11.12"));
        assert!(!satisfies_min_version("nightly", "2025.11.12"));
    }

    #[test]
    fn the_embedded_manifest_min_versions_are_ones_our_own_logic_can_use() {
        let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));
        for tool in &manifest.tools {
            assert!(
                parse_version(&tool.min_version).is_some(),
                "{} minVersion {:?} is unparseable",
                tool.name,
                tool.min_version
            );
        }
        // The QuickJS floor comes straight from spec §5.3 (QuickJS-NG >= 0.12.0).
        let qjs = manifest.tool(QJS).expect("qjs");
        assert_eq!(qjs.min_version, "0.12.0");
        assert!(satisfies_min_version(
            qjs.version.as_deref().unwrap_or_default(),
            "0.12.0"
        ));
    }

    #[test]
    fn the_target_triple_is_the_one_the_bundler_would_use() {
        let triple = current_target_triple().expect("triple");
        assert!(!triple.is_empty());
        assert!(triple.contains('-'), "{triple} does not look like a triple");
    }
}
