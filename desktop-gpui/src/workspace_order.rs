//! Ordering and removal affect saved layouts only, never agent lifecycles.
use monitter_lib::experimental_desktop::NativeWorkspaceSet;

pub fn reorder(set: &mut NativeWorkspaceSet, source: &str, target: &str) -> bool {
    let Some(from) = set.items.iter().position(|item| item.id == source) else { return false; };
    let Some(to) = set.items.iter().position(|item| item.id == target) else { return false; };
    if from == to { return false; }
    let item = set.items.remove(from);
    set.items.insert(to, item);
    true
}

pub fn remove(set: &mut NativeWorkspaceSet, id: &str) -> bool {
    if set.items.len() <= 1 { return false; }
    let Some(index) = set.items.iter().position(|item| item.id == id) else { return false; };
    set.items.remove(index);
    if set.active_id == id {
        set.active_id = set.items[index.min(set.items.len() - 1)].id.clone();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use monitter_lib::experimental_desktop::NativeWorkspace;
    fn fixture() -> NativeWorkspaceSet {
        NativeWorkspaceSet { active_id: "b".into(), items: ["a", "b", "c"].into_iter().map(|id| NativeWorkspace {
            id: id.into(), name: id.into(), icon: "folder".into(), color: "#38BFD2".into(),
            layout: Some(serde_json::json!({"pane":id})),
        }).collect() }
    }
    #[test]
    fn moves_both_directions_preserving_selection_and_layouts() {
        let mut set = fixture(); let original = set.clone();
        assert!(reorder(&mut set, "a", "c"));
        assert_eq!(set.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        assert_eq!(set.active_id, "b");
        assert_eq!(set.items[2], original.items[0]);
        assert!(reorder(&mut set, "a", "b")); assert_eq!(set, original);
        assert!(!reorder(&mut set, "a", "a")); assert!(!reorder(&mut set, "missing", "a"));
        assert!(!reorder(&mut set, "a", "missing")); assert_eq!(set, original);
    }
    #[test]
    fn removal_selects_neighbors_and_protects_last_workspace() {
        let mut set = fixture(); let c = set.items[2].clone();
        assert!(remove(&mut set, "b")); assert_eq!(set.active_id, "c"); assert_eq!(set.items[1], c);
        assert!(remove(&mut set, "c")); assert_eq!(set.active_id, "a");
        let last = set.clone(); assert!(!remove(&mut set, "a")); assert_eq!(set, last);
        let mut set = fixture(); assert!(remove(&mut set, "a")); assert_eq!(set.active_id, "b");
        let previous = set.clone(); assert!(!remove(&mut set, "missing")); assert_eq!(set, previous);
    }
}
