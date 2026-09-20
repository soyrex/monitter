use crate::model::SlashCommand;

pub(crate) fn codex_catalog() -> Vec<SlashCommand> {
    [
        ("compact", "Compact this Codex thread's context", None),
        (
            "review",
            "Start a native Codex review of current changes",
            Some("optional review instructions"),
        ),
        (
            "goal",
            "View, set, or clear this Codex thread's goal",
            Some("objective, clear, pause, or resume"),
        ),
        ("model", "Choose the model and reasoning effort", None),
        ("usage", "Show current Codex account usage", None),
        ("status", "Show this Codex chat's runtime settings", None),
        ("skills", "List skills available to this Codex workspace", None),
        ("mcp", "List MCP servers available to this Codex thread", None),
    ]
    .into_iter()
    .map(|(name, description, input_hint)| SlashCommand {
        name: name.into(),
        description: description.into(),
        input_hint: input_hint.map(str::to_owned),
        source: "codex".into(),
        provider: "codex".into(),
    })
    .collect()
}

/// ACP command names are provider-owned but need a small, unambiguous grammar
/// before Monitter stores or forwards them. Providers such as Gemini advertise
/// hierarchical commands (for example, `memory show`), so spaces separate
/// safe command segments rather than being rejected outright.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('/')
        && name.split_whitespace().count() <= 8
        && name.split_whitespace().all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.'))
        })
}

/// Parse a single-token command. Native commands use this fixed catalog.
pub(crate) fn parse(value: &str) -> Option<(&str, &str)> {
    let trimmed = value.trim();
    let body = trimmed.strip_prefix('/')?;
    let split = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..split];
    if !valid_name(name) {
        return None;
    }
    Some((name, body[split..].trim()))
}

/// Match the longest advertised ACP command at the start of a slash input.
/// The remainder is its argument text. Matching against the catalog avoids
/// treating Gemini's `/memory show` as `/memory` plus an argument.
pub(crate) fn parse_advertised<'a>(value: &'a str, names: impl Iterator<Item = &'a str>) -> Option<(&'a str, &'a str)> {
    let trimmed = value.trim();
    let body = trimmed.strip_prefix('/')?;
    let matched = names
        .filter(|name| valid_name(name))
        .filter(|name| {
            body.get(..name.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
                && body
                    .get(name.len()..)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        })
        .max_by_key(|name| name.len())?;
    Some((&body[..matched.len()], body[matched.len()..].trim()))
}

pub(crate) fn bounded_notice(value: impl Into<String>) -> String {
    const LIMIT: usize = 12_000;
    let value = value.into();
    if value.chars().count() <= LIMIT {
        value
    } else {
        format!("{}\n…", value.chars().take(LIMIT).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_and_arguments_without_paths() {
        assert_eq!(parse(" /review focus on auth "), Some(("review", "focus on auth")));
        assert_eq!(parse("/usage"), Some(("usage", "")));
        assert_eq!(parse("/tmp/file"), None);
        assert_eq!(parse("//usage"), None);
    }

    #[test]
    fn parses_the_longest_advertised_hierarchical_command() {
        let names = ["memory", "memory show", "memory refresh"];
        assert_eq!(
            parse_advertised("/memory show details", names.iter().copied()),
            Some(("memory show", "details"))
        );
        assert_eq!(parse_advertised("/memoryx", names.iter().copied()), None);
        assert!(valid_name("memory show"));
        assert!(!valid_name("memory/show"));
    }

    #[test]
    fn codex_catalog_is_provider_qualified_and_unique() {
        let commands = codex_catalog();
        assert!(commands.iter().all(|item| item.source == "codex" && item.provider == "codex"));
        let names = commands.iter().map(|item| item.name.as_str()).collect::<std::collections::HashSet<_>>();
        assert_eq!(names.len(), commands.len());
    }
}
