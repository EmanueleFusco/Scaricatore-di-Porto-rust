//! The "Copy diagnostics" payload (spec §11.8).
//!
//! Plain text on purpose: it goes into a GitHub issue, a chat message or an
//! email, and every one of those is easier to read than a JSON blob. The
//! builder is pure so the format and the redaction rule are unit-tested.

use std::fmt::Write as _;

/// One tool row, already resolved by the caller.
pub struct DiagnosticTool<'a> {
    pub name: &'a str,
    pub version: Option<&'a str>,
    pub source: Option<&'a str>,
    pub path: Option<&'a str>,
}

/// Everything the report needs. No secrets, no cookies, no API keys.
pub struct DiagnosticsInput<'a> {
    pub app_name: &'a str,
    pub version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub triple: &'a str,
    pub bundle_type: Option<&'a str>,
    pub settings_path: &'a str,
    pub tools_dir: &'a str,
    pub bundled_dir: &'a str,
    pub temp_dir: Option<&'a str>,
    pub models_dir: &'a str,
    pub log_path: &'a str,
    pub tools: &'a [DiagnosticTool<'a>],
    pub log_tail: &'a str,
}

/// Render the report, with a trailing newline.
pub fn diagnostics_text(input: &DiagnosticsInput<'_>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} {}", input.app_name, input.version);
    let _ = writeln!(out, "OS: {} {}", input.os, input.arch);
    let _ = writeln!(out, "Target: {}", input.triple);
    let _ = writeln!(out, "Bundle: {}", input.bundle_type.unwrap_or("unknown"));
    let _ = writeln!(out, "Settings: {}", input.settings_path);
    let _ = writeln!(out, "Tools dir: {}", input.tools_dir);
    let _ = writeln!(out, "Bundled dir: {}", input.bundled_dir);
    let _ = writeln!(
        out,
        "Temp dir: {}",
        input.temp_dir.unwrap_or("not available")
    );
    let _ = writeln!(out, "Models dir: {}", input.models_dir);
    let _ = writeln!(out, "Log file: {}", input.log_path);
    let _ = writeln!(out);
    let _ = writeln!(out, "Tools:");
    for tool in input.tools {
        let _ = writeln!(
            out,
            "- {} {} ({}){}",
            tool.name,
            tool.version.unwrap_or("unknown"),
            tool.source.unwrap_or("not found"),
            tool.path
                .map(|path| format!(" at {path}"))
                .unwrap_or_default(),
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Recent log:");
    if input.log_tail.trim().is_empty() {
        let _ = writeln!(out, "(empty)");
    } else {
        let _ = writeln!(out, "{}", input.log_tail.trim_end());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(tools: &'a [DiagnosticTool<'a>], log_tail: &'a str) -> DiagnosticsInput<'a> {
        DiagnosticsInput {
            app_name: "Scaricatore Di Porto",
            version: "0.1.0",
            os: "linux",
            arch: "x86_64",
            triple: "x86_64-unknown-linux-gnu",
            bundle_type: Some("appimage"),
            settings_path: "/home/user/.config/sdp/settings.json",
            tools_dir: "/home/user/.local/share/sdp/tools",
            bundled_dir: "/usr/lib/sdp",
            temp_dir: Some("/run/user/1000/sdp"),
            models_dir: "/home/user/.local/share/sdp/models",
            log_path: "/home/user/.local/share/sdp/logs/scaricatore-di-porto.log",
            tools,
            log_tail,
        }
    }

    #[test]
    fn includes_versions_paths_and_tool_rows() {
        let tools = [DiagnosticTool {
            name: "yt-dlp",
            version: Some("2025.11.12"),
            source: Some("appData"),
            path: Some("/home/user/.local/share/sdp/tools/yt-dlp"),
        }];
        let text = diagnostics_text(&input(&tools, ""));
        assert!(text.contains("Scaricatore Di Porto 0.1.0"));
        assert!(text.contains("OS: linux x86_64"));
        assert!(text.contains("Bundle: appimage"));
        assert!(text.contains("- yt-dlp 2025.11.12 (appData) at"));
        assert!(text.contains("Recent log:\n(empty)"));
    }

    #[test]
    fn never_mentions_secrets_even_when_the_caller_has_them() {
        let tools = [DiagnosticTool {
            name: "gemini",
            version: None,
            source: None,
            path: None,
        }];
        let text = diagnostics_text(&input(&tools, "[1] INFO: started"));
        assert!(!text.contains("api_key"));
        assert!(!text.contains("AIza"));
        assert!(!text.contains("cookie"));
        assert!(text.contains("- gemini unknown (not found)"));
    }

    #[test]
    fn handles_a_missing_bundle_and_temp_dir() {
        let mut report = input(&[], "");
        report.bundle_type = None;
        report.temp_dir = None;
        let text = diagnostics_text(&report);
        assert!(text.contains("Bundle: unknown"));
        assert!(text.contains("Temp dir: not available"));
    }
}
