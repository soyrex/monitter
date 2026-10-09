//! Pure native appearance palette math mirrored from the Svelte app-theme helpers.
use super::design::{self, Palette};

pub const PRESETS: &[(&str, &str)] = &[
    ("monitter", "Monitter"),
    ("catppuccin", "Catppuccin"),
    ("dracula", "Dracula"),
    ("gruvbox", "Gruvbox"),
    ("nord", "Nord"),
    ("one", "One"),
    ("solarized", "Solarized"),
    ("tokyo-night", "Tokyo Night"),
    ("wombat", "Wombat"),
    ("xterm", "XTerm"),
];

pub fn preset_label(id: &str) -> &'static str {
    PRESETS
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map_or("Monitter", |(_, label)| label)
}

pub fn preset_id(label: &str) -> &'static str {
    PRESETS
        .iter()
        .find(|(_, candidate)| *candidate == label)
        .map_or("monitter", |(id, _)| id)
}

pub fn parse_accent(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits = value.strip_prefix('#')?;
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

fn mix(background: u32, accent: u32, amount: f64) -> u32 {
    let amount = amount.clamp(0.0, 1.0);
    let mut result = 0;
    for shift in [16, 8, 0] {
        let from = ((background >> shift) & 0xff) as f64;
        let to = ((accent >> shift) & 0xff) as f64;
        result |= ((from * (1.0 - amount) + to * amount).round() as u32) << shift;
    }
    result
}

/// Match `applyThemeContrast` followed by AppSurface's effective surface tint.
pub fn palette(
    preset: &str,
    dark: bool,
    accent_override: Option<&str>,
    contrast: u16,
    tint: u16,
) -> Palette {
    let mut palette = design::PRESETS
        .iter()
        .find(|(id, _, _)| *id == preset)
        .map(|(_, light, dark_palette)| if dark { *dark_palette } else { *light })
        .unwrap_or_else(|| {
            let (_, light, dark_palette) = design::PRESETS[0];
            if dark { dark_palette } else { light }
        });
    palette.dark = dark;
    let contrast = f64::from(contrast.min(100)) / 100.0;
    let background = if dark { 0x000000 } else { 0xffffff };
    let foreground = if dark { 0xffffff } else { 0x000000 };
    if contrast > 0.0 {
        for color in [
            &mut palette.paper,
            &mut palette.sidebar,
            &mut palette.card,
            &mut palette.soft,
            &mut palette.code,
        ] {
            *color = mix(*color, background, contrast * 0.65);
        }
        palette.panel = palette.card;
        palette.line = mix(palette.line, foreground, contrast * 0.35);
        palette.ink = mix(palette.ink, foreground, contrast * 0.55);
        palette.muted = mix(palette.muted, foreground, contrast * 0.5);
    }
    let accent = accent_override
        .and_then(parse_accent)
        .unwrap_or(palette.accent);
    let tint = f64::from(tint.min(50)) / 100.0 * if dark { 1.0 } else { 0.5 };
    for color in [
        &mut palette.paper,
        &mut palette.sidebar,
        &mut palette.card,
        &mut palette.line,
        &mut palette.soft,
        &mut palette.code,
    ] {
        *color = mix(*color, accent, tint);
    }
    palette.panel = palette.card;
    palette.accent = accent;
    palette
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn monitter_zero_adjustment_matches_source_palette() {
        let light = palette("monitter", false, None, 0, 0);
        assert_eq!(light.paper, 0xf7f7f7);
        assert_eq!(light.sidebar, 0xf0f0f0);
        assert_eq!(light.panel, 0xffffff);
        assert_eq!(light.line, 0xd8d8d8);
        assert_eq!(light.accent, 0x00a8f0);
        let dark = palette("monitter", true, None, 0, 0);
        assert_eq!(dark.paper, 0x151515);
        assert_eq!(dark.sidebar, 0x0f0f0f);
        assert_eq!(dark.panel, 0x202020);
    }

    #[::core::prelude::v1::test]
    fn tint_and_contrast_use_svelte_rgb_rounding() {
        let palette = palette("monitter", false, Some("#00A8F0"), 0, 5);
        assert_eq!(palette.paper, 0xf1f5f7);
        assert_eq!(palette.sidebar, 0xeaeeF0);
        let higher = super::mix(0x151515, 0x000000, 0.65);
        assert_eq!(higher, 0x070707);
    }

    #[::core::prelude::v1::test]
    fn accent_accepts_only_exact_rgb_hex() {
        assert_eq!(parse_accent("#Ab10F0"), Some(0xab10f0));
        assert_eq!(parse_accent("#fff"), None);
        assert_eq!(parse_accent("red"), None);
        assert_eq!(parse_accent(" #00A8F0 "), Some(0x00a8f0));
    }
}
