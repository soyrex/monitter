//! Presentation-only destinations. IDs remain stable for command-palette links.
use gpui_kit::assets::IconName;

pub(super) struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub group: &'static str,
    pub icon: IconName,
}

pub(super) const CATEGORIES: &[Category] = &[
    Category { id: "profile", label: "Profile", description: "How agents address you in new chats.", group: "Workspace", icon: IconName::CircleUser },
    Category { id: "appearance", label: "Appearance", description: "Theme, colours and window surfaces.", group: "Workspace", icon: IconName::Palette },
    Category { id: "interface", label: "Layout & panes", description: "Workspace style, sizing and pane controls.", group: "Workspace", icon: IconName::PanelsTopLeft },
    Category { id: "typography", label: "Fonts & text", description: "Readable text across the interface, chats and terminals.", group: "Workspace", icon: IconName::ALargeSmall },
    Category { id: "behaviour", label: "Keyboard & focus", description: "Keyboard navigation and how panes receive focus.", group: "Workspace", icon: IconName::Settings2 },
    Category { id: "conversation", label: "Conversation", description: "Writing messages, streaming replies and showing activity.", group: "Workspace", icon: IconName::MessageSquare },
    Category { id: "agents", label: "Agents", description: "Configure agents, their hosts and permission modes.", group: "Agents & integrations", icon: IconName::Bot },
    Category { id: "system-one", label: "System One", description: "Default backend and routing with per-agent overrides.", group: "Agents & integrations", icon: IconName::Brain },
    Category { id: "providers", label: "Providers & models", description: "Review configured providers and choose models for chats.", group: "Agents & integrations", icon: IconName::Network },
    Category { id: "accounts", label: "Accounts", description: "Local harness accounts available to your agents.", group: "Agents & integrations", icon: IconName::User },
    Category { id: "collaboration", label: "Collaboration", description: "Agent delegation, project boards and shared guidance.", group: "Agents & integrations", icon: IconName::Users },
    Category { id: "extensions", label: "MCP & plugins", description: "Tools, servers, skills and plugins available to agents.", group: "Agents & integrations", icon: IconName::Blocks },
    Category { id: "mail", label: "Mail", description: "Connect mail accounts and assign them to agents.", group: "Agents & integrations", icon: IconName::Mail },
    Category { id: "work-history", label: "Work History", description: "Private local collection, history, retention and exports.", group: "Security & access", icon: IconName::Archive },
    Category { id: "security", label: "Session lock", description: "Protect this desktop with a PIN while agents keep running.", group: "Security & access", icon: IconName::ShieldCheck },
    Category { id: "approvals", label: "Approval rules", description: "Review and revoke saved always-approve rules.", group: "Security & access", icon: IconName::BadgeCheck },
    Category { id: "secrets", label: "Environment & secrets", description: "Manage shared environment variables for local agents.", group: "Security & access", icon: IconName::KeyRound },
    Category { id: "lan", label: "LAN access", description: "Connect to this desktop from a browser on your network.", group: "Security & access", icon: IconName::Wifi },
];

pub(super) fn category(id: &str) -> &'static Category {
    // Retain older links without exposing duplicate destinations in the menu.
    let canonical = match id { "general" => "profile", "terminal" => "typography", other => other };
    CATEGORIES.iter().find(|item| item.id == canonical).unwrap_or(&CATEGORIES[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::AssetSource;

    #[test]
    fn every_preferences_destination_has_a_bundled_distinct_icon() {
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for item in CATEGORIES {
            assert!(ids.insert(item.id), "duplicate destination {}", item.id);
            let path = item.icon.path();
            assert!(paths.insert(path.clone()), "duplicate icon for {}", item.id);
            let bytes = crate::native_icons::DesktopIcons.load(&path).unwrap()
                .unwrap_or_else(|| panic!("missing icon for {}: {path}", item.id));
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
        }
    }

    #[test]
    fn legacy_links_resolve_to_visible_destinations() {
        assert_eq!(category("general").id, "profile");
        assert_eq!(category("terminal").id, "typography");
        assert_eq!(category("interface").id, "interface");
        assert_eq!(category("behaviour").id, "behaviour");
        assert_eq!(category("unknown").id, "profile");
    }
}
