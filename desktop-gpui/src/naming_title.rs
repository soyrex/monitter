//! Retained, local title feedback; the authoritative title is never changed.
use gpui_kit::{App, AppContext as _, Context, Global, IntoElement, Render, SharedString, WeakEntity, Window, AnyElement, div, rgb, Styled as _, ParentElement as _};
use std::{collections::HashMap, hash::{Hash, Hasher}, time::{Duration, SystemTime, UNIX_EPOCH}};

#[derive(Default)]
struct Titles(HashMap<(u64, String, String), WeakEntity<NamingTitle>>);
impl Global for Titles {}

struct NamingTitle {
    task: String,
    title: SharedString,
    color: u32,
    seed: u64,
    displayed: SharedString,
    ticking: bool,
}

pub fn render(task: &str, surface: &str, title: SharedString, color: u32, window: &Window, cx: &mut App) -> AnyElement {
    if cx.try_global::<Titles>().is_none() { cx.set_global(Titles::default()); }
    let key = (window.window_handle().window_id().as_u64(), task.to_owned(), surface.to_owned());
    let existing = cx.global::<Titles>().0.get(&key).and_then(WeakEntity::upgrade);
    let entity = existing.unwrap_or_else(|| {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut hash);
        let seed = hash.finish();
        let entity = cx.new(|_| NamingTitle { task: task.to_owned(), title: title.clone(), displayed: title.clone(), color, seed, ticking: false });
        let registry = &mut cx.global_mut::<Titles>().0;
        registry.retain(|_, field| field.upgrade().is_some());
        registry.insert(key, entity.downgrade());
        entity
    });
    entity.update(cx, |field, cx| {
        if field.title != title || field.color != color {
            field.title = title.clone();
            field.displayed = title;
            field.color = color;
            cx.notify();
        }
        if !crate::native_motion::motion_allowed() {
            field.displayed = field.title.clone();
        } else if !field.ticking && crate::activity_effects::is_naming(task, cx) {
            field.ticking = true;
            cx.spawn(async move |weak, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let keep = weak.update(cx, |field, cx| {
                        if !crate::native_motion::motion_allowed() || !crate::activity_effects::is_naming(&field.task, cx) {
                            field.ticking = false;
                            field.displayed = field.title.clone();
                            cx.notify();
                            return false;
                        }
                        field.displayed = scramble(&field.title, &mut field.seed).into();
                        cx.notify();
                        true
                    }).unwrap_or(false);
                    if !keep { break; }
                }
            }).detach();
        }
    });
    entity.into_any_element()
}

fn random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut n = *seed;
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
fn scramble(title: &str, seed: &mut u64) -> String {
    let mut chars: Vec<_> = title.chars().collect();
    let mut eligible: Vec<_> = chars.iter().enumerate().filter_map(|(i, c)| c.is_alphabetic().then_some(i)).collect();
    let count = ((eligible.len() as f32 * 0.3).round() as usize).min(eligible.len());
    for i in 0..count {
        let selected = i + random(seed) as usize % (eligible.len() - i);
        eligible.swap(i, selected);
        let index = eligible[i];
        let original = chars[index];
        let base = if original.is_uppercase() { b'A' } else { b'a' };
        let mut replacement = (base + (random(seed) % 26) as u8) as char;
        if replacement == original { replacement = (base + ((replacement as u8 - base + 1) % 26)) as char; }
        chars[index] = replacement;
    }
    chars.into_iter().collect()
}
impl Render for NamingTitle {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().text_color(rgb(self.color)).child(self.displayed.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::scramble;
    #[test]
    fn naming_scramble_preserves_unicode_positions_and_source() {
        let title = String::from("Éclair 中文 — café! 123");
        let original = title.clone();
        let out = scramble(&title, &mut 42);
        assert_eq!(title, original);
        assert_eq!(title.chars().count(), out.chars().count());
        for (before, after) in title.chars().zip(out.chars()) {
            if !before.is_alphabetic() { assert_eq!(before, after); }
        }
        let letters = title.chars().filter(|c| c.is_alphabetic()).count();
        let changed = title.chars().zip(out.chars()).filter(|(a,b)| a != b).count();
        assert_eq!(changed, (letters as f32 * 0.3).round() as usize);
        assert_ne!(out, title);
    }
}
