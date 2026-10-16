//! Presentation-only exchange runs. Every source row belongs to exactly one
//! virtual item; no hidden/zero-height siblings or synthetic lifecycle records.
use super::ConversationItem;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExchangeContext {
    pub task_id: String,
    pub peers: HashMap<String, String>,
    pub provenance: HashMap<String, (String, String)>,
}
impl ExchangeContext {
    pub fn from_snapshot(snapshot: &Value, task_id: &str) -> Self {
        let peers = snapshot["collaborations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let (agent, task) = if row["fromTaskId"].as_str() == Some(task_id) {
                    ("toAgentId", "toTaskId")
                } else if row["toTaskId"].as_str() == Some(task_id) {
                    ("fromAgentId", "fromTaskId")
                } else {
                    return None;
                };
                let agent = row[agent].as_str().filter(|v| !v.is_empty())?;
                let task = row[task].as_str().filter(|task| !task.is_empty())?;
                Some((
                    row["id"].as_str()?.to_owned(),
                    format!("agent:{agent}:task:{task}"),
                ))
            })
            .collect();
        let provenance = snapshot["collaborations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let (agent, task) = if row["fromTaskId"].as_str() == Some(task_id) {
                    ("toAgentId", "toTaskId")
                } else if row["toTaskId"].as_str() == Some(task_id) {
                    ("fromAgentId", "fromTaskId")
                } else {
                    return None;
                };
                Some((
                    row["id"].as_str()?.to_owned(),
                    (
                        row[agent].as_str()?.to_owned(),
                        row[task].as_str()?.to_owned(),
                    ),
                ))
            })
            .collect();
        Self {
            task_id: task_id.into(),
            peers,
            provenance,
        }
    }
    fn identity(&self, item: &ConversationItem) -> Option<(String, String)> {
        let (value, collaboration) = match item {
            ConversationItem::Message(row)
                if row["role"] == "user"
                    || (row["role"] == "system"
                        && row["collaborationId"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty())) =>
            {
                let sender = row["senderAgentId"].as_str().filter(|v| !v.is_empty());
                let collaboration = row["collaborationId"].as_str().unwrap_or_default();
                if let Some(peer) = self.peers.get(collaboration) {
                    let explicit_task = row["senderTaskId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .or_else(|| row["fromTaskId"].as_str().filter(|id| !id.is_empty()));
                    let (expected_agent, expected_task) = self.provenance.get(collaboration)?;
                    if sender.is_some_and(|agent| agent != expected_agent)
                        || explicit_task.is_some_and(|task| task != expected_task)
                    {
                        return None;
                    }
                    return Some((peer.clone(), format!("collaboration:{collaboration}")));
                }
                // A sender is evidence of identity; an unresolved collaboration ID is not.
                if !collaboration.is_empty() {
                    return None;
                }
                let sender = sender?;
                let source_task = row["senderTaskId"]
                    .as_str()
                    .or_else(|| row["fromTaskId"].as_str())
                    .filter(|task| !task.is_empty())?;
                return Some((
                    format!("agent:{sender}:task:{source_task}"),
                    format!("message:{}", row["id"].as_str()?),
                ));
            }
            ConversationItem::Activity(row) if row["kind"] == "collaboration" => {
                (row, row["detail"].as_str()?)
            }
            _ => return None,
        };
        let _ = value;
        self.peers
            .get(collaboration)
            .map(|peer| (peer.clone(), format!("collaboration:{collaboration}")))
    }
}
/// Activity turns remain intact; an approval is always a cluster boundary.
fn activity_members(item: &ConversationItem) -> Option<&[std::rc::Rc<Value>]> {
    match item {
        ConversationItem::ToolGroup(values) | ConversationItem::ReasoningGroup(values) => {
            Some(values)
        }
        ConversationItem::ProcessGroup {
            values, approvals, ..
        } if approvals.is_empty() => Some(values),
        _ => None,
    }
}
pub fn group(items: Vec<ConversationItem>, context: &ExchangeContext) -> Vec<ConversationItem> {
    group_with_boundaries(items, context, &HashSet::new())
}
pub fn group_with_boundaries(
    items: Vec<ConversationItem>,
    context: &ExchangeContext,
    boundaries: &HashSet<String>,
) -> Vec<ConversationItem> {
    let mut result = Vec::with_capacity(items.len());
    let mut pending: Vec<ConversationItem> = Vec::new();
    let mut peers = HashSet::new();
    let mut first = 0;
    let mut day = None;
    let mut exchanges = HashSet::new();
    let flush = |pending: &mut Vec<ConversationItem>,
                 exchanges: &mut HashSet<String>,
                 peers: &mut HashSet<String>,
                 result: &mut Vec<ConversationItem>| {
        if pending.len() > 1 && !exchanges.is_empty() {
            result.push(ConversationItem::ExchangeGroup {
                members: std::mem::take(pending),
                peer_key: if peers.len() == 1 {
                    peers.iter().next().unwrap().clone()
                } else {
                    "multiple-peers".into()
                },
                exchange_count: exchanges.len(),
            });
        } else {
            result.append(pending);
        }
        exchanges.clear();
        peers.clear();
    };
    for item in items {
        if !pending.is_empty() && boundaries.iter().any(|key| item.contains_source(key)) {
            flush(&mut pending, &mut exchanges, &mut peers, &mut result);
        }
        let at = item.at();
        let current_day = crate::timestamps::local_datetime(at).map(|time| time.date_naive());
        let correct_task = match &item {
            ConversationItem::Message(row) | ConversationItem::Activity(row) => row["taskId"]
                .as_str()
                .is_none_or(|task| task == context.task_id),
            _ => true,
        };
        let exchange = correct_task.then(|| context.identity(&item)).flatten();
        let tool = activity_members(&item).filter(|rows| {
            !rows.is_empty()
                && rows.iter().all(|row| {
                    let end = row["createdAt"].as_i64().unwrap_or(0);
                    row["taskId"].as_str() == Some(context.task_id.as_str())
                        && matches!(
                            row["kind"].as_str(),
                            Some("tool" | "reasoning" | "subagent")
                        )
                        && end >= at
                        && end - at <= 60_000
                        && crate::timestamps::local_datetime(end).map(|time| time.date_naive())
                            == current_day
                })
        });
        if at > 0 && current_day.is_some() && (exchange.is_some() || tool.is_some()) {
            let end = tool
                .and_then(|rows| {
                    rows.iter()
                        .filter_map(|row| row["createdAt"].as_i64())
                        .max()
                })
                .unwrap_or(at);
            if !pending.is_empty() && (at < first || end - first > 60_000 || current_day != day) {
                flush(&mut pending, &mut exchanges, &mut peers, &mut result);
            }
            if pending.is_empty() {
                first = at;
                day = current_day;
            }
            if let Some((identity, exchange)) = exchange {
                peers.insert(identity);
                exchanges.insert(exchange);
            }
            pending.push(item);
        } else {
            flush(&mut pending, &mut exchanges, &mut peers, &mut result);
            result.push(item);
        }
    }
    flush(&mut pending, &mut exchanges, &mut peers, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::rc::Rc;
    fn context() -> ExchangeContext {
        ExchangeContext::from_snapshot(
            &json!({"collaborations":[
        {"id":"a","fromTaskId":"here","toTaskId":"peer","toAgentId":"codex"},
        {"id":"b","fromTaskId":"peer","fromAgentId":"codex","toTaskId":"here"},
        {"id":"other","fromTaskId":"here","toTaskId":"different","toAgentId":"codex"}]}),
            "here",
        )
    }
    fn message(id: &str, collab: &str, at: i64) -> ConversationItem {
        ConversationItem::Message(Rc::new(
            json!({"id":id,"taskId":"here","role":"user","collaborationId":collab,"createdAt":at,"text":id,"attachments":[{"name":"Recorded.txt"}]}),
        ))
    }
    fn tools(id: &str, at: i64) -> ConversationItem {
        ConversationItem::ToolGroup(vec![Rc::new(
            json!({"id":id,"taskId":"here","kind":"tool","title":"commandExecution","createdAt":at,"detail":"{\"type\":\"commandExecution\",\"command\":\"pwd\"}"}),
        )])
    }
    #[::core::prelude::v1::test]
    fn exchange_group_activity_cluster_preserves_interleaved_turns_and_distinct_same_profile_peers()
    {
        let base = 1_760_000_000_000;
        let queued = ConversationItem::Activity(Rc::new(
            json!({"id":"queued","taskId":"here","kind":"collaboration","detail":"a","createdAt":base+1000}),
        ));
        let original = vec![
            tools("shell", base),
            queued,
            tools("send-and-plan", base + 2000),
            message("one", "a", base + 3000),
            tools("wait", base + 4000),
            message("other-peer", "other", base + 5000),
            tools("last", base + 6000),
        ];
        let projected = group(original.clone(), &context());
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].stable_key(), "tool-group:shell");
        assert!(projected[0].contains_source("activity:wait"));
        assert_eq!(
            projected[0]
                .leaves()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            original
        );
        assert!(
            matches!(&projected[0],ConversationItem::ExchangeGroup{peer_key,exchange_count:2,..} if peer_key=="multiple-peers")
        );
        assert_eq!(projected[0].row_count(), 7);
    }
    #[::core::prelude::v1::test]
    fn exchange_group_activity_cluster_requires_an_exchange_and_respects_nested_anchors_and_task_boundaries()
     {
        let base = 1_760_000_000_000;
        let original = vec![tools("one", base), tools("two", base + 1)];
        assert_eq!(group(original.clone(), &context()), original);
        let rows = vec![
            tools("one", base),
            message("delivery", "a", base + 1),
            tools("two", base + 2),
        ];
        let grouped =
            group_with_boundaries(rows, &context(), &HashSet::from(["activity:two".into()]));
        assert_eq!(grouped.len(), 2);
        let wrong = ConversationItem::ToolGroup(vec![Rc::new(
            json!({"id":"foreign","taskId":"elsewhere","kind":"tool","createdAt":base+2}),
        )]);
        assert_eq!(
            group(
                vec![
                    message("one", "a", base),
                    wrong,
                    message("two", "b", base + 3)
                ],
                &context()
            )
            .len(),
            3
        );
    }
    #[::core::prelude::v1::test]
    fn exchange_group_activity_cluster_never_absorbs_an_approval_or_unknown_route() {
        let base = 1_760_000_000_000;
        let approval = super::super::ApprovalRow {
            id: "approval".into(),
            task_id: "here".into(),
            status: "approved".into(),
            rule_id: None,
            tool: String::new(),
            summary: String::new(),
            created_at: base + 1,
            resolved_at: None,
        };
        for barrier in [
            ConversationItem::Approval(approval),
            message("unknown", "missing", base + 1),
        ] {
            assert_eq!(
                group(
                    vec![
                        message("one", "a", base),
                        barrier,
                        message("two", "b", base + 2)
                    ],
                    &context()
                )
                .len(),
                3
            );
        }
        let wrong_task = ConversationItem::Message(Rc::new(
            json!({"id":"foreign-message","taskId":"elsewhere","role":"user","collaborationId":"a","createdAt":base+1}),
        ));
        assert_eq!(
            group(
                vec![
                    message("one", "a", base),
                    wrong_task,
                    message("two", "b", base + 2)
                ],
                &context()
            )
            .len(),
            3
        );
        let original = vec![tools("early", base), message("later", "a", base + 60_001)];
        assert_eq!(group(original.clone(), &context()), original);
    }
    #[::core::prelude::v1::test]
    fn exchange_group_activity_cluster_deduplicates_tool_lifecycle_identifiers() {
        let row = |id: &str| {
            Rc::new(
                json!({"id":id,"taskId":"here","kind":"tool","createdAt":1,"detail":"{\"type\":\"mcpToolCall\",\"id\":\"same-call\",\"server\":\"monitter\",\"tool\":\"send_message\"}"}),
            )
        };
        let tools = ConversationItem::ToolGroup(vec![row("started"), row("completed")]);
        assert_eq!(tools.activity_tool_keys().len(), 1);
        let failed = ConversationItem::ToolGroup(vec![Rc::new(
            json!({"id":"failed","kind":"tool","detail":"{\"status\":\"failed\",\"id\":\"same-call\"}"}),
        )]);
        assert_eq!(failed.activity_failed_tool_keys().len(), 1);
        assert_eq!(tools.row_count(), 2, "raw lifecycle details are retained");
    }
    #[::core::prelude::v1::test]
    fn exchange_group_conflicting_sender_provenance_is_a_barrier() {
        let base = 1_760_000_000_000;
        for (agent, task) in [("someone-else", "peer"), ("codex", "different")] {
            let conflict = ConversationItem::Message(Rc::new(
                json!({"id":"conflict","taskId":"here","role":"user","collaborationId":"a","senderAgentId":agent,"senderTaskId":task,"createdAt":base+1}),
            ));
            assert_eq!(
                group(
                    vec![
                        message("one", "a", base),
                        conflict,
                        message("two", "b", base + 2)
                    ],
                    &context()
                )
                .len(),
                3
            );
        }
    }
    #[::core::prelude::v1::test]
    fn exchange_group_is_bounded_from_first_member_and_preserves_every_source() {
        let base = 1_760_000_000_000;
        let original = vec![
            message("one", "a", base),
            message("two", "b", base + 40_000),
            message("three", "b", base + 70_000),
        ];
        let grouped = group(original.clone(), &context());
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].stable_key(), original[0].stable_key());
        assert_eq!(grouped[0].row_count(), 2);
        assert!(grouped[0].contains_source("message:two"));
        assert_eq!(
            grouped
                .iter()
                .flat_map(ConversationItem::leaves)
                .cloned()
                .collect::<Vec<_>>(),
            original
        );
        if let ConversationItem::ExchangeGroup { exchange_count, .. } = &grouped[0] {
            assert_eq!(*exchange_count, 2);
        } else {
            panic!("adjacent exchanges grouped");
        }
    }
    #[::core::prelude::v1::test]
    fn exchange_group_human_assistant_unknown_peer_other_task_and_local_midnight_are_barriers() {
        let base = 1_760_000_000_000;
        for barrier in [
            message("unknown", "missing", base + 1),
            ConversationItem::Message(Rc::new(
                json!({"id":"human","role":"user","createdAt":base+1}),
            )),
            ConversationItem::Message(Rc::new(
                json!({"id":"assistant","role":"assistant","createdAt":base+1}),
            )),
            ConversationItem::Activity(Rc::new(
                json!({"id":"turn","kind":"turn","createdAt":base+1}),
            )),
            ConversationItem::Message(Rc::new(
                json!({"id":"ambiguous","role":"user","senderAgentId":"codex","createdAt":base+1}),
            )),
        ] {
            let projected = group(
                vec![
                    message("one", "a", base),
                    barrier,
                    message("two", "b", base + 2),
                ],
                &context(),
            );
            assert_eq!(projected.len(), 3);
        }
        let before = crate::timestamps::local_datetime(base)
            .unwrap()
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            group(
                vec![
                    message("one", "a", before),
                    message("two", "b", before + 2_000)
                ],
                &context()
            )
            .len(),
            2
        );
    }
    #[::core::prelude::v1::test]
    fn exchange_group_lifecycle_rows_do_not_inflate_exchange_count() {
        let base = 1_760_000_000_000;
        let event = ConversationItem::Activity(Rc::new(
            json!({"id":"queued","kind":"collaboration","detail":"a","createdAt":base}),
        ));
        let grouped = group(vec![event, message("delivery", "a", base + 1)], &context());
        assert!(matches!(
            &grouped[0],
            ConversationItem::ExchangeGroup {
                exchange_count: 1,
                ..
            }
        ));
    }
}
