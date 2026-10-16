//! Task-bound Git pane identities are separate from the singleton sidebar route.
pub(super) fn task_id(id: &str) -> Option<&str> {
    let (task, serial) = id.strip_prefix("git:")?.rsplit_once(':')?;
    (!task.is_empty() && serial.parse::<u64>().is_ok_and(|n| n > 0)).then_some(task)
}
pub(super) fn next_id(task: &str, mut retained: impl FnMut(&str) -> bool) -> String {
    (1_u64..).map(|n| format!("git:{task}:{n}")).find(|id| !retained(id)).unwrap()
}
#[cfg(test)]
mod tests {
    #[test]
    fn repeated_git_panes_keep_distinct_task_bound_identities() {
        let first = super::next_id("task", |_| false);
        let second = super::next_id("task", |id| id == first);
        assert_ne!(first, second);
        assert_eq!(super::task_id(&first), Some("task"));
        assert_eq!(super::task_id(&second), Some("task"));
        assert_eq!(super::task_id("route:repository"), None);
    }
}
