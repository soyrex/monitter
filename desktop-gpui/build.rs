use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    for file in [
        "renderer.tsx",
        "controller.mjs",
        "calendar.mjs",
        "charts.mjs",
        "media.mjs",
        "vendor/core.js",
        "vendor/provenance.json",
        "bundle.mjs",
        "package.json",
        "package-lock.json",
    ] {
        println!("cargo:rerun-if-changed=openui/{file}");
    }
    println!("cargo:rerun-if-env-changed=CARGO_MANIFEST_DIR");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let compiled = out.join("openui.compiled.js");
    let source = fs::read_to_string(root.join("openui/renderer.tsx"))
        .expect("read trusted renderer from the active checkout");
    let code =
        nsh_bundler::compile_module("renderer.tsx", &source, &[])
            .expect("compile trusted OpenUI NSH renderer");
    fs::write(&compiled, code).expect("write compiled OpenUI renderer");
    let status = Command::new("node")
        .arg(root.join("openui/bundle.mjs"))
        .arg(compiled)
        .arg(out.join("openui-renderer.js"))
        .current_dir(&root)
        .status()
        .expect("OpenUI bundling requires Node and desktop-gpui/openui npm ci --ignore-scripts");
    assert!(
        status.success(),
        "OpenUI bundle failed; run npm ci --ignore-scripts in desktop-gpui/openui, then retry"
    );
}
