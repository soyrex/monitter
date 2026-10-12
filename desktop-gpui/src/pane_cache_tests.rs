//! Regression coverage for the content cache and per-frame visibility probe
//! used by `NativePanel` in `docking.rs`.
use super::{NativePanel, PaintState, PaneSpec};
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, div,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct CountedContent {
    renders: Rc<Cell<usize>>,
}

impl Render for CountedContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full()
    }
}

struct NativePanelCacheHarness {
    panel: Entity<NativePanel>,
    renders: Rc<Cell<usize>>,
}

impl Render for NativePanelCacheHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full().child(self.panel.clone())
    }
}

/// A workspace redraw must replay cached pane content, while a notification
/// from that content invalidates the cache. The visibility canvas is outside
/// the cached subtree, so it must still mark the pane painted on each frame.
#[gpui::test]
fn native_panel_caches_content_but_repaints_visibility_each_frame(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));

    let renders = Rc::new(Cell::new(0));
    let harness_renders = Rc::new(Cell::new(0));
    let paint = Rc::new(RefCell::new(PaintState {
        shown: true,
        ..Default::default()
    }));
    let content_entity = Rc::new(RefCell::new(None));
    let render_count = renders.clone();
    let paint_state = paint.clone();
    let retained_content = content_entity.clone();
    let retained_harness_renders = harness_renders.clone();
    let (harness, cx) = cx.add_window_view(move |_, cx| {
        let content = cx.new(|_| CountedContent {
            renders: render_count,
        });
        *retained_content.borrow_mut() = Some(content.clone());
        let focus = cx.focus_handle();
        let panel = cx.new(|_| NativePanel {
            spec: PaneSpec {
                id: "cache-test-pane".into(),
                title: "Cache test".into(),
                content: content.clone().into(),
                focus,
                lifecycle: None,
                closable: true,
            },
            paint: paint_state,
            removed: false,
            retaining_layout: Rc::new(Cell::new(false)),
            appearance: Rc::new(Cell::new(None)),
            auto_hide_tabs: Rc::new(Cell::new(false)),
        });
        NativePanelCacheHarness {
            panel,
            renders: retained_harness_renders,
        }
    });

    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let content_baseline = renders.get();
    assert!(content_baseline > 0, "initial frames render pane content");
    assert!(paint.borrow().painted.contains("cache-test-pane"));

    // An unrelated parent invalidation redraws NativePanel but should replay
    // its cached content. Clearing the probe first proves its canvas still ran.
    paint.borrow_mut().painted.clear();
    let parent_baseline = harness_renders.get();
    harness.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        harness_renders.get() > parent_baseline,
        "parent redraw occurred"
    );
    assert_eq!(
        renders.get(),
        content_baseline,
        "unrelated parent redraw reuses content"
    );
    assert!(
        paint.borrow().painted.contains("cache-test-pane"),
        "uncached visibility canvas marks the pane painted on the replayed frame"
    );

    // A content entity change must invalidate the cached subtree.
    let content = content_entity
        .borrow()
        .as_ref()
        .expect("test content entity retained")
        .clone();
    content.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        renders.get(),
        content_baseline + 1,
        "content notification refreshes cache"
    );
}

struct ScrollingContent {
    list: gpui::ListState,
    renders: Rc<Cell<usize>>,
}
impl Render for ScrollingContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        gpui::list(self.list.clone(), |_, _, _| {
            div().h(gpui::px(60.)).w_full().into_any_element()
        })
        .w_full()
        .h_full()
    }
}

#[gpui::test]
fn cached_native_panel_scrolls_repeatedly_after_parent_replay(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let state = gpui::ListState::new(100, gpui::ListAlignment::Top, gpui::px(100.));
    let list = state.clone();
    let renders = Rc::new(Cell::new(0));
    let render_count = renders.clone();
    let (harness, cx) = cx.add_window_view(move |_, cx| {
        let content = cx.new(|_| ScrollingContent {
            list,
            renders: render_count,
        });
        let focus = cx.focus_handle();
        let panel = cx.new(|_| NativePanel {
            spec: PaneSpec {
                id: "chat:scroll-test".into(),
                title: "Scroll".into(),
                content: content.into(),
                focus,
                lifecycle: None,
                closable: true,
            },
            paint: Rc::new(RefCell::new(PaintState::default())),
            removed: false,
            retaining_layout: Rc::new(Cell::new(false)),
            appearance: Rc::new(Cell::new(None)),
            auto_hide_tabs: Rc::new(Cell::new(false)),
        });
        NativePanelCacheHarness {
            panel,
            renders: Rc::new(Cell::new(0)),
        }
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    harness.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let baseline = renders.get();
    for _ in 0..3 {
        let before = state.logical_scroll_top();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(gpui::px(100.), gpui::px(100.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-120.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after = state.logical_scroll_top();
        assert!(
            after.item_ix > before.item_ix || after.offset_in_item > before.offset_in_item,
            "each wheel moves the list"
        );
    }
    assert!(renders.get() > baseline, "wheel invalidates cached content");
}

struct NestedCachedParent {
    child: Entity<CountedContent>,
    renders: Rc<Cell<usize>>,
    width: f32,
    color: u32,
}
impl Render for NestedCachedParent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().w(gpui::px(self.width)).h_full().text_color(gpui::rgb(self.color))
            .child(self.child.clone().cached(gpui::StyleRefinement::default().size_full()))
    }
}
struct NestedCacheHarness(Entity<NestedCachedParent>);
impl Render for NestedCacheHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone().cached(gpui::StyleRefinement::default().size_full()))
    }
}

#[gpui::test]
fn nested_cache_rebuilds_skipped_child_and_respects_changes_and_explicit_refresh(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let parent_renders = Rc::new(Cell::new(0));
    let child_renders = Rc::new(Cell::new(0));
    let parent_slot = Rc::new(RefCell::new(None));
    let retained = parent_slot.clone();
    let parents = parent_renders.clone();
    let children = child_renders.clone();
    let (_, cx) = cx.add_window_view(move |_, cx| {
        let child = cx.new(|_| CountedContent { renders: children });
        let parent = cx.new(|_| NestedCachedParent { child, renders: parents, width: 180., color: 0x222222 });
        *retained.borrow_mut() = Some(parent.clone());
        NestedCacheHarness(parent)
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let parent = parent_slot.borrow().as_ref().unwrap().clone();
    let child = parent.update(cx, |parent, _| parent.child.clone());
    let initial_child = child_renders.get();
    let initial_parent = parent_renders.get();
    parent.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(parent_renders.get() > initial_parent);
    assert!(child_renders.get() > initial_child, "a child skipped during parent replay rebuilds its frame-specific ranges");
    child.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after_notify = child_renders.get();
    assert!(after_notify > initial_child, "child notification invalidates its cache");
    parent.update(cx, |parent, cx| { parent.width = 240.; cx.notify(); });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after_bounds = child_renders.get();
    assert!(after_bounds > after_notify, "changed child bounds invalidate its cache");
    parent.update(cx, |parent, cx| { parent.color = 0xff0000; cx.notify(); });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after_text = child_renders.get();
    assert!(after_text > after_bounds, "inherited text style invalidates its cache");
    cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
    assert!(child_renders.get() > after_text, "explicit refresh invalidates clean descendants");
}
