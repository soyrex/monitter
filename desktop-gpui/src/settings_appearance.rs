//! Appearance, interface density, pane styling and typography preferences.
use super::*;

use gpui_kit::component::{color_picker::ColorPicker, select::SelectItem, switch::Switch};

/// Rich font choices keep their canonical family name as the saved value.
#[derive(Clone)]
pub(super) struct FontChoice {
    pub(super) label: SharedString,
}
impl FontChoice {
    fn preview(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .font_family(self.label.clone())
                    .text_size(crate::settings::settings_text_size(1.))
                    .child("Aa"),
            )
            .child(
                div()
                    .text_size(crate::settings::settings_text_size(0.86))
                    .child(self.label.clone()),
            )
    }
}
impl SelectItem for FontChoice {
    type Value = SharedString;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &SharedString {
        &self.label
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(self.preview().into_any_element())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.preview()
    }
}
pub(super) fn font_choices(families: Vec<SharedString>) -> Vec<FontChoice> {
    families
        .into_iter()
        .map(|label| FontChoice { label })
        .collect()
}

#[derive(Clone)]
pub(super) struct PaletteChoice {
    pub(super) label: SharedString,
    colors: [u32; 3],
}
impl PaletteChoice {
    fn preview(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .rounded(px(4.))
                    .overflow_hidden()
                    .border_1()
                    .children(
                        self.colors
                            .map(|color| div().w(px(12.)).h(px(24.)).bg(rgb(color))),
                    ),
            )
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.label.clone()),
            )
    }
}
impl SelectItem for PaletteChoice {
    type Value = SharedString;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &SharedString {
        &self.label
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(self.preview().into_any_element())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.preview()
    }
}
pub(super) fn palette_choices(dark: bool) -> Vec<PaletteChoice> {
    native_appearance::PRESETS
        .iter()
        .map(|(id, label)| {
            let p = native_appearance::palette(id, dark, None, 0, 0);
            PaletteChoice {
                label: SharedString::from(*label),
                colors: [p.paper, p.panel, p.accent],
            }
        })
        .collect()
}
fn range_footer(left: &str, right: &str, palette: Palette) -> Div {
    div()
        .flex()
        .justify_between()
        .text_size(crate::settings::settings_text_size(0.8))
        .text_color(rgb(palette.muted))
        .child(left.to_owned())
        .child(right.to_owned())
}
fn slider_reset(
    id: &'static str,
    state: &Entity<SliderState>,
    value: f32,
    left: &str,
    right: &str,
    palette: Palette,
    cx: &mut Context<Desktop>,
) -> Div {
    let state = state.clone();
    range_footer(left, right, palette).child(
        Button::new(id)
            .compact()
            .label(format!("Reset to default · {value}"))
            .on_click(cx.listener(move |view, _, window, cx| {
                state.update(cx, |state, cx| state.set_value(value, window, cx));
                if id == "reset-window-transparency" {
                    view.patch_settings(json!({"windowTransparency":value.round() as u16}), cx);
                } else {
                    view.save_native_appearance(cx);
                }
            })),
    )
}

