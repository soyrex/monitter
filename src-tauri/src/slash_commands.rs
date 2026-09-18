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

/// Parse a provider command. A slash followed by a filesystem-like path is
/// deliberately not a command; command names use the same conservative token
/// grammar accepted for ACP discovery.
pub(crate) fn parse(value: &str) -> Option<(&str, &str)> {
    let trimmed = value.trim();
    let body = trimmed.strip_prefix('/')?;
    let split = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..split];
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.'))
    {
        return None;
    }
    Some((name, body[split..].trim()))
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
    fn codex_catalog_is_provider_qualified_and_unique() {
        let commands = codex_catalog();
        assert!(commands.iter().all(|item| item.source == "codex" && item.provider == "codex"));
        let names = commands.iter().map(|item| item.name.as_str()).collect::<std::collections::HashSet<_>>();
        assert_eq!(names.len(), commands.len());
    }
}
