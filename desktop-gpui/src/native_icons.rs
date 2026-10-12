//! Extend the standard Kit assets with only the icons used by this desktop.
use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Paperclip,
        CornerDownLeft,
        Square,
        Mic,
        Mail,
        RefreshCw,
        Eye,
        FileText,
        MessageSquare,
        X,
        Globe,
        ChevronLeft,
        ChevronRight,
        ArrowRight,
        Plus,
        Goal,
        Trash,
        Monitor,
        ChevronDown,
        Copy,
        Reply,
        GitFork,
        BookOpen,
        Brain,
        Wrench,
        Briefcase,
        Code,
        Rocket,
        Palette,
        Database,
        Layers,
        Folder,
        Activity,
        Command,
        LayoutGrid,
        Archive,
        Radio,
        PanelsTopLeft,
        Blocks,
        KeyRound,
        Users,
        BadgeCheck,
        Wifi,
        ShieldCheck
    ]
);
pub struct DesktopIcons;
impl AssetSource for DesktopIcons {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn composer_and_reader_icons_resolve_to_bundled_svg_assets() {
        for icon in [
            gpui_kit::assets::IconName::X,
            gpui_kit::assets::IconName::FileText,
            gpui_kit::assets::IconName::MessageSquare,
            gpui_kit::assets::IconName::Eye,
            gpui_kit::assets::IconName::Mail,
            gpui_kit::assets::IconName::Mic,
            gpui_kit::assets::IconName::Paperclip,
            gpui_kit::assets::IconName::Globe,
            gpui_kit::assets::IconName::ChevronLeft,
            gpui_kit::assets::IconName::ChevronRight,
            gpui_kit::assets::IconName::ArrowRight,
            gpui_kit::assets::IconName::Plus,
            gpui_kit::assets::IconName::Goal,
            gpui_kit::assets::IconName::Trash,
            gpui_kit::assets::IconName::Monitor,
            gpui_kit::assets::IconName::ChevronDown,
            gpui_kit::assets::IconName::Copy,
            gpui_kit::assets::IconName::Reply,
            gpui_kit::assets::IconName::GitFork,
            gpui_kit::assets::IconName::BookOpen,
            gpui_kit::assets::IconName::Brain,
            gpui_kit::assets::IconName::Wrench,
            gpui_kit::assets::IconName::Briefcase,
            gpui_kit::assets::IconName::Code,
            gpui_kit::assets::IconName::Rocket,
            gpui_kit::assets::IconName::Palette,
            gpui_kit::assets::IconName::Database,
            gpui_kit::assets::IconName::Layers,
            gpui_kit::assets::IconName::Folder,
            gpui_kit::assets::IconName::Activity,
            gpui_kit::assets::IconName::Command,
            gpui_kit::assets::IconName::LayoutGrid,
            gpui_kit::assets::IconName::Archive,
            gpui_kit::assets::IconName::Radio,
            gpui_kit::assets::IconName::PanelsTopLeft,
        ] {
            let path = icon.path();
            let bytes = DesktopIcons
                .load(&path)
                .unwrap()
                .expect("required icon must be embedded");
            assert!(
                std::str::from_utf8(&bytes).unwrap().contains("<svg"),
                "{path}"
            );
        }
    }
}