pub(super) fn render(
    desktop: &mut Desktop,
    window: &mut Window,
    cx: &mut Context<Desktop>,
    palette: Palette,
    category: &str,
) -> AnyElement {
    let Some(state) = desktop.native_settings.as_ref() else {
        return div().child("Preferences are loading…").into_any_element();
    };
    let presentation = SearchCardPresentation::new(desktop);
    let card = |title: &str, description: &str, palette: Palette| presentation.card(title, description, palette);
    let settings = desktop.snapshot["settings"].clone();
    let mut content = div()
        .w_full()
        .max_w(px(760.))
        .mx_auto()
        .flex()
        .flex_col()
        .gap(px(crate::design::space::SECTION));

    let theme = field(&settings, "theme");
    let surface = field(&settings, "windowSurface");
    let theme_card =
        card(
            "Appearance mode",
            "Use the system appearance or choose light or dark.",
            palette,
        )
        .flex_row().flex_wrap().items_center().justify_between()
        .child(div().flex_shrink_0().flex().gap(px(2.)).p(px(3.))
            .rounded(px(design::radius::CONTROL)).bg(design::surface_color(palette.code)).children(
            ["system", "light", "dark"].map(|mode| {
                Button::new(SharedString::from(format!("theme-{mode}")))
                    .compact().selected(theme == mode)
                    .label(if theme == mode {
                        format!("✓ {}", mode)
                    } else {
                        mode.to_owned()
                    })
                    .disabled(desktop.busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.patch_settings(json!({"theme":mode}), cx)
                    }))
            }),
        ));
    let appearance_mode = theme_card;
    let mut theme_card = card("Primary window surface", "Set the window material and background transparency.", palette)
        .child("Window surface")
        .child(
            div().flex().flex_wrap().gap(px(2.)).p(px(3.))
                .rounded(px(design::radius::CONTROL)).bg(design::surface_color(palette.code)).children(
                ["opaque", "translucent", "glass"].map(|value| {
                    let selected = surface == value || (surface.is_empty() && value == "opaque");
                    Button::new(SharedString::from(format!("surface-{value}")))
                        .compact().selected(selected)
                        .label(if selected {
                            format!("✓ {}", value)
                        } else {
                            value.to_owned()
                        })
                        .disabled(desktop.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.patch_settings(json!({"windowSurface":value}), cx)
                        }))
                }),
            ),
        )
        .child(div().text_size(crate::settings::settings_text_size(0.8)).text_color(rgb(palette.muted)).child(
            "Glass uses the platform blur material where supported; it does not enable Apple's newer Liquid Glass effect.",
        ));
    let transparency = state.window_transparency.read(cx).value().end().round() as u16;
    theme_card = theme_card
        .child(format!("Window transparency: {transparency}%"))
        .child(
            Slider::new(&state.window_transparency).disabled(desktop.busy || surface == "opaque"),
        )
        .child(slider_reset(
            "reset-window-transparency",
            &state.window_transparency,
            18.,
            "Opaque",
            "100%",
            palette,
            cx,
        ));
    let advanced_open = state.appearance_advanced;
    let mut advanced = div()
        .flex()
        .flex_col()
        .gap(px(crate::design::space::SECTION));
    if category == "appearance" {
        content = content.child(appearance_mode).child(theme_card);
    }
    if category == "appearance" && advanced_open {
        let mode = crate::wallpaper_theme::mode();
        let wallpaper = card("Advanced · Wallpaper accent", "Highlight uses a wallpaper colour; Mood follows its tones. Manual uses your chosen accent. Saved on this device.", palette)
            .child(div().flex().flex_wrap().gap(px(crate::design::space::TIGHT)).children(crate::wallpaper_theme::Mode::ALL.map(|choice| {
                Button::new(SharedString::from(format!("wallpaper-accent-{}", choice.id())))
                    .label(if mode == choice { format!("✓ {}", choice.label()) } else { choice.label().to_owned() })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if let Err(error) = crate::wallpaper_theme::set_mode(choice) { view.error = Some(error); }
                        view.refresh_wallpaper(window, cx);
                        view.applied_style = None;
                        view.invalidate_surface_blends(cx);
                    }))
            })))
            .child(format!("Strength · {}%", state.wallpaper_strength.read(cx).value().end().round() as u16))
            .child(Slider::new(&state.wallpaper_strength).disabled(mode == crate::wallpaper_theme::Mode::Manual));
        let wallpaper = wallpaper.child(
            div()
                .flex()
                .items_center()
                .gap(px(crate::design::space::TIGHT))
                .child(div().size(px(14.)).rounded_full().bg(rgb(palette.accent)))
                .child(format!("Current accent · #{:06X}", palette.accent)),
        );
        advanced = advanced.child(wallpaper);
    }

    let native = state.appearance.as_ref();
    let mut app_theme = card(
        "App theme",
        "Choose a palette for light and dark mode. Changes save automatically.",
        palette,
    );
    if let Some(native) = native {
        let tile_theme = desktop
            .preferences
            .as_ref()
            .map(|preferences| preferences.data.appearance.tile_theme)
            .unwrap_or_default();
        let tile_layout = card(
            "Tile layout",
            "Use the original workspace or rounded Tern tiles. Saved on this device.",
            palette,
        )
        .child(
            div().flex().gap(px(crate::design::space::TIGHT)).children(
                [
                    (NativeTileTheme::Monitter, "Monitter"),
                    (NativeTileTheme::Tern, "Tern"),
                ]
                .map(|(choice, label)| {
                    Button::new(SharedString::from(format!("tile-theme-{label}")))
                        .label(if tile_theme == choice {
                            format!("✓ {label}")
                        } else {
                            label.to_owned()
                        })
                        .disabled(desktop.preferences_busy || desktop.preferences.is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.save_native_tile_theme(choice, cx)
                        }))
                }),
            ),
        );
        app_theme = app_theme.child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(220.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(settings_hint("Light palette", palette))
                        .child(
                            Select::new(&native.light)
                                .text_size(crate::settings::settings_text_size(0.86))
                                .accessibility_label("Light palette")
                                .disabled(desktop.preferences_busy),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(220.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(settings_hint("Dark palette", palette))
                        .child(
                            Select::new(&native.dark)
                                .text_size(crate::settings::settings_text_size(0.86))
                                .accessibility_label("Dark palette")
                                .disabled(desktop.preferences_busy),
                        ),
                ),
        );
        let workspace_accent = card(
            "Workspace accent",
            "Set the accent shared with connected workspace clients.",
            palette,
        )
        .child("Workspace accent (#RRGGBB)")
        .child(
            div().w_full().max_w(px(280.)).child(
                Input::new(&state.accent)
                    .text_size(crate::settings::settings_text_size(0.86))
                    .aria_label("Workspace accent")
                    .disabled(desktop.busy),
            ),
        )
        .child(
            div().w_full().flex().justify_end().pt(px(8.)).child(
                Button::new("save-workspace-accent")
                    .label("Save workspace accent")
                    .disabled(desktop.busy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let Some(settings) = &view.native_settings else {
                            return;
                        };
                        let accent = settings.accent.read(cx).value().to_string();
                        let digits = accent.trim().strip_prefix('#').unwrap_or(&accent);
                        if digits.len() != 6
                            || !digits.chars().all(|digit| digit.is_ascii_hexdigit())
                        {
                            view.error = Some("Accent must be a six-digit #RRGGBB value.".into());
                            cx.notify();
                            return;
                        }
                        view.patch_settings(
                            json!({"accent":format!("#{}", digits.to_ascii_uppercase())}),
                            cx,
                        );
                    })),
            ),
        );
        let accent_card = card(
            "Accent colour",
            "Use one accent in both modes and adjust its surface tint.",
            palette,
        )
        .child(div().flex().items_center().gap(px(10.)).children(
            [0x00a8f0, 0x3978d4, 0x8755c7, 0xc44c79, 0xc27524].map(|color| {
                let input = native.accent.clone();
                div()
                    .id(SharedString::from(format!("accent-swatch-{color}")))
                    .size(px(28.))
                    .rounded_full()
                    .bg(rgb(color))
                    .border_2()
                    .border_color(rgb(palette.line))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        input.update(cx, |input, cx| {
                            input.set_value(format!("#{color:06x}"), window, cx)
                        });
                        view.save_native_appearance(cx);
                    }))
            }),
        ))
        .child(ColorPicker::new(&native.accent_picker).label("Custom accent colour"))
        .child("Custom accent (#RRGGBB; blank uses each palette accent)")
        .child(
            div().w_full().max_w(px(280.)).child(
                Input::new(&native.accent)
                    .text_size(crate::settings::settings_text_size(0.86))
                    .aria_label("Custom palette accent")
                    .disabled(desktop.preferences_busy),
            ),
        )
        .child(format!(
            "Surface tint: {}%",
            native.tint.read(cx).value().end().round() as u16
        ))
        .child(Slider::new(&native.tint).disabled(desktop.preferences_busy))
        .child(slider_reset(
            "reset-tint",
            &native.tint,
            5.,
            "0%",
            "50%",
            palette,
            cx,
        ))
        .child(
            div().w_full().flex().justify_end().pt(px(8.)).child(
                Button::new("save-native-appearance")
                    .primary()
                    .label(if desktop.preferences_busy {
                        "Saving…"
                    } else {
                        "Save palette settings"
                    })
                    .disabled(desktop.preferences_busy || desktop.preferences.is_none())
                    .on_click(cx.listener(|view, _, _, cx| view.save_native_appearance(cx))),
            ),
        );

        let borders_card = card(
            "Borders & dividers",
            "Adjust structural lines in both themes.",
            palette,
        )
        .child(format!(
            "Border and divider opacity: {}%",
            native.border_opacity.read(cx).value().end().round() as u16
        ))
        .child(Slider::new(&native.border_opacity).disabled(desktop.preferences_busy))
        .child(slider_reset(
            "reset-borders",
            &native.border_opacity,
            100.,
            "0%",
            "100%",
            palette,
            cx,
        ));
        let contrast_card = card(
            "Contrast",
            "Increase the contrast between surfaces and text.",
            palette,
        )
        .child(format!(
            "Theme contrast: {}%",
            native.contrast.read(cx).value().end().round() as u16
        ))
        .child(Slider::new(&native.contrast).disabled(desktop.preferences_busy))
        .child(slider_reset(
            "reset-contrast",
            &native.contrast,
            0.,
            "Preset",
            "Maximum",
            palette,
            cx,
        ));
        let scale_card = card(
            "Interface scale",
            "Resize text and controls for this native desktop.",
            palette,
        )
        .child(format!(
            "Interface scale: {}%",
            native.interface_scale.read(cx).value().end().round() as u16
        ))
        .child(Slider::new(&native.interface_scale).disabled(desktop.preferences_busy))
        .child(slider_reset(
            "reset-scale",
            &native.interface_scale,
            125.,
            "80%",
            "200%",
            palette,
            cx,
        ));
        if category == "appearance" {
            content = content
                .child(app_theme)
                .child(contrast_card)
                .child(accent_card)
                .child(borders_card);
        }
        if category == "interface" {
            content = content.child(tile_layout).child(scale_card);
        }
        if category == "appearance" && advanced_open {
            advanced = advanced.child(workspace_accent);
        }
    } else {
        app_theme = app_theme.child("Loading private native appearance preferences…");
        if category == "appearance" {
            content = content.child(app_theme);
        }
    }
    let palette_source = card(
        "Native palette source",
        "Choose the native rendering palette source.",
        palette,
    )
    .child(
        div().flex().gap(px(crate::design::space::TIGHT)).children(
            [
                ("monitter", "Monitter"),
                ("reference", "Warm reference paper"),
            ]
            .map(|(id, label)| {
                Button::new(SharedString::from(format!("palette-source-{id}")))
                    .label(if desktop.native_theme_preset == id {
                        format!("✓ {label}")
                    } else {
                        label.to_owned()
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.native_theme_preset = id.into();
                        view.applied_style = None;
                        cx.notify();
                    }))
            }),
        ),
    );
    if category == "appearance" && advanced_open {
        advanced = advanced.child(palette_source);
        let mut blends = card(
            "Advanced · Surface blending",
            "Blend each area with the surfaces behind it. Text and icons keep their colours. Saved on this device.",
            palette,
        );
        for area in crate::surface_blend::Area::ALL {
            let selected = crate::surface_blend::current(area);
            blends = blends.child(area.label()).child(
                div().flex().gap(px(crate::design::space::TIGHT)).children(
                    crate::surface_blend::Mode::ALL.map(|mode| {
                        Button::new(SharedString::from(format!(
                            "surface-blend-{}-{}",
                            area.id(),
                            mode.id()
                        )))
                        .label(if selected == mode {
                            format!("✓ {}", mode.label())
                        } else {
                            mode.label().to_owned()
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Err(error) = crate::surface_blend::set(area, mode) {
                                view.error = Some(error);
                            }
                            view.invalidate_surface_blends(cx);
                        }))
                    }),
                ),
            );
        }
        advanced = advanced.child(blends);
    }
    if category == "interface" {
        let selected_motion = crate::native_motion::current();
        let motion = card("Motion", "System and Subtle respect Reduce Motion. On overrides it; Off disables native animations. Saved on this device.", palette)
            .child(div().flex().flex_wrap().gap(px(crate::design::space::TIGHT)).children(crate::native_motion::MotionPreference::ALL.map(|mode| {
                Button::new(SharedString::from(format!("native-motion-{}", mode.id())))
                    .label(if selected_motion == mode { format!("✓ {}", mode.label()) } else { mode.label().to_owned() })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::native_motion::set_preference(mode, cx);
                        cx.notify();
                    }))
            })));
        content = content.child(motion);
        let selected_animation = crate::native_motion::ActivityAnimation::from_snapshot(
            settings["activityAnimation"].as_str(),
        );
        let activity_animation = card(
            "Working animation",
            "Choose one accent effect for active chat panes. Motion controls whether it moves.",
            palette,
        )
        .child(
            div().flex().flex_wrap().gap(px(crate::design::space::TIGHT)).children(
                crate::native_motion::ActivityAnimation::ALL.map(|mode| {
                    Button::new(SharedString::from(format!("activity-animation-{}", mode.id())))
                        .label(if selected_animation == mode {
                            format!("✓ {}", mode.label())
                        } else {
                            mode.label().to_owned()
                        })
                        .disabled(desktop.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.patch_settings(json!({"activityAnimation":mode.id()}), cx);
                        }))
                }),
            ),
        );
        content = content.child(activity_animation);
        let auto_size_panes = desktop.dock.read(cx).auto_size_panes();
        let auto_size = card(
            "Auto Size Panes",
            "Keep split panes evenly sized when panes are added. Resizing a divider turns this off.",
            palette,
        )
        .child(settings_toggle(
            "Auto Size Panes",
            "New panes rebalance the workspace automatically until you resize a divider.",
            auto_size_panes,
            palette,
        ).child(
            Switch::new("settings-auto-size-panes")
                .checked(auto_size_panes)
                .accessibility_label("Auto Size Panes")
                .on_change(cx.listener(|view, enabled, window, cx| {
                    view.dock.update(cx, |dock, cx| dock.set_auto_size_panes(*enabled, window, cx));
                })),
        ));
        content = content.child(auto_size);
    }

    let density = ["tight", "normal", "spacious"][state
        .interface_density
        .read(cx)
        .value()
        .end()
        .round()
        .clamp(0., 2.) as usize];
    let interface = card(
        "Interface density",
        "Adjust the height and spacing of tabs, sidebars and pane controls.",
        palette,
    )
    .child(format!("Interface density: {density}"))
    .child(Slider::new(&state.interface_density).disabled(desktop.busy))
    .child(
        div()
            .flex()
            .justify_between()
            .text_size(crate::settings::settings_text_size(0.8))
            .text_color(rgb(palette.muted))
            .child("Tight")
            .child("Normal")
            .child("Spacious"),
    );
    if category == "interface" {
        content = content.child(interface);
    }
    let mut interface =
        card(
            "Tabs",
            "Choose a classic rounded strip or a modern full-height straight-edge strip.",
            palette,
        )
        .child("Tab style")
        .child(div().flex().gap(px(crate::design::space::TIGHT)).children(
            ["classic", "modern"].map(|value| {
                let selected = field(&settings, "tabStyle");
                Button::new(SharedString::from(format!("tab-style-{value}")))
                    .label(
                        if selected == value || (selected.is_empty() && value == "classic") {
                            format!("✓ {value}")
                        } else {
                            value.to_owned()
                        },
                    )
                    .disabled(desktop.busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.patch_settings(json!({"tabStyle":value}), cx)
                    }))
            }),
        ));
    for (label, key, default) in [
        ("Show tab close buttons", "showTabCloseButtons", true),
        ("Auto hide tabs", "autoHideTabs", false),
        ("Highlight active pane", "showActivePaneBorder", true),
        ("Dim inactive panes", "dimInactivePanes", true),
    ] {
        if key == "showActivePaneBorder" {
            if category == "interface" {
                content = content.child(interface);
            }
            interface = card(
                "Pane appearance",
                "Keep the active workspace easy to identify.",
                palette,
            );
        }
        let value = settings[key].as_bool().unwrap_or(default);
        let detail = match key {
            "showTabCloseButtons" => "Tabs can also be closed with your chosen keyboard shortcuts.",
            "autoHideTabs" => "Reveal a pane's tabs by hovering over its top bar.",
            "showActivePaneBorder" => "Draw an accent-colour border around the active pane.",
            _ => "Reduce visual weight for panes that are not active.",
        };
        interface = interface.child(
            settings_toggle(label, detail, value, palette).child(
                Switch::new(SharedString::from(format!("toggle-{key}")))
                    .checked(value)
                    .disabled(desktop.busy)
                    .accessibility_label(label)
                    .on_change(cx.listener(move |view, enabled, _, cx| {
                        view.patch_settings(json!({(key):*enabled}), cx)
                    })),
            ),
        );
    }
    let show_active_border = settings["showActivePaneBorder"].as_bool().unwrap_or(true);
    let border_width = state.active_pane_border_width.read(cx).value().end().round() as u8;
    interface = interface
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(format!("Active pane outline: {border_width}px")),
        )
        .child(
            Slider::new(&state.active_pane_border_width)
                .disabled(desktop.busy || !show_active_border),
        )
        .child(range_footer("1px", "8px", palette));
    let dim_inactive = settings["dimInactivePanes"].as_bool().unwrap_or(true);
    let opacity_percent = state.inactive_pane_opacity.read(cx).value().end().round() as u16;
    interface = interface
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(format!("Inactive pane opacity: {opacity_percent}%")),
        )
        .child(Slider::new(&state.inactive_pane_opacity).disabled(desktop.busy || !dim_inactive))
        .child(range_footer("10%", "90%", palette));
    if category == "interface" {
        content = content.child(interface);
    }

    if category == "appearance" {
        content = content.child(
            Button::new("appearance-advanced")
                .label(if advanced_open {
                    "▾ Advanced native controls"
                } else {
                    "▸ Advanced native controls"
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    if let Some(state) = view.native_settings.as_mut() {
                        state.appearance_advanced = !state.appearance_advanced;
                    }
                    cx.notify();
                })),
        );
        if advanced_open {
            content = content.child(advanced);
        }
    }
    let controls = &state.fonts;
    let mut typography = card(
        "Fonts",
        "Family and weight changes save immediately; size changes save when you leave the field. Sizes: 8–32 px.",
        palette,
    );
    for ((family_key, _, _, label, _), row) in TYPOGRAPHY.iter().zip(controls) {
        let show_row = if category == "terminal" {
            *family_key == "terminalFont"
        } else {
            true
        };
        if !show_row {
            continue;
        }
        typography = typography.child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(12.))
                .pb(px(16.))
                .border_b_1()
                .border_color(rgb(palette.line))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(200.))
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(settings_hint(format!("{label} font"), palette))
                                .child(
                                    Select::new(&row.family)
                                        .text_size(crate::settings::settings_text_size(0.86))
                                        .cleanable(true)
                                        .placeholder("Default font")
                                        .accessibility_label(format!("{label} font family"))
                                        .disabled(desktop.busy),
                                ),
                        )
                        .child(
                            div()
                                .w(px(120.))
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(settings_hint("Size (px)", palette))
                                .child(
                                    Input::new(&row.size)
                                        .text_size(crate::settings::settings_text_size(0.86))
                                        .aria_label(format!("{label} font size"))
                                        .disabled(desktop.busy),
                                ),
                        ),
                )
                .child(format!(
                    "{label} font weight: {}",
                    row.weight.read(cx).value().end().round() as u16
                ))
                .child(Slider::new(&row.weight).disabled(desktop.busy))
                .child(settings_hint(
                    "Light · Regular · Medium · Semibold · Bold",
                    palette,
                )),
        );
    }
    let line_heights = card("Line height", "Control the vertical space in chat messages and terminal output.", palette)
        .child(format!("Chat line height: {:.2}×",state.chat_line_height.read(cx).value().end()))
        .child(Slider::new(&state.chat_line_height).disabled(desktop.busy))
        .child(range_footer("Compact · 1×", "Airy · 2.5×", palette))
        .child(format!("Terminal line height: {:.2}×",state.terminal_line_height.read(cx).value().end()))
        .child(Slider::new(&state.terminal_line_height).disabled(desktop.busy))
        .child(range_footer("Compact · 1×", "Airy · 2.5×", palette))
        .child(settings_hint("These values are relative to each surface’s chosen font size. Terminal changes refit open terminal tabs immediately.", palette));
    if category == "typography" {
        content = content.child(typography).child(line_heights);
    } else if category == "terminal" {
        content = content.child(typography).child(
            card(
                "Terminal colour theme",
                "Applies immediately to every native terminal in this workspace.",
                palette,
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(crate::design::space::TIGHT))
                    .children(crate::terminal_theme::THEMES.iter().map(|theme| {
                        let id = theme.id;
                        let selected = crate::terminal_theme::current().id == id;
                        Button::new(SharedString::from(format!("terminal-theme-{id}")))
                            .compact()
                            .label(if selected {
                                format!("✓ {}", theme.label)
                            } else {
                                theme.label.to_owned()
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                if let Some(theme) = crate::terminal_theme::choose(id) {
                                    cx.set_global(crate::terminal_theme::Selection(
                                        theme.palette_index,
                                    ));
                                    cx.notify();
                                }
                            }))
                    })),
            ),
        );
    }

    if let Some(error) = &desktop.preferences_error {
        content = content.child(div().text_color(rgb(0xbd655b)).child(error.clone()));
    }
    if let Some(error) = &desktop.error {
        content = content.child(div().text_color(rgb(0xbd655b)).child(error.clone()));
    }
    let _ = window;
    content.into_any_element()
}
