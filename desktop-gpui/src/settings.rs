//! Native preferences and real task-scoped harness model catalogs.
use crate::workspace_windows::DesktopContextExt as _;
use super::*;
#[path = "lan_settings.rs"]
mod lan_settings;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, IndexPath, Theme, ThemeMode,
    select::{Select, SelectState, SelectEvent},
    color_picker::{ColorPickerState, ColorPickerEvent},
    slider::{Slider, SliderEvent, SliderState},
};
use monitter_lib::experimental_desktop::{NativeAppearance, NativeTileTheme};
use std::collections::HashSet;
use gpui_kit::{ScrollAnchor, ScrollHandle};
#[path = "native_appearance.rs"]
pub(super) mod native_appearance;
#[path = "settings_appearance.rs"]
mod settings_appearance;
#[path = "settings_general.rs"]
mod settings_general;
#[path = "settings_services.rs"]
mod settings_services;
#[path = "settings_navigation.rs"]
mod settings_navigation;
#[path = "settings_search.rs"]
mod settings_search;

static SETTINGS_FONT_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(14_f32.to_bits());
/// Settings use the interface font size, independent of the workspace's 16px rem grid.
pub(crate) fn settings_text_size(ratio: f32) -> Pixels {
    gpui_kit::px(f32::from_bits(SETTINGS_FONT_BITS.load(std::sync::atomic::Ordering::Relaxed)) * ratio)
}
pub(super) fn settings_hint(text: impl Into<SharedString>, palette: Palette) -> Div {
    div().min_w_0().whitespace_normal().text_size(crate::settings::settings_text_size(0.8)).line_height(gpui_kit::relative(1.5))
        .text_color(rgb(palette.muted)).child(text.into())
}
pub(super) fn settings_card(title: &str, description: &str, palette: Palette) -> Div {
    let selector = format!("settings-card-{title}");
    div().debug_selector(move || selector).w_full().min_w_0().flex_shrink_0().flex().flex_col().gap(design::ui_scaled_px(14.))
        .p(design::ui_scaled_px(18.)).border_1().border_color(rgb(palette.line))
        .rounded(design::ui_scaled_px(design::radius::CARD)).bg(design::surface_color(palette.panel))
        .child(div().flex().flex_col().gap(design::ui_scaled_px(4.))
            .child(div().text_size(crate::settings::settings_text_size(1.)).font_weight(FontWeight::SEMIBOLD).child(title.to_owned()))
            .child(settings_hint(description.to_owned(), palette)))
}
pub(super) fn settings_toggle(label: &str, detail: &str, _: bool, palette: Palette) -> Div {
    div().w_full().min_w_0().flex().items_center().justify_between().gap(design::ui_scaled_px(16.))
        .py(design::ui_scaled_px(5.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(design::ui_scaled_px(3.))
            .child(div().text_size(crate::settings::settings_text_size(0.86)).font_weight(FontWeight::SEMIBOLD).child(label.to_owned()))
            .child(settings_hint(detail.to_owned(), palette)))
}

const TYPOGRAPHY: [(&str, &str, &str, &str, u64); 3] = [
    (
        "interfaceFont",
        "interfaceFontSize",
        "interfaceFontWeight",
        "Interface",
        14,
    ),
    ("chatFont", "chatFontSize", "chatFontWeight", "Chat", 13),
    (
        "terminalFont",
        "terminalFontSize",
        "terminalFontWeight",
        "Terminal",
        14,
    ),
];
struct NativeFontControls {
    family: Entity<SelectState<Vec<settings_appearance::FontChoice>>>,
    size: Entity<InputState>,
    weight: Entity<SliderState>,
}
fn font_weight_slider(value: f32) -> SliderState {
    // Set the upper bound first: setters immediately clamp against the current range.
    SliderState::new().max(700.).min(300.).step(100.).default_value(value)
}
struct NativeAppearanceControls {
    light: Entity<SelectState<Vec<settings_appearance::PaletteChoice>>>,
    dark: Entity<SelectState<Vec<settings_appearance::PaletteChoice>>>,
    accent_picker: Entity<ColorPickerState>,
    accent: Entity<InputState>,
    contrast: Entity<SliderState>,
    tint: Entity<SliderState>,
    border_opacity: Entity<SliderState>,
    interface_scale: Entity<SliderState>,
}
impl NativeAppearanceControls {
    fn new(appearance: &NativeAppearance, window: &mut Window, cx: &mut Context<Desktop>) -> Self {
        let light_choices = settings_appearance::palette_choices(false);
        let dark_choices = settings_appearance::palette_choices(true);
        let selected = |id: &str| {
            light_choices.iter().position(|choice| choice.label.as_ref() == native_appearance::preset_label(id)).map(IndexPath::new)
        };
        let light_selected = selected(&appearance.light_palette);
        let dark_selected = selected(&appearance.dark_palette);
        let accent = appearance.custom_accent.as_deref().and_then(native_appearance::parse_accent).unwrap_or(0x00a8e8);
        Self {
            light: cx.new(|cx| SelectState::new(light_choices, light_selected, window, cx)),
            dark: cx.new(|cx| SelectState::new(dark_choices, dark_selected, window, cx)),
            accent_picker: cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(accent))),
            accent: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(appearance.custom_accent.clone().unwrap_or_default())
                    .placeholder("Use preset accent")
            }),
            contrast: cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(100.)
                    .step(1.)
                    .default_value(appearance.contrast as f32)
            }),
            tint: cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(50.)
                    .step(1.)
                    .default_value(appearance.surface_tint as f32)
            }),
            border_opacity: cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(100.)
                    .step(1.)
                    .default_value(appearance.border_opacity as f32)
            }),
            interface_scale: cx.new(|_| {
                SliderState::new()
                    .min(80.)
                    .max(200.)
                    .step(5.)
                    .default_value(
                        design::normalize_interface_scale(appearance.interface_scale) as f32,
                    )
            }),
        }
    }
}
fn font_number(value: &str, min: f64, max: f64, label: &str) -> Result<f64, String> {
    let number = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("Enter a numeric {label}."))?;
    if !number.is_finite() || !(min..=max).contains(&number) {
        return Err(format!("{label} must be between {min} and {max}."));
    }
    Ok(number)
}
fn blend_line(paper: u32, line: u32, opacity: u16) -> u32 {
    let alpha = opacity.min(100) as u32;
    let component = |shift: u32| {
        let background = (paper >> shift) & 0xff;
        let foreground = (line >> shift) & 0xff;
        (foreground * alpha + background * (100 - alpha) + 50) / 100
    };
    (component(16) << 16) | (component(8) << 8) | component(0)
}
pub(super) struct SearchCardPresentation {
    target: Option<&'static str>,
    anchor: Option<ScrollAnchor>,
}
impl SearchCardPresentation {
    pub(super) fn new(desktop: &Desktop) -> Self {
        Self {
            target: desktop.native_settings.as_ref().and_then(|state| state.search_target).map(settings_search::card_title),
            anchor: desktop.native_settings.as_ref().map(|state| state.target_anchor.clone()),
        }
    }
    pub(super) fn card(&self, title: &str, description: &str, palette: Palette) -> Stateful<Div> {
        settings_card(title, description, palette)
            .id(SharedString::from(format!("preferences-setting-card-{title}")))
            .when(self.target == Some(title), |card| card.border_color(rgb(palette.accent)).anchor_scroll(self.anchor.clone()))
    }
}
pub(super) struct NativeSettings {
    category: String,
    search: Entity<InputState>,
    search_selected: usize,
    search_results: Vec<&'static settings_search::Entry>,
    search_target: Option<&'static str>,
    page_scroll: ScrollHandle,
    search_scroll: ScrollHandle,
    target_anchor: ScrollAnchor,
    appearance_advanced: bool,
    lan: Option<Entity<lan_settings::LanSettingsPane>>,
    services: Option<Entity<settings_services::ServicePreferencesPane>>,
    service_revision: Option<String>,
    user_name: Entity<InputState>,
    accent: Entity<InputState>,
    fonts: Vec<NativeFontControls>,
    chat_line_height: Entity<SliderState>,
    terminal_line_height: Entity<SliderState>,
    window_transparency: Entity<SliderState>,
    wallpaper_strength: Entity<SliderState>,
    interface_density: Entity<SliderState>,
    inactive_pane_opacity: Entity<SliderState>,
    _slider_subscriptions: Vec<Subscription>,
    board_guidance: Entity<TextareaState>,
    appearance: Option<NativeAppearanceControls>,
    catalogs: HashMap<String, Value>,
    choices: HashMap<String, Value>,
    loading: HashSet<String>,
    errors: HashMap<String, String>,
}
impl NativeSettings {
    /// Carry retained form controls with their workspace. Composite widgets
    /// repair private input hosts when rendered; explicitly pin their roots
    /// now so emitted settings events resolve the owning workspace beforehand.
    pub(super) fn rehost_to_window(&mut self, window: &mut Window, cx: &mut App) -> Result<(), String> {
        let mut ids = vec![
            self.chat_line_height.entity_id(), self.terminal_line_height.entity_id(),
            self.window_transparency.entity_id(), self.wallpaper_strength.entity_id(),
            self.interface_density.entity_id(), self.inactive_pane_opacity.entity_id(),
        ];
        for font in &self.fonts { ids.extend([font.family.entity_id(), font.weight.entity_id()]); }
        ids.extend(self.lan.iter().map(Entity::entity_id));
        ids.extend(self.services.iter().map(Entity::entity_id));
        if let Some(appearance) = &self.appearance {
            ids.extend([
                appearance.light.entity_id(), appearance.dark.entity_id(), appearance.accent_picker.entity_id(),
                appearance.contrast.entity_id(), appearance.tint.entity_id(),
                appearance.border_opacity.entity_id(), appearance.interface_scale.entity_id(),
            ]);
        }
        for id in ids { cx.rehost_entity_window(id, window).map_err(|error| error.to_string())?; }
        for input in [&self.search, &self.user_name, &self.accent].into_iter().chain(self.fonts.iter().map(|font| &font.size)) {
            input.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
        }
        self.board_guidance.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
        if let Some(appearance) = &self.appearance {
            appearance.accent.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
        }
        Ok(())
    }

