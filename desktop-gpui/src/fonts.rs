//! Embedded desktop fonts; never installs or changes system fonts.
use gpui_kit::{App, Global, SharedString};
use std::{borrow::Cow, cell::{Cell, RefCell}, sync::{Arc, OnceLock}, time::Instant};

/// Application-local catalog. Font registration and explicit picker refresh
/// invalidate it; changing typography settings reuses the current snapshot.
/// Keep this separate from the toolkit's pre-registration system-font probe:
/// that probe cannot include the embedded faces registered afterward.
struct InstalledFonts {
    families: RefCell<Option<Arc<Vec<String>>>>,
    revision: Cell<u64>,
    bundled_registered: Cell<bool>,
    enumeration_count: Cell<u64>,
    enumeration_total_us: Cell<u64>,
}
impl Global for InstalledFonts {}
impl Default for InstalledFonts {
    fn default() -> Self {
        Self {
            families: RefCell::new(None), revision: Cell::new(1),
            bundled_registered: Cell::new(false), enumeration_count: Cell::new(0),
            enumeration_total_us: Cell::new(0),
        }
    }
}

/// Call after any future runtime `add_fonts` operation. No platform enumeration
/// happens here; existing typography signatures can observe the new revision.
pub fn invalidate(cx: &mut App) {
    if let Some(catalog) = cx.try_global::<InstalledFonts>() {
        *catalog.families.borrow_mut() = None;
        catalog.revision.set(catalog.revision.get().wrapping_add(1));
    } else { cx.set_global(InstalledFonts::default()); }
}

pub fn catalog_revision(cx: &App) -> u64 {
    cx.try_global::<InstalledFonts>().map_or(0, |catalog| catalog.revision.get())
}

/// A single immutable installed-font snapshot shared by interface, chat and
/// Markdown typography. These UI-thread accesses do not hold a lock over I/O.
pub fn installed(cx: &App) -> Arc<Vec<String>> {
    let catalog = cx.try_global::<InstalledFonts>();
    if let Some(families) = catalog.and_then(|catalog| catalog.families.borrow().clone()) {
        return families;
    }
    let families = enumerate(cx, catalog);
    if let Some(catalog) = catalog {
        *catalog.families.borrow_mut() = Some(families.clone());
    }
    families
}

fn enumerate(cx: &App, catalog: Option<&InstalledFonts>) -> Arc<Vec<String>> {
    let started = Instant::now();
    let mut families = cx.text_system().all_font_names();
    families.sort();
    families.dedup();
    let families = Arc::new(families);
    if let Some(catalog) = catalog {
        let elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let count = catalog.enumeration_count.get().saturating_add(1);
        let total_us = catalog.enumeration_total_us.get().saturating_add(elapsed_us);
        catalog.enumeration_count.set(count);
        catalog.enumeration_total_us.set(total_us);
        static DIAGNOSTICS: OnceLock<bool> = OnceLock::new();
        if count <= 8 && *DIAGNOSTICS.get_or_init(|| std::env::var_os("MONITTER_WORKSPACE_TIMING").is_some()) {
            eprintln!("[native-fonts] enumeration={count} elapsed_us={elapsed_us} total_elapsed_us={total_us} family_count={}", families.len());
        }
    }
    families
}

/// Opening the font picker deliberately refreshes fonts added outside this
/// process. Unchanged catalogs preserve typography signatures and measurements.
pub fn refresh(cx: &mut App) -> Arc<Vec<String>> {
    if cx.try_global::<InstalledFonts>().is_none() {
        cx.set_global(InstalledFonts::default());
    }
    let catalog = cx.global::<InstalledFonts>();
    let families = enumerate(cx, Some(catalog));
    let changed = catalog.families.borrow().as_ref()
        .is_none_or(|current| current.as_ref() != families.as_ref());
    if changed {
        catalog.revision.set(catalog.revision.get().wrapping_add(1));
        *catalog.families.borrow_mut() = Some(families.clone());
        families
    } else {
        // Retain the existing immutable snapshot identity as well as revision.
        catalog.families.borrow().as_ref().unwrap().clone()
    }
}

/// Preserve existing requested -> bundled -> system fallback precedence.
/// A successfully registered bundled face is known locally, so default startup
/// does not need to enumerate every installed system font just to find it.
pub fn resolve_family(requested: &str, bundled: &str, cx: &App) -> SharedString {
    let known_bundled = matches!(bundled, "IBM Plex Sans" | "IBM Plex Mono")
        && cx.try_global::<InstalledFonts>().is_some_and(|catalog| catalog.bundled_registered.get());
    if known_bundled && (requested.is_empty() || requested == bundled) { return bundled.to_owned().into(); }
    let names = installed(cx);
    if !requested.is_empty() && names.iter().any(|name| name == requested) { requested.to_owned().into() }
    else if names.iter().any(|name| name == bundled) { bundled.to_owned().into() }
    else { ".SystemUIFont".into() }
}


/// Register before resolving the theme's font families. Keep system fallbacks
/// available if a platform rejects an embedded face.
pub fn register(cx: &mut App) -> Result<(), String> {
    let result = cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf").as_slice()),
            Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf").as_slice()),
        ])
        .map_err(|error| format!("Could not load bundled IBM Plex fonts: {error}"));
    // Invalidate on failure as well: a platform may have installed some faces
    // before rejecting another. Failed registration uses platform resolution.
    invalidate(cx);
    if let Some(catalog) = cx.try_global::<InstalledFonts>() {
        catalog.bundled_registered.set(result.is_ok());
    }
    result
}
