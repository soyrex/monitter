use super::{ChatPane, ChatEvent};
use gpui_kit::{AppContext as _, Context, Entity, Render, Window, IntoElement, ParentElement as _, Styled as _, TestAppContext, VisualTestContext, MouseButton, div, point, px, size};
use serde_json::json;
use std::{rc::Rc, cell::RefCell};

struct LinkSurface { dock: Entity<crate::docking::DockWorkspace> }
impl Render for LinkSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.dock.clone())
    }
}

#[gpui::test]
fn chat_links_click_in_retained_text_reaches_the_chat_owner(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let source = Rc::new(json!({
        "settings":{"theme":"dark"},
        "tasks":[{"id":"link-task","agentId":"agent","provider":"codex","status":"idle","queue":[]}],
        "agents":[{"id":"agent","provider":"codex","name":"Codex"}],
        "messages":[{"id":"link-message","taskId":"link-task","role":"assistant","text":"[Open website](https://example.com)","createdAt":1700000000000_i64}],
        "events":[],"hosts":[],"channels":[],"approvalRequests":[]
    }));
    let mut retained = None;
    let mut retained_surface = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let pane = cx.new(|cx| ChatPane::performance_fixture("link-task".into(), source, window, cx));
        let factory: crate::docking::PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| crate::docking::DockWorkspace::new("chat-link-dock", factory, window, cx));
        let spec = crate::docking::PaneSpec {
            id: "chat:link-task".into(), title: "Link chat".into(),
            content: pane.clone().into(), focus: gpui_kit::Focusable::focus_handle(pane.read(cx), cx),
            lifecycle: None, closable: true,
        };
        dock.update(cx, |dock, cx| {
            dock.add(spec, gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
        });
        let links = cx.new(|_| LinkSurface { dock });
        retained_surface = Some(links.clone());
        retained = Some(pane);
        gpui_kit::component::Root::new(links, window, cx)
    });
    let visual: &mut VisualTestContext = visual;
    let pane = retained.unwrap();
    let observed = Rc::new(RefCell::new(Vec::new()));
    let captured = observed.clone();
    let _subscription = visual.update(|_, cx| cx.subscribe(&pane, move |_, event: &ChatEvent, _| {
        if let ChatEvent::OpenBrowser { url, task_id } = event {
            captured.borrow_mut().push((url.clone(), task_id.clone()));
        }
    }));
    visual.simulate_resize(size(px(800.), px(700.)));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|_, cx| retained_surface.as_ref().unwrap().update(cx, |_, cx| cx.notify()));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let body = visual.debug_bounds("message-body-link-message").expect("rendered chat link");
    let position = body.origin + point(px(10.), px(10.));
    visual.simulate_mouse_down(position, MouseButton::Left, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(&*observed.borrow(), &[("https://example.com".into(), "link-task".into())]);
}
