//! Native controls for the single profile-scoped Rust pairing runtime.
//! Creating this entity never creates a browser view or takes keyboard focus.
use crate::{design::ui_scaled_px as px, pairing_runtime::{Command, RuntimeHandle, Status}};
use gpui_kit::{component::{ActiveTheme, Disableable, button::*, switch::Switch}, prelude::*, *};
use monitter_lib::experimental_desktop::DesktopService;
use std::time::Duration;

pub struct PairingHost {
    runtime: Option<RuntimeHandle>,
    status: Status,
    error: Option<String>,
    reveal: bool,
}
impl PairingHost {
    pub fn new(service: DesktopService, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let start = cx.background_executor().spawn(async move { RuntimeHandle::start(service) });
        cx.spawn(async move |view, cx| {
            let result = start.await;
            if view.update(cx, |view, cx| {
                match result {
                    Ok(runtime) => { view.status = runtime.status(); view.runtime = Some(runtime); }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            }).is_err() { return; }
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if view.update(cx, |view, cx| {
                    if let Some(runtime) = &view.runtime {
                        let status = runtime.status();
                        if status != view.status { view.status = status; cx.notify(); }
                    }
                }).is_err() { break; }
            }
        }).detach();
        Self { runtime: None, status: Status::default(), error: None, reveal: false }
    }
    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.error = self.runtime.as_ref().ok_or_else(|| "Pairing is starting.".to_owned())
            .and_then(|runtime| runtime.command(command)).err();
        cx.notify();
    }
}
impl Render for PairingHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut panel = div().flex().flex_col().gap(px(16.)).p(px(18.)).w_full()
            .border_1().rounded(px(10.)).border_color(cx.theme().border)
            .bg(cx.theme().background).text_color(cx.theme().foreground)
            .text_size(crate::settings::settings_text_size(0.86))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Pair a phone"))
            .child(div().text_color(muted).child("Scan this QR code in the Monitter iPhone app. Approve once to remember your phone until you revoke it."))
            .child(div().flex().items_center().justify_between().gap(px(12.))
                .child(if self.status.enabled {
                    self.status.connection_state.clone()
                } else { "Phone access is off".to_owned() })
                .child(Switch::new("pairing-native-enabled").checked(self.status.enabled)
                    .disabled(self.runtime.is_none()).accessibility_label("Enable phone pairing")
                    .on_change(cx.listener(|view, enabled, _, cx| view.command(Command::SetEnabled(*enabled), cx)))));
        if self.status.enabled {
            if let Some(invitation) = self.status.invitation.as_ref() {
                match qrcode::QrCode::with_error_correction_level(invitation.as_bytes(), qrcode::EcLevel::M) {
                    Ok(code) => {
                        let width = code.width();
                        let colors = code.to_colors();
                        panel = panel.child(div().flex().flex_col().gap(px(8.))
                            .child(canvas(move |bounds, _, _| bounds, move |_, bounds, window, _| {
                                let module = (f32::from(bounds.size.width) / (width + 8) as f32).floor().max(1.);
                                window.paint_quad(fill(bounds, rgb(0xffffff)));
                                for y in 0..width {
                                    for x in 0..width {
                                        if colors[y * width + x] == qrcode::Color::Dark {
                                            window.paint_quad(fill(Bounds::new(
                                                point(bounds.origin.x + gpui_kit::px((x + 4) as f32 * module), bounds.origin.y + gpui_kit::px((y + 4) as f32 * module)),
                                                size(gpui_kit::px(module), gpui_kit::px(module))), rgb(0x111510)));
                                        }
                                    }
                                }
                            }).w(px(280.)).h(px(280.)))
                            .child(div().text_color(muted).child("iPhone → Scan desktop QR. Compare the verification number, then choose Remember phone.")));
                    }
                    Err(_) => { panel = panel.child(div().child("QR code unavailable. Use the PIN below.")); }
                }
            }
        }
        if let Some(note) = &self.status.migration_note {
            panel = panel.child(div().text_color(muted).child(note.clone()));
        }
        if let Some(pin) = self.status.pin.clone().filter(|_| self.status.enabled) {
            let display = if self.reveal { format!("{} {} {}", &pin[..3], &pin[3..6], &pin[6..]) } else { "••• ••• •••".into() };
            panel = panel.child(div().flex().flex_col().gap(px(8.))
                .child(div().text_size(crate::settings::settings_text_size(1.5)).font_weight(FontWeight::SEMIBOLD).child(display))
                .child(div().text_color(muted).child("This PIN stays the same across restarts. Compare the verification number before approving a device."))
                .child(div().flex().gap(px(8.)).flex_wrap()
                    .child(Button::new("pairing-native-reveal").label(if self.reveal {"Hide PIN"} else {"Show PIN"})
                        .on_click(cx.listener(|view, _, _, cx| { view.reveal = !view.reveal; cx.notify(); })))
                    .child(Button::new("pairing-native-copy").label("Copy PIN")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(pin.clone()))))));
        }
        if let Some(error) = self.error.as_ref().or(self.status.error.as_ref()) {
            panel = panel.child(div().child(error.clone()))
                .child(Button::new("pairing-native-retry").label("Retry").disabled(self.runtime.is_none())
                    .on_click(cx.listener(|view, _, _, cx| view.command(Command::Retry, cx))));
        }
        for pending in self.status.pending.clone() {
            let approve = pending.id.clone(); let remember = pending.id.clone(); let reject = pending.id.clone();
            panel = panel.child(div().flex().flex_col().gap(px(8.)).p(px(12.)).border_1().rounded(px(8.)).border_color(cx.theme().border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{} requests {} access", pending.name, pending.scope)))
                .child(div().child(format!("Verification number: {}", pending.sas)))
                .child(div().text_color(muted).child("Approve only if the same number is displayed on the device you are pairing."))
                .child(div().flex().gap(px(8.)).flex_wrap()
                    .child(Button::new(SharedString::from(format!("pair-once-{}", pending.id))).label("Allow once")
                        .on_click(cx.listener(move |view, _, _, cx| view.command(Command::Approve { id: approve.clone(), remember: false }, cx))))
                    .child(Button::new(SharedString::from(format!("pair-remember-{}", pending.id))).label("Remember phone").primary()
                        .on_click(cx.listener(move |view, _, _, cx| view.command(Command::Approve { id: remember.clone(), remember: true }, cx))))
                    .child(Button::new(SharedString::from(format!("pair-reject-{}", pending.id))).label("Reject")
                        .on_click(cx.listener(move |view, _, _, cx| view.command(Command::Reject { id: reject.clone() }, cx))))));
        }
        if !self.status.devices.is_empty() {
            panel = panel.child(div().font_weight(FontWeight::SEMIBOLD).child("Remembered devices"));
            let mut seen = std::collections::HashSet::new();
            for (index, device) in self.status.devices.clone().into_iter().enumerate() {
                // Controller and owner are grants on one authenticated device.
                // Group by its cryptographic identity, never by a supplied name.
                if !seen.insert(device.public_key.clone()) { continue; }
                let owner = self.status.devices.iter().any(|grant| grant.public_key == device.public_key && grant.scope == "owner");
                let controller = self.status.devices.iter().any(|grant| grant.public_key == device.public_key && grant.scope == "controller");
                let access = if owner && controller { "Controller + full workspace" } else if owner { "Full workspace" } else { "Controller" };
                panel = panel.child(div().flex().items_center().justify_between().gap(px(8.))
                    .child(format!("{} · {}", device.name, access))
                    .child(Button::new(SharedString::from(format!("pair-revoke-{index}"))).label("Revoke device")
                        .on_click(cx.listener(move |view, _, _, cx| view.command(Command::Revoke { public_key: device.public_key.clone(), scope: "controller".into() }, cx)))));
            }
        }
        panel.child(div().text_color(muted).child("Only the approved scope is available to each connected device."))
    }
}
