//! Stable, local display titles for otherwise untitled tasks.
use serde_json::Value;

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn at(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

/// Give untitled chats useful, deterministic identity without a model call.
pub(crate) fn title<'a>(task: &Value, messages: impl Iterator<Item = &'a Value>) -> String {
    let title = field(task, "title");
    if !title.trim().is_empty() && !title.trim().eq_ignore_ascii_case("new chat") {
        return title.to_owned();
    }
    if let Some(request) = messages
        .filter(|message| {
            field(message, "role") == "user"
                && field(message, "senderAgentId").is_empty()
                && !field(task, "id").is_empty()
                && field(message, "taskId") == field(task, "id")
        })
        .filter(|message| field(message, "text").split_whitespace().next().is_some())
        .min_by_key(|message| at(message, "createdAt"))
    {
        // Read just enough normalized characters to know whether to truncate.
        // Never copy or collect the complete request, which may contain logs.
        let text: String = field(request, "text")
            .split_whitespace()
            .enumerate()
            .flat_map(|(index, word)| {
                std::iter::once(' ')
                    .filter(move |_| index > 0)
                    .chain(word.chars())
            })
            .take(65)
            .collect();
        return if text.chars().count() > 64 {
            format!("{}…", text.chars().take(63).collect::<String>())
        } else {
            text
        };
    }
    let id = field(task, "id");
    let short: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if short.is_empty() {
        "New chat".to_owned()
    } else {
        format!("New chat · {short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ux_sweep_task_identity_rejects_foreign_and_missing_task_ids() {
        let task = json!({"id":"12345678-task","title":"New chat"});
        let messages = [
            json!({"role":"user","text":"Missing task identity","createdAt":1}),
            json!({"taskId":"other","role":"user","text":"Foreign task","createdAt":2}),
        ];
        assert_eq!(title(&task, messages.iter()), "New chat · 12345678");
        let matching =
            json!({"taskId":"12345678-task","role":"user","text":"Correct task","createdAt":3});
        assert_eq!(
            title(&task, messages.iter().chain(std::iter::once(&matching))),
            "Correct task"
        );
        assert_eq!(title(&json!({}), messages.iter()), "New chat");
    }

    #[test]
    fn ux_sweep_task_identity_bounds_large_content_and_normalizes_whitespace() {
        let task = json!({"id":"task","title":"New chat"});
        let huge = json!({"taskId":"task","role":"user","text":format!("{}  Inspect\n\t{}", " ".repeat(1_000_000), "界".repeat(1_000_000))});
        let result = title(&task, std::iter::once(&huge));
        assert_eq!(result.chars().count(), 64);
        assert!(result.starts_with("Inspect 界"));
        assert!(result.ends_with('…'));
        let small =
            json!({"taskId":"task","role":"user","text":"  Inspect\n\t the  Watch panel  "});
        assert_eq!(
            title(&task, std::iter::once(&small)),
            "Inspect the Watch panel"
        );
        let whitespace = json!({"taskId":"task","role":"user","text":" ".repeat(1_000_000)});
        assert_eq!(
            title(&task, std::iter::once(&whitespace)),
            "New chat · task"
        );
    }
}
