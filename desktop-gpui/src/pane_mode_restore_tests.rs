//! Native layout mode survives both retained entities and factory reconstruction.
use super::{DockWorkspace, PaneFactory, PaneSpec};
use gpui::{App, Context, Window};
use gpui_kit::component::dock::DockPlacement;
use serde_json::{Value, json};
use std::rc::Rc;
use gpui::{AppContext as _, Focusable as _, TestAppContext};

fn chat_spec(id: &str, window: &mut Window, cx: &mut App) -> PaneSpec {
    let content = cx.new(|cx| crate::chat::ChatPane::performance_fixture(
        id.trim_start_matches("chat:").into(),
        Rc::new(json!({"settings":{},"tasks":[],"messages":[],"events":[]})),
        window, cx,
    ));
    PaneSpec { id: id.into(), title: id.to_owned().into(),
        focus: content.read(cx).focus_handle(cx), content: content.into(),
        lifecycle: None, closable: true }
}

fn set_mode(dock: &DockWorkspace, id: &str, watch: bool, cx: &mut Context<DockWorkspace>) {
    let content = dock.panels.borrow()[id].read(cx).spec.content.clone();
    content.downcast::<crate::chat::ChatPane>().unwrap()
        .update(cx, |chat, cx| chat.restore_watch_mode(watch, cx));
}
fn mode(dock: &DockWorkspace, id: &str, cx: &App) -> bool {
    dock.panels.borrow()[id].read(cx).spec.content.clone()
        .downcast::<crate::chat::ChatPane>().unwrap().read(cx).persisted_watch_mode()
}
fn remove_modes(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.remove("watchMode");
            for value in fields.values_mut() { remove_modes(value); }
        }
        Value::Array(values) => for value in values { remove_modes(value); },
        _ => {}
    }
}

#[gpui_kit::test]
fn workspace_pane_mode_mixed_chat_watch_restore_retained_cold_and_legacy_layouts(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut dock = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|id, window, cx| Some(chat_spec(id, window, cx)));
        let entity = cx.new(|cx| DockWorkspace::new("mode-restore", factory, window, cx));
        dock = Some(entity.clone());
        gpui_kit::component::Root::new(entity, window, cx)
    });
    let dock = dock.unwrap();
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        for id in ["chat:watch", "chat:chat"] {
            let spec = chat_spec(id, window, cx);
            dock.add(spec, DockPlacement::Center, None, window, cx).unwrap();
        }
        set_mode(dock, "chat:watch", true, cx);
        let saved = dock.serialize(cx).unwrap();
        let projected = monitter_gpui_desktop::workspace_store::backend_layout_projection(&saved)
            .expect("a real mixed-mode dock must be saveable by the resident backend");
        assert!(!projected.to_string().contains("watchMode"));
        assert!(saved.to_string().contains("watchMode"), "local layout retains modes");
        let mut expected = saved.clone();
        remove_modes(&mut expected);
        assert_eq!(projected, expected, "projection preserves split/tab structure and identities");
        let identity = dock.panels.borrow()["chat:watch"].entity_id();
        set_mode(dock, "chat:watch", false, cx);
        set_mode(dock, "chat:chat", true, cx);
        dock.restore_retaining(saved.clone(), window, cx).unwrap();
        assert!(mode(dock, "chat:watch", cx));
        assert!(!mode(dock, "chat:chat", cx));
        assert_eq!(dock.panels.borrow()["chat:watch"].entity_id(), identity);
        let factory: PaneFactory = Rc::new(|id, window, cx| Some(chat_spec(id, window, cx)));
        let cold = cx.new(|cx| DockWorkspace::new("mode-cold", factory, window, cx));
        cold.update(cx, |cold, cx| {
            cold.restore(saved.clone(), window, cx).unwrap();
            assert!(mode(cold, "chat:watch", cx));
            assert!(!mode(cold, "chat:chat", cx));
        });
        let mut legacy = saved;
        remove_modes(&mut legacy);
        set_mode(dock, "chat:chat", true, cx);
        dock.restore_retaining(legacy, window, cx).unwrap();
        assert!(!mode(dock, "chat:watch", cx));
        assert!(!mode(dock, "chat:chat", cx));
    }));
}
