//! Retained session PIN UI. Credential work stays off the window thread.
use crate::{
    design::{self, Palette, ui_scaled_px as px},
    session_lock_store as store,
};
use gpui_kit::{
    component::{
        Disableable, FocusTrapElement as _, Sizable,
        button::*,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct LockState(pub bool);
impl Global for LockState {}
pub fn locked(cx: &App) -> bool {
    cx.try_global::<LockState>().is_some_and(|state| state.0)
}
pub enum LockEvent {
    Locked,
    Unlocked,
}
pub struct SessionLock {
    disconnected_fixture: bool,
    focus: FocusHandle,
    locked: bool,
    configured: Option<bool>,
    initialized: bool,
    busy: bool,
    locking: bool,
    pin: Entity<InputState>,
    current: Entity<InputState>,
    confirmation: Entity<InputState>,
    error: Option<String>,
    notice: Option<String>,
    attempts: u32,
    retry_after: Option<Instant>,
    retry_timer: Option<Task<()>>,
    palette: Palette,
    lock_generation: u64,
}
impl EventEmitter<LockEvent> for SessionLock {}
impl SessionLock {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_storage(!cfg!(test), window, cx)
    }
    /// Native workspace previews must not open the independent PIN database.
    #[cfg(feature = "performance-profiling")]
    pub(crate) fn new_for_ui_fixture(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_storage(false, window, cx)
    }
    fn new_with_storage(storage: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The initial shell is fail-closed until storage supplies authority.
        // Even a cold or contended SQLite database cannot hold up first paint.
        cx.set_global(LockState(storage));
        let pin = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("PIN"));
        let current = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Current PIN")
        });
        let confirmation = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Confirm PIN")
        });
        cx.subscribe_in(&pin, window, |view, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                if view.locked {
                    view.unlock(window, cx);
                } else {
                    view.save_pin(window, cx);
                }
            }
        })
        .detach();
        cx.subscribe_in(
            &confirmation,
            window,
            |view, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.save_pin(window, cx);
                }
            },
        )
        .detach();
        if storage {
            Self::load_storage(cx);
        }
        Self {
            disconnected_fixture: !storage,
            focus: cx.focus_handle(),
            locked: storage,
            configured: (!storage).then_some(false),
            initialized: !storage,
            busy: storage,
            locking: false,
            pin,
            current,
            confirmation,
            error: None,
            notice: None,
            attempts: 0,
            retry_after: None,
            retry_timer: None,
            palette: design::palette(&serde_json::Value::Null, WindowAppearance::Dark),
            lock_generation: 0,
        }
    }
    fn load_storage(cx: &mut Context<Self>) {
        let worker = cx.background_executor().spawn(async {
            store::read_startup_state()
        });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok((snapshot, configured)) => {
                        view.initialized = true;
                        view.configured = Some(configured);
                        view.locked = snapshot.locked;
                        view.attempts = snapshot.attempts;
                        let remaining = snapshot
                            .retry_until_ms
                            .saturating_sub(unix_ms())
                            .min(60_000);
                        view.retry_after = (remaining > 0)
                            .then(|| Instant::now() + Duration::from_millis(remaining));
                        view.error = None;
                        cx.set_global(LockState(view.locked));
                        if !view.locked {
                            cx.emit(LockEvent::Unlocked);
                        }
                    }
                    Err(error) => {
                        // Failed authority reads never reveal cached panes.
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub fn requires_lock_persistence(&self) -> bool {
        self.initialized && self.locked
    }
    /// Keep the one PIN form and its input observers on the surviving primary.
    pub(crate) fn rehost_to_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = cx.entity_id();
        cx.rehost_entity_window(id, window)
            .map_err(|error| error.to_string())?;
        for input in [&self.pin, &self.current, &self.confirmation] {
            input.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
        }
        Ok(())
    }
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn show_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn configured(&self) -> bool {
        self.configured == Some(true)
    }
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            window.focus(&self.focus, cx);
        } else {
            self.pin.update(cx, |input, cx| input.focus(window, cx));
        }
    }
    pub fn clear_fields(&self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.pin, &self.current, &self.confirmation] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
    pub fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disconnected_fixture {
            return;
        }
        if self.locked || !self.configured() || self.busy {
            return;
        }
        self.clear_fields(window, cx);
        self.error = None;
        self.notice = None;
        self.locked = true;
        self.locking = true;
        self.busy = true;
        self.lock_generation = self.lock_generation.wrapping_add(1);
        cx.set_global(LockState(true));
        cx.emit(LockEvent::Locked);
        let worker = cx
            .background_executor()
            .spawn(async { store::set_locked(true) });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                view.locking = false;
                if let Err(error) = result {
                    view.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
        self.focus(window, cx);
        cx.notify();
    }
    fn retry_storage(&mut self, cx: &mut Context<Self>) {
        if self.disconnected_fixture {
            return;
        }
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        if !self.initialized {
            Self::load_storage(cx);
            cx.notify();
            return;
        }
        let worker = cx
            .background_executor()
            .spawn(async { store::configured() });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(configured) => view.configured = Some(configured),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disconnected_fixture {
            self.error =
                Some("Session PIN changes are disabled in the disconnected preview.".into());
            cx.notify();
            return;
        }
        if self.locked
            || self.busy
            || self.configured.is_none()
            || self.retry_after.is_some_and(|t| Instant::now() < t)
        {
            return;
        }
        let pin = self.pin.read(cx).value().to_string();
        let current = self.current.read(cx).value().to_string();
        let confirmation = self.confirmation.read(cx).value().to_string();
        if pin != confirmation {
            self.error = Some("The PINs don’t match.".into());
            cx.notify();
            return;
        }
        if !(4..=8).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
            self.error = Some("Choose a PIN with 4–8 digits.".into());
            cx.notify();
            return;
        }
        let configured = self.configured();
        self.clear_fields(window, cx);
        self.busy = true;
        self.error = None;
        self.notice = None;
        let worker = cx.background_executor().spawn(async move {
            if configured && !store::verify_pin(&current)? {
                return Err("The current PIN is incorrect.".into());
            }
            store::set_pin(&pin)
        });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(()) => {
                        view.configured = Some(true);
                        view.attempts = 0;
                        view.retry_after = None;
                        view.notice = Some("PIN saved. You can now lock Monitter.".into());
                    }
                    Err(error) => {
                        view.error = Some(error);
                        view.attempts = view.attempts.saturating_add(1);
                        view.delay_retry(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disconnected_fixture {
            return;
        }
        if !self.initialized
            || !self.locked
            || self.busy
            || self.retry_after.is_some_and(|t| Instant::now() < t)
        {
            return;
        }
        let pin = self.pin.read(cx).value().to_string();
        self.clear_fields(window, cx);
        self.busy = true;
        self.error = None;
        let next_attempt = self.attempts.saturating_add(1);
        let worker = cx.background_executor().spawn(async move {
            if !store::verify_pin(&pin)? {
                store::record_failure(
                    next_attempt,
                    unix_ms() + retry_delay(next_attempt).as_millis() as u64,
                )?;
                return Ok(false);
            }
            store::set_locked(false)?;
            Ok(true)
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = worker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(true) => {
                        view.locked = false;
                        view.attempts = 0;
                        view.retry_after = None;

                        cx.set_global(LockState(false));
                        cx.emit(LockEvent::Unlocked);
                    }
                    Ok(false) => {
                        view.attempts = view.attempts.saturating_add(1);
                        view.error = Some("Incorrect PIN.".into());
                        view.delay_retry(cx);
                    }
                    Err(error) => {
                        view.error = Some(error);
                        view.attempts = view.attempts.saturating_add(1);
                        view.delay_retry(cx);
                    }
                }
                if view.locked {
                    view.focus(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn delay_retry(&mut self, cx: &mut Context<Self>) {
        let delay = retry_delay(self.attempts);
        if delay.is_zero() {
            return;
        }
        self.retry_after = Some(Instant::now() + delay);
        self.start_retry_timer(cx);
    }
    fn start_retry_timer(&mut self, cx: &mut Context<Self>) {
        if self.retry_timer.is_some() {
            return;
        }
        self.retry_timer = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep = view
                    .update(cx, |view, cx| {
                        let keep = view.retry_after.is_some_and(|t| Instant::now() < t);
                        if !keep {
                            view.retry_after = None;
                            view.retry_timer = None;
                        }
                        cx.notify();
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }
    pub fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let retrying = self.retry_after.is_some_and(|t| Instant::now() < t);
        let mut card=div().debug_selector(|| "settings-card-Session lock".into()).w_full().min_w_0().flex().flex_col().gap(px(14.)).p(px(18.)).rounded(px(10.)).border_1().border_color(rgb(self.palette.line)).bg(design::surface_color(self.palette.panel))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Session lock"))
            .child(div().text_size(crate::settings::settings_text_size(0.8)).whitespace_normal().text_color(rgb(self.palette.muted)).child(if cfg!(target_os="macos") {"Use a 4–8 digit PIN to lock Monitter. Agents continue running. Lock with ⌘⇧L or the app menu."} else {"Use a 4–8 digit PIN to lock Monitter. Agents continue running. Lock with Ctrl+Shift+L or the app menu."}));
        if self.configured() {
            card = card.child(
                div()
                    .w_full()
                    .max_w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Current PIN"),
                    )
                    .child(
                        Input::new(&self.current)
                            .aria_label("Current session PIN")
                            .disabled(self.busy),
                    ),
            );
        }
        card = card
            .child(
                div()
                    .w_full()
                    .max_w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("New PIN"),
                    )
                    .child(
                        Input::new(&self.pin)
                            .aria_label("New session PIN")
                            .disabled(self.busy),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Confirm PIN"),
                    )
                    .child(
                        Input::new(&self.confirmation)
                            .aria_label("Confirm session PIN")
                            .disabled(self.busy),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("save-session-pin")
                            .primary()
                            .label(if self.busy {
                                "Saving…"
                            } else if self.configured() {
                                "Change PIN"
                            } else {
                                "Set PIN"
                            })
                            .disabled(self.busy || self.configured.is_none() || retrying)
                            .on_click(cx.listener(|view, _, window, cx| view.save_pin(window, cx))),
                    )
                    .child(
                        Button::new("lock-monitter")
                            .label("Lock Monitter")
                            .disabled(self.busy || !self.configured())
                            .on_click(cx.listener(|view, _, window, cx| view.lock(window, cx))),
                    ),
            );
        if self.configured.is_none() && self.error.is_some() {
            card = card.child(
                Button::new("retry-session-storage")
                    .label("Retry PIN storage")
                    .disabled(self.busy)
                    .on_click(cx.listener(|view, _, _, cx| view.retry_storage(cx))),
            );
        }
        if retrying {
            card = card.child("Please wait before trying again.");
        }
        if let Some(error) = &self.error {
            card = card.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            card = card.child(
                div()
                    .text_color(rgb(self.palette.accent))
                    .child(notice.clone()),
            );
        }
        let _ = window;
        card.into_any_element()
    }
}
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn retry_delay(attempts: u32) -> Duration {
    if attempts < 3 {
        Duration::ZERO
    } else {
        Duration::from_secs((5u64.saturating_mul(1u64 << (attempts - 3).min(4))).min(60))
    }
}
impl Render for SessionLock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.retry_after.is_some() {
            self.start_retry_timer(cx);
        }
        let palette = self.palette;
        let focus = self.focus.clone();
        if !focus.contains_focused(window, cx) {
            self.focus(window, cx);
        }
        let remaining = self
            .retry_after
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs() + 1)
            .unwrap_or(0);
        div()
            .id("monitter-session-lock")
            .role(Role::Button)
            .aria_label("Monitter is locked. Type your PIN and press Return to unlock")
            .track_focus(&focus)
            .focus_trap("session-lock-focus", &focus)
            .relative()
            .size_full()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.focus(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if view.initialized
                    && !view.busy
                    && !view.retry_after.is_some_and(|t| Instant::now() < t)
                {
                    let mut pin = view.pin.read(cx).value().to_string();
                    if keystroke.key == "enter" {
                        if pin.len() >= 4 {
                            view.unlock(window, cx);
                        }
                    } else if keystroke.key == "backspace" {
                        if keystroke.modifiers.platform || keystroke.modifiers.control {
                            pin.clear();
                        } else {
                            pin.pop();
                        }
                        view.pin
                            .update(cx, |input, cx| input.set_value(pin, window, cx));
                        cx.notify();
                    } else if !keystroke.modifiers.control && !keystroke.modifiers.platform {
                        if let Some(text) = &keystroke.key_char {
                            for digit in text.chars().filter(char::is_ascii_digit) {
                                if pin.len() < 8 {
                                    pin.push(digit);
                                }
                            }
                            view.pin
                                .update(cx, |input, cx| input.set_value(pin, window, cx));
                            view.error = None;
                            cx.notify();
                        }
                    }
                }
                // All lock-screen input stays here; no native text insertion
                // can reach the inert workspace or duplicate the first digit.
                window.prevent_default();
                cx.stop_propagation();
            }))
            .child({
                let blur = div()
                    .absolute()
                    .inset_0()
                    .bg(
                        Background::from(rgba(if palette.dark { 0x00000045 } else { 0xffffff45 }))
                            .backdrop_blur(20.),
                    );
                if self.lock_generation > 0 && crate::native_motion::motion_allowed() {
                    blur.with_animation(
                        ("session-lock-blur", self.lock_generation),
                        Animation::new(Duration::from_millis(200)).with_max_fps(60.),
                        |blur, progress| blur.opacity(progress),
                    )
                    .into_any_element()
                } else {
                    blur.into_any_element()
                }
            })
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .w(px(144.))
                    .h(px(36.))
                    .child(
                        img(crate::branding::sidebar_wordmark(288.))
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(48.))
                            .left(px(-88.))
                            .w(px(320.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .id("session-lock-pin-marks")
                                    .role(Role::Label)
                                    .aria_label(format!(
                                        "{} PIN digits entered",
                                        self.pin.read(cx).value().len()
                                    ))
                                    .text_size(px(36.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.ink))
                                    .child(if self.initialized {
                                        "*".repeat(self.pin.read(cx).value().len())
                                    } else if self.busy {
                                        "Opening…".into()
                                    } else {
                                        String::new()
                                    }),
                            )
                            .when(!self.initialized && !self.busy, |entry| {
                                entry.child(
                                    Button::new("retry-initial-session-storage")
                                        .label("Retry opening")
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.retry_storage(cx)),
                                        ),
                                )
                            })
                            .when_some(self.error.clone(), |entry, error| {
                                entry.child(div().text_sm().text_color(rgb(0xd65b5b)).child(error))
                            })
                            .when(remaining > 0, |entry| {
                                entry.child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(palette.ink))
                                        .child(format!("Try again in {remaining}s")),
                                )
                            }),
                    ),
            )
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn session_lock_retry_delay_is_bounded_and_progressive() {
        assert_eq!(super::retry_delay(2).as_secs(), 0);
        assert_eq!(super::retry_delay(3).as_secs(), 5);
        assert_eq!(super::retry_delay(4).as_secs(), 10);
        assert_eq!(super::retry_delay(u32::MAX).as_secs(), 60);
    }
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "session_lock_ui_tests.rs"]
mod ui_tests;
