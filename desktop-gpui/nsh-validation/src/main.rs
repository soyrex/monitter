use std::{rc::Rc, time::Duration};
use gpui_kit::{*, component::{Root, button::{Button, ButtonVariants}}};
use nsh_platform_gpui::{App as NshApp, AppOptions, AppView, Bundle, NativeComponent};
use serde_json::{Value, json};

fn record(kind: &str, value: Value) {
    use std::io::Write;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/nsh-validation/runtime.jsonl");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "{}", json!({"kind":kind,"value":value,"at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()})).unwrap();
}
struct Probe { value: String, pane: usize }
impl NativeComponent for Probe {
    fn render(&mut self, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        Some(div().size_full().bg(rgb(0x25553a)).text_color(rgb(0xffffff)).child(format!("Rust host received: {}", self.value)).into_any_element())
    }
    fn set_prop(&mut self, name: &str, value: Option<&Value>, _: &mut Window, _: &mut App) {
        if name == "beat" { record("beat", json!({"pane":self.pane,"beat":value})); }
        if name == "value" {
            self.value = value.and_then(Value::as_str).unwrap_or_default().to_owned();
            record("probe", json!({"pane": self.pane, "value": self.value}));
        }
    }
}
impl Drop for Probe { fn drop(&mut self) { record("native-drop",json!({"pane":self.pane})); } }
fn make_view(pane: usize, window: &mut Window, cx: &mut App) -> Entity<AppView> {
    cx.new(|cx| {
        let options = AppOptions {
            app_id: format!("com.monitter.nsh-validation.{pane}"),
            data_dir: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/nsh-validation/data")),
            limits: nsh_platform_gpui::Limits { memory_bytes: 32 * 1024 * 1024, call_budget: Duration::from_millis(100), load_budget: Duration::from_secs(3), max_nodes: 1000, ..Default::default() },
            ..Default::default()
        };
        let view = AppView::new(options, window, cx);
        view.on_error(move |error| record("error", json!({"pane":pane,"message":error})));
        view.register_component("monitter.probe", Rc::new(move |_| Box::new(Probe { value: String::new(), pane })));
        load(&view, pane);
        view
    })
}
fn load(view: &AppView, pane: usize) {
    let mut bundle = Bundle::source(include_str!(concat!(env!("OUT_DIR"), "/app.js")).to_owned());
    bundle.app_id = Some(format!("com.monitter.nsh-validation.{pane}"));
    bundle.policy.declared = Some(nsh_core::access::Capabilities::default());
    bundle.policy.granted = Some(nsh_core::access::Capabilities::default());
    view.load(NshApp::Js(bundle));
    record("load", json!(pane));
}
struct Host { panes: [Option<Entity<AppView>>; 2], count: usize }
impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().flex().flex_1().gap_3();
        for pane in &self.panes {
            let mut box_ = div().flex_1().h_full().overflow_hidden().border_1().border_color(rgb(0x69778b));
            if let Some(view) = pane { box_ = box_.child(view.clone()); }
            else { box_ = box_.child("NSH view dropped"); }
            content = content.child(box_);
        }
        div().size_full().flex().flex_col().p_4().gap_3().bg(rgb(0x121923)).text_color(rgb(0xffffff))
            .child("Monitter GPUI Kit 0.7.0 + patched Fast + NSH (isolated validation)")
            .child(div().flex().gap_2()
                .child(Button::new("host-count").label(format!("Host: {}", self.count)).primary().on_click(cx.listener(|this, _, _, cx| {this.count += 1; record("host-count", json!(this.count)); cx.notify();})))
                .child(Button::new("reload").label("Reload left").on_click(cx.listener(|this, _, _, cx| {if let Some(view)=&this.panes[0] {load(view.read(cx), 0);} cx.notify();})))
                .child(Button::new("stop").label("Stop left").on_click(cx.listener(|this, _, _, cx| {if let Some(view)=&this.panes[0] {view.read(cx).stop();record("stop",json!(0));} cx.notify();})))
                .child(Button::new("drop").label("Drop left").on_click(cx.listener(|this, _, _, cx| {this.panes[0]=None;record("drop",json!(0)); cx.notify();})))
                .child(Button::new("restore").label("Restore left").on_click(cx.listener(|this, _, window, cx| {if this.panes[0].is_none() {this.panes[0]=Some(make_view(0,window,cx));} cx.notify();})))
            ).child(content)
    }
}
fn main() {
    record("start", json!({"nsh":"c1511fa4dda6ce2ed04e8f47ed087a1e7dee876b","webview":false,"capabilities":"empty"}));
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1100.), px(650.)), cx);
        cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar: Some(TitlebarOptions {title:Some("Monitter NSH validation".into()),..Default::default()}), ..Default::default() }, |window,cx| {
            let left=make_view(0,window,cx); let right=make_view(1,window,cx);
            let host=cx.new(|_| Host {panes:[Some(left),Some(right)],count:0});
            cx.new(|cx| Root::new(host,window,cx))
        }).unwrap();
        cx.activate(true);
    });
}
