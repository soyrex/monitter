//! Exercise the actual desktop overview mount and retained navigation without provider IO.
use crate::design::ui_scaled_px as px;
use crate::{Desktop, chat, design, docking};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext,
    VisualTestContext, Window, size,
};
use monitter_lib::experimental_desktop::{DesktopService, NativePreferences, NativeTileTheme};
use serde_json::json;
use std::rc::Rc;

struct Harness(Entity<Desktop>);
impl Render for Harness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |desktop, cx| {
            desktop.render(window, cx).into_any_element()
        })
    }
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    theme: &str,
    tile: NativeTileTheme,
    font: u32,
) -> (Entity<Desktop>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let mut retained = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let desktop = cx.new(|cx| {
            let mut desktop = Desktop::new(window, cx);
            desktop.preferences = Some(NativePreferences::default());
            desktop.preferences.as_mut().unwrap().data.appearance.tile_theme = tile;
            desktop.preferences_busy = false;
            desktop.window.preferences_restore = false;
            desktop.window.preferences_restoring = false;
            desktop.window.preferences_restore_suppressed = false;
            desktop.startup_pending = false;
            desktop.workspace_window_restore_pending = false;
            desktop.snapshot = Rc::new(json!({
                "settings":{"theme":theme,"interfaceFontSize":font,"interfaceDensity":"normal",
                    "nativeTileTheme":if tile == NativeTileTheme::Tern {"tern"} else {"monitter"}},
                "tasks":[{"id":"overview-task","title":"Review native activity","agentId":"overview-agent","projectId":"overview-project","status":"running","updatedAt":chrono::Utc::now().timestamp_millis(),"queue":[]}],
                "agents":[{"id":"overview-agent","name":"Codex","provider":"codex"}],
                "projects":[{"id":"overview-project","name":"Monitter","color":"#7c5cbf","icon":"rocket"}],
                "messages":[],"events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]
            }));
            desktop
        });
        retained = Some(desktop.clone());
        gpui_kit::component::Root::new(cx.new(|_| Harness(desktop)), window, cx)
    });
    (retained.unwrap(), visual)
}

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn activity_overview_keeps_sidebar_toggle_and_hides_split_controls_at_supported_fonts(
    cx: &mut TestAppContext,
) {
    for (theme, tile, font) in [
        ("light", NativeTileTheme::Monitter, 14),
        ("dark", NativeTileTheme::Tern, 32),
    ] {
        let (desktop, visual) = fixture(cx, theme, tile, font);
        draw(visual);
        let toggle = visual
            .debug_bounds("activity-overview-toggle")
            .expect("icon in workspace bar");
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual);
        desktop.read_with(visual, |desktop, _| {
            assert!(desktop.window.activity_overview_open)
        });
        assert!(visual.debug_bounds("activity-overview-pane").is_some());
        for width in [540., 1500.] {
            visual.simulate_resize(size(px(width), px(960.)));
            draw(visual);
            let overview = visual
                .debug_bounds("activity-overview")
                .expect("overview mounted");
            let tabs = visual
                .debug_bounds("named-workspace-tabs")
                .expect("workspace tabs retained");
            let card = visual
                .debug_bounds("activity-card-overview-task")
                .expect("real task card mounted");
            let header = visual
                .debug_bounds("activity-header")
                .expect("native toolbar");
            let footer = visual
                .debug_bounds("activity-footer")
                .expect("native legend");
            let scroll = visual
                .debug_bounds("activity-overview-scroll")
                .expect("scrollable graph");
            assert!(
                overview.top() >= tabs.bottom(),
                "overview begins below workspace bar"
            );
            assert!(
                header.left() >= overview.left() && header.right() <= overview.right(),
                "toolbar fits desktop width"
            );
            assert!(
                footer.left() >= overview.left() && footer.right() <= overview.right(),
                "legend fits desktop width"
            );
            assert!(
                scroll.top() >= header.bottom() && scroll.bottom() <= footer.top(),
                "graph scroll remains between toolbar and legend"
            );
            assert!(
                card.size.width >= px(190.) && card.size.height >= px(99.),
                "real card geometry is retained inside scrollable graph"
            );
            for selector in [
                "activity-card-title-overview-task",
                "activity-card-footer-overview-task",
            ] {
                let row = visual
                    .debug_bounds(selector)
                    .expect("compact card text row");
                assert!(
                    row.top() >= card.top() && row.bottom() <= card.bottom(),
                    "card text must remain inside compact geometry at supported fonts"
                );
                assert!(row.size.height > px(0.), "card text cannot collapse");
            }
            assert!(visual.debug_bounds("native-preferences-sidebar").is_some(), "sidebar remains mounted");
            assert!(visual.debug_bounds("cycle-main-sidebar").is_some(), "sidebar toggle remains available");
            assert!(visual.debug_bounds("split-workspace-vertical").is_none(), "vertical split is hidden in Activity");
            assert!(visual.debug_bounds("split-workspace-horizontal").is_none(), "horizontal split is hidden in Activity");
            let pane = visual
                .debug_bounds("activity-overview-pane")
                .expect("rounded activity pane");
            let sidebar_bounds = visual
                .debug_bounds("native-preferences-sidebar")
                .expect("sidebar remains mounted");
            assert!(
                pane.left() > sidebar_bounds.right() && pane.right() < px(width),
                "pane has a workspace gutter"
            );
            assert!(
                overview.left() >= pane.left() && overview.right() <= pane.right(),
                "overview fits its pane"
            );
            assert!(visual.debug_bounds("activity-project-icon").is_some(), "project icon is mounted");
            assert!(visual.debug_bounds("activity-project-spinner").is_some(), "working project has a spinner");
            assert!(visual.debug_bounds("activity-card-spinner-overview-task").is_some(), "working agent has a spinner");
        }
        let sidebar_toggle = visual.debug_bounds("cycle-main-sidebar").expect("sidebar toggle in workspace bar");
        let sidebar_hidden = desktop.read_with(visual, |desktop, _| desktop.window.sidebar_hidden);
        if !sidebar_hidden {
            visual.simulate_click(sidebar_toggle.center(), Default::default());
            draw(visual);
            desktop.read_with(visual, |desktop, _| assert!(desktop.window.sidebar_hidden));
        }
        let sidebar_toggle = visual.debug_bounds("cycle-main-sidebar").expect("toggle remains available while hidden");
        visual.simulate_click(sidebar_toggle.center(), Default::default());
        draw(visual);
        desktop.read_with(visual, |desktop, _| {
            assert!(!desktop.window.sidebar_hidden, "sidebar toggle reopens it in Activity");
            assert!(desktop.window.activity_overview_open, "sidebar toggle leaves Activity open");
        });
        visual.simulate_keystrokes("escape");
        draw(visual);
        desktop.read_with(visual, |desktop, _| {
            assert!(!desktop.window.activity_overview_open)
        });
        visual.update(|window, cx| {
            desktop.update(cx, |desktop, cx| {
                let overview = desktop
                    .window
                    .activity_overview
                    .as_ref()
                    .unwrap()
                    .entity_id();
                let dock = desktop.window.dock.entity_id();
                desktop.close_activity_overview(window, cx);
                desktop.toggle_activity_overview(window, cx);
                assert_eq!(
                    desktop
                        .window
                        .activity_overview
                        .as_ref()
                        .unwrap()
                        .entity_id(),
                    overview
                );
                assert_eq!(
                    desktop.window.dock.entity_id(),
                    dock,
                    "toggle retains workspace and pane state"
                );
            })
        });
        // Theme globals are app-scoped: retire this fixture before opening
        // the next window with a different stored appearance.
        visual.update(|window, _| window.remove_window());
    }
    design::set_tile_theme(NativeTileTheme::Monitter);
}

