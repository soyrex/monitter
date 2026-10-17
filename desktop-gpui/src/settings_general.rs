//! Profile, keyboard, conversation and board controls use the canonical save_settings path.
use super::*;
use crate::workspace_windows::DesktopContextExt as _;
use gpui_kit::component::{Selectable as _, switch::Switch};

fn stack() -> Div {
    div()
        .w_full()
        .max_w(px(760.))
        .mx_auto()
        .flex()
        .flex_col()
        .gap(px(14.))
}

fn toggle(
    desktop: &Desktop,
    cx: &mut Context<Desktop>,
    palette: Palette,
    label: &str,
    detail: &str,
    key: &'static str,
    enabled: bool,
) -> Div {
    settings_toggle(label, detail, enabled, palette).child(
        Switch::new(SharedString::from(format!("settings-{key}")))
            .checked(enabled)
            .disabled(desktop.busy)
            .accessibility_label(label.to_owned())
            .on_change(cx.listener(move |view, enabled, _, cx| {
                view.patch_settings(json!({(key):*enabled}), cx);
            })),
    )
}

fn choices(
    desktop: &Desktop,
    cx: &mut Context<Desktop>,
    palette: Palette,
    key: &'static str,
    label: &'static str,
    current: &str,
    options: &[(&'static str, &'static str)],
) -> gpui_kit::Stateful<Div> {
    div()
        .w_full()
        .flex()
        .gap(px(3.))
        .p(px(3.))
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(palette.line))
        .bg(crate::design::surface_color(palette.soft))
        .id(SharedString::from(format!("settings-segmented-{key}")))
        .role(Role::RadioGroup)
        .aria_label(label)
        .children(options.iter().map(|(value, text)| {
            let value = *value;
            Button::new(SharedString::from(format!("settings-{key}-{value}")))
                .ghost()
                .flex_1()
                .label(*text)
                .role(Role::RadioButton)
                .selected(current == value)
                .accessibility_label(*text)
                .disabled(desktop.busy)
                .on_click(
                    cx.listener(move |view, _, _, cx| {
                        view.patch_settings(json!({(key):value}), cx)
                    }),
                )
        }))
}

pub(super) fn render_profile(
    desktop: &mut Desktop,
    _window: &mut Window,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    let presentation = SearchCardPresentation::new(desktop);
    let settings_card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    let Some(state) = desktop.native_settings.as_ref() else {
        return settings_hint("Preferences are loading…", palette).into_any_element();
    };
    let name = state.user_name.clone();
    let profile = settings_card("Your name", "Used by agents in new chats.", palette).child(
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div().flex_1().min_w_0().max_w(px(420.)).child(
                    Input::new(&name)
                        .text_size(crate::settings::settings_text_size(0.86))
                        .aria_label("Your name")
                        .disabled(desktop.busy),
                ),
            )
            .child(
                Button::new("save-profile-name")
                    .primary()
                    .label("Save name")
                    .disabled(desktop.busy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let Some(settings) = &view.native_settings else {
                            return;
                        };
                        let name = settings.user_name.read(cx).value().trim().to_owned();
                        if name.chars().any(char::is_control) || name.chars().count() > 80 {
                            view.error = Some(
                            "Name must be at most 80 characters and contain no control characters."
                                .into(),
                        );
                            cx.notify();
                            return;
                        }
                        view.patch_settings(json!({"userName":name}), cx);
                    })),
            ),
    );
    stack().child(profile).into_any_element()
}

pub(super) fn render_security(
    desktop: &mut Desktop,
    window: &mut Window,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    if desktop.current_workspace_window != crate::workspace_window_state::WindowId::main() {
        return stack().child(settings_card("Session lock", "Manage the shared session lock in the main window.", palette)
            .child(Button::new("session-lock-main-window").label("Open in Main Window")
                .on_click(cx.workspace_listener(|desktop, _, window, cx| {
                    desktop.close_preferences(window, cx);
                    let Some(handle)=desktop.window_hosts.get(&crate::workspace_window_state::WindowId::main()).map(|host|host.handle) else{return;};
                    let weak=cx.weak_entity();
                    cx.defer(move|cx| {let _=handle.update(cx,|_,window,cx| {let _=weak.update(cx,|desktop,cx| {
                        desktop.with_workspace_window(window,cx,|desktop,window,cx| {
                            desktop.show_preferences(window,cx);
                            if desktop.window.native_settings.is_none() {
                                desktop.window.native_settings=Some(crate::settings::NativeSettings::new(&desktop.snapshot["settings"],window,cx));
                            }
                            if let Some(settings)=&mut desktop.window.native_settings {settings.select_security();}
                            window.activate_window();cx.notify();
                        });
                    });});});
                })))).into_any_element();
    }
    let lock = desktop.session_lock.update(cx, |lock, cx| {
        lock.set_palette(palette);
        lock.render_settings(window, cx)
    });
    stack().child(lock).into_any_element()
}

