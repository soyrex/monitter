fn main() {
 println!("cargo:rerun-if-changed=app.tsx");
 let code = nsh_bundler::compile_module("App.tsx", include_str!("app.tsx"), &[]).expect("compile local NSH test UI");
 std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("app.js"), code).unwrap();
}