    pub(super) fn select_category(&mut self, category: &str) { self.category = settings_navigation::category(category).id.into(); self.page_scroll.set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.))); }
    // The native Lock Session action calls this when no PIN is configured.
    pub(super) fn select_security(&mut self) { self.select_category("security"); }
    pub fn new(settings: &Value, window: &mut Window, cx: &mut Context<Desktop>) -> Self {
        let mut families = cx.text_system().all_font_names();
        families.sort();
        families.dedup();
        let choices = settings_appearance::font_choices(families.into_iter().map(SharedString::from).collect());
        let fonts = TYPOGRAPHY
            .iter()
            .map(|(family_key, size_key, weight_key, _, default_size)| {
                let saved = field(settings, family_key);
                let fallback = if *family_key == "terminalFont" {
                    "IBM Plex Mono"
                } else {
                    "IBM Plex Sans"
                };
                let selected = choices
                    .iter()
                    .position(|family| {
                        family.label.as_ref() == if saved.is_empty() { fallback } else { &saved }
                    })
                    .map(IndexPath::new);
                NativeFontControls {
                    family: cx.new(|cx| {
                        SelectState::new(choices.clone(), selected, window, cx).searchable(true)
                    }),
                    size: cx.new(|cx| {
                        InputState::new(window, cx).default_value(
                            settings[*size_key]
                                .as_u64()
                                .unwrap_or(*default_size)
                                .to_string(),
                        )
                    }),
                    weight: cx.new(|_| {
                        font_weight_slider(settings[*weight_key].as_u64().unwrap_or(400) as f32)
                    }),
                }
            })
            .collect();
        let page_scroll = ScrollHandle::new();
        let target_anchor = ScrollAnchor::for_handle(page_scroll.clone());
        let mut state = Self {
            category: "profile".into(),
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search preferences…")),
            search_selected: 0,
            search_results: Vec::new(),
            search_target: None,
            page_scroll,
            search_scroll: ScrollHandle::new(),
            target_anchor,
            appearance_advanced: false,
            lan: None,
            services: None,
            service_revision: None,
            fonts,
            chat_line_height: cx.new(|_| {
                SliderState::new()
                    .min(1.)
                    .max(2.5)
                    .step(0.05)
                    .default_value(settings["chatLineHeight"].as_f64().unwrap_or(1.65) as f32)
            }),
            terminal_line_height: cx.new(|_| {
                SliderState::new()
                    .min(1.)
                    .max(2.5)
                    .step(0.05)
                    .default_value(settings["terminalLineHeight"].as_f64().unwrap_or(1.) as f32)
            }),
            wallpaper_strength: cx.new(|_| SliderState::new().min(0.).max(100.).step(1.).default_value(wallpaper_theme::strength() as f32)),
            window_transparency: cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(70.)
                    .step(1.)
                    .default_value(settings["windowTransparency"].as_u64().unwrap_or(18) as f32)
            }),
            interface_density: cx.new(|_| {
                SliderState::new().min(0.).max(2.).step(1.).default_value(
                    match field(settings, "interfaceDensity").as_str() {
                        "tight" => 0.,
                        "spacious" => 2.,
                        _ => 1.,
                    },
                )
            }),
            inactive_pane_opacity: cx.new(|_| {
                SliderState::new().min(10.).max(90.).step(5.).default_value(
                    (settings["inactivePaneOpacity"].as_f64().unwrap_or(0.6) * 100.) as f32,
                )
            }),
            _slider_subscriptions: Vec::new(),
            board_guidance: {
                let area = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Use project boards when work could affect coordination…")
                });
                area.update(cx, |area, cx| {
                    area.set_value(field(settings, "boardGuidanceText"), window, cx)
                });
                area
            },
            appearance: None,
            user_name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(field(settings, "userName"))
                    .placeholder("Your name")
            }),
            accent: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(field(settings, "accent"))
                    .placeholder("#3978d4")
            }),
            catalogs: HashMap::new(),
            choices: HashMap::new(),
            loading: HashSet::new(),
            errors: HashMap::new(),
        };
        state._slider_subscriptions.push(cx.subscribe_workspace(&state.search, window, |view, input, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Change => {
                    if let Some(state) = &mut view.native_settings {
                        let query = input.read(cx).value();
                        state.search_selected = 0;
                        if !query.trim().is_empty() { state.search_target = None; state.page_scroll.set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.))); }
                        state.search_results = settings_search::search(&query);
                        state.search_scroll.set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { secondary: false, shift: false } => view.open_preferences_search_result(window, cx),
                _ => {}
            }
        }));
        for slider in state
            .fonts
            .iter()
            .map(|row| &row.weight)
            .chain([&state.chat_line_height, &state.terminal_line_height])
        {
            state
                ._slider_subscriptions
                .push(cx.observe(slider, |_, _, cx| cx.notify()));
        }
        state._slider_subscriptions.push(cx.observe(&state.board_guidance, |_, _, cx| cx.notify()));
        state._slider_subscriptions.push(cx.subscribe_workspace(&state.user_name, window, |view, input, event: &InputEvent, _, cx| {
            if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) { return; }
            let name=input.read(cx).value().trim().to_owned();
            if name.chars().any(char::is_control) || name.chars().count()>80 {
                view.error=Some("Name must be at most 80 characters and contain no control characters.".into()); cx.notify(); return;
            }
            if field(&view.snapshot["settings"],"userName") != name { view.patch_settings(json!({"userName":name}),cx); }
        }));
        for row in &state.fonts {
            state._slider_subscriptions.push(cx.subscribe_workspace(&row.family, window, |view, _, _: &SelectEvent<Vec<settings_appearance::FontChoice>>, _, cx| view.save_typography(cx)));
            state._slider_subscriptions.push(cx.subscribe_workspace(&row.weight, window, |view, _, event: &SliderEvent, _, cx| {
                if matches!(event, SliderEvent::Release(_)) { view.save_typography(cx); } else { cx.notify(); }
            }));
            state._slider_subscriptions.push(cx.subscribe_workspace(&row.size, window, |view, _, event: &gpui_kit::component::input::InputEvent, _, cx| {
                if matches!(event, gpui_kit::component::input::InputEvent::Blur | gpui_kit::component::input::InputEvent::PressEnter { .. }) { view.save_typography(cx); }
            }));
        }
        for slider in [&state.chat_line_height, &state.terminal_line_height] {
            state._slider_subscriptions.push(cx.subscribe_workspace(slider, window, |view, _, event: &SliderEvent, _, cx| {
                if matches!(event, SliderEvent::Release(_)) { view.save_typography(cx); } else { cx.notify(); }
            }));
        }
        state._slider_subscriptions.push(cx.subscribe_workspace(&state.wallpaper_strength, window, |view, _, event: &SliderEvent, _, cx| {
            if let SliderEvent::Release(value) = event {
                if let Err(error) = wallpaper_theme::set_strength(value.end().round() as u16) { view.error = Some(error); }
                view.applied_style = None;
                view.invalidate_surface_blends(cx);
            } else { cx.notify(); }
        }));
        state._slider_subscriptions.push(cx.subscribe_workspace(
            &state.window_transparency, window,
            |view, _, event: &SliderEvent, _, cx| {
                if let SliderEvent::Release(value) = event {
                    view.patch_settings(
                        json!({"windowTransparency": value.end().round() as u16}),
                        cx,
                    );
                } else {
                    cx.notify();
                }
            },
        ));
        state._slider_subscriptions.push(cx.subscribe_workspace(
            &state.interface_density, window,
            |view, _, event: &SliderEvent, _, cx| {
                if let SliderEvent::Release(value) = event {
                    let density =
                        ["tight", "normal", "spacious"][value.end().round().clamp(0., 2.) as usize];
                    view.patch_settings(json!({"interfaceDensity": density}), cx);
                } else {
                    cx.notify();
                }
            },
        ));
        state._slider_subscriptions.push(cx.subscribe_workspace(
            &state.inactive_pane_opacity, window,
            |view, _, event: &SliderEvent, _, cx| {
                if let SliderEvent::Release(value) = event {
                    view.patch_settings(
                        json!({"inactivePaneOpacity": value.end().round() as f64 / 100.}),
                        cx,
                    );
                } else {
                    cx.notify();
                }
            },
        ));
        state
    }
}

