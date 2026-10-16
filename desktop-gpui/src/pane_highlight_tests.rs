//! GPUI coverage matching DockWorkspace / DockArea / NativePanel cache topology.
use super::pane_highlight_state::OverlayFrameState;
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, canvas, div,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct CachedChatContent {
    renders: Rc<Cell<usize>>,
}

impl Render for CachedChatContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full().child("cached chat content")
    }
}

/// Represents NativePanel::pane_overlay, which TabGroupRenderer adds outside
/// NativePanel's cached chat content.
struct PaneOverlay {
    frame: Rc<RefCell<OverlayFrameState>>,
    paints: Rc<Cell<usize>>,
    renders: Rc<Cell<usize>>,
    active: bool,
}

struct SparkleSibling {
    renders: Rc<Cell<usize>>,
}

impl Render for SparkleSibling {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full()
    }
}

impl Render for PaneOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        self.frame.borrow_mut().observe_candidate(self.active);
        if !self.active {
            return div().size_full().into_any_element();
        }

        let frame = self.frame.clone();
        let paints = self.paints.clone();
        gpui::deferred(
            canvas(
                |_, _, _| (),
                move |_, _, _, _| {
                    paints.set(paints.get() + 1);
                    if frame.borrow_mut().mark_painted() {
                        super::record_show();
                    }
                    super::record_geometry_paint();
                },
            )
            .absolute()
            .inset_0(),
        )
        .with_priority(1)
        .into_any_element()
    }
}

struct DockWorkspaceHarness {
    chat: Entity<CachedChatContent>,
    overlay: Entity<PaneOverlay>,
    sparkle: Entity<SparkleSibling>,
    frame: Rc<RefCell<OverlayFrameState>>,
    hides: Rc<Cell<usize>>,
}

impl Render for DockWorkspaceHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Mirrors Desktop::render followed by DockWorkspace::render. The dock
        // observation starts empty; the uncached TabGroup overlay promotes it.
        self.frame.borrow_mut().begin_frame();
        self.frame.borrow_mut().observe_candidate(false);
        let frame = self.frame.clone();
        let hides = self.hides.clone();
        div()
            .size_full()
            .child(
                self.chat
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full()),
            )
            .child(self.overlay.clone())
            .child(self.sparkle.clone())
            .child(
                gpui::deferred(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, _| {
                            if frame.borrow_mut().finish_frame() {
                                super::record_hide();
                                hides.set(hides.get() + 1);
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .with_priority(5),
            )
    }
}

#[gpui::test]
fn active_outline_survives_cached_chat_replay_and_hides_when_pane_loses_candidate(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| gpui_kit::init(cx));

    let frame = Rc::new(RefCell::new(OverlayFrameState::default()));
    let content_renders = Rc::new(Cell::new(0));
    let overlay_renders = Rc::new(Cell::new(0));
    let paints = Rc::new(Cell::new(0));
    let sparkle_renders = Rc::new(Cell::new(0));
    let hides = Rc::new(Cell::new(0));
    let retained_overlay = Rc::new(RefCell::new(None));
    let content_count = content_renders.clone();
    let overlay_count = overlay_renders.clone();
    let paint_count = paints.clone();
    let retained = retained_overlay.clone();
    let retained_frame = frame.clone();
    let retained_hides = hides.clone();
    let retained_sparkle_renders = sparkle_renders.clone();
    let (harness, cx) = cx.add_window_view(move |_, cx| {
        let chat = cx.new(|_| CachedChatContent {
            renders: content_count,
        });
        let overlay = cx.new(|_| PaneOverlay {
            frame: retained_frame.clone(),
            paints: paint_count,
            renders: overlay_count,
            active: true,
        });
        let sparkle = cx.new(|_| SparkleSibling {
            renders: retained_sparkle_renders,
        });
        *retained.borrow_mut() = Some(overlay.clone());
        DockWorkspaceHarness {
            chat,
            overlay,
            sparkle,
            frame: retained_frame,
            hides: retained_hides,
        }
    });

    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        content_renders.get() > 0,
        "initial frame renders chat content"
    );
    assert!(
        overlay_renders.get() > 0,
        "initial frame renders active pane overlay"
    );
    assert!(paints.get() > 0, "deferred pane outline paints");
    assert_eq!(hides.get(), 0, "active candidate remains visible");
    let initial_counts = super::performance_counts();

    let content_baseline = content_renders.get();
    let overlay_baseline = overlay_renders.get();
    let paint_baseline = paints.get();
    // Represents a SparkleField animation tick invalidating its own retained
    // child while the parent workspace still performs an ordinary draw.
    let sparkle = harness.read_with(cx, |harness, _| harness.sparkle.clone());
    sparkle.update(cx, |_, cx| cx.notify());
    harness.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        content_renders.get(),
        content_baseline,
        "chat content cache replays"
    );
    assert!(
        overlay_renders.get() > overlay_baseline,
        "TabGroup overlay is outside that cache"
    );
    assert!(
        paints.get() > paint_baseline,
        "the current active outline is painted again"
    );
    assert_eq!(
        hides.get(),
        0,
        "the workspace's default absent observation is promoted by the active pane"
    );
    assert!(
        sparkle_renders.get() > 0,
        "the animated decoration sibling stays present"
    );
    let steady_counts = super::performance_counts();
    assert_eq!(
        steady_counts.shows - initial_counts.shows,
        0,
        "steady active frames do not reshow the native view"
    );
    assert_eq!(
        steady_counts.hides - initial_counts.hides,
        0,
        "steady active frames do not hide the native view"
    );
    assert!(
        steady_counts.geometry_paints > initial_counts.geometry_paints,
        "outline geometry is painted without visibility toggles"
    );

    let overlay = retained_overlay.borrow().as_ref().unwrap().clone();
    overlay.update(cx, |overlay, cx| {
        overlay.active = false;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        hides.get(),
        1,
        "a rendered pane with no candidate hides the outline once"
    );
    let absent_counts = super::performance_counts();
    assert_eq!(
        absent_counts.hides - steady_counts.hides,
        1,
        "native counters record the real visibility transition"
    );
}
