//! Device-local ownership of workspace views. This does not change backend workspaces.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_WORKSPACES: usize = 24;
pub const MAX_WINDOWS: usize = MAX_WORKSPACES + 1;
pub const PRIMARY_WINDOW_ID: &str = "main";
const MAX_WORKSPACE_ID_BYTES: usize = 256;
const MAX_CODEC_BYTES: usize = 32 * 1024;

/// Safe, stable device-local ID. Never interpreted as a filesystem path.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WindowId(String);
impl WindowId {
    pub fn main() -> Self {
        Self(PRIMARY_WINDOW_ID.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn detached(number: u64) -> Self {
        Self(format!("workspace-window-{number}"))
    }
    fn number(&self) -> Option<u64> {
        self.0.strip_prefix("workspace-window-")?.parse().ok()
    }
}
impl TryFrom<String> for WindowId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == PRIMARY_WINDOW_ID {
            return Ok(Self(value));
        }
        let number = value
            .strip_prefix("workspace-window-")
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .ok_or("Invalid window ID")?;
        if value != format!("workspace-window-{number}") {
            return Err("Invalid window ID".into());
        }
        Ok(Self(value))
    }
}
impl From<WindowId> for String {
    fn from(value: WindowId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWindow {
    window_id: WindowId,
    workspace_ids: Vec<String>,
    active_workspace_id: Option<String>,
}
impl WorkspaceWindow {
    pub fn window_id(&self) -> &WindowId {
        &self.window_id
    }
    pub fn workspace_ids(&self) -> &[String] {
        &self.workspace_ids
    }
    pub fn active_workspace_id(&self) -> Option<&str> {
        self.active_workspace_id.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "WireState", into = "WireState")]
pub struct WorkspaceWindowState {
    primary_window_id: WindowId,
    windows: Vec<WorkspaceWindow>,
    next_window_number: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireState {
    version: u32,
    primary_window_id: WindowId,
    windows: Vec<WorkspaceWindow>,
    next_window_number: u64,
}
impl From<WorkspaceWindowState> for WireState {
    fn from(s: WorkspaceWindowState) -> Self {
        Self {
            version: 1,
            primary_window_id: s.primary_window_id,
            windows: s.windows,
            next_window_number: s.next_window_number,
        }
    }
}
impl TryFrom<WireState> for WorkspaceWindowState {
    type Error = String;
    fn try_from(s: WireState) -> Result<Self, Self::Error> {
        if s.version != 1
            || s.primary_window_id != WindowId::main()
            || s.windows.is_empty()
            || s.windows.len() > MAX_WINDOWS
            || s.next_window_number == 0
        {
            return Err("Invalid workspace window state".into());
        }
        let mut window_ids = HashSet::new();
        let mut workspace_ids = HashSet::new();
        for w in &s.windows {
            if !window_ids.insert(w.window_id.clone())
                || w.window_id
                    .number()
                    .is_some_and(|n| n >= s.next_window_number)
            {
                return Err("Invalid or duplicate window ID".into());
            }
            for id in &w.workspace_ids {
                if !valid_workspace_id(id)
                    || !workspace_ids.insert(id.clone())
                    || workspace_ids.len() > MAX_WORKSPACES
                {
                    return Err("Invalid or duplicate workspace assignment".into());
                }
            }
            if w.active_workspace_id
                .as_ref()
                .is_some_and(|id| !w.workspace_ids.contains(id))
                || (!w.workspace_ids.is_empty() && w.active_workspace_id.is_none())
            {
                return Err("Invalid active workspace".into());
            }
        }
        if s.windows[0].window_id != s.primary_window_id {
            return Err("Primary window must be first".into());
        }
        Ok(Self {
            primary_window_id: s.primary_window_id,
            windows: s.windows,
            next_window_number: s.next_window_number,
        })
    }
}
fn valid_workspace_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_WORKSPACE_ID_BYTES && !id.chars().any(char::is_control)
}
impl Default for WorkspaceWindowState {
    fn default() -> Self {
        Self {
            primary_window_id: WindowId::main(),
            windows: vec![WorkspaceWindow {
                window_id: WindowId::main(),
                workspace_ids: vec![],
                active_workspace_id: None,
            }],
            next_window_number: 1,
        }
    }
}
impl WorkspaceWindowState {
    pub fn new(ids: &[String]) -> Result<Self, String> {
        let mut s = Self::default();
        s.reconcile(ids)?;
        Ok(s)
    }
    pub fn primary_window_id(&self) -> &WindowId {
        &self.primary_window_id
    }
    pub fn windows(&self) -> &[WorkspaceWindow] {
        &self.windows
    }
    pub fn window(&self, id: &WindowId) -> Option<&WorkspaceWindow> {
        self.windows.iter().find(|w| &w.window_id == id)
    }
    pub fn window_for_workspace(&self, id: &str) -> Option<&WindowId> {
        self.windows
            .iter()
            .find(|w| w.workspace_ids.iter().any(|w| w == id))
            .map(|w| &w.window_id)
    }
    pub fn set_active_workspace(
        &mut self,
        window: &WindowId,
        workspace: &str,
    ) -> Result<(), String> {
        let w = self
            .windows
            .iter_mut()
            .find(|w| &w.window_id == window)
            .ok_or("Unknown window")?;
        if !w.workspace_ids.iter().any(|id| id == workspace) {
            return Err("Workspace is not assigned to this window".into());
        }
        w.active_workspace_id = Some(workspace.into());
        Ok(())
    }
    /// Target index is the final index, clamped to its assignment count.
    pub fn transfer_workspace(
        &mut self,
        workspace: &str,
        target: &WindowId,
        index: usize,
    ) -> Result<(), String> {
        let source = self
            .windows
            .iter()
            .position(|w| w.workspace_ids.iter().any(|id| id == workspace))
            .ok_or("Unknown workspace")?;
        let destination = self
            .windows
            .iter()
            .position(|w| &w.window_id == target)
            .ok_or("Unknown target window")?;
        let position = self.windows[source]
            .workspace_ids
            .iter()
            .position(|id| id == workspace)
            .unwrap();
        let id = self.windows[source].workspace_ids.remove(position);
        if self.windows[source].active_workspace_id.as_deref() == Some(workspace) {
            self.windows[source].active_workspace_id = self.windows[source]
                .workspace_ids
                .get(position.min(self.windows[source].workspace_ids.len().saturating_sub(1)))
                .cloned();
        }
        let destination = &mut self.windows[destination];
        destination
            .workspace_ids
            .insert(index.min(destination.workspace_ids.len()), id.clone());
        destination.active_workspace_id = Some(id);
        Ok(())
    }
    /// Restore an exact device-local host ID before assigning its workspaces.
    /// Existing hosts (including main) are unchanged; newly allocated IDs will
    /// always be above every restored host number.
    pub fn ensure_window(&mut self, id: &WindowId) -> Result<(), String> {
        if self.window(id).is_some() {
            return Ok(());
        }
        let number = id
            .number()
            .filter(|number| *number > 0)
            .ok_or("Invalid detached window ID")?;
        if self.windows.len() >= MAX_WINDOWS {
            return Err("Window limit reached".into());
        }
        let next = number.checked_add(1).ok_or("Window ID limit reached")?;
        self.windows.push(WorkspaceWindow {
            window_id: id.clone(),
            workspace_ids: vec![],
            active_workspace_id: None,
        });
        self.next_window_number = self.next_window_number.max(next);
        Ok(())
    }

    pub fn detach_workspace(&mut self, workspace: &str) -> Result<WindowId, String> {
        if self.window_for_workspace(workspace).is_none() {
            return Err("Unknown workspace".into());
        }
        if self.windows.len() >= MAX_WINDOWS {
            return Err("Window limit reached".into());
        }
        let next = self
            .next_window_number
            .checked_add(1)
            .ok_or("Window ID limit reached")?;
        let id = WindowId::detached(self.next_window_number);
        self.next_window_number = next;
        self.windows.push(WorkspaceWindow {
            window_id: id.clone(),
            workspace_ids: vec![],
            active_workspace_id: None,
        });
        self.transfer_workspace(workspace, &id, 0)?;
        Ok(id)
    }
    pub fn reattach_workspace(&mut self, workspace: &str) -> Result<(), String> {
        self.transfer_workspace(workspace, &self.primary_window_id.clone(), usize::MAX)
    }
    /// Closing a secondary window appends all its workspaces to the primary.
    pub fn close_window(&mut self, id: &WindowId) -> Result<Vec<String>, String> {
        if id == &self.primary_window_id {
            return Err("The primary window cannot be closed through assignment state".into());
        }
        let index = self
            .windows
            .iter()
            .position(|w| &w.window_id == id)
            .ok_or("Unknown window")?;
        let closed = self.windows.remove(index);
        let main = &mut self.windows[0];
        if main.active_workspace_id.is_none() {
            main.active_workspace_id = closed.active_workspace_id.clone();
        }
        main.workspace_ids
            .extend(closed.workspace_ids.iter().cloned());
        Ok(closed.workspace_ids)
    }
    /// Preserve each valid window's order; place newly discovered workspaces in main.
    /// Empty secondary assignments are removed after deleted workspaces disappear.
    pub fn reconcile(&mut self, known: &[String]) -> Result<(), String> {
        if known.len() > MAX_WORKSPACES || known.iter().any(|id| !valid_workspace_id(id)) {
            return Err("Invalid workspace list or workspace limit reached".into());
        }
        let known_set: HashSet<&str> = known.iter().map(String::as_str).collect();
        if known_set.len() != known.len() {
            return Err("Duplicate workspace IDs".into());
        }
        let mut assigned = HashSet::new();
        for w in &mut self.windows {
            w.workspace_ids
                .retain(|id| known_set.contains(id.as_str()) && assigned.insert(id.clone()));
            if !w
                .active_workspace_id
                .as_ref()
                .is_some_and(|id| w.workspace_ids.contains(id))
            {
                w.active_workspace_id = w.workspace_ids.first().cloned();
            }
        }
        self.windows
            .retain(|w| w.window_id == self.primary_window_id || !w.workspace_ids.is_empty());
        let main = &mut self.windows[0];
        for id in known {
            if assigned.insert(id.clone()) {
                main.workspace_ids.push(id.clone());
            }
        }
        if main.active_workspace_id.is_none() {
            main.active_workspace_id = main.workspace_ids.first().cloned();
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_CODEC_BYTES {
            return Err("Workspace window state exceeds size limit".into());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_CODEC_BYTES {
            return Err("Workspace window state exceeds size limit".into());
        }
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn transfers_preserve_single_ownership_and_order() {
        let mut s = WorkspaceWindowState::new(&ids(&["a", "b", "c"])).unwrap();
        let detached = s.detach_workspace("b").unwrap();
        s.transfer_workspace("c", &detached, 0).unwrap();
        assert_eq!(
            s.window(&detached).unwrap().workspace_ids(),
            ids(&["c", "b"])
        );
        assert_eq!(s.windows()[0].workspace_ids(), ids(&["a"]));
        s.transfer_workspace("b", &detached, 0).unwrap();
        assert_eq!(
            s.window(&detached).unwrap().workspace_ids(),
            ids(&["b", "c"])
        );
        assert_eq!(s.window_for_workspace("b"), Some(&detached));
        assert!(
            s.transfer_workspace("a", &WindowId::detached(999), 0)
                .is_err()
        );
        assert_eq!(s.windows()[0].workspace_ids(), ids(&["a"]));
    }
    #[test]
    fn reconcile_retains_valid_layout_and_closes_deleted_only_windows() {
        let mut s = WorkspaceWindowState::new(&ids(&["a", "b", "c"])).unwrap();
        let b = s.detach_workspace("b").unwrap();
        let c = s.detach_workspace("c").unwrap();
        s.reconcile(&ids(&["c", "a", "new"])).unwrap();
        assert!(s.window(&b).is_none());
        assert_eq!(s.window(&c).unwrap().workspace_ids(), ids(&["c"]));
        assert_eq!(s.windows()[0].workspace_ids(), ids(&["a", "new"]));
    }
    #[test]
    fn closing_last_workspace_returns_it_to_usable_empty_main() {
        let mut s = WorkspaceWindowState::new(&ids(&["a"])).unwrap();
        let window = s.detach_workspace("a").unwrap();
        assert!(s.windows()[0].workspace_ids().is_empty());
        assert_eq!(s.close_window(&window).unwrap(), ids(&["a"]));
        assert_eq!(s.windows()[0].active_workspace_id(), Some("a"));
        assert!(s.close_window(&WindowId::main()).is_err());
        let second = s.detach_workspace("a").unwrap();
        assert_ne!(window, second);
    }
    #[test]
    fn ensuring_persisted_window_ids_is_idempotent_bounded_and_monotonic() {
        let mut s = WorkspaceWindowState::new(&ids(&["a"])).unwrap();
        let restored = WindowId::try_from("workspace-window-42".to_owned()).unwrap();
        s.ensure_window(&WindowId::main()).unwrap();
        s.ensure_window(&restored).unwrap();
        s.ensure_window(&restored).unwrap();
        assert_eq!(s.windows().len(), 2);
        assert!(s.window(&restored).unwrap().workspace_ids().is_empty());
        assert_eq!(
            s.detach_workspace("a").unwrap().as_str(),
            "workspace-window-43"
        );
        let before = s.clone();
        assert!(s.ensure_window(&WindowId::detached(u64::MAX)).is_err());
        assert_eq!(s, before);
        for number in 100..(100 + MAX_WINDOWS as u64 - 3) {
            s.ensure_window(&WindowId::detached(number)).unwrap();
        }
        assert_eq!(s.windows().len(), MAX_WINDOWS);
        let full = s.clone();
        assert!(s.ensure_window(&WindowId::detached(500)).is_err());
        assert_eq!(s, full);
        s.ensure_window(&restored).unwrap();
        assert_eq!(
            WorkspaceWindowState::decode(&s.encode().unwrap()).unwrap(),
            s
        );
    }

    #[test]
    fn codec_rejects_duplicates_paths_and_oversized_payload() {
        let mut s = WorkspaceWindowState::new(&ids(&["a"])).unwrap();
        s.detach_workspace("a").unwrap();
        assert_eq!(
            WorkspaceWindowState::decode(&s.encode().unwrap()).unwrap(),
            s
        );
        let mut value = serde_json::to_value(&s).unwrap();
        value["windows"][0]["workspaceIds"] = serde_json::json!(["a"]);
        value["windows"][0]["activeWorkspaceId"] = serde_json::json!("a");
        assert!(WorkspaceWindowState::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(WindowId::try_from("../evil".to_owned()).is_err());
        assert!(WorkspaceWindowState::decode(&vec![b' '; MAX_CODEC_BYTES + 1]).is_err());
    }
}