#[cfg(test)]
mod slider_range_tests {
    use super::font_weight_slider;
    use gpui_kit::component::slider::SliderValue;

    #[test]
    fn font_weight_range_is_configured_before_its_default_value() {
        let state = font_weight_slider(400.);

        assert_eq!(state.min_value(), 300.);
        assert_eq!(state.max_value(), 700.);
        assert_eq!(state.value(), SliderValue::Single(400.));
        assert_eq!(state.percentage().end, 0.25);
    }
}
pub(super) use super::design::{Palette, palette};
impl Desktop {
    pub(super) fn apply_native_theme(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Palette {
        let settings = &self.snapshot["settings"];
        design::set_surface(settings);
        let surface = settings["windowSurface"].as_str().unwrap_or("opaque");
        if self.window.applied_window_surface.as_deref() != Some(surface) {
            if crate::window_surface::apply(window, surface) {
                self.window.applied_window_surface = Some(surface.to_owned());
            }
        }
        let stored_appearance = self
            .preferences
            .as_ref()
            .map(|preferences| preferences.data.appearance.clone())
            .unwrap_or_default();
        let is_dark = design::palette(settings, window.appearance()).dark;
        let interface_scale = design::normalize_interface_scale(stored_appearance.interface_scale);
        design::set_interface_scale(interface_scale);
        design::set_tile_theme(stored_appearance.tile_theme);
        design::set_border_opacity(stored_appearance.border_opacity);
        let mut palette = if self.native_theme_preset == "reference" {
            design::palette_for(settings, window.appearance(), "reference")
        } else {
            let preset = if is_dark {
                &stored_appearance.dark_palette
            } else {
                &stored_appearance.light_palette
            };
            let base = native_appearance::palette(
                preset,
                is_dark,
                stored_appearance.custom_accent.as_deref(),
                stored_appearance.contrast,
                stored_appearance.surface_tint,
            );
            if preset == "monitter" && wallpaper_theme::mode() != wallpaper_theme::Mode::Manual {
                if let Some(accents) = self.wallpaper_accent.read(cx).accents() {
                    let sampled = if wallpaper_theme::mode() == wallpaper_theme::Mode::Mood { accents.mood } else { accents.highlight };
                    let accent = format!("#{:06x}", wallpaper_theme::mix(base.accent, sampled, wallpaper_theme::strength()));
                    native_appearance::palette(preset, is_dark, Some(&accent), stored_appearance.contrast, stored_appearance.surface_tint)
                } else { base }
            } else { base }
        };
        crate::surface_blend::set_theme_dark(palette.dark);
        palette.line = blend_line(
            palette.paper,
            palette.line,
            stored_appearance.border_opacity,
        );
        let signature = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{:?}",
            palette.dark,
            palette.paper,
            palette.accent,
            field(settings, "interfaceFont"),
            settings["interfaceFontSize"],
            settings["chatFontSize"],
            stored_appearance.light_palette,
            stored_appearance.dark_palette,
            stored_appearance
                .custom_accent
                .as_deref()
                .unwrap_or_default(),
            stored_appearance.contrast,
            stored_appearance.surface_tint,
            stored_appearance.border_opacity,
            interface_scale,
            self.native_theme_preset,
            surface,
            settings["windowTransparency"],
            stored_appearance.tile_theme,
        );
        if self.window.applied_style.as_ref() != Some(&signature) {
            Theme::change(
                if palette.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
            let accent = field(settings, "accent");
            if let Some(color) = accent
                .strip_prefix('#')
                .filter(|color| color.len() == 6)
                .and_then(|color| u32::from_str_radix(color, 16).ok())
            {
                Theme::update(cx, |theme| {
                    theme.colors.primary = rgb(color).into();
                    theme.colors.primary_hover = rgb(color).into();
                    theme.colors.primary_active = rgb(color).into();
                });
            }
            let font = field(settings, "interfaceFont");
            let names = cx.text_system().all_font_names();
            let family = if !font.is_empty() && names.iter().any(|name| name == &font) {
                font
            } else if names.iter().any(|name| name == "IBM Plex Sans") {
                "IBM Plex Sans".into()
            } else {
                ".SystemUIFont".into()
            };
            let size = settings["interfaceFontSize"]
                .as_f64()
                .unwrap_or(design::INTERFACE_FONT_SIZE as f64)
                .clamp(8., 32.) as f32;
            Theme::update(cx, |theme| {
                theme.font_family = family.into();
                theme.font_size = design::ui_px(size, interface_scale);
                let composer_size = settings["chatFontSize"].as_f64()
                    .unwrap_or(design::CHAT_FONT_SIZE as f64).clamp(8., 32.) as f32;
                theme.tooltip_font_size = Some(px(composer_size - 1.));
                theme.mono_font_size = design::ui_px(13., interface_scale);
                // Text fields, selects, buttons and their native focus rings
                // share a control radius; larger cards share the card radius.
                theme.radius = design::ui_px(design::radius::CONTROL, interface_scale);
                theme.radius_lg = design::ui_px(design::radius::CARD, interface_scale);
                theme.colors.primary = rgb(palette.accent).into();
                theme.colors.primary_hover = rgb(palette.accent).into();
                theme.colors.primary_active = rgb(palette.accent).into();
                theme.colors.background = design::surface_color(palette.paper).into();
                theme.colors.foreground = rgb(palette.ink).into();
                // Shared by native Input/Textarea and selectable transcript TextViews.
                theme.colors.selection = design::text_selection_color(palette).into();
                theme.colors.border = rgb(palette.line).into();
                theme.colors.input = design::surface_color(palette.card).into();
                theme.colors.sidebar = design::surface_color(palette.sidebar).into();
                theme.colors.tab_bar = design::surface_color(palette.panel).into();
                theme.colors.tab_active = design::surface_color(palette.paper).into();
                theme.colors.tab_foreground = rgb(palette.muted).into();
                theme.colors.tab_active_foreground = rgb(palette.ink).into();
                theme.colors.muted = design::surface_color(palette.soft).into();
                theme.colors.muted_foreground = rgb(palette.muted).into();
                theme.colors.button = design::surface_color(palette.card).into();
                theme.colors.popover = design::menu_surface_color(palette).into();
                theme.colors.popover_foreground = rgb(palette.ink).into();
                theme.colors.accent = design::surface_color(palette.soft).into();
                theme.colors.accent_foreground = rgb(palette.ink).into();
                // Widget surfaces keep their existing hues, but use the same
                // window alpha as native cards. A deliberately clear spacer
                // remains clear rather than becoming a new stacked layer.
                let alpha = design::surface_color(palette.paper).a;
                for surface in [
                    &mut theme.colors.accordion,
                    &mut theme.colors.group_box,
                    &mut theme.colors.description_list_label,
                    &mut theme.colors.button_active,
                    &mut theme.colors.button_hover,
                    &mut theme.colors.button_danger,
                    &mut theme.colors.button_danger_active,
                    &mut theme.colors.button_danger_hover,
                    &mut theme.colors.button_info,
                    &mut theme.colors.button_info_active,
                    &mut theme.colors.button_info_hover,
                    &mut theme.colors.button_primary,
                    &mut theme.colors.button_primary_active,
                    &mut theme.colors.button_primary_hover,
                    &mut theme.colors.button_secondary,
                    &mut theme.colors.button_secondary_active,
                    &mut theme.colors.button_secondary_hover,
                    &mut theme.colors.button_success,
                    &mut theme.colors.button_success_active,
                    &mut theme.colors.button_success_hover,
                    &mut theme.colors.button_warning,
                    &mut theme.colors.button_warning_active,
                    &mut theme.colors.button_warning_hover,
                    &mut theme.colors.secondary,
                    &mut theme.colors.secondary_active,
                    &mut theme.colors.secondary_hover,
                    &mut theme.colors.list,
                    &mut theme.colors.list_active,
                    &mut theme.colors.list_even,
                    &mut theme.colors.list_head,
                    &mut theme.colors.list_hover,
                    &mut theme.colors.table,
                    &mut theme.colors.table_active,
                    &mut theme.colors.table_even,
                    &mut theme.colors.table_head,
                    &mut theme.colors.table_foot,
                    &mut theme.colors.table_hover,
                    &mut theme.colors.title_bar,
                    &mut theme.colors.status_bar,
                    &mut theme.colors.sidebar_accent,
                    &mut theme.colors.sidebar_primary,
                    &mut theme.colors.tab,
                    &mut theme.colors.tab_bar_segmented,
                ] {
                    if surface.a > 0. {
                        surface.a = alpha;
                    }
                }
            });
            self.window.dock.update(cx, |dock, cx| {
                dock.set_tab_appearance(settings, interface_scale, cx);
            });
            self.window.applied_style = Some(signature);
        }
        self.window.dock.update(cx, |dock, cx| {
            dock.set_pane_appearance(settings, palette, cx)
        });
        if crate::activity_effects::sync_cached(&self.snapshot, palette, cx) {
            self.window.dock.update(cx, |dock, cx| dock.refresh_activity(cx));
        }
        self.publish_report_theme(palette, cx);
        palette
    }
    pub(super) fn patch_settings(&mut self, patch: Value, cx: &mut Context<Self>) {
        let Some(mut settings) = self
            .snapshot
            .get("settings")
            .filter(|v| v.is_object())
            .cloned()
        else {
            self.window.error = Some("Settings have not loaded yet.".into());
            cx.notify();
            return;
        };
        for (key, value) in patch.as_object().into_iter().flatten() {
            settings[key] = value.clone();
        }
        self.command("save_settings", json!({"settings":settings}), cx);
    }
    fn save_typography(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &self.window.native_settings else {
            return;
        };
        let result = (|| {
            let mut patch = serde_json::Map::new();
            for ((family_key, size_key, weight_key, _, _), controls) in
                TYPOGRAPHY.iter().zip(&state.fonts)
            {
                let family = controls
                    .family
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .unwrap_or_default();
                let size = font_number(
                    controls.size.read(cx).value().as_ref(),
                    8.0,
                    32.0,
                    "Font size",
                )?;
                let weight = controls.weight.read(cx).value().end().round() as u64;
                if size.fract() != 0.0 {
                    return Err("Font size must be whole pixels; font weight must be 300, 400, 500, 600 or 700.".into());
                }
                patch.insert((*family_key).into(), json!(family.to_string()));
                patch.insert((*size_key).into(), json!(size as u64));
                patch.insert((*weight_key).into(), json!(weight));
            }
            patch.insert(
                "chatLineHeight".into(),
                json!((state.chat_line_height.read(cx).value().end() as f64 * 100.).round() / 100.),
            );
            patch.insert(
                "terminalLineHeight".into(),
                json!(
                    (state.terminal_line_height.read(cx).value().end() as f64 * 100.).round()
                        / 100.
                ),
            );
            Ok::<_, String>(Value::Object(patch))
        })();
        match result {
            Ok(patch) => self.patch_settings(patch, cx),
            Err(error) => {
                self.window.error = Some(error);
                cx.notify();
            }
        }
    }
    fn save_native_tile_theme(&mut self, theme: NativeTileTheme, cx: &mut Context<Self>) {
        if self.preferences_busy {
            return;
        }
        let Some(preferences) = self.preferences.as_mut() else {
            return;
        };
        if preferences.data.appearance.tile_theme == theme {
            return;
        }
        preferences.data.appearance.tile_theme = theme;
        self.workspaces_dirty = true;
        design::set_tile_theme(theme);
        self.window.applied_style = None;
        self.preferences_error = None;
        self.queue_preferences_save(cx);
        cx.notify();
    }
    fn save_native_appearance(&mut self, cx: &mut Context<Self>) {
        if self.preferences_busy || self.preferences.is_none() {
            return;
        }
        let result: Result<(String, String, Option<String>, u16, u16, u16, u16), String> = (|| {
            let controls = self.window.native_settings
                .as_ref()
                .and_then(|settings| settings.appearance.as_ref())
                .ok_or("Native appearance preferences are still loading.")?;
            let light = controls
                .light
                .read(cx)
                .selected_value()
                .cloned()
                .ok_or("Choose a light palette.")?;
            let dark = controls
                .dark
                .read(cx)
                .selected_value()
                .cloned()
                .ok_or("Choose a dark palette.")?;
            let accent = controls.accent.read(cx).value().to_string();
            let contrast = controls.contrast.read(cx).value().end().round() as u16;
            let tint = controls.tint.read(cx).value().end().round() as u16;
            let border_opacity = controls.border_opacity.read(cx).value().end().round() as u16;
            let interface_scale = controls.interface_scale.read(cx).value().end().round() as u16;
            let accent = if accent.trim().is_empty() {
                None
            } else if native_appearance::parse_accent(&accent).is_some() {
                Some(format!(
                    "#{}",
                    accent.trim().trim_start_matches('#').to_ascii_uppercase()
                ))
            } else {
                return Err("Custom accent must be blank or a six-digit #RRGGBB value.".into());
            };
            Ok((
                native_appearance::preset_id(&light).to_owned(),
                native_appearance::preset_id(&dark).to_owned(),
                accent,
                contrast,
                tint,
                border_opacity,
                interface_scale,
            ))
        })(
        );
        let (light, dark, accent, contrast, tint, border_opacity, interface_scale) = match result {
            Ok(values) => values,
            Err(error) => {
                self.preferences_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let preferences = self.preferences.as_mut().unwrap();
        preferences.data.appearance.light_palette = light;
        preferences.data.appearance.dark_palette = dark;
        preferences.data.appearance.custom_accent = accent;
        preferences.data.appearance.contrast = contrast;
        preferences.data.appearance.surface_tint = tint;
        preferences.data.appearance.border_opacity = border_opacity;
        preferences.data.appearance.interface_scale = interface_scale;
        preferences.data.appearance.warm_paper = self.native_theme_preset == "reference";
        self.window.applied_style = None;
        self.preferences_error = None;
        self.queue_preferences_save(cx);
        cx.notify();
    }
    fn load_model_catalog(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(id) = self.window.selected.clone() else {
            return;
        };
        let Some(service) = self.service.clone() else {
            return;
        };
        let Some(settings) = &mut self.window.native_settings else {
            return;
        };
        if !settings.loading.insert(id.clone()) {
            return;
        }
        settings.errors.remove(&id);
        let target = id.clone();
        let task = cx.background_executor().spawn(async move {
            service.invoke(
                "get_model_catalog",
                json!({"target":{"taskId":target,"refresh":true}}),
            )
        });
        self.spawn_workspace(window, cx, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |view, _, cx| {
                if let Some(settings) = &mut view.window.native_settings {
                    settings.loading.remove(&id);
                    match result {
                        Ok(catalog) => {
                            settings
                                .choices
                                .insert(id.clone(), catalog["current"].clone());
                            settings.catalogs.insert(id, catalog);
                        }
                        Err(error) => {
                            settings.errors.insert(id, error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn render_model_catalog_content(
        &mut self,
        palette: Palette,
        cx: &mut Context<Desktop>,
    ) -> AnyElement {
        let Some(id) = self.window.selected.clone() else {
            return div()
                .child("Select a chat to read its actual provider model catalog.")
                .into_any_element();
        };
        let state = self.window.native_settings.as_ref().unwrap();
        let loading = state.loading.contains(&id);
        let mut panel = div()
            .w_full()
            .max_w(px(820.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(design::space::TIGHT))
            .p(px(design::space::INSET))
            .border_1()
            .rounded(px(crate::design::radius::CARD))
            .bg(crate::design::surface_color(palette.card))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Selected chat model"),
            )
            .child("Read the current harness catalog; nothing changes until you apply a choice.")
            .child(
                Button::new("load-models")
                    .label(if loading {
                        "Reading harness catalog…"
                    } else {
                        "Read actual harness model catalog"
                    })
                    .disabled(loading)
                    .on_click(cx.workspace_listener(|view, _, window, cx| view.load_model_catalog(window, cx))),
            );
        if let Some(error) = self.window.native_settings
            .as_ref()
            .and_then(|state| state.errors.get(&id))
        {
            panel = panel.child(
                div()
                    .text_color(rgb(0xbd655b))
                    .child(format!("Catalog error: {error}")),
            );
        }
        if let Some(catalog) = self.window.native_settings
            .as_ref()
            .and_then(|state| state.catalogs.get(&id).cloned())
        {
            panel = panel.child(format!(
                "Source: {} · {}",
                field(&catalog, "source"),
                field(&catalog, "warning")
            ));
            let choice = self.window.native_settings
                .as_ref()
                .and_then(|state| state.choices.get(&id).cloned())
                .unwrap_or_else(|| catalog["current"].clone());
            let models = rows(&catalog, "models");
            if models.is_empty() {
                panel = panel.child("This harness reported no selectable models.");
            }
            for model in &models {
                let target = id.clone();
                let model_id = field(model, "id");
                let effort = model["defaultEffort"].clone();
                let selected = field(&choice, "model") == model_id;
                panel = panel.child(Button::new(SharedString::from(format!("model-{model_id}")))
                    .label(format!("{}{} · {}", if selected { "✓ " } else { "" }, field(model, "name"), field(model, "description")))
                    .on_click(cx.workspace_listener(move |view, _, _, cx| {
                        if let Some(state) = &mut view.window.native_settings {
                            state.choices.insert(target.clone(), json!({"model":model_id,"reasoningEffort":effort,"fastMode":null}));
                        }
                        cx.notify();
                    })));
            }
            if let Some(model) = models
                .iter()
                .find(|model| field(model, "id") == field(&choice, "model"))
            {
                for effort in rows(model, "reasoningEfforts") {
                    let target = id.clone();
                    let effort_id = field(&effort, "id");
                    let current = field(&choice, "reasoningEffort");
                    panel = panel.child(
                        Button::new(SharedString::from(format!("effort-{effort_id}")))
                            .label(format!(
                                "{}Effort: {} · {}",
                                if current == effort_id { "✓ " } else { "" },
                                effort_id,
                                field(&effort, "description")
                            ))
                            .on_click(cx.workspace_listener(move |view, _, _, cx| {
                                if let Some(choice) = view.window.native_settings
                                    .as_mut()
                                    .and_then(|state| state.choices.get_mut(&target))
                                {
                                    choice["reasoningEffort"] = json!(effort_id);
                                }
                                cx.notify();
                            })),
                    );
                }
                if model["supportsFast"] == true {
                    for fast in [false, true] {
                        let target = id.clone();
                        let selected = choice["fastMode"].as_bool() == Some(fast);
                        panel = panel.child(
                            Button::new(if fast { "fast-on" } else { "fast-off" })
                                .label(format!(
                                    "{}Fast mode {}",
                                    if selected { "✓ " } else { "" },
                                    if fast { "on" } else { "off" }
                                ))
                                .on_click(cx.workspace_listener(move |view, _, _, cx| {
                                    if let Some(choice) = view.window.native_settings
                                        .as_mut()
                                        .and_then(|state| state.choices.get_mut(&target))
                                    {
                                        choice["fastMode"] = json!(fast);
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
            panel = panel.child(
                Button::new("apply-task-model")
                    .primary()
                    .label("Apply to selected chat · next turn")
                    .disabled(self.window.busy || models.is_empty())
                    .on_click(cx.workspace_listener(move |view, _, _, cx| {
                        view.command(
                            "set_task_model_settings",
                            json!({"taskId":id,"settings":choice}),
                            cx,
                        )
                    })),
            );
        }
        panel.into_any_element()
    }

    pub(super) fn render_preferences_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.window.native_settings.is_none() {
            self.window.native_settings =
                Some(NativeSettings::new(&self.snapshot["settings"], window, cx));
        }
        if self.window.native_settings
            .as_ref()
            .is_some_and(|state| state.lan.is_none())
        {
            if let Some(service) = self.service.clone() {
                let lan = cx.new(|cx| lan_settings::LanSettingsPane::new(service, window, cx));
                self.window.native_settings.as_mut().unwrap().lan = Some(lan);
            }
        }
        if self.window.native_settings
            .as_ref()
            .is_some_and(|state| state.services.is_none())
        {
            if let Some(service) = self.service.clone() {
                let snapshot = self.snapshot.as_ref().clone();
                let services = cx.new(|cx| {
                    settings_services::ServicePreferencesPane::new(service, &snapshot, window, cx)
                });
                self.window.native_settings.as_mut().unwrap().services = Some(services);
            }
        }
        if self.window.native_settings
            .as_ref()
            .is_some_and(|state| state.appearance.is_none())
        {
            if let Some(appearance) = self
                .preferences
                .as_ref()
                .map(|preferences| preferences.data.appearance.clone())
            {
                let controls = NativeAppearanceControls::new(&appearance, window, cx);
                let state = self.window.native_settings.as_mut().unwrap();
                for slider in [
                    &controls.contrast,
                    &controls.tint,
                    &controls.border_opacity,
                    &controls.interface_scale,
                ] {
                    state
                        ._slider_subscriptions
                        .push(cx.observe(slider, |_, _, cx| cx.notify()));
                }
                for select in [&controls.light, &controls.dark] {
                    state._slider_subscriptions.push(cx.subscribe_workspace(select, window, |view, _, _: &SelectEvent<Vec<settings_appearance::PaletteChoice>>, _, cx| view.save_native_appearance(cx)));
                }
                for slider in [&controls.contrast, &controls.tint, &controls.border_opacity, &controls.interface_scale] {
                    state._slider_subscriptions.push(cx.subscribe_workspace(slider, window, |view, _, event: &SliderEvent, _, cx| {
                        if matches!(event, SliderEvent::Release(_)) { view.save_native_appearance(cx); } else { cx.notify(); }
                    }));
                }
                state._slider_subscriptions.push(cx.subscribe_workspace(&controls.accent_picker, window, |view, _, event: &ColorPickerEvent, window, cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else { return };
                    let rgba: gpui_kit::Rgba = (*color).into();
                    let hex = format!("#{:06X}", u32::from(rgba) >> 8);
                    if let Some(native) = view.window.native_settings.as_ref().and_then(|s| s.appearance.as_ref()) {
                        native.accent.update(cx, |input, cx| input.set_value(hex, window, cx));
                    }
                    view.save_native_appearance(cx);
                }));
                state._slider_subscriptions.push(cx.subscribe_workspace(&controls.accent, window, |view, input, event: &gpui_kit::component::input::InputEvent, window, cx| {
                    let text=input.read(cx).value().to_string();
                    if let Some(color)=native_appearance::parse_accent(&text) {
                        if let Some(native)=view.window.native_settings.as_ref().and_then(|s|s.appearance.as_ref()) {
                            native.accent_picker.update(cx, |picker,cx| picker.set_value(rgb(color),window,cx));
                        }
                    }
                    if matches!(event,gpui_kit::component::input::InputEvent::Blur | gpui_kit::component::input::InputEvent::PressEnter { .. }) { view.save_native_appearance(cx); }
                }));
                state.appearance = Some(controls);
            }
        }
        let snapshot = self.snapshot.as_ref().clone();
        let revision = self.revision.clone();
        let service_pane = self.window.native_settings
            .as_ref()
            .and_then(|state| state.services.clone());
        let update_service_snapshot = self.window.native_settings
            .as_ref()
            .is_some_and(|state| state.service_revision != revision);
        if update_service_snapshot {
            if let Some(service_pane) = &service_pane {
                service_pane.update(cx, |pane, cx| pane.set_snapshot(&snapshot, cx));
            }
            if let Some(state) = &mut self.window.native_settings {
                state.service_revision = revision;
            }
        }
        let palette = self.apply_native_theme(window, cx);
        SETTINGS_FONT_BITS.store(f32::from(cx.theme().font_size).to_bits(), std::sync::atomic::Ordering::Relaxed);
        let category = self.window.native_settings.as_ref().unwrap().category.clone();
        let destination = settings_navigation::category(&category);
        let title = destination.label;
        let mut body = match category.as_str() {
            "general" | "profile" => settings_general::render_profile(self, window, cx, palette),
            "security" => settings_general::render_security(self, window, cx, palette),
            "behaviour" => settings_general::render_behavior(self, cx, palette),
            "conversation" => settings_general::render_conversation(self, cx, palette),
            "appearance" | "interface" | "typography" | "terminal" => {
                settings_appearance::render(self, window, cx, palette, &category)
            }
            "agents" => {
                if self.window.agent_editor.is_none() {
                    if let Some(service) = self.service.clone() {
                        self.window.agent_editor = Some(cx.new(|cx| {
                            agents::AgentHostPane::new(
                                service,
                                self.snapshot.as_ref().clone(),
                                window,
                                cx,
                            )
                        }));
                    }
                }
                if let Some(editor) = &self.window.agent_editor {
                    if self.window.agent_editor_revision != self.revision {
                        editor.update(cx, |editor, cx| {
                            editor.set_snapshot(&self.snapshot, window, cx)
                        });
                        self.window.agent_editor_revision = self.revision.clone();
                    }
                    div()
                        .h_full()
                        .min_h(px(500.))
                        .child(editor.clone())
                        .into_any_element()
                } else {
                    div().child("Opening agent settings…").into_any_element()
                }
            }
            "lan" => self.window.native_settings
                .as_ref()
                .and_then(|state| state.lan.clone())
                .map(|pane| pane.into_any_element())
                .unwrap_or_else(|| {
                    div()
                        .child("LAN settings are unavailable.")
                        .into_any_element()
                }),
            "accounts" | "providers" | "extensions" | "secrets" | "mail" | "approvals"
            | "collaboration" => {
                if let Some(service_pane) = service_pane {
                    service_pane.update(cx, |pane, cx| {
                        pane.render_section(&category, palette, window, cx)
                    })
                } else {
                    div()
                        .child("Native settings service is unavailable.")
                        .into_any_element()
                }
            }
            _ => div()
                .child("Choose a settings category.")
                .into_any_element(),
        };
        if category == "providers" {
            let catalog = self.render_model_catalog_content(palette, cx);
            body = div()
                .flex()
                .flex_col()
                .gap(px(design::space::SECTION))
                .child(body)
                .child(catalog)
                .into_any_element();
        }
        if category == "collaboration" {
            let boards = settings_general::render_boards(self, cx, palette);
            body = div().w_full().flex().flex_col().gap(design::ui_scaled_px(16.))
                .child(body).child(boards).into_any_element();
        }
        if matches!(category.as_str(), "providers" | "accounts" | "collaboration") {
            body = div().w_full().min_w_0().flex().flex_col().gap(design::ui_scaled_px(16.))
                .child(div().flex().items_center().justify_between().flex_wrap().gap(design::ui_scaled_px(8.))
                    .child(settings_hint("Configure these choices for each agent.", palette))
                    .child(Button::new("preferences-open-agents").compact().label("Configure agents")
                        .on_click(cx.workspace_listener(|view, _, _, cx| {
                            if let Some(state) = &mut view.window.native_settings { state.select_category("agents"); }
                            cx.notify();
                        }))))
                .child(body).into_any_element();
        }
        let state = self.window.native_settings.as_ref().unwrap();
        let search = state.search.clone();
        let query = search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let results = state.search_results.clone();
        let search_selected = state.search_selected;
        let search_target = state.search_target;
        let page_scroll = state.page_scroll.clone();
        let search_scroll = state.search_scroll.clone();
        if searching {
            let mut list = div().id("preferences-search-results").max_h(window.viewport_size().height * 0.55).overflow_y_scroll().track_scroll(&search_scroll).debug_selector(|| "preferences-search-results".into()).flex().flex_col().gap(design::ui_scaled_px(6.));
            if results.is_empty() { list = list.child(settings_hint("No preferences found. Try a setting name or another keyword.", palette)); }
            for (index, entry) in results.iter().enumerate() {
                let entry = *entry;
                list = list.child(Button::new(SharedString::from(format!("preferences-search-result-{index}"))).ghost().selected(index == search_selected)
                    .w_full().h_auto().justify_start().py(design::ui_scaled_px(10.))
                    .accessibility_label(format!("{} — {}", entry.label, settings_navigation::category(entry.category).label))
                    .child(div().debug_selector(move || format!("preferences-search-result-{index}")).min_w_0().flex().flex_col().gap(design::ui_scaled_px(3.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(entry.label))
                        .child(settings_hint(format!("{} · {}", settings_navigation::category(entry.category).label, entry.description), palette)))
                    .on_click(cx.workspace_listener(move |view, _, window, cx| {
                        if let Some(state) = &mut view.window.native_settings { state.search_selected = index; }
                        view.open_preferences_search_result(window, cx);
                    })));
            }
            body = list.into_any_element();
        }
        // At large interface scales, the icon rail also preserves room for forms.
        let compact = window.viewport_size().width * 0.9 < design::ui_scaled_px(740.);
        let mut rows = Vec::<AnyElement>::new();
        let mut previous_group = "";
        for item in settings_navigation::CATEGORIES {
            if item.group != previous_group {
                rows.push(div().mt(design::ui_scaled_px(if previous_group.is_empty() { 0. } else { 10. }))
                    .when(!compact, |this| this.px(design::ui_scaled_px(9.)).pb(design::ui_scaled_px(6.))
                        .text_size(settings_text_size(0.71)).font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted)).child(item.group))
                    .when(compact && !previous_group.is_empty(), |this| this.border_t_1().border_color(rgb(palette.line)).pt(design::ui_scaled_px(8.)))
                    .into_any_element());
                previous_group = item.group;
            }
            let id = item.id;
            let selected = destination.id == id;
            rows.push(Button::new(SharedString::from(format!("settings-category-{id}"))).ghost()
                .selected(selected)
                .w_full().h_auto().compact().justify_start().py(design::ui_scaled_px(5.)).px(design::ui_scaled_px(8.))
                .bg(if selected { rgba((palette.accent << 8) | 0x21) } else { rgba(0) })
                .text_color(rgb(if selected { palette.accent } else { palette.ink }))
                .accessibility_label(item.label).tooltip(format!("{} — {}", item.label, item.description))
                .child(div().debug_selector(move || format!("settings-nav-{id}")).w_full().min_w_0().flex().items_center().gap(design::ui_scaled_px(9.))
                    .child(div().flex_shrink_0().size(design::ui_scaled_px(18.)).flex().items_center().justify_center()
                        .child(gpui_kit::component::Icon::new(item.icon.clone()).size(design::ui_scaled_px(16.))))
                    .when(!compact, |this| this.child(div().flex_1().min_w_0().whitespace_normal()
                        .text_size(settings_text_size(0.86)).font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                        .child(item.label))))
                .on_click(cx.workspace_listener(move |view, _, window, cx| {
                    view.clear_preferences_search(window, cx);
                    if let Some(state) = &mut view.window.native_settings { state.select_category(id); state.search_target = None; }
                    view.window.error = None;
                    cx.notify();
                })).into_any_element());
        }
        let navigation = div().id("preferences-navigation").min_h_0().overflow_y_scroll()
            .w(design::ui_scaled_px(if compact { 58. } else { 224. })).flex_shrink_0().h_full()
            .flex().flex_col().gap(design::ui_scaled_px(2.)).py(design::ui_scaled_px(14.)).px(design::ui_scaled_px(if compact { 8. } else { 12. }))
            .border_r_1().border_color(rgb(palette.line)).bg(design::surface_color(palette.paper))
            .children(rows);
        let status = if self.preferences_busy {
            "Saving private preferences…"
        } else if self.window.busy {
            "Saving settings…"
        } else {
            ""
        };
        div()
            .id("native-preferences-content")
            .size_full()
            .flex()
            .flex_col()
            .capture_key_down(cx.workspace_listener(|view, event: &KeyDownEvent, window, cx| {
                let Some(state) = &mut view.window.native_settings else { return; };
                if !state.search.read(cx).focus_handle(cx).contains_focused(window, cx) { return; }
                if state.search.update(cx, |input, cx| input.marked_text_range(window, cx).is_some()) { return; }
                let count = state.search_results.len();
                if count > 0 && matches!(event.keystroke.key.as_str(), "up" | "down") {
                    state.search_selected = if event.keystroke.key == "down" { (state.search_selected + 1) % count } else { (state.search_selected + count - 1) % count };
                    state.search_scroll.scroll_to_item(state.search_selected);
                    cx.stop_propagation(); cx.notify();
                }
            }))
            .bg(crate::design::surface_color(palette.paper))
            .font_family(cx.theme().font_family.clone()).text_size(crate::settings::settings_text_size(0.86)).text_color(rgb(palette.ink))
            .child(div().flex_shrink_0().p(design::ui_scaled_px(12.)).border_b_1().border_color(rgb(palette.line))
                .child(Input::new(&search).cleanable(true).aria_label("Search preferences").text_size(settings_text_size(0.86))))
            .child(div().flex_1().min_h_0().flex().flex_row().child(navigation)
            .child(
                div()
                    .id(SharedString::from(format!("native-preferences-page-scroll-{}", destination.id)))
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(design::space::SECTION))
                    .p(design::ui_scaled_px(if compact { 16. } else { 28. }))
                    .overflow_y_scroll().track_scroll(&page_scroll)
                    .child(div().w_full().flex_shrink_0().max_w(design::ui_scaled_px(760.)).mx_auto().flex().flex_col().gap(design::ui_scaled_px(8.))
                        .child(div().flex().items_center().flex_wrap().justify_between().gap(design::ui_scaled_px(8.))
                            .child(div().text_size(settings_text_size(1.5)).font_weight(FontWeight::SEMIBOLD).child(if searching { "Search results" } else { title }))
                            .when(!status.is_empty(), |this| this.child(settings_hint(status, palette))))
                        .child(settings_hint(if searching { "Select a setting to open its category." } else { destination.description }, palette))
                        .when(!searching, |this| this.when_some(search_target, |this, target| this.child(settings_hint(format!("Selected setting: {target}"), palette)))))
                    .child(div().w_full().min_w_0().flex_shrink_0().max_w(design::ui_scaled_px(760.)).mx_auto().child(body))
                    .when_some(self.window.error.clone(), |this, error| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(design::space::TIGHT))
                                .child(div().flex_1().text_color(rgb(0xbd655b)).child(error))
                                .child(
                                    Button::new("dismiss-settings-error")
                                        .compact()
                                        .label("Dismiss")
                                        .on_click(cx.workspace_listener(|view, _, _, cx| {
                                            view.window.error = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .when_some(
                        if category == "interface" {
                            self.preferences_error.clone()
                        } else {
                            None
                        },
                        |this, error| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(design::space::TIGHT))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_color(rgb(0xbd655b))
                                            .child(format!("Workspace layout: {error}")),
                                    )
                                    .child(
                                        Button::new("dismiss-layout-settings-error")
                                            .compact()
                                            .label("Dismiss")
                                            .on_click(cx.workspace_listener(|view, _, _, cx| {
                                                view.preferences_error = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                        },
                    ),
            )
            )
            .into_any_element()
    }

    pub(super) fn preferences_search_composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.window.native_settings.as_ref().is_some_and(|state| state.search.update(cx, |input, cx| input.marked_text_range(window, cx).is_some()))
    }
    pub(super) fn clear_preferences_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(state) = &mut self.window.native_settings else { return false; };
        if state.search.read(cx).value().is_empty() { return false; }
        state.search.update(cx, |input, cx| input.set_value("", window, cx));
        state.search_selected = 0;
        state.search_results.clear();
        cx.notify();
        true
    }
    fn open_preferences_search_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preferences_search_composing(window, cx) { return; }
        let Some(state) = &mut self.window.native_settings else { return; };
        if state.search.read(cx).value().trim().is_empty() { return; }
        let Some(entry) = state.search_results.get(state.search_selected).copied() else { return; };
        state.search_results.clear();
        state.select_category(entry.category);
        state.appearance_advanced = true;
        state.search.update(cx, |input, cx| input.set_value("", window, cx));
        state.search_target = Some(entry.label);
        let anchor = state.target_anchor.clone();
        // Lay out the destination card before requesting its nested anchor.
        if entry.label != "Agent delegation" && matches!(entry.category, "profile" | "appearance" | "interface" | "typography" | "behaviour" | "conversation" | "collaboration") {
            window.on_next_frame(move |window, cx| anchor.scroll_to(window, cx));
        }
        window.focus(&self.window.preferences_focus, cx);
        cx.notify();
    }
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_preferences_content(window, cx)
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod modal_layout_tests {
    use super::*;
    use ::core::prelude::v1::test;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, size};
    struct SettingsHarness { desktop: Entity<Desktop> }
    impl Render for SettingsHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.desktop.update(cx, |desktop, cx| desktop.render_preferences_modal(window, cx))
        }
    }
    #[gpui_kit::test]
    fn preferences_search_is_local_and_preserves_drafts(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let mut desktop_entity = None;
        let (_, mut visual) = cx.add_window_view(|window, cx| {
            let desktop = cx.new(|cx| {
                let mut desktop = Desktop::new(window, cx);
                desktop.snapshot = Rc::new(json!({"settings":{"theme":"dark","userName":"Preview"},"agents":[],"projects":[],"tasks":[]}));
                desktop.preferences = Some(NativePreferences::default());
                desktop.preferences_open = true;
                desktop
            });
            desktop_entity = Some(desktop.clone());
            let harness = cx.new(|_| SettingsHarness { desktop });
            gpui_kit::component::Root::new(harness, window, cx)
        });
        let desktop = desktop_entity.unwrap();
        visual.simulate_resize(size(gpui_kit::px(1200.), gpui_kit::px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, cx| desktop.update(cx, |desktop, cx| {
            let state = desktop.native_settings.as_ref().unwrap();
            state.board_guidance.update(cx, |input, cx| input.set_value("Unsaved private draft", window, cx));
            window.focus(&state.search.read(cx).focus_handle(cx), cx);
        }));
        visual.simulate_input("transparncy");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let result = visual.debug_bounds("preferences-search-result-0").expect("typo has a visible result");
        visual.simulate_click(result.center(), Default::default());
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let card = visual.debug_bounds("settings-card-Primary window surface").expect("target card renders");
        let dialog = visual.debug_bounds("preferences-card").unwrap();
        assert!(card.top() >= dialog.top() && card.top() < dialog.bottom(), "target card scrolled into dialog");
        assert!(card.size.height > gpui_kit::px(150.), "target card contains its controls as well as the header");
        visual.read(|cx| {
            let desktop = desktop.read(cx);
            let state = desktop.native_settings.as_ref().unwrap();
            assert_eq!(state.category, "appearance");
            assert_eq!(state.search_target, Some("Window transparency"));
            assert_eq!(state.board_guidance.read(cx).value().as_str(), "Unsaved private draft");
            assert!(!desktop.busy && desktop.service.is_none());
        });
        visual.update(|window, cx| desktop.update(cx, |desktop, cx| {
            let state = desktop.native_settings.as_ref().unwrap();
            window.focus(&state.search.read(cx).focus_handle(cx), cx);
        }));
        visual.simulate_input("zzqx nonexistent");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("preferences-search-results").is_some());
        assert!(visual.debug_bounds("preferences-search-result-0").is_none());
        visual.simulate_keystrokes("escape");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.read(|cx| {
            assert!(desktop.read(cx).preferences_open);
            assert!(desktop.read(cx).native_settings.as_ref().unwrap().search.read(cx).value().is_empty());
        });
        // Marked text belongs to the native input method, including Enter,
        // Escape and arrows; it must not choose a result or close Preferences.
        for key in ["down", "enter", "escape"] {
            visual.update(|window, cx| desktop.update(cx, |desktop, cx| {
                let state = desktop.native_settings.as_ref().unwrap();
                state.search.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.replace_and_mark_text_in_range(None, "font", Some(4..4), window, cx);
                });
            }));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.simulate_keystrokes(key);
            visual.update(|window, cx| desktop.update(cx, |desktop, cx| {
                assert!(desktop.preferences_open);
                let state = desktop.native_settings.as_ref().unwrap();
                assert_eq!(state.category, "appearance");
                assert_eq!(state.search_selected, 0);
                state.search.update(cx, |input, cx| { input.unmark_text(window, cx); input.set_value("", window, cx); });
            }));
        }
        visual.simulate_input("font size");
        visual.simulate_keystrokes("enter");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.read(|cx| assert_eq!(desktop.read(cx).native_settings.as_ref().unwrap().category, "typography"));
        visual.simulate_keystrokes("escape");
        visual.read(|cx| assert!(!desktop.read(cx).preferences_open));
    }

    #[gpui_kit::test]
    fn design_system_settings_modal_navigation_renders_rich_controls_and_stays_in_dialog(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let mut desktop_entity=None;
        let (_, visual)=cx.add_window_view(|window,cx| {
            let desktop=cx.new(|cx| {
                let mut desktop=Desktop::new(window,cx);
                desktop.snapshot=Rc::new(json!({"settings":{"theme":"dark","userName":"Preview","interfaceFontSize":14,"interfaceScale":100},"agents":[],"projects":[],"tasks":[]}));
                desktop.preferences=Some(NativePreferences::default());
                desktop.preferences_open=true;
                desktop
            });
            desktop_entity=Some(desktop.clone());
            let harness=cx.new(|_| SettingsHarness{desktop});
            gpui_kit::component::Root::new(harness,window,cx)
        });
        let desktop=desktop_entity.unwrap();
        visual.simulate_resize(size(gpui_kit::px(1200.),gpui_kit::px(800.)));
        visual.update(|window,cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("settings-card-Your name").is_some());
        for (category,selector,card_selector) in [
            ("appearance","settings-nav-appearance","settings-card-Appearance mode"),
            ("interface","settings-nav-interface","settings-card-Tile layout"),
            ("typography","settings-nav-typography","settings-card-Fonts"),
            ("behaviour","settings-nav-behaviour","settings-card-Keyboard shortcuts"),
            ("conversation","settings-nav-conversation","settings-card-Messages"),
            ("collaboration","settings-nav-collaboration","settings-card-Project boards"),
            ("security","settings-nav-security","settings-card-Session lock"),
        ] {
            let mut button=visual.debug_bounds(selector).expect("category is present");
            if button.bottom() > gpui_kit::px(700.) {
                visual.simulate_event(gpui_kit::ScrollWheelEvent {
                    position: gpui_kit::point(button.center().x, gpui_kit::px(400.)),
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-400.))),
                    modifiers: Default::default(), touch_phase: gpui_kit::TouchPhase::Moved,
                });
                visual.update(|window,cx| window.draw(cx).clear(cx));
                button=visual.debug_bounds(selector).expect("category remains present after scroll");
            }
            visual.simulate_click(button.center(),Default::default());
            visual.update(|window,cx| window.draw(cx).clear(cx));
            visual.read(|cx| {
                assert!(desktop.read(cx).preferences_open,"navigation keeps modal open");
                assert_eq!(desktop.read(cx).native_settings.as_ref().unwrap().category,category);
                assert!(!desktop.read(cx).busy,"navigation never writes settings");
                assert!(desktop.read(cx).service.is_none(),"fixture stays offline");
            });
            assert!(visual.debug_bounds(card_selector).is_some(),"category cards render");
        }
        // Compact navigation keeps every destination reachable, with form
        // content contained by the dialog instead of stretching the window.
        visual.simulate_resize(size(gpui_kit::px(700.),gpui_kit::px(700.)));
        visual.update(|window,cx| window.draw(cx).clear(cx));
        let dialog = visual.debug_bounds("preferences-card").unwrap();
        for selector in [
            "settings-nav-profile", "settings-nav-appearance", "settings-nav-interface",
            "settings-nav-typography", "settings-nav-behaviour", "settings-nav-conversation",
            "settings-nav-agents", "settings-nav-providers", "settings-nav-accounts",
            "settings-nav-collaboration", "settings-nav-extensions", "settings-nav-mail",
            "settings-nav-security", "settings-nav-approvals", "settings-nav-secrets", "settings-nav-lan",
        ] {
            let nav = visual.debug_bounds(selector).unwrap();
            assert!(nav.size.width < gpui_kit::px(50.), "compact menu uses icons");
            assert!(nav.left() >= dialog.left() && nav.right() <= dialog.right());
        }
        desktop.update(visual, |desktop, cx| {
            desktop.native_settings.as_mut().unwrap().select_security();
            assert_eq!(desktop.native_settings.as_ref().unwrap().category, "security");
            cx.notify();
        });
        visual.simulate_resize(size(gpui_kit::px(1800.),gpui_kit::px(900.)));
        visual.update(|window,cx| window.draw(cx).clear(cx));
        let card=visual.debug_bounds("preferences-card").expect("modal card is present");
        assert_eq!(card.size.width,(gpui_kit::px(1800.) * 0.9).min(design::ui_scaled_px(1296.)));
        assert_eq!(card.center().x,gpui_kit::px(900.));
        visual.simulate_click(gpui_kit::point(gpui_kit::px(20.),gpui_kit::px(450.)),Default::default());
        visual.read(|cx| assert!(!desktop.read(cx).preferences_open,"outside click dismisses the modal"));
    }
}
