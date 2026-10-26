//! Bounded, presentation-only index. Never reads a snapshot or user-entered values.
#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    pub category: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub keywords: &'static str,
}
macro_rules! entries {
    ($(($category:literal, $label:literal, $description:literal, $keywords:literal)),* $(,)?) => {
        pub(super) const ENTRIES: &[Entry] = &[$(Entry { category: $category, label: $label, description: $description, keywords: $keywords }),*];
    };
}
entries![
    ("system-one", "System One backend", "Default backend for routing, classification and advisory decisions.", "SYS-1 Jev TypeSafe Laya local inference"),
    ("system-one", "System One routing", "Default task routing with per-agent overrides.", "SYS-1 routing recommend auto inherit"),
    ("profile", "Your name", "How agents address you in new chats.", "profile username"),
    ("appearance", "Appearance mode", "System, light or dark appearance.", "theme"),
    ("appearance", "Window transparency", "Primary window surface and background transparency.", "opaque glass blur material opacity"),
    ("appearance", "App theme", "Choose light and dark palettes.", "colours colors"),
    ("appearance", "Workspace accent", "Accent shared with connected workspace clients.", "colour color"),
    ("appearance", "Accent colour", "Custom palette accent and surface tint.", "color tint"),
    ("appearance", "Borders & dividers", "Adjust structural line opacity.", "border divider"),
    ("appearance", "Contrast", "Contrast between surfaces and text.", "readability"),
    ("appearance", "Wallpaper accent", "Advanced wallpaper highlight, mood and strength.", "background"),
    ("appearance", "Native palette source", "Choose the native rendering palette source.", "reference warm paper"),
    ("appearance", "Surface blending", "Advanced blending of window areas.", "background transparency"),
    ("interface", "Tile layout", "Original workspace or rounded Tern tiles.", "layout style"),
    ("interface", "Interface scale", "Resize text and controls for this desktop.", "zoom sizing"),
    ("interface", "Reduce motion", "Motion: System, Subtle, On or Off animations.", "animation accessibility"),
    ("interface", "Interface density", "Height and spacing of tabs, sidebars and panes.", "tight normal spacious"),
    ("interface", "Auto Size Panes", "Keep split panes evenly sized when adding panes; divider resizing turns this off.", "automatic balance rebalance split resize dividers sizing"),
    ("interface", "Tabs", "Tab style, close buttons and auto hide tabs.", "tab strip"),
    ("interface", "Pane appearance", "Highlight active panes and dim inactive panes.", "opacity border outline thickness width pixels px"),
    ("typography", "Font size", "Interface, chat and terminal font sizes.", "fonts text px"),
    ("typography", "Font family & weight", "Choose interface, chat and terminal fonts.", "typography bold monospace"),
    ("typography", "Line height", "Spacing in chat messages and terminal output.", "text leading"),
    ("typography", "Terminal colour theme", "Native terminal colours.", "color palette"),
    ("behaviour", "Keyboard shortcuts", "Standard or Vim workspace navigation.", "shortcut mode keys"),
    ("behaviour", "Focus follows mouse", "Activate a pane when the pointer enters it.", "workspace focus"),
    ("conversation", "Enter to send", "Shortcut for sending messages.", "messages keyboard shift"),
    ("conversation", "Voice transcription", "Whisper or Apple on-device transcription engine.", "microphone recordings dictation"),
    ("conversation", "Default to steering busy agents", "Queue or steer messages during a running turn.", "busy message followup"),
    ("conversation", "Streaming replies", "How replies appear as they arrive.", "stream animation typing speed cursor slow fast"),
    ("conversation", "Transcript minimap", "Show message notches and previews beside the transcript.", "scroll overview navigation notches markers scrollbar"),
    ("conversation", "Show tool activity", "Collapsible harness activity blocks.", "tools"),
    ("conversation", "Compress tool calls", "Combine calls of the same kind.", "thinking activity"),
    ("conversation", "Tint my messages", "Accent colour for your message bubbles.", "color"),
    ("conversation", "Show reasoning summaries", "Summaries supplied by the harness.", "thinking"),
    ("agents", "Agents & hosts", "Configure agents, remote hosts and permission modes.", "ssh harness working directory"),
    ("providers", "Providers & models", "Configured providers and available models.", "llm catalog reasoning"),
    ("accounts", "Harness accounts", "Local harness accounts available to agents.", "authentication login codex"),
    ("collaboration", "Agent delegation", "Collaboration settings for agents.", "team"),
    ("collaboration", "Enable project boards", "Private coordination notes shown as channels.", "collaboration"),
    ("collaboration", "Automatically approve Monitter tools", "Approve the built-in Monitter MCP tools without harness prompts in Codex and Claude.", "mcp tools approvals permissions codex claude"),
    ("collaboration", "Default agent guidance", "Default project-board guidance and instructions.", "coordination notes"),
    ("extensions", "MCP servers", "Tools and servers available to agents.", "plugins extensions command url"),
    ("extensions", "Skills & plugins", "Instructions and extensions available to agents.", "mcp"),
    ("mail", "Mail accounts", "Connect mail accounts and assign them to agents.", "gmail oauth email"),
    ("work-history", "Work History collection", "Collect new agent work privately on this desktop.", "local private recording collector pause resume status health"),
    ("work-history", "Recorded work", "Filter and browse local work history.", "history project agent outcome date refresh"),
    ("work-history", "Retention & storage", "Set local history retention and storage limits.", "history days raw summary database quota"),
    ("work-history", "Export & delete", "Export JSON or CSV and delete selected local run history.", "history export json csv delete privacy"),
    ("security", "Session lock", "Protect the desktop with a PIN.", "password security idle unlock"),
    ("approvals", "Approval rules", "Review and revoke saved always-approve rules.", "permissions security"),
    ("secrets", "Environment & secrets", "Shared environment variables for local agents.", "env credentials keys"),
    ("pairing", "Pair a phone", "Pair an iPhone or secure browser using a fixed PIN and device approval.", "qr pin iphone mobile relay remote control remembered device"),
    ("lan", "LAN access", "Connect from a browser on your network.", "wifi mobile pairing token"),
];
pub(super) fn card_title(label: &'static str) -> &'static str {
    match label {
        "Retention & storage" => "Work History collection",
        "Window transparency" => "Primary window surface",
        "Wallpaper accent" => "Advanced · Wallpaper accent",
        "Surface blending" => "Advanced · Surface blending",
        "Reduce motion" => "Motion",
        "Font size" | "Font family & weight" => "Fonts",
        "Focus follows mouse" => "Workspace focus",
        "Enter to send" => "Messages",
        "Default to steering busy agents" => "Busy agents",
        "Show tool activity" | "Compress tool calls" | "Tint my messages" | "Show reasoning summaries" | "Transcript minimap" => "Conversation activity",
        "Enable project boards" | "Default agent guidance" => "Project boards",
        "Automatically approve Monitter tools" => "Monitter MCP approvals",
        other => other,
    }
}
fn normalize(text: &str) -> String {
    text.chars().take(160).flat_map(char::to_lowercase).map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0]; row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1).min(old + 1).min(diagonal + usize::from(ca != *cb));
            diagonal = old;
        }
    }
    row[b.len()]
}
fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = needle.chars();
    let mut next = chars.next();
    for ch in haystack.chars() { if next == Some(ch) { next = chars.next(); } }
    next.is_none()
}
pub(super) fn search(query: &str) -> Vec<&'static Entry> {
    let query = normalize(query);
    if query.is_empty() { return vec![]; }
    let mut matches = Vec::new();
    for (index, entry) in ENTRIES.iter().enumerate() {
        let label = normalize(entry.label);
        let text = normalize(&format!("{} {} {} {}", entry.label, entry.description, entry.keywords, super::settings_navigation::category(entry.category).label));
        let words: Vec<_> = text.split_whitespace().collect();
        let score = if label == query { Some(0) } else if label.contains(&query) { Some(10) } else {
            query.split_whitespace().try_fold(20, |score, token| {
                let best = words.iter().filter_map(|word| {
                    if *word == token { Some(0) }
                    else if word.starts_with(token) { Some(2) }
                    else if token.len() >= 4 && token.chars().count().abs_diff(word.chars().count()) <= 2 && distance(token, word) <= if token.len() >= 8 { 2 } else { 1 } { Some(7) }
                    else if token.len() >= 3 && subsequence(token, word) { Some(12 + word.len().saturating_sub(token.len())) }
                    else { None }
                }).min();
                best.map(|best| score + best)
            })
        };
        if let Some(score) = score { matches.push((score, index, entry)); }
    }
    matches.sort_by_key(|(score, index, _)| (*score, *index));
    matches.into_iter().take(30).map(|(_, _, entry)| entry).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn finds_controls_and_typos() {
        for (query, label) in [("font size", "Font size"), ("reduce motion", "Reduce motion"), ("transparncy", "Window transparency"), ("guidance", "Default agent guidance"), ("trnsp", "Window transparency")] {
            assert_eq!(search(query)[0].label, label, "{query}");
        }
    }
    #[test] fn ranking_and_empty_results() {
        assert_eq!(search("Tabs")[0].label, "Tabs");
        assert!(search(" ").is_empty());
        assert!(search("zzqx nonexistent").is_empty());
        assert_eq!(search("FONT-SIZE")[0].category, "typography");
        assert_eq!(search("layout panes")[0].category, "interface");
        assert_eq!(search("fonts text")[0].category, "typography");
        assert!(search(&"x".repeat(10000)).is_empty());
    }
    #[test] fn work_history_search_resolves_private_preferences_cards() {
        for query in ["work history", "retention", "export csv", "collector health"] {
            assert_eq!(search(query)[0].category, "work-history", "{query}");
        }
        assert_eq!(card_title("Retention & storage"), "Work History collection");
    }
    #[test] fn index_covers_every_category() {
        for category in super::super::settings_navigation::CATEGORIES {
            assert!(ENTRIES.iter().any(|entry| entry.category == category.id), "{}", category.id);
        }
    }
}
