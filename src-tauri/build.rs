fn main() {
    #[cfg(target_os = "macos")]
    {
        cc::Build::new()
            .file("native-browser-auth.m")
            .flag("-fobjc-arc")
            .compile("monitter_native_browser_auth");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=WebKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rerun-if-changed=native-browser-auth.m");
    }
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new()),
    )
    .expect("failed to build the Monitter Tauri application");
}
