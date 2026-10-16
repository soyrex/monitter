//! Explicit owner-only controls for the independent experimental LAN listener.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{ActiveTheme, Disableable, button::*, switch::Switch},
    component::input::{Input, InputState},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::{DesktopService, NativeLanStatus};

pub struct LanSettingsPane {
    service: DesktopService,
    status: Option<NativeLanStatus>,
    busy: bool,
    reveal: bool,
    error: Option<String>,
    access_code: Entity<InputState>,
}
#[derive(Clone, Copy)]
enum Operation {
    Read,
    Start,
    Stop,
}
impl LanSettingsPane {
    pub fn new(service: DesktopService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut pane = Self {
            service,
            status: None,
            busy: false,
            reveal: false,
            error: None,
            access_code: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Six digit access code")
            }),
        };
        pane.request(Operation::Read, cx);
        pane
    }
    fn request(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.reveal = false;
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            match operation {
                Operation::Read => service.read_lan_status(),
                Operation::Start => service.start_lan(),
                Operation::Stop => service.stop_lan(),
            }
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(status) => view.status = Some(status),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_access_code(&mut self, cx: &mut Context<Self>) {
        let code = self.access_code.read(cx).value().to_string();
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            self.error = Some("Enter exactly six digits for the access code.".into());
            cx.notify();
            return;
        }
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.reveal = false;
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            service.set_lan_access_code(code)
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(status) => {
                        view.status = Some(status);
                        view.error = None;
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for LanSettingsPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.status.as_ref().is_some_and(|status| status.running);
        let muted = cx.theme().muted_foreground;
        let mut panel = div().flex().flex_col().gap(px(16.)).p(px(18.)).border_1().rounded(px(10.))
            .border_color(cx.theme().border).bg(cx.theme().background).text_color(cx.theme().foreground).text_size(crate::settings::settings_text_size(0.86))
            .child(div().text_size(crate::settings::settings_text_size(1.)).font_weight(FontWeight::SEMIBOLD).child("Open Monitter on your network"))
            .child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(muted).child("Connect browsers to this independent GPUI workspace. Its data, ports, TLS identity and access code are separate from Monitter."))
            .child(div().flex().items_center().justify_between().gap(px(12.))
                .child(div().flex_1().flex().flex_col().gap(px(4.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Experimental LAN access"))
                    .child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(muted).child(if running { "Listening · access code required" } else { "Stopped · starts on next backend launch" })))
                .child(Switch::new("lan-enabled").checked(running).disabled(self.busy || self.status.is_none()).accessibility_label("Enable experimental LAN access")
                    .on_change(cx.listener(|view, enabled, _, cx| view.request(if *enabled { Operation::Start } else { Operation::Stop }, cx)))))
            .child(Button::new("lan-refresh").label("Refresh status").disabled(self.busy).on_click(cx.listener(|view, _, _, cx| view.request(Operation::Read, cx))));
        if let Some(status) = &self.status {
            panel = panel
                .child(div().flex().flex_col().gap(px(6.))
                    .child(div().font_weight(FontWeight::MEDIUM).child("Change access code"))
                    .child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(muted)
                        .child("Choose a six digit code that stays the same across restarts. Saving applies it immediately."))
                    .child(Input::new(&self.access_code)
                        .text_size(crate::settings::settings_text_size(0.86))
                        .aria_label("New six digit LAN access code")
                        .disabled(self.busy))
                    .child(Button::new("lan-save-code")
                        .label("Save code")
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, _, cx| view.save_access_code(cx)))));
            for (index, url) in status.info.urls.iter().enumerate() {
                let copy = url.clone();
                let open = url.clone();
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(10.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(px(6.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(url.clone()))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new(SharedString::from(format!("lan-copy-{index}")))
                                        .label("Copy address")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("lan-open-{index}")))
                                        .label("Open in browser")
                                        .on_click(move |_, _, cx| cx.open_url(&open)),
                                ),
                        ),
                );
            }
            if running && !status.info.token.is_empty() {
                panel = panel.child(
                    Button::new("lan-reveal")
                        .label(if self.reveal {
                            "Hide access code"
                        } else {
                            "Reveal access code"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.reveal = !view.reveal;
                            cx.notify();
                        })),
                );
                if self.reveal {
                    let code = status.info.token.clone();
                    panel = panel.child(format!("Access code: {code}")).child(
                        Button::new("lan-copy-code")
                            .label("Copy access code")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))
                            }),
                    );
                }
            }
            if let Some(fingerprint) = &status.info.local_ca_fingerprint {
                panel=panel.child(format!("Local CA fingerprint: {fingerprint}"))
                    .child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(muted).child("HTTPS uses this installation’s local certificate authority. Check the fingerprint before explicitly trusting its certificate on another device."));
            }
            if let Some(error) = &status.info.error {
                panel = panel.child(error.clone());
            }
        }
        if self.busy {
            panel = panel.child("Updating LAN status…");
        }
        if let Some(error) = &self.error {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        panel.child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(muted).child("Existing iOS LAN mode requires port 18436 and rejects the experimental 19436/19437 ports. Use a browser for this listener. Encrypted phone pairing and shared-chat relay hosting are not provided by these controls."))
    }
}