#[gpui_kit::test]
fn activity_overview_card_opens_live_watch_panel_without_workspace_navigation(
    cx: &mut TestAppContext,
) {
    let (desktop, visual) = fixture(cx, "light", NativeTileTheme::Monitter, 14);
    let mut retained_chat = None;
    let mut other_chat = None;
    visual.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            let mut snapshot = (*desktop.snapshot).clone();
            snapshot["tasks"].as_array_mut().unwrap().push(json!({
                "id":"overview-task-two","title":"Review second agent",
                "agentId":"overview-agent-two","projectId":"overview-project",
                "status":"running","updatedAt":chrono::Utc::now().timestamp_millis(),
                "queue":[]
            }));
            snapshot["agents"].as_array_mut().unwrap().push(json!({
                "id":"overview-agent-two","name":"Claude","provider":"claude"
            }));
            desktop.snapshot = Rc::new(snapshot);
            let pane = cx.new(|cx| {
                chat::ChatPane::performance_fixture(
                    "overview-task".into(),
                    desktop.snapshot.clone(),
                    window,
                    cx,
                )
            });
            desktop
                .chat_panes
                .borrow_mut()
                .insert("overview-task".into(), pane.clone());
            retained_chat = Some(pane.clone());
            let second = cx.new(|cx| {
                chat::ChatPane::performance_fixture(
                    "overview-task-two".into(),
                    desktop.snapshot.clone(),
                    window,
                    cx,
                )
            });
            desktop.chat_panes.borrow_mut().insert("overview-task-two".into(), second.clone());
            other_chat = Some(second);
            desktop.window.dock.update(cx, |dock, cx| {
                dock.add(
                    docking::PaneSpec {
                        id: "chat:overview-task".into(),
                        title: "Existing chat".into(),
                        content: pane.clone().into(),
                        focus: pane.focus_handle(cx),
                        lifecycle: None,
                        closable: true,
                    },
                    gpui_kit::component::dock::DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            });
            desktop.toggle_activity_overview(window, cx);
        })
    });
    draw(visual);
    let selected = desktop.read_with(visual, |desktop, _| desktop.window.selected.clone());
    let pane = retained_chat.unwrap();
    let second = other_chat.unwrap();
    for iteration in 0..2 {
        let card = visual.debug_bounds("activity-card-overview-task").unwrap();
        visual.simulate_click(card.center(), Default::default());
        draw(visual);
        assert!(visual.debug_bounds("activity-agent-panel").is_some());
        assert!(visual.debug_bounds("chat-watcher").is_some());
        let panel = visual.debug_bounds("activity-agent-panel").unwrap();
        assert!(panel.size.width <= px(460.) && panel.size.height > px(440.), "docked panel bounds: {panel:?}");
        visual.simulate_resize(size(px(540.), px(700.)));
        draw(visual);
        let narrow_panel = visual.debug_bounds("activity-agent-panel").expect("panel remains mounted at narrow width");
        let narrow_overview = visual.debug_bounds("activity-overview").unwrap();
        assert!(narrow_panel.left() >= narrow_overview.left() && narrow_panel.right() <= narrow_overview.right(),
            "narrow panel stays inside Activity: {narrow_panel:?} / {narrow_overview:?}");
        visual.simulate_resize(size(px(1160.), px(780.)));
        draw(visual);
        assert!(visual.debug_bounds("watcher-now").unwrap().size.height <= px(78.), "panel status stays within its original height");
        assert!(visual.debug_bounds("watcher-agents").is_none(), "empty subagent section is omitted");
        if iteration == 0 {
            assert!(visual.debug_bounds("activity-watch-active-glow").is_some());
            assert!(visual.debug_bounds("activity-watch-sparkles").is_some());
        }
        desktop.read_with(visual, |desktop, cx| {
            assert!(desktop.window.activity_overview_open);
            assert_eq!(desktop.window.selected, selected);
            assert_eq!(
                desktop.chat_panes.borrow()["overview-task"].entity_id(),
                pane.entity_id()
            );
            assert!(
                !pane.read(cx).watcher_is_open(),
                "panel must preserve docked Chat mode"
            );
            assert!(pane.read(cx).user_questions_in_watch());
            assert!(
                pane.read(cx).wants_history_publication(),
                "panel keeps live history subscribed"
            );
            assert!(!pane.read(cx).can_trim_hidden_history());
        });
        let steer = visual
            .debug_bounds("watcher-back")
            .expect("native Watch steer button");
        visual.simulate_click(steer.center(), Default::default());
        draw(visual);
        assert!(visual.debug_bounds("watcher-interject-menu").is_some());
        visual.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                let mut next = (*pane.snapshot).clone();
                next["tasks"][0]["status"] = json!("completed");
                next["events"] = json!([{"taskId":"overview-task","title":"Panel live update","createdAt":chrono::Utc::now().timestamp_millis()}]);
                pane.sync(Rc::new(next), Some("panel-live-update".into()), cx);
            });
        });
        draw(visual);
        assert!(
            visual.debug_bounds("watcher-latest-update").is_some(),
            "retained Watch remains mounted after publication"
        );
        assert!(visual.debug_bounds("activity-watch-active-glow").is_none(), "completed task stops pulsing");
        assert!(visual.debug_bounds("activity-watch-sparkles").is_none(), "completed task drops its animation field");
        // First Escape dismisses the native steer popover; the panel then
        // supports both keyboard and pointer dismissal.
        visual.simulate_keystrokes("escape");
        draw(visual);
        if iteration == 0 {
            visual.simulate_keystrokes("escape");
        } else {
            let close = visual.debug_bounds("activity-agent-panel-close").unwrap();
            visual.simulate_click(close.center(), Default::default());
        }
        draw(visual);
        assert!(visual.debug_bounds("activity-agent-panel").is_none());
        assert!(visual.debug_bounds("activity-card-overview-task").is_some());
        desktop.read_with(visual, |desktop, cx| {
            assert!(desktop.window.activity_overview_open);
            assert_eq!(desktop.window.selected, selected);
            assert!(
                !pane.read(cx).user_questions_in_watch(),
                "close cleans up panel mode"
            );
        });
    }
    let first = visual.debug_bounds("activity-card-overview-task").unwrap();
    visual.simulate_click(first.center(), Default::default());
    draw(visual);
    assert!(pane.read_with(visual, |pane, _| pane.user_questions_in_watch()));
    let next = visual.debug_bounds("activity-card-overview-task-two").unwrap();
    let panel = visual.debug_bounds("activity-agent-panel").unwrap();
    assert!(next.center().x < panel.left(), "second card stays accessible beside panel: {next:?} / {panel:?}");
    visual.simulate_click(next.center(), Default::default());
    draw(visual);
    assert!(visual.debug_bounds("activity-agent-panel").is_some());
    assert!(second.read_with(visual, |pane, _| pane.user_questions_in_watch()));
    assert!(!pane.read_with(visual, |pane, _| pane.user_questions_in_watch()),
        "switching cards releases the previous Watch view");
    desktop.read_with(visual, |desktop, _| assert_eq!(desktop.window.selected, selected));
    visual.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.open_activity_overview_task("overview-task".into(), window, cx);
        });
    });
    draw(visual);
    visual.update(|window, cx| {
        desktop.update(cx, |desktop, cx| desktop.close_activity_overview(window, cx));
    });
    draw(visual);
    pane.read_with(visual, |pane, _| {
        assert!(
            !pane.user_questions_in_watch(),
            "programmatic dismissal must release Watch polling and modal state"
        )
    });
    second.read_with(visual, |pane, _| {
        assert!(!pane.user_questions_in_watch(), "switch target also releases on overview close")
    });
}

