use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn extract_function(source: &str, marker: &str) -> String {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("could not find production marker {marker:?}"));
    let body_start = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .expect("function has a body");
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = body_start;
    let mut string = false;
    let mut character = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment_depth = 0usize;

    while index < bytes.len() {
        let current = bytes[index];
        let next = bytes.get(index + 1).copied();

        if line_comment {
            if current == b'\n' {
                line_comment = false;
            }
        } else if block_comment_depth > 0 {
            if current == b'/' && next == Some(b'*') {
                block_comment_depth += 1;
                index += 1;
            } else if current == b'*' && next == Some(b'/') {
                block_comment_depth -= 1;
                index += 1;
            }
        } else if string || character {
            if escaped {
                escaped = false;
            } else if current == b'\\' {
                escaped = true;
            } else if (string && current == b'"') || (character && current == b'\'') {
                string = false;
                character = false;
            }
        } else if current == b'/' && next == Some(b'/') {
            line_comment = true;
            index += 1;
        } else if current == b'/' && next == Some(b'*') {
            block_comment_depth = 1;
            index += 1;
        } else if current == b'"' {
            string = true;
        } else if current == b'\'' {
            character = true;
        } else if current == b'{' {
            depth += 1;
        } else if current == b'}' {
            depth -= 1;
            if depth == 0 {
                return source[start..=index].to_owned();
            }
        }
        index += 1;
    }
    panic!("unterminated function {marker:?}");
}

fn generated_module(source: &str, use_borrowed_rows: bool) -> String {
    let mut output = String::from(
        "use serde_json::{json, Value};\nuse std::collections::HashMap;\n\n",
    );
    output.push_str(&extract_function(source, "fn field("));
    output.push_str("\n\n");
    output.push_str(&extract_function(source, "fn rows("));
    output.push_str("\n\n");
    if use_borrowed_rows {
        output.push_str(&extract_function(source, "fn rows_ref<"));
        output.push_str("\n\n");
    }
    let mut records = extract_function(source, "fn records(");
    records = records.replacen("fn records(", "pub fn records(", 1);
    output.push_str(&records);
    output.push_str("\n\n");
    output.push_str(&extract_function(source, "fn human_name("));
    output
}

fn write_module(out_dir: &Path, name: &str, contents: &str) {
    fs::write(out_dir.join(name), contents).expect("write generated benchmark module");
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..").canonicalize().expect("repository root");
    let source_path = root.join("desktop-gpui/src/collaboration.rs");
    println!("cargo:rerun-if-changed={}", source_path.display());

    let baseline = Command::new("git")
        .current_dir(&root)
        .args(["show", "ed040e6:desktop-gpui/src/collaboration.rs"])
        .output()
        .expect("run git show for baseline collaboration source");
    assert!(baseline.status.success(), "baseline ed040e6 source must exist");
    let baseline = String::from_utf8(baseline.stdout).expect("baseline source is UTF-8");
    let current = fs::read_to_string(&source_path).expect("read current collaboration source");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    write_module(&out_dir, "baseline_records.rs", &generated_module(&baseline, false));
    write_module(&out_dir, "current_records.rs", &generated_module(&current, true));
}
