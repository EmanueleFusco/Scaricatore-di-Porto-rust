fn main() {
    tauri_build::build();
    embed_common_controls_v6_in_tests();
}

/// Windows: embed the Common Controls v6 manifest into the **test** binaries.
///
/// Without it, an executable that imports comctl32 v6 symbols loads the legacy
/// comctl32 (v5), which does not export them, and dies before `main` with
/// `STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)`. That is why
/// `tests/settings_ipc.rs` (it uses `tauri::test::mock_builder`) could not
/// start on Windows, while `tests/process_tree.rs` (it never touches Tauri)
/// ran fine.
///
/// `tauri-build` already embeds the same manifest in the application binary,
/// through `embed-resource`, which emits `cargo:rustc-link-arg-bins`
/// (embed-resource 3.0.12, `src/lib.rs:443`). Bins only: that is precisely the
/// gap. So the flag here is `-tests`, **not** the global `-arg` — using the
/// global one would embed the manifest a second time into the application
/// binary and fail the link with duplicated resources.
///
/// Known upstream bug, not ours:
///   tauri#13419, tauri#14580 (open), discussion tauri#11179.
///
/// The decision is made on `CARGO_CFG_TARGET_*` and not on `#[cfg(windows)]`:
/// a build script is compiled for the **host**, so `#[cfg(windows)]` would be
/// false when cross-checking this crate for Windows from Linux — which is
/// exactly what `cargo clippy --target x86_64-pc-windows-msvc` does.
fn embed_common_controls_v6_in_tests() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // `/MANIFEST:*` are MSVC linker flags; they mean nothing to the GNU linker.
    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
    assert!(
        manifest.is_file(),
        "missing {}: the Windows test binaries would fail to start with \
         STATUS_ENTRYPOINT_NOT_FOUND",
        manifest.display()
    );

    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest.display()
    );
    // The usual workarounds also add `/WX` (linker warnings are errors). We
    // deliberately do not: it would turn unrelated warnings into build
    // failures for no benefit to this fix.
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
}
