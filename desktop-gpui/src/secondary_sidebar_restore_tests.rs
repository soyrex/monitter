//! Exercise production restore/reveal and native measurement without provider IO.
use super::ChatPane;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use monitter_lib::experimental_desktop::{
    DesktopService, NativeChatPanePreferences, NativeDetailTab,
};
use serde_json::json;
use std::rc::Rc;

struct Placeholder;
impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("Sidebar restore fixture")
    }
}

fn mount(cx: &mut TestAppContext) -> (Entity<ChatPane>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut pane = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        pane = Some(cx.new(|cx| {
            let mut pane = ChatPane::new(
                "sidebar-restore-task".into(),
                DesktopService::disconnected_for_ui_fixture(),
                Rc::new(json!({"settings":{}, "tasks":[], "messages":[], "events":[]})),
                Some("sidebar-restore-fixture".into()), window, cx,
            );
            pane.service = None;
            pane
        }));
        let placeholder = cx.new(|_| Placeholder);
        gpui_kit::component::Root::new(placeholder, window, cx)
    });
    (pane.unwrap(), visual)
}

#[gpui_kit::test]
fn secondary_sidebar_restore_is_settled_before_first_native_measurement(
    cx: &mut TestAppContext,
) {
    let (pane, visual) = mount(cx);
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            let saved = NativeChatPanePreferences {
                visible: true, width: 280., tab: NativeDetailTab::Timeline,
            };
            pane.restore_secondary_sidebar(&saved, window, cx);
            assert!(pane.detail_motion_settle_pending);
            pane.update_pane_width(1_000., window, cx);
            assert!(!pane.detail_motion_settle_pending);
            assert_eq!(pane.secondary_sidebar_state(), saved);
            assert_eq!(pane.detail_motion.sample(true, false), (1., false));
            pane.toggle_companion(cx);
            assert_eq!(pane.detail_motion.sample(false, false), (1., true));
            let hidden = NativeChatPanePreferences {
                visible: false, width: 300., tab: NativeDetailTab::Subagents,
            };
            pane.restore_secondary_sidebar(&hidden, window, cx);
            assert_eq!(pane.secondary_sidebar_state(), hidden);
            assert_eq!(pane.detail_motion.sample(false, false), (0., false));
        });
    });
}

#[gpui_kit::test]
fn retained_pane_reveal_and_layout_breakpoint_cancel_outgoing_sidebar_motion(
    cx: &mut TestAppContext,
) {
    let (pane, visual) = mount(cx);
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.update_pane_width(1_000., window, cx);
            pane.set_dock_visible(true, cx);
            pane.toggle_companion(cx);
            assert!(pane.detail_motion.sample(false, false).1);
            pane.set_dock_visible(false, cx);
            pane.set_dock_visible(true, cx);
            assert_eq!(pane.detail_motion.sample(false, false), (0., false));
            pane.show_companion(cx);
            assert_eq!(pane.detail_motion.sample(true, false), (0., true));
            pane.update_pane_width(400., window, cx);
            assert!(!pane.companion_visible);
            assert_eq!(pane.detail_motion.sample(false, false), (0., false));
            pane.update_pane_width(1_000., window, cx);
            assert_eq!(pane.detail_motion.sample(false, false), (0., false));
            pane.show_companion(cx);
            assert_eq!(pane.detail_motion.sample(true, false), (0., true));
        });
    });
}
