//! Mounted offline fixtures: no WKWebView, datastore, network, or real clipboard.
use super::*;
use ::core::prelude::v1::test;
use gpui_kit::{TestAppContext, component::{Root, Theme, ThemeMode}};
use std::{cell::RefCell, rc::Rc};

fn mounted(cx: &mut TestAppContext, light: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    });
    let slot = Rc::new(RefCell::new(None));
    let output = slot.clone();
    let (_, visual) = cx.add_window_view(move |window,cx| {
        let tab = cx.new(|cx| {
            let mut tab = BrowserTab::new_with_native(window,cx,Err("offline context fixture".into()),None);
            tab.visible = true;
            tab.unloaded = false;
            tab.docked_mode = true;
            tab.error = None;
            tab.location = "https://example.com/current".into();
            tab
        });
        *output.borrow_mut() = Some(tab.clone());
        Root::new(tab,window,cx)
    });
    let tab = slot.borrow().as_ref().unwrap().clone();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let events = opened.clone();
    let _subscription = visual.update(|_,cx| cx.subscribe(&tab,move |_,event:&BrowserTabEvent,_| {
        if let BrowserTabEvent::OpenInNewPane(url) = event { events.borrow_mut().push(url.clone()); }
    }));
    for width in [280.,600.] {
        visual.simulate_resize(size(gpui::px(width),gpui::px(320.)));
        for link in [false,true] {
            for point in [(0.02,0.02),(0.49,0.49),(0.98,0.98)] {
                visual.update(|window,cx| tab.update(cx,|tab,cx| {
                    tab.context_menu = Some(link.then(|| "https://example.com/selected".into()));
                    tab.context_point = point;
                    tab.focus.focus(window,cx);
                    cx.notify();
                }));
                visual.update(|window,cx| window.draw(cx).clear(cx));
                let card = visual.debug_bounds("browser-context-card").expect("context menu mounted");
                assert!(card.left() >= gpui::px(-0.5) && card.right() <= gpui::px(width+0.5), "menu stays inside narrow pane: {card:?}");
                assert!(card.top() >= gpui::px(-0.5) && card.bottom() <= gpui::px(320.5), "menu stays inside vertical bounds: {card:?}");
                assert!(card.size.width >= gpui::px(210.), "full menu width preserved at every click position");
                for selector in ["browser-context-system", "browser-context-new-pane", "browser-context-copy"] {
                    if let Some(item) = visual.debug_bounds(selector) {
                        assert!(item.size.height > gpui::px(0.), "item has visible height");
                        assert!(item.top() >= card.top() && item.bottom() <= card.bottom(), "item fully inside card: {selector}: {item:?}, {card:?}");
                        assert!(item.left() >= card.left() && item.right() <= card.right(), "item inside card horizontally");
                    }
                }
                assert!(visual.debug_bounds("browser-context-system").is_some());
                assert_eq!(visual.debug_bounds("browser-context-new-pane").is_some(),link);
                assert_eq!(visual.debug_bounds("browser-context-copy").is_some(),link);
                visual.simulate_keystrokes("escape");
                visual.update(|_,cx| tab.update(cx,|tab,_| {
                    assert!(tab.context_menu.is_none(),"Escape dismisses context menu");
                    assert!(tab.context_snapshot.is_none());
                }));
            }
        }
    }
    // The selected link, rather than current page, must be emitted by the item.
    visual.update(|window,cx| tab.update(cx,|tab,cx| {
        tab.context_menu=Some(Some("https://example.com/selected".into()));
        tab.context_point=(0.1,0.1);
        tab.focus.focus(window,cx); cx.notify();
    }));
    visual.update(|window,cx| window.draw(cx).clear(cx));
    let item=visual.debug_bounds("browser-context-new-pane").unwrap();
    visual.simulate_click(item.center(),Default::default());
    assert_eq!(&*opened.borrow(),&["https://example.com/selected".to_string()]);
    visual.update(|_,cx| tab.update(cx,|tab,_| assert!(tab.context_menu.is_none())));
    visual.update(|window,cx| tab.update(cx,|tab,cx| {
        tab.context_menu=Some(Some("https://example.com/selected".into()));
        tab.context_point=(0.1,0.1);
        tab.focus.focus(window,cx); cx.notify();
    }));
    visual.update(|window,cx| window.draw(cx).clear(cx));
    let copy=visual.debug_bounds("browser-context-copy").unwrap();
    visual.simulate_click(copy.center(),Default::default());
    visual.update(|_,cx| {
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(),"https://example.com/selected");
        tab.update(cx,|tab,_| assert!(tab.context_menu.is_none()));
    });
    visual.update(|window,cx| tab.update(cx,|tab,cx| {
        tab.context_menu=Some(None);tab.context_point=(0.1,0.1);tab.focus.focus(window,cx);cx.notify();
    }));
    visual.update(|window,cx| window.draw(cx).clear(cx));
    visual.simulate_click(point(gpui::px(590.),gpui::px(300.)),Default::default());
    visual.update(|_,cx| tab.update(cx,|tab,cx| {
        assert!(tab.context_menu.is_none(),"outside click dismisses menu");
        tab.context_menu=Some(None);tab.hide(cx);
        assert!(tab.context_menu.is_none(),"hide invalidates context");
        assert!(tab.context_snapshot.is_none());
        tab.context_menu=Some(None);tab.close(cx);
        assert!(tab.context_menu.is_none(),"close invalidates context");
        assert!(tab.context_click.is_none());
    }));
}
#[gpui_kit::test]
fn browser_context_mounted_dark(cx: &mut TestAppContext) { mounted(cx,false); }
#[gpui_kit::test]
fn browser_context_mounted_light(cx: &mut TestAppContext) { mounted(cx,true); }

