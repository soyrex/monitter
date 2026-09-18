use monitter_lib::profile_init::initialize_blank_profile;
use std::{env, path::PathBuf, process};

fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    let result = match arguments.as_slice() {
        [_, command, directory] if command == "init-empty-profile" => {
            initialize_blank_profile(PathBuf::from(directory))
        }
        _ => Err("Usage: monitter-state init-empty-profile <absolute-new-directory>".into()),
    };
    if let Err(error) = result {
        eprintln!("monitter-state: {error}");
        process::exit(1);
    }
}
