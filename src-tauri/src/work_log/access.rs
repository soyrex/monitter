use serde::{Deserialize, Serialize};
// Owner-issued, backend-scoped grants for agent Work Log operations.
// A reference, title, project label or parentTaskId is never a capability.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    ReadLog,
    ReadContext,
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallerIdentity {
    /// Supplied by the authenticated backend turn, never tool arguments.
    pub installation_id: String,
    pub profile_id: String,
    pub task_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceIdentity {
    /// Resolved from persisted source state, never from a title or reference text.
    pub installation_id: String,
    pub profile_id: String,
    pub task_id: String,
    pub project_id: Option<String>,
    /// Persisted lineage root/link, including fork and handoff links.
    pub lineage_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GrantScope {
    Project(String),
    Lineage(String),
    Task(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerGrant {
    pub installation_id: String,
    pub profile_id: String,
    pub caller_task_id: String,
    pub operation: Operation,
    pub scope: GrantScope,
    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccessError {
    Denied,
}

/// Call on every query/context page, including after an asynchronous read.
/// `grants` must be freshly loaded owner-private records for this invocation.
pub fn authorize(
    caller: &CallerIdentity,
    source: &SourceIdentity,
    operation: Operation,
    grants: &[OwnerGrant],
) -> Result<(), AccessError> {
    if caller.installation_id.is_empty()
        || caller.profile_id.is_empty()
        || caller.task_id.is_empty()
        || caller.run_id.is_empty()
        || source.task_id.is_empty()
        || caller.installation_id != source.installation_id
        || caller.profile_id != source.profile_id
    {
        return Err(AccessError::Denied);
    }
    if grants.iter().any(|grant| {
        !grant.revoked
            && grant.operation == operation
            && grant.installation_id == caller.installation_id
            && grant.profile_id == caller.profile_id
            && grant.caller_task_id == caller.task_id
            && match &grant.scope {
                GrantScope::Project(id) => {
                    !id.is_empty() && source.project_id.as_deref() == Some(id)
                }
                GrantScope::Lineage(id) => {
                    !id.is_empty() && source.lineage_id.as_deref() == Some(id)
                }
                GrantScope::Task(id) => !id.is_empty() && source.task_id == *id,
            }
    }) {
        Ok(())
    } else {
        Err(AccessError::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_scopes_operations_and_revocation() {
        let caller = CallerIdentity {
            installation_id: "install".into(),
            profile_id: "private".into(),
            task_id: "reader".into(),
            run_id: "run".into(),
        };
        let source = SourceIdentity {
            installation_id: "install".into(),
            profile_id: "private".into(),
            task_id: "source".into(),
            project_id: Some("project".into()),
            lineage_id: Some("lineage".into()),
        };
        let mut grant = OwnerGrant {
            installation_id: "install".into(),
            profile_id: "private".into(),
            caller_task_id: "reader".into(),
            operation: Operation::ReadLog,
            scope: GrantScope::Project("project".into()),
            revoked: false,
        };
        assert_eq!(
            authorize(&caller, &source, Operation::ReadLog, &[grant.clone()]),
            Ok(())
        );
        assert_eq!(
            authorize(&caller, &source, Operation::ReadContext, &[grant.clone()]),
            Err(AccessError::Denied)
        );
        assert_eq!(
            authorize(&caller, &source, Operation::Report, &[grant.clone()]),
            Err(AccessError::Denied)
        );
        grant.revoked = true;
        assert_eq!(
            authorize(&caller, &source, Operation::ReadLog, &[grant]),
            Err(AccessError::Denied)
        );
    }

    #[test]
    fn lineage_is_explicit_and_profile_isolated() {
        let caller = CallerIdentity {
            installation_id: "i".into(),
            profile_id: "p".into(),
            task_id: "child".into(),
            run_id: "r".into(),
        };
        let mut source = SourceIdentity {
            installation_id: "i".into(),
            profile_id: "p".into(),
            task_id: "parent".into(),
            project_id: None,
            lineage_id: Some("tree".into()),
        };
        let grant = OwnerGrant {
            installation_id: "i".into(),
            profile_id: "p".into(),
            caller_task_id: "child".into(),
            operation: Operation::ReadContext,
            scope: GrantScope::Lineage("tree".into()),
            revoked: false,
        };
        assert_eq!(
            authorize(&caller, &source, Operation::ReadContext, &[grant.clone()]),
            Ok(())
        );
        source.lineage_id = None;
        assert_eq!(
            authorize(&caller, &source, Operation::ReadContext, &[grant.clone()]),
            Err(AccessError::Denied)
        );
        source.lineage_id = Some("tree".into());
        source.profile_id = "other".into();
        assert_eq!(
            authorize(&caller, &source, Operation::ReadContext, &[grant]),
            Err(AccessError::Denied)
        );
    }
}
