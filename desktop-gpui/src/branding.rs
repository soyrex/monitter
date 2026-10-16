//! The same bundled artwork used by the current desktop, without network IO.
use gpui_kit::{Image, ImageFormat};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

static WORDMARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../assets/branding/monitter-wordmark.png").to_vec(),
    ))
});
static MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../assets/branding/monitter-mark.png").to_vec(),
    ))
});
pub fn wordmark() -> Arc<Image> {
    WORDMARK.clone()
}

// GPUI's atlas uses bilinear filtering without mipmaps. Shrinking the 2169px
// master directly to a small sidebar wordmark aliases its edges. These PNGs
// are Lanczos reductions of that same master with premultiplied alpha; choose
// the smallest one covering the actual device-pixel width (Retina included).
static SIDEBAR_WORDMARKS: LazyLock<Vec<(u16, Arc<Image>)>> = LazyLock::new(|| {
    macro_rules! reduced {
        ($width:literal) => {
            ($width, Arc::new(Image::from_bytes(ImageFormat::Png,
                include_bytes!(concat!("../assets/branding/monitter-wordmark-", stringify!($width), ".png")).to_vec())))
        };
    }
    vec![reduced!(96), reduced!(144), reduced!(192), reduced!(240),
        reduced!(288), reduced!(336), reduced!(384)]
});

pub fn sidebar_wordmark(device_width: f32) -> Arc<Image> {
    SIDEBAR_WORDMARKS.iter()
        .find(|(width, _)| *width as f32 >= device_width)
        .or_else(|| SIDEBAR_WORDMARKS.last())
        .expect("bundled sidebar wordmarks")
        .1.clone()
}
pub fn mark() -> Arc<Image> {
    MARK.clone()
}

static PROVIDERS: LazyLock<HashMap<&'static str, Arc<Image>>> = LazyLock::new(|| {
    macro_rules! icon {
        ($name:literal) => {
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!(concat!("../assets/branding/", $name, ".svg")).to_vec(),
            ))
        };
    }
    HashMap::from([
        ("codex", icon!("codex-color")),
        ("claude", icon!("claude-color")),
        ("minimax", icon!("minimax-color")),
        ("opencode", icon!("opencode")),
        ("hermes", icon!("hermesagent")),
        ("openai", icon!("openai")),
        ("deepseek", icon!("deepseek-color")),
        ("gemini", icon!("gemini-color")),
        ("glm", icon!("glmv-color")),
        ("grok", icon!("grok")),
        ("kimi", icon!("kimi-color")),
        ("mistral", icon!("mistral-color")),
        ("qwen", icon!("qwen-color")),
    ])
});
static MONOCHROME: &[&str] = &["opencode", "hermes", "openai", "grok"];
static TINTED_PROVIDERS: LazyLock<Mutex<HashMap<(&'static str, u32), Arc<Image>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn provider_key(provider: &str, model: &str) -> Option<&'static str> {
    let name = model.to_ascii_lowercase();
    let tokens: Vec<_> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let has = |needle: &str| tokens.contains(&needle);
    let inferred = if has("claude") {
        Some("claude")
    } else if has("gpt") || has("openai") || tokens.iter().any(|s| matches!(*s, "o1" | "o3" | "o4"))
    {
        Some("openai")
    } else if has("minimax") || has("mmx") {
        Some("minimax")
    } else if has("kimi") || has("moonshot") {
        Some("kimi")
    } else if has("deepseek") {
        Some("deepseek")
    } else if has("qwen") {
        Some("qwen")
    } else if has("glm") || has("zhipu") {
        Some("glm")
    } else if has("gemini") {
        Some("gemini")
    } else if has("grok") {
        Some("grok")
    } else if has("mistral") {
        Some("mistral")
    } else {
        None
    };
    let key = inferred.unwrap_or(if provider == "opencode-go" {
        "opencode"
    } else {
        provider
    });
    PROVIDERS
        .get_key_value(key)
        .map(|(&static_key, _)| static_key)
}

/// Match provider/model brand selection; unknown brands retain the caller's
/// explicit fallback, rather than claiming a model identity.
pub fn provider(provider: &str, model: &str) -> Option<Arc<Image>> {
    provider_key(provider, model).and_then(|key| PROVIDERS.get(key).cloned())
}

/// Return provider artwork themed to the current ink color where its SVG uses
/// `currentColor`. Both tinted and original-color variants are cached.
pub fn provider_for_ink(provider: &str, model: &str, ink: u32) -> Option<Arc<Image>> {
    let key = provider_key(provider, model)?;
    if !MONOCHROME.contains(&key) {
        return PROVIDERS.get(key).cloned();
    }
    if let Ok(cache) = TINTED_PROVIDERS.lock() {
        if let Some(image) = cache.get(&(key, ink)) {
            return Some(image.clone());
        }
    }
    let svg = match key {
        "opencode" => include_bytes!("../assets/branding/opencode.svg").as_slice(),
        "hermes" => include_bytes!("../assets/branding/hermesagent.svg").as_slice(),
        "openai" => include_bytes!("../assets/branding/openai.svg").as_slice(),
        "grok" => include_bytes!("../assets/branding/grok.svg").as_slice(),
        _ => return PROVIDERS.get(key).cloned(),
    };
    let themed = String::from_utf8_lossy(svg).replace("currentColor", &format!("#{ink:06x}"));
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, themed.into_bytes()));
    if let Ok(mut cache) = TINTED_PROVIDERS.lock() {
        cache.insert((key, ink), image.clone());
    }
    Some(image)
}