// Keep the former entry point available while the parent migrates category routing.
pub(super) fn render_general(
    desktop: &mut Desktop,
    window: &mut Window,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    render_profile(desktop, window, cx, palette)
}

pub(super) fn render_behavior(
    desktop: &mut Desktop,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    let presentation = SearchCardPresentation::new(desktop);
    let settings_card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    let settings = desktop.snapshot["settings"].clone();
    let shortcut = field(&settings, "shortcutMode");
    let shortcut = if shortcut.is_empty() {
        "standard"
    } else {
        &shortcut
    };
    let keyboard = settings_card("Keyboard shortcuts", "Choose how workspace commands and pane navigation behave.", palette)
        .child(div().font_weight(FontWeight::MEDIUM).child("Pane navigation shortcuts"))
        .child(choices(desktop, cx, palette, "shortcutMode", "Pane navigation shortcuts", shortcut,
            &[("standard", "Standard"), ("vim", "Standard + Vim alternates")]))
        .child(settings_hint(if cfg!(target_os = "macos") { "Use Command (⌘) and Option (⌥) on macOS." } else { "Use Ctrl and Alt on this platform." }, palette))
        .children({
            let command = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
            let option = if cfg!(target_os = "macos") { "⌥" } else { "Alt" };
            let mut rows = vec![
                (if cfg!(target_os = "macos") { "⌘+Ctrl+arrow".to_owned() } else { "Super+Ctrl+arrow".to_owned() }, "Move the focused pane"),
                (format!("{command}+Ctrl+F / {command}+Enter"), "Toggle fullscreen"),
                (format!("{command}+1–9"), "Select workspace by tab position"),
                (format!("{command}+Shift+arrow / {command}+[ / {command}+]"), "Previous / next workspace"),
                (format!("{command}+{option}+Up / Down"), "Focus the pane above or below"),
            ];
            if shortcut == "vim" {
                rows.insert(0, ("Ctrl+W, then arrow or h/j/k/l".to_owned(), "Focus any Monitter pane"));
                rows.insert(1, ("Ctrl+W, then s / v".to_owned(), "Split the current pane"));
            }
            rows.insert(0, (format!("{command}+W"), "Close the current tab"));
            rows.into_iter().map(|(keys, action)| div().w_full().min_w_0().flex().flex_wrap().gap(px(12.))
                .child(div().w(px(240.)).flex_shrink_0().font_weight(FontWeight::MEDIUM).child(keys))
                .child(div().flex_1().min_w(px(140.)).whitespace_normal().text_color(rgb(palette.muted)).child(action)))
        })
        .child(settings_hint("Vim alternates reserve Ctrl+W as a pane-command prefix across the workspace. Native Neovim keeps Ctrl+W for editing in insert, command-line and terminal modes. Standard shortcuts remain available.", palette));
    let focus = settings_card(
        "Workspace focus",
        "Set how split panes respond as you move through them.",
        palette,
    )
    .child(toggle(
        desktop,
        cx,
        palette,
        "Focus follows mouse",
        "Activate a pane when the pointer enters it.",
        "focusFollowsMouse",
        settings["focusFollowsMouse"] == true,
    ));

    stack().child(keyboard).child(focus).into_any_element()
}