struct OffsetCachedFixture { tab: Entity<BrowserTab> }
impl Render for OffsetCachedFixture {
    fn render(&mut self,_:&mut Window,_:&mut Context<Self>)->impl IntoElement {
        div().relative().size_full().child(
            div().absolute().left(gpui::px(300.)).top(gpui::px(100.))
                .w(gpui::px(280.)).h(gpui::px(320.))
                .debug_selector(|| "browser-context-offset-pane".into())
                .child(self.tab.clone().cached(StyleRefinement::default().size_full()))
        )
    }
}
fn has_visible_menu_quad(window: &Window, card: Bounds<Pixels>, background: Background) -> bool {
    let expected = card.scale(window.scale_factor());
    window.painted_quads().iter().any(|quad| {
        let near = |a: gpui::ScaledPixels,b: gpui::ScaledPixels| (a.as_f32()-b.as_f32()).abs() <= 0.5;
        let visible = quad.bounds.intersect(&quad.content_mask.bounds);
        quad.background == background
            && near(quad.bounds.origin.x,expected.origin.x)
            && near(quad.bounds.origin.y,expected.origin.y)
            && near(quad.bounds.size.width,expected.size.width)
            && near(quad.bounds.size.height,expected.size.height)
            && visible.size.width.as_f32() >= expected.size.width.as_f32()-1.
            && visible.size.height.as_f32() >= expected.size.height.as_f32()-1.
    })
}

#[gpui_kit::test]
fn browser_context_offset_cached_pane_click_routes_selected_url(cx:&mut TestAppContext) {
    cx.update(|cx| {gpui_kit::init(cx);crate::surface_blend::initialize_for_ui_fixture();});
    let slot=Rc::new(RefCell::new(None));let output=slot.clone();
    let (root,visual)=cx.add_window_view(move |window,cx| {
        let tab=cx.new(|cx| {
            let mut tab=BrowserTab::new_with_native(window,cx,Err("offset offline fixture".into()),None);
            tab.visible=true;tab.unloaded=false;tab.docked_mode=true;tab.error=None;
            tab.context_menu=Some(Some("https://example.com/offset-selected".into()));
            tab.context_point=(0.98,0.98);tab
        });
        *output.borrow_mut()=Some(tab.clone());
        let fixture=cx.new(|_|OffsetCachedFixture{tab});Root::new(fixture,window,cx)
    });
    let tab=slot.borrow().as_ref().unwrap().clone();
    let opened=Rc::new(RefCell::new(Vec::new()));let events=opened.clone();
    let _subscription=visual.update(|_,cx|cx.subscribe(&tab,move |_,event:&BrowserTabEvent,_| {
        if let BrowserTabEvent::OpenInNewPane(url)=event {events.borrow_mut().push(url.clone());}
    }));
    visual.simulate_resize(size(gpui::px(900.),gpui::px(640.)));
    // Prime the cached pane, then invalidate only its browser content as a real
    // native callback does, without rebuilding the parent dock's geometry.
    visual.update(|window,cx|window.draw(cx).clear(cx));
    visual.update(|window,cx|tab.update(cx,|tab,cx| {tab.context_point=(0.49,0.49);tab.focus.focus(window,cx);cx.notify();}));
    visual.update(|window,cx|window.draw(cx).clear(cx));
    let pane=visual.debug_bounds("browser-context-offset-pane").unwrap();
    let card=visual.debug_bounds("browser-context-card").unwrap();
    assert!(card.size.width>=gpui::px(210.));
    assert!(card.left()>=pane.left() && card.right()<=pane.right(),"offset card fits pane horizontally: {card:?} {pane:?}");
    assert!(card.top()>=pane.top() && card.bottom()<=pane.bottom(),"offset card fits pane vertically: {card:?} {pane:?}");
    for selector in ["browser-context-system","browser-context-new-pane","browser-context-copy"] {
        let row=visual.debug_bounds(selector).unwrap();
        assert!(row.size.height>gpui::px(0.) && row.top()>=card.top() && row.bottom()<=card.bottom(),"offset row inside card");
    }
    let background=visual.update(|window,cx| {
        let palette=tab.read(cx).palette.unwrap_or_else(|| crate::design::palette(&serde_json::Value::Null,window.appearance()));
        let background=crate::surface_blend::raised_background(crate::surface_blend::Area::Composer,palette.card);
        assert!(has_visible_menu_quad(window,card,background),"menu must produce a fully visible GPU quad, not only accessibility nodes");
        background
    });
    // Repaint the outer root while retaining the cached browser content. This
    // exercises scene replay rather than merely checking the initial layout.
    visual.update(|_,cx|root.update(cx,|_,cx|cx.notify()));
    visual.update(|window,cx|window.draw(cx).clear(cx));
    visual.update(|window,_|assert!(has_visible_menu_quad(window,card,background),"menu quad survives cached scene replay"));
    visual.simulate_keystrokes("escape");
    visual.update(|window,cx|window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("browser-context-card").is_none());
    visual.update(|window,_|assert!(!has_visible_menu_quad(window,card,background),"Escape removes the menu's GPU quad"));
    visual.update(|window,cx|tab.update(cx,|tab,cx| {
        tab.context_menu=Some(Some("https://example.com/offset-selected".into()));
        tab.focus.focus(window,cx);cx.notify();
    }));
    visual.update(|window,cx|window.draw(cx).clear(cx));
    let item=visual.debug_bounds("browser-context-new-pane").unwrap();
    visual.simulate_click(item.center(),Default::default());
    assert_eq!(&*opened.borrow(),&["https://example.com/offset-selected".to_string()]);
}
