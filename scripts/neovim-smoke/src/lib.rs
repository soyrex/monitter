//! Focused tests compile the actual backend and redraw model without GPUI.
#![allow(dead_code)]
#[path = "../../../src-tauri/src/neovim.rs"]
mod backend;
#[path = "../../../desktop-gpui/src/neovim_model.rs"]
mod model;

#[cfg(test)]
mod integration {
    use super::*;
    use serde_json::json;
    #[test]
    #[ignore = "launches isolated real Neovim"]
    fn real_redraw_decodes_grid_and_commandline() {
        let opened = backend::invoke("open_neovim", json!({"clean":true})).unwrap();
        let id = opened["id"].as_str().unwrap();
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = backend::invoke("neovim_command", json!({"id":self.0,"command":"qa!"}));
            }
        }
        let _cleanup = Cleanup(id.to_owned());
        backend::invoke("neovim_input", json!({"id":id,"keys":":echo 'native-ui'"})).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let state = backend::invoke("read_neovim", json!({"id":id,"after":0})).unwrap();
            let mut model = model::NeovimModel::new();
            for batch in state["events"].as_array().unwrap() {
                model.apply_redraw(&batch["events"]);
            }
            if model.cmdline_visible {
                let cmd = model.cmdlines.values().next().unwrap();
                assert_eq!(cmd.firstc, ":");
                assert_eq!(
                    cmd.content
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<String>(),
                    "echo 'native-ui'"
                );
                assert!(!model.grids.is_empty());
                assert!(!model.windows.is_empty());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "real commandline was not decoded: {}",
                state["events"]
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore = "launches isolated real Neovim"]
    fn modal_prompt_keeps_redraw_reads_responsive() {
        let opened = backend::invoke("open_neovim", json!({"clean":true})).unwrap();
        let id = opened["id"].as_str().unwrap().to_owned();
        let command_id = id.clone();
        let prompt = std::thread::spawn(move || {
            backend::invoke(
                "neovim_command",
                json!({"id":command_id,"command":"let g:monitter_prompt = input('Native prompt: ')"}),
            )
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        let started = std::time::Instant::now();
        let state = backend::invoke("read_neovim", json!({"id":id,"after":0})).unwrap();
        let elapsed = started.elapsed();
        let resize_started = std::time::Instant::now();
        backend::invoke(
            "neovim_resize",
            json!({"id":id,"grid":1,"cols":92,"rows":35}),
        )
        .unwrap();
        let resize_elapsed = resize_started.elapsed();
        backend::invoke("neovim_input", json!({"id":id,"keys":"answer<CR>"})).unwrap();
        let result = prompt.join().unwrap();
        let _ = backend::invoke("neovim_command", json!({"id":id,"command":"qa!"}));
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "redraw read blocked for {elapsed:?}"
        );
        assert!(
            resize_elapsed < std::time::Duration::from_secs(1),
            "resize blocked input for {resize_elapsed:?}"
        );
        assert!(state["events"].to_string().contains("Native prompt"));
        result.unwrap();
    }
}
