//! A modal/prefix can relocate an ancestor cache while skipping child caches.
use gpui::{AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window, div, px, point};
use std::{cell::{Cell, RefCell}, rc::Rc};

struct TextChild { renders: Rc<Cell<usize>>, clicks: Rc<Cell<usize>> }
impl Render for TextChild {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get()+1);
        let clicks = self.clicks.clone();
        div().id("cached-modal-text-child").size_full().child("Cached attachment transcript text")
            .on_mouse_down(gpui::MouseButton::Left, move |_,_,_| clicks.set(clicks.get()+1))
    }
}
struct TextParent { child: Entity<TextChild>, renders: Rc<Cell<usize>> }
impl Render for TextParent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get()+1);
        div().size_full().child(self.child.clone().cached(gpui::StyleRefinement::default().size_full()))
    }
}
struct Harness { parent: Entity<TextParent>, prefix: bool }
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().size_full();
        // Absolute prefix shapes text before the parent without changing its bounds.
        if self.prefix { body = body.child(div().absolute().top(px(300.)).child("New modal text before cached workspace")); }
        body.child(self.parent.clone().cached(gpui::StyleRefinement::default().size_full()))
    }
}
#[gpui::test]
fn cached_modal_replay_rebuilds_skipped_descendant_ranges_and_preserves_hitboxes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let children = Rc::new(Cell::new(0));
    let parents = Rc::new(Cell::new(0));
    let clicks = Rc::new(Cell::new(0));
    let slot = Rc::new(RefCell::new(None));
    let retained = slot.clone();
    let child_count = children.clone(); let parent_count = parents.clone(); let click_count = clicks.clone();
    let (root,cx) = cx.add_window_view(move |_,cx| {
        let child = cx.new(|_| TextChild { renders: child_count, clicks: click_count });
        let parent = cx.new(|_| TextParent { child, renders: parent_count });
        *retained.borrow_mut() = Some(parent.clone());
        Harness { parent, prefix: false }
    });
    cx.run_until_parked();
    cx.update(|window,cx| window.draw(cx).clear(cx));
    let child_before = children.get(); let parent_before = parents.get();
    root.update(cx, |root,cx| { root.prefix = true; cx.notify(); });
    cx.run_until_parked();
    cx.update(|window,cx| window.draw(cx).clear(cx));
    assert_eq!(parents.get(),parent_before,"clean ancestor continues to replay");
    assert_eq!(children.get(),child_before,"ancestor replay skips child render");
    let parent = slot.borrow().as_ref().unwrap().clone();
    parent.update(cx, |_,cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window,cx| window.draw(cx).clear(cx));
    assert!(children.get()>child_before,"direct traversal rebuilds stale descendant ranges");
    cx.simulate_event(gpui::MouseDownEvent { position: point(px(50.),px(50.)), button: gpui::MouseButton::Left, ..Default::default() });
    assert_eq!(clicks.get(),1,"rebuilt text child keeps its actual hitbox");
    // Removing preceding text is the dangerous shrinking-vector direction.
    root.update(cx, |root,cx| { root.prefix = false; cx.notify(); });
    cx.run_until_parked();
    cx.update(|window,cx| window.draw(cx).clear(cx));
    parent.update(cx, |_,cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window,cx| window.draw(cx).clear(cx));
}
