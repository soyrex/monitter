//! Embedded desktop fonts; never installs or changes system fonts.
use std::borrow::Cow;

/// Register before resolving the theme's font families. Keep system fallbacks
/// available if a platform rejects an embedded face.
pub fn register(cx: &gpui_kit::App) -> Result<(), String> {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf").as_slice()),
        ])
        .map_err(|error| format!("Could not load bundled IBM Plex fonts: {error}"))
}
