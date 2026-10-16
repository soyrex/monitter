//! Disconnected native backdrop check. Compiled only in profiling builds.
use crate::{design, window_surface};
use gpui_kit::{component::{ActiveTheme as _, Sizable as _, button::*}, prelude::*, *};
use serde_json::json;

struct SurfaceReference;
impl Render for SurfaceReference {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().children([0xf04b4b, 0x42c97a, 0x476ff0, 0xffffff].into_iter()
            .map(|color| div().flex_1().h_full().bg(rgb(color))))
    }
}
struct SurfacePreview { mode: &'static str }
impl Render for SurfacePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = json!({"windowSurface":if self.mode == "clear" {"glass"} else {self.mode},"windowTransparency":if self.mode == "clear" {70} else {31},"theme":"dark"});
        design::set_surface(&settings);
        if self.mode == "clear" {
            window.set_background_appearance(WindowBackgroundAppearance::Transparent);
        } else {
            window_surface::apply(window, self.mode);
        }
        let palette = design::palette(&settings, window.appearance());
        div().size_full().p(px(28.)).flex().flex_col().gap(px(20.))
            .bg(design::surface_color(palette.paper)).text_color(rgb(palette.ink))
            .font_family(cx.theme().font_family.clone())
            .child(div().text_size(px(22.)).child("Native backdrop preview"))
            .child("Isolated window · no backend, agent, profile or saved preference access")
            .child(format!("{} · {}% transparency", self.mode, if self.mode == "clear" {70} else {31}))
            .child(div().flex().gap(px(12.)).children(["glass", "translucent", "opaque", "clear"].into_iter().map(|mode| {
                Button::new(mode).compact().label(mode).on_click(cx.listener(move |this, _, _, cx| {
                    this.mode=mode; cx.notify();
                }))
            })))
            .child("Content behind this window should be blurred in Glass and Translucent modes. Opaque provides the comparison.")
    }
}

pub(crate) fn open(cx: &mut App) {
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    // The component root normally paints an opaque theme background. This
    // isolated material check must leave it clear; the preview paints its own tint.
    gpui_kit::component::Theme::update(cx, |theme| theme.colors.background = rgba(0).into());
    if let Err(error)=monitter_gpui_desktop::fonts::register(cx) { eprintln!("{error}"); }
    gpui_kit::open_window(WindowOptions {
        window_bounds:Some(WindowBounds::Windowed(Bounds::centered(None,size(px(860.),px(580.)),cx))),
        titlebar:Some(TitlebarOptions { title:Some("Monitter — isolated backdrop reference".into()), ..Default::default() }),
        ..Default::default()
    },cx,|_,cx|cx.new(|_|SurfaceReference)).expect("open isolated color reference");
    gpui_kit::open_window(WindowOptions {
        window_bounds:Some(WindowBounds::Windowed(Bounds::centered(None,size(px(660.),px(440.)),cx))),
        titlebar:Some(TitlebarOptions { title:Some("Monitter — native backdrop preview".into()), appears_transparent:true, ..Default::default() }),
        ..Default::default()
    },cx,|window,cx| {
        window.on_window_should_close(cx, |_, cx| { cx.quit(); true });
        cx.new(|_|SurfacePreview{mode:"glass"})
    }).expect("open isolated backdrop preview");
    cx.activate(true);
}