pub(super) fn render_conversation(
    desktop: &mut Desktop,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    let presentation = SearchCardPresentation::new(desktop);
    let settings_card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    let settings = desktop.snapshot["settings"].clone();
    let mut activity = settings_card(
        "Conversation activity",
        "Control supporting information shown alongside messages.",
        palette,
    );
    for (label, detail, key, enabled) in [
        (
            "Show tool activity",
            "Show collapsible blocks for activity supplied by the harness.",
            "showToolActivity",
            settings["showToolActivity"].as_bool().unwrap_or(true),
        ),
        (
            "Compress tool calls",
            "Combine calls of the same kind across thinking time within each turn. Open the summary to see every call.",
            "compressToolCalls",
            settings["compressToolCalls"] == true,
        ),
        (
            "Tint my messages",
            "Use the accent colour for your message bubbles.",
            "tintUserMessages",
            settings["tintUserMessages"] == true,
        ),
        (
            "Show reasoning summaries",
            "Show summaries supplied by the harness.",
            "showReasoningSummaries",
            settings["showReasoningSummaries"].as_bool().unwrap_or(true),
        ),
    ] {
        activity = activity.child(toggle(desktop, cx, palette, label, detail, key, enabled));
    }
    let send = settings["sendWithEnter"] == true;
    let messages = settings_card(
        "Messages",
        "Choose the keyboard shortcut that sends a task or channel message.",
        palette,
    )
    .child(toggle(
        desktop,
        cx,
        palette,
        "Enter to send",
        if send {
            "Enter sends. Shift+Enter adds a new line."
        } else {
            "Enter adds a new line. Use the Send button to send."
        },
        "sendWithEnter",
        send,
    ));
    let voice_engine = field(&settings, "voiceTranscriptionEngine");
    let voice = settings_card("Voice transcription", "Choose how voice recordings are transcribed on this desktop. The result is added to your editable message draft.", palette)
        .child(div().font_weight(FontWeight::MEDIUM).child("Transcription engine"))
        .child(choices(desktop, cx, palette, "voiceTranscriptionEngine", "Voice transcription engine",
            if voice_engine.is_empty() { "whisper" } else { &voice_engine },
            &[("whisper", "Whisper (local)"), ("apple", "Apple on-device")]))
        .child(settings_hint("Apple on-device transcription is available in the macOS app. Whisper remains the default. Voice recordings become editable draft text before you send them.", palette));
    let busy = settings_card("Busy agents", "Choose what happens when you send another message during a running turn.", palette)
        .child(settings_toggle("Default to steering busy agents", "Off queues a message for the next turn. On steers a live Codex turn or an ACP agent that explicitly supports it; other harnesses queue safely.", settings["busyMessageMode"] == "steer", palette)
            .child(Switch::new("settings-busy-message-mode").checked(settings["busyMessageMode"] == "steer")
                .disabled(desktop.busy).accessibility_label("Default to steering busy agents")
                .on_change(cx.listener(|view, enabled, _, cx| {
                    view.patch_settings(json!({"busyMessageMode":if *enabled {"steer"} else {"queue"}}), cx);
                }))))
        .child(settings_hint("Follow-ups stay queued until the active harness confirms the steer. Queued messages appear above the input box.", palette));
    stack()
        .child(messages)
        .child(voice)
        .child(busy)
        .child(streaming_controls(&presentation, cx, palette))
        .child(activity)
        .into_any_element()
}

