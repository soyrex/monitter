//! A saved workspace presentation, with no services, timers, or input handlers.
use crate::{design, docking::{DockWorkspace, PaneSpec}};
use gpui_kit::{prelude::*, *};
use monitter_gpui_desktop::startup_cache::Presentation;
use monitter_lib::experimental_desktop::NativeChatPanePreferences;
use std::{rc::Rc, sync::OnceLock, time::Instant};

static STARTED: OnceLock<Instant> = OnceLock::new();
pub fn start() { STARTED.get_or_init(Instant::now); }
pub fn mark(stage: &str) {
    let elapsed_ms = STARTED.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.;
    eprintln!("[native-startup] stage={stage} elapsed_ms={elapsed_ms:.2}");
    #[cfg(not(test))]
    {
        static STAGES: std::sync::Mutex<Vec<(String, f64)>> = std::sync::Mutex::new(Vec::new());
        let Ok(mut stages) = STAGES.lock() else { return; };
        if stages.iter().any(|(name, _)| name == stage) { return; }
        stages.push((stage.to_owned(), elapsed_ms));
        if stage != "live-workspace-frame" { return; }
        let stages = stages.clone();
        // Persist only timing metadata, after the live paint and off the UI
        // thread, so launches from Finder are measurable without a terminal.
        std::thread::spawn(move || {
            let save = || -> Result<(), Box<dyn std::error::Error>> {
                let profile = monitter_lib::experimental_desktop::profile_dir()?;
                let path = profile.join("startup-timing.json");
                let temporary = profile.join(format!("startup-timing.{}.tmp", std::process::id()));
                let data = serde_json::json!({"pid":std::process::id(), "recordedAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(), "stages":stages});
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
                use std::io::Write;
                let mut file = options.open(&temporary)?;
                file.write_all(&serde_json::to_vec(&data)?)?;
                std::fs::rename(temporary, path)?;
                Ok(())
            };
            if let Err(error) = save() { eprintln!("[native-startup] timing_write={error}"); }
        });
    }
}
struct SavedPane {
    lines: Vec<String>,
    sidebar: Option<NativeChatPanePreferences>,
    palette: design::Palette,
}
impl Render for SavedPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let body = div().flex_1().min_w_0().h_full().overflow_hidden().flex().flex_col().justify_end().p_4().gap_3()
            .children(self.lines.iter().map(|line| div().text_sm().child(line.clone())));
        let mut root = div().size_full().flex().overflow_hidden().bg(design::surface_color(palette.paper)).text_color(rgb(palette.ink)).child(body);
        if let Some(sidebar) = self.sidebar.as_ref().filter(|sidebar| sidebar.visible) {
            root = root.child(div().h_full().w(design::ui_scaled_px(sidebar.width)).max_w(relative(0.45)).flex_shrink_0()
                .border_l_1().border_color(rgb(palette.line)).p_3().text_sm()
                .child(format!("{:?}", sidebar.tab)));
        }
        root
    }
}
pub fn restore(cache: &Presentation, palette: design::Palette, window: &mut Window, cx: &mut App) -> Result<Entity<DockWorkspace>, String> {
    let dock = cx.new(|cx| DockWorkspace::new("startup-presentation", Rc::new(|_, _, _| None), window, cx));
    dock.update(cx, |dock, cx| {
        dock.set_auto_size_panes(false, window, cx);
        dock.set_tab_appearance(&cache.snapshot["settings"], design::interface_scale(), cx);
        dock.set_pane_appearance(&cache.snapshot["settings"], palette, cx);
        for id in DockWorkspace::saved_ids(&cache.layout)? {
            let pane = cache.panes.get(&id).cloned().unwrap_or_default();
            let sidebar = id.strip_prefix("chat:").and_then(|id| cache.preferences.data.chat_panes.get(id)).cloned();
            let content = cx.new(|_| SavedPane { lines: pane.lines, sidebar, palette });
            dock.register(PaneSpec { id, title: pane.title.into(), content: content.into(), focus: cx.focus_handle(), lifecycle: None, closable: false }, cx)?;
        }
        dock.restore_retaining(cache.layout.clone(), window, cx)?;
        if let Some(id) = crate::workspace_visibility::remembered_focus(&cache.layout) { dock.focus(id, window, cx); }
        Ok::<_, String>(())
    })?;
    Ok(dock)
}

#[cfg(all(test, feature = "visual-export"))]
mod tests {
    use super::{restore, design, Presentation};
    use gpui_kit::{AppContext as _, TestAppContext};
    use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
    use monitter_lib::experimental_desktop::NativePreferences;
    use serde_json::json;
    #[gpui_kit::test]
    fn startup_preview_restores_saved_tabs_without_backend_or_live_panes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let saved = serde_json::to_value(DockAreaState {
            version: Some(1),
            center: PanelState { panel_name: "TabPanel".into(), children: ["chat:first", "chat:second"].iter().map(|id| PanelState {
                panel_name: "MonitterNativePane".into(), children: vec![], info: PanelInfo::panel(json!({"id": id})),
            }).collect(), info: PanelInfo::tabs(1) },
            left_dock: None, right_dock: None, bottom_dock: None,
        }).unwrap();
        let cache = Presentation::capture(&json!({"settings":{"theme":"light"},"tasks":[{"id":"first","title":"First"},{"id":"second","title":"Second"}]}), &NativePreferences::default(), None, saved.clone(), &["chat:first".into(), "chat:second".into()]);
        let mut preview = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let palette = design::palette(&cache.snapshot["settings"], window.appearance());
            let dock = restore(&cache, palette, window, cx).unwrap();
            preview = Some(dock.clone());
            gpui_kit::component::Root::new(dock, window, cx)
        });
        let dock = preview.unwrap();
        visual.run_until_parked();
        visual.update(|window, cx| {
            let restored = dock.read(cx).serialize(cx).unwrap();
            // DockArea normalizes a bare saved TabPanel into a root stack.
            assert_eq!(restored["center"]["panel_name"], "StackPanel");
            assert_eq!(restored["center"]["children"], json!([saved["center"].clone()]));
            assert!(!dock.read(cx).auto_size_panes());
            assert_eq!(dock.read(cx).focused_pane_id(window, cx).as_deref(), Some("chat:second"));
        });
    }
}