#[gpui_kit::test]
fn activity_hub_native_drag_moves_hub_and_settles_connected_card(cx: &mut TestAppContext) {
    let (desktop, visual) = fixture(cx, "light", NativeTileTheme::Monitter, 14);
    visual.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.toggle_activity_overview(window, cx)
        })
    });
    draw(visual);
    let hub = visual
        .debug_bounds("activity-hub-overview-project")
        .unwrap();
    let card = visual.debug_bounds("activity-card-overview-task").unwrap();
    let start = hub.center();
    let end = gpui_kit::point(start.x + px(70.), start.y + px(50.));
    visual.simulate_mouse_down(start, gpui_kit::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(end, Some(gpui_kit::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(end, gpui_kit::MouseButton::Left, Default::default());
    draw(visual);
    let moved = visual
        .debug_bounds("activity-hub-overview-project")
        .unwrap();
    assert!(
        (f32::from(moved.left() - hub.left()) - f32::from(end.x - start.x)).abs() < 1.,
        "native hub drag: before={hub:?}, after={moved:?}, requested={:?}",
        end - start
    );
    for _ in 0..120 {
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(visual);
    }
    let followed = visual.debug_bounds("activity-card-overview-task").unwrap();
    let settled_hub = visual.debug_bounds("activity-hub-overview-project").unwrap();
    assert!(settled_hub.left() > moved.left() + px(10.), "released project carries momentum");
    assert!(settled_hub.left() < moved.left() + px(150.), "glide stays bounded");
    assert!((f32::from(followed.left() - card.left()) - f32::from(settled_hub.left() - hub.left())).abs() < 1.);
    assert!((f32::from(followed.top() - card.top()) - f32::from(settled_hub.top() - hub.top())).abs() < 1.);
}

#[gpui_kit::test]
fn activity_overview_retains_unseen_messages_before_first_open(cx: &mut TestAppContext) {
    let (desktop, visual) = fixture(cx, "dark", NativeTileTheme::Tern, 14);
    draw(visual);
    let retained = desktop.read_with(visual, |desktop, _| {
        assert!(!desktop.window.activity_overview_open);
        desktop.window.activity_overview.clone().expect("observe before first opening")
    });
    desktop.update(visual, |desktop, cx| {
        let snapshot = Rc::make_mut(&mut desktop.snapshot);
        snapshot["collaborations"] = json!([{
            "id":"received-while-hidden","kind":"message","status":"completed",
            "fromTaskId":"overview-task","toTaskId":"recipient",
            "createdAt":chrono::Utc::now().timestamp_millis()-600_000
        }]);
        cx.notify();
    });
    draw(visual);
    retained.read_with(visual, |overview, _| {
        assert_eq!(overview.communication_viewed_time("overview-task","recipient"),
            Some(std::time::Duration::ZERO), "unseen time must not expire before first opening");
    });
    desktop.update(visual, |desktop, cx| {
        assert_eq!(desktop.window.activity_overview.as_ref(), Some(&retained));
        cx.notify();
    });
    let toggle = visual.debug_bounds("activity-overview-toggle").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    draw(visual);
    desktop.read_with(visual, |desktop, _| {
        assert!(desktop.window.activity_overview_open);
        assert_eq!(desktop.window.activity_overview.as_ref(), Some(&retained));
    });
}
