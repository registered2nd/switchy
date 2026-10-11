fn main() {
    // Embed a small Windows launcher so manual sign-in needs no installed compiler.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let source = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("no_browser.rs");
        let output =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("no_browser.exe");
        let status = std::process::Command::new(std::env::var_os("RUSTC").unwrap())
            .arg(&source)
            .args([
                "--edition=2021",
                "--crate-name=switchy_no_browser",
                "--target",
            ])
            .arg(std::env::var("TARGET").unwrap())
            .args(["-C", "opt-level=s", "-C", "panic=abort", "-o"])
            .arg(&output)
            .status()
            .expect("could not compile the manual sign-in browser launcher");
        assert!(
            status.success(),
            "manual sign-in browser launcher compilation failed"
        );
        println!("cargo:rerun-if-changed={}", source.display());
    }
    tauri_build::build();

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    #[cfg(target_os = "windows")]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }
}
