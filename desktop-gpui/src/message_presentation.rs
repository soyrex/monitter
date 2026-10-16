//! Display provenance without changing the role sent to the harness.
use crate::design::{self, Palette};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin { User, Assistant, Peer, System }

pub(super) fn origin(message: &Value) -> Origin {
    let nonempty = |key: &str| message[key].as_str().is_some_and(|s| !s.is_empty());
    match message["role"].as_str() {
        Some("system") => Origin::System,
        Some("user") if nonempty("senderAgentId") || nonempty("collaborationId") => Origin::Peer,
        Some("user") => Origin::User,
        _ => Origin::Assistant,
    }
}

pub(super) fn peer_label(_message: &Value, name: Option<&str>) -> String {
    let name = name.filter(|s| !s.trim().is_empty()).unwrap_or("Peer agent");
    // These stored messages are delivered into this chat, including replies
    // represented as system messages on the calling task.
    format!("← {name}")
}

pub(super) fn grey(rgb: u32) -> u32 {
    let level = (((rgb >> 16) & 255) * 299 + ((rgb >> 8) & 255) * 587 + (rgb & 255) * 114) / 1000;
    level * 0x010101
}

pub(super) struct NeutralColours {
    pub background: u32, pub border: u32, pub ink: u32, pub muted: u32,
}
pub(super) fn neutral_colours(palette: Palette) -> NeutralColours {
    NeutralColours {
        background: super::mix_rgb(grey(palette.panel), grey(palette.ink), 0.04),
        border: super::mix_rgb(grey(palette.line), grey(palette.ink), 0.12),
        ink: grey(palette.ink), muted: grey(palette.muted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn message_presentation_uses_metadata_even_for_same_agent_profile() {
        let message = json!({"role":"user","senderAgentId":"codex","collaborationId":"delivery","text":"Hello"});
        assert_eq!(origin(&message), Origin::Peer);
        assert_eq!(peer_label(&message, Some("Codex")), "← Codex");
        assert_eq!(origin(&json!({"role":"user","collaborationId":"delivery"})), Origin::Peer);
        assert_eq!(peer_label(&message, None), "← Peer agent");
    }
    #[::core::prelude::v1::test]
    fn message_presentation_keeps_copied_agent_context_distinct_from_user() {
        let copied = json!({"role":"user","senderAgentId":"original","collaborationId":null});
        assert_eq!(origin(&copied), Origin::Peer);
        assert_eq!(peer_label(&copied, Some("Codex")), "← Codex");
        assert_eq!(origin(&json!({"role":"user","text":"Peer context from Codex:\nI typed this"})), Origin::User);
        assert_eq!(origin(&json!({"role":"user","text":"@(Alex): Hello"})), Origin::User);
    }
    #[::core::prelude::v1::test]
    fn message_presentation_system_results_remain_system_cards() {
        assert_eq!(origin(&json!({"role":"system","senderAgentId":"a","collaborationId":"c"})), Origin::System);
        assert_eq!(origin(&json!({"role":"assistant","senderAgentId":"a"})), Origin::Assistant);
    }
    #[::core::prelude::v1::test]
    fn message_presentation_colours_are_achromatic_in_light_and_dark_themes() {
        for appearance in [gpui_kit::WindowAppearance::Light, gpui_kit::WindowAppearance::Dark] {
            let palette = design::palette(&json!({"accent":"#ff0000"}), appearance);
            let colours = neutral_colours(palette);
            for colour in [colours.background, colours.border, colours.ink, colours.muted] {
                assert_eq!((colour >> 16) & 255, colour & 255);
                assert_eq!((colour >> 8) & 255, colour & 255);
            }
            assert_ne!(colours.background, colours.ink);
        }
    }
}