pub(super) fn render_boards(
    desktop: &mut Desktop,
    cx: &mut Context<Desktop>,
    palette: Palette,
) -> AnyElement {
    let presentation = SearchCardPresentation::new(desktop);
    let settings_card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    let settings = desktop.snapshot["settings"].clone();
    if let Some(state) = desktop.native_settings.as_ref() {
        let guidance = state.board_guidance.clone();
        let length = guidance.read(cx).value().chars().count();
        let boards = settings_card("Project boards", "Private coordination notes shown as #channels for each project.", palette)
            .child(toggle(desktop, cx, palette, "Enable project boards", "Agents working in a project receive recent notes at turn start and can check for newer notes at meaningful checkpoints. Posting never wakes another agent. Turning this off hides boards but preserves their notes.", "projectBoardEnabled", settings["projectBoardEnabled"] == true))
            .child(div().font_weight(FontWeight::MEDIUM).child("Default agent guidance"))
            .child(Textarea::new(&guidance).text_size(crate::settings::settings_text_size(0.86)).aria_label("Default project-board guidance").h(px(150.)).disabled(desktop.busy))
            .child(settings_hint(&format!("{length}/8000 characters. Project-specific guidance overrides this; blank uses Monitter's built-in wording. Guidance is context and does not grant permission."), palette))
            .child(div().flex().flex_wrap().gap(px(8.)).justify_end()
                .child(Button::new("reset-board-guidance").label("Reset to built-in wording").disabled(desktop.busy)
                    .on_click(cx.listener(|view, _, window, cx| {
                        if let Some(state) = &view.native_settings {
                            state.board_guidance.update(cx, |area, cx| area.set_value("", window, cx));
                        }
                        view.patch_settings(json!({"boardGuidanceText":null}), cx);
                    })))
                .child(Button::new("save-board-guidance").primary().label("Save guidance").disabled(desktop.busy || length > 8000)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let Some(state) = &view.native_settings else { return; };
                        let text = state.board_guidance.read(cx).value().to_string();
                        if text.chars().count() > 8000 {
                            view.error = Some("Board guidance is limited to 8,000 characters.".into()); cx.notify(); return;
                        }
                        view.patch_settings(json!({"boardGuidanceText":if text.trim().is_empty() {Value::Null} else {json!(text)}}), cx);
                    }))));
        let mcp_approval_supported = settings.get("autoApproveMonitterMcp").and_then(Value::as_bool);
        let mcp_approval = settings_card("Monitter MCP approvals", "Choose whether built-in Monitter tools can run without a harness approval prompt.", palette)
            .child(settings_toggle(
                "Automatically approve Monitter tools",
                "Let Codex and Claude use built-in Monitter tools without asking each time. Host terminal commands and skill installation still need approval. Changes apply when the agent process next starts; running agents keep their current setting.",
                mcp_approval_supported == Some(true),
                palette,
            ).child(Switch::new("settings-auto-approve-monitter-mcp")
                .checked(mcp_approval_supported == Some(true))
                .disabled(desktop.busy || mcp_approval_supported.is_none())
                .accessibility_label("Automatically approve Monitter tools")
                .on_change(cx.listener(|view, enabled, _, cx| {
                    view.patch_settings(json!({"autoApproveMonitterMcp":*enabled}), cx);
                }))))
            .child(if mcp_approval_supported.is_none() {
                settings_hint("This setting requires a Monitter backend update.", palette)
            } else {
                settings_hint("This setting affects future harness launches and resumes after the current process retires.", palette)
            });
        return stack().child(boards).child(mcp_approval).into_any_element();
    }
    stack()
        .child(settings_hint("Preferences are loading…", palette))
        .into_any_element()
}

fn streaming_controls(presentation: &SearchCardPresentation, cx: &mut Context<Desktop>, palette: Palette) -> Stateful<Div> {
    let settings_card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    use crate::streaming_display::{self, Mode, Options, Selection};
    let selected = streaming_display::current(cx);
    let mut card = settings_card("Streaming replies",
        "Show replies as they arrive, or reveal text one character at a time with a cursor. Saved on this desktop; existing history appears immediately.", palette)
        .child(div().flex().flex_wrap().gap(px(8.)).children(Mode::ALL.map(|mode| {
            Button::new(SharedString::from(format!("streaming-display-{}", mode.id())))
                .label(if selected.mode == mode { format!("✓ {}", mode.label()) } else { mode.label().to_owned() })
                .accessibility_label(format!("Streaming display: {}", mode.label()))
                .on_click(cx.listener(move |_, _, _, cx| {
                    streaming_display::set(Options { mode, ..streaming_display::current(cx) }, cx); cx.notify();
                }))
        }))).child("Typing speed")
        .child(div().flex().flex_wrap().gap(px(8.)).children([(120, "Slow"), (600, "Normal"), (1800, "Fast")].map(|(speed, label)| {
            Button::new(SharedString::from(format!("streaming-speed-{speed}")))
                .label(if selected.characters_per_second == speed { format!("✓ {label}") } else { label.to_owned() })
                .accessibility_label(format!("Typing speed: {label}"))
                .disabled(selected.mode != Mode::Typing)
                .on_click(cx.listener(move |_, _, _, cx| {
                    streaming_display::set(Options { characters_per_second: speed, ..streaming_display::current(cx) }, cx); cx.notify();
                }))
        })));
    if let Some(error) = cx
        .try_global::<Selection>()
        .and_then(|selection| selection.error.as_ref())
    {
        card = card.child(error.clone());
    }
    card
}
