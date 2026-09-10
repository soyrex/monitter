use std::path::PathBuf;
fn main() {
    let mut state_dir = None;
    let mut cwd = None;
    let mut prompt = None;
    let mut second = None;
    let mut host = None;
    let mut cancel = false;
    let mut provider = String::from("codex");
    let mut model = None;
    let mut expected_status = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--state-dir" => state_dir = args.next().map(PathBuf::from),
            "--cwd" => cwd = args.next().map(PathBuf::from),
            "--prompt" => prompt = args.next(),
            "--second-prompt" => second = args.next(),
            "--host-json" => {
                host = args
                    .next()
                    .map(|raw| serde_json::from_str(&raw).expect("valid Host JSON"))
            }
            "--provider" => provider = args.next().expect("provider name"),
            "--model" => model = args.next(),
            "--cancel" => cancel = true,
            "--expect-status" => expected_status = args.next(),
            _ => {
                eprintln!("Usage: monitter-smoke --state-dir DIR --cwd DIR --prompt TEXT [--second-prompt TEXT] [--host-json HOST_JSON] [--cancel] [--expect-status completed|interrupted|error]");
                std::process::exit(2);
            }
        }
    }
    let state_dir = state_dir.unwrap_or_else(|| {
        eprintln!("--state-dir is required");
        std::process::exit(2)
    });
    let cwd = cwd.unwrap_or_else(|| {
        eprintln!("--cwd is required");
        std::process::exit(2)
    });
    let prompt = prompt.unwrap_or_else(|| {
        eprintln!("--prompt is required");
        std::process::exit(2)
    });
    let expected_status = expected_status.unwrap_or_else(|| {
        if cancel {
            "interrupted".into()
        } else {
            "completed".into()
        }
    });
    match monitter_lib::smoke_provider_sequence(
        &state_dir,
        host,
        &cwd,
        &prompt,
        second.as_deref(),
        cancel,
        &expected_status,
        &provider,
        model.as_deref(),
    ) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize result")
            );
            if !result.ok {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("Monitter smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
