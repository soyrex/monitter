//! Bridge real Neovim windows to retained Monitter dock panes.
//! Neovim owns buffers; Monitter owns physical placement and keyboard focus.
use crate::workspace_windows::DesktopContextExt as _;
use crate::*;
use gpui_kit::component::{
    Placement,
    dock::{DockPlacement, InsertTarget},
};

pub fn pane_id(session: &str, win: i64) -> String {
    format!("neovim:{session}:{win}")
}
pub fn parse_id(id: &str) -> Option<(&str, i64)> {
    let (session, win) = id.strip_prefix("neovim:")?.rsplit_once(':')?;
    let win = win.parse().ok()?;
    (!session.is_empty() && win > 0).then_some((session, win))
}

pub fn parse_files_id(id: &str) -> Option<&str> {
    id.strip_prefix("neovim-files:")
        .filter(|session| !session.is_empty())
}
fn files_id(session: &str) -> String {
    format!("neovim-files:{session}")
}

fn leaves(layout: &Value) -> Vec<i64> {
    if layout[0] == "leaf" {
        return layout[1].as_i64().into_iter().collect();
    }
    layout[1]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(leaves)
        .collect()
}

/// Find the closest existing sibling in Neovim's logical split tree. Existing
/// Monitter panes keep their placement, including user drag/reorder decisions.
fn insertion(layout: &Value, win: i64, existing: &HashSet<i64>) -> Option<(i64, &'static str)> {
    let kind = layout[0].as_str()?;
    if kind == "leaf" {
        return None;
    }
    let children = layout[1].as_array()?;
    let index = children
        .iter()
        .position(|child| leaves(child).contains(&win))?;
    if let Some(nested) = insertion(&children[index], win, existing) {
        return Some(nested);
    }
    for distance in 1..children.len() {
        if let Some(left) = index.checked_sub(distance) {
            if let Some(anchor) = leaves(&children[left])
                .into_iter()
                .rev()
                .find(|id| existing.contains(id))
            {
                return Some((anchor, if kind == "row" { "right" } else { "down" }));
            }
        }
        if index + distance < children.len() {
            if let Some(anchor) = leaves(&children[index + distance])
                .into_iter()
                .find(|id| existing.contains(id))
            {
                return Some((anchor, if kind == "row" { "left" } else { "up" }));
            }
        }
    }
    None
}

impl Desktop {
    pub(super) fn open_neovim_pane(
        &mut self,
        origin: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.window.neovim_pending {
            return Ok(());
        }
        // Resolve a local task/project only; never interpret an SSH path locally.
        let task = self.window.selected.as_ref().and_then(|id| {
            rows(&self.snapshot, "tasks")
                .into_iter()
                .find(|task| field(task, "id") == *id)
        });
        let cwd = task.and_then(|task| {
            let host_id = field(&task, "hostId");
            let local = rows(&self.snapshot, "hosts")
                .iter()
                .any(|host| field(host, "id") == host_id && field(host, "kind") == "local");
            local
                .then(|| field(&task, "cwd"))
                .filter(|cwd| !cwd.is_empty())
        });
        let origin = origin.or_else(|| self.window.dock.read(cx).focused_pane_id(window, cx));
        self.open_neovim_request(json!({"cwd":cwd,"cols":100,"rows":32}), origin, window, cx)
    }

    pub(super) fn open_neovim_file(
        &mut self,
        path: &std::path::Path,
        origin: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.window.neovim_pending {
            return Err("An editor is opening. Try the file link again in a moment.".into());
        }
        let cwd = path.parent().ok_or("The file link has no parent directory.")?;
        self.open_neovim_request(json!({"cwd":cwd,"path":path,"cols":100,"rows":32}), origin, window, cx)
    }

    fn open_neovim_request(
        &mut self,
        args: Value,
        origin: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let service = self.service.clone().ok_or("Native backend is not ready.")?;
        self.window.neovim_pending = true;
        let request = cx.background_executor().spawn(async move {
            service.invoke("open_neovim", args)
        });
        self.spawn_workspace(window, cx, async move |view, cx| {
            let result = request.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.window.neovim_pending = false;
                match result {
                    Ok(value) => {
                        let id = field(&value, "id");
                        if id.is_empty() {
                            view.window.error = Some("Neovim session response omitted its ID.".into());
                        } else if !view.closing {
                            view.window.neovim_origins
                                .insert(id.clone(), origin.unwrap_or_default());
                            view.ensure_neovim_session(&id, window, cx);
                        }
                    }
                    Err(error) => view.window.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    fn ensure_neovim_session(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<neovim::NeovimSession>> {
        if let Some(session) = self.window.neovim_sessions.get(id) {
            session.update(cx, |session, cx| session.set_workspace(&self.snapshot, cx));
            return Some(session.clone());
        }
        let service = self.service.clone()?;
        let session = cx.new(|cx| neovim::NeovimSession::new(service, id.to_owned(), cx));
        session.update(cx, |session, cx| session.set_workspace(&self.snapshot, cx));
        let id_owned = id.to_owned();
        cx.subscribe_workspace(
            &session,
            window,
            move |view, _, _: &neovim::NeovimSessionEvent, window, cx| {
                if !view.closing && !view.window.preferences_restoring {
                    if let Err(error) = view.reconcile_neovim(&id_owned, window, cx) {
                        view.window.error = Some(error);
                    }
                    cx.notify();
                }
            },
        )
        .detach();
        self.window.neovim_sessions.insert(id.to_owned(), session.clone());
        Some(session)
    }

    pub(super) fn show_neovim_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let key = self.window.dock
            .read(cx)
            .focused_pane_id(window, cx)
            .ok_or("Focus a Neovim editor first.")?;
        let pane = self.window.neovim_panes
            .get(&key)
            .ok_or("Focus a Neovim editor first.")?;
        pane.update(cx, |pane, cx| pane.show_files(window, cx));
        Ok(())
    }

    pub(super) fn neovim_spec(
        &mut self,
        id: &str,
        win: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<docking::PaneSpec, String> {
        let key = pane_id(id, win);
        let session = self
            .ensure_neovim_session(id, window, cx)
            .ok_or("Neovim backend unavailable.")?;
        let pane = self.window.neovim_panes
            .entry(key.clone())
            .or_insert_with(|| {
                cx.new(|cx| neovim::NeovimPane::new(session.clone(), win, window, cx))
            })
            .clone();
        let title = session.read(cx).snapshot["windows"]
            .as_array()
            .and_then(|windows| windows.iter().find(|row| row["id"].as_i64() == Some(win)))
            .map(editor_title)
            .unwrap_or_else(|| "Neovim".into());
        let weak = cx.weak_entity();
        let active_key = key.clone();
        let visible_pane = pane.downgrade();
        let pane_entity_id = pane.entity_id();
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
            match event {
                docking::PaneLifecycle::Visible(visible) => { let _ = visible_pane.update(cx, |pane, cx| pane.set_visible(visible, cx)); }
                docking::PaneLifecycle::Removed => { let _ = visible_pane.update(cx, |pane, cx| pane.set_visible(false, cx)); }
                _ => {}
            }
            if matches!(event, docking::PaneLifecycle::Active(true)) {
                let weak = weak.clone();
                let key = active_key.clone();
                cx.defer(move |cx| {
                    // The dock can move before this deferred activation runs.
                    // Follow the retained pane, then enter its actual host view.
                    cx.with_window(pane_entity_id, |window, cx| {
                        let _ = weak.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, _, cx| {
                            if !view.closing && view.window.dock.read(cx).contains(&key) {
                                view.window.sidebar = Sidebar::Chats;
                                view.window.dock_current = Some(key);
                                cx.notify();
                            }
                        }));
                    });
                });
            }
        });
        Ok(docking::PaneSpec {
            id: key,
            title: title.into(),
            content: pane.clone().into(),
            focus: pane.focus_handle(cx),
            lifecycle: Some(lifecycle),
            // Neovim must acknowledge a close before removing the native pane.
            // The workspace close action uses that path.
            closable: false,
        })
    }

    /// Release a Neovim pane after DockWorkspace reports an actual removal.
    /// Layout switches run in retaining mode and do not emit this event, so
    /// panes in inactive named workspaces remain cached. Their saved layout
    /// can also recreate a released pane later from the live session snapshot.
    pub(super) fn release_removed_neovim_pane(&mut self, key: &str, cx: &App) {
        if parse_id(key).is_some() && !self.window.dock.read(cx).contains(key) {
            self.window.neovim_panes.remove(key);
        }
    }

    pub(super) fn reconcile_neovim(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(session) = self.window.neovim_sessions.get(id).cloned() else {
            return Ok(());
        };
        session.update(cx, |session, cx| session.set_workspace(&self.snapshot, cx));
        let snapshot = session.read(cx).snapshot.clone();
        if snapshot["exited"] == true {
            return self.remove_neovim_session_panes(id, window, cx);
        }
        let layout = snapshot["layout"].clone();
        if layout.is_null() {
            return Ok(());
        }
        let wanted: HashSet<i64> = leaves(&layout).into_iter().collect();
        let mut existing: HashSet<i64> = self.window.neovim_panes
            .keys()
            .filter_map(|key| {
                parse_id(key)
                    .filter(|(session, _)| *session == id)
                    .map(|(_, win)| win)
            })
            .filter(|win| {
                self.window.dock
                    .read(cx)
                    .group_for_pane(&pane_id(id, *win), cx)
                    .is_some()
            })
            .collect();
        let focused = self.window.dock.read(cx).focused_pane_id(window, cx);
        let owns_focus = focused
            .as_ref()
            .and_then(|key| parse_id(key))
            .is_some_and(|(session, _)| session == id);
        let origin = self.window.neovim_origins.get(id).cloned();
        let opening = origin.is_some();
        if existing.is_empty() && !opening {
            return Ok(());
        }
        let mut added = false;
        for win in leaves(&layout)
            .into_iter()
            .filter(|win| wanted.contains(win))
        {
            if existing.contains(&win) {
                continue;
            }
            let spec = self.neovim_spec(id, win, window, cx)?;
            let key = spec.id.clone();
            let anchor = insertion(&layout, win, &existing)
                .map(|(win, direction)| (pane_id(id, win), direction));
            let group = anchor
                .as_ref()
                .and_then(|(key, _)| self.window.dock.read(cx).group_for_pane(key, cx));
            let origin_group = origin
                .as_ref()
                .and_then(|key| self.window.dock.read(cx).group_for_pane(key, cx));
            self.window.dock.update(cx, |dock, cx| {
                dock.add(spec, DockPlacement::Center, None, window, cx)?;
                if let (Some(group), Some((_, direction))) = (group, anchor) {
                    let placement = match direction {
                        "left" => Placement::Left,
                        "up" => Placement::Top,
                        "down" => Placement::Bottom,
                        _ => Placement::Right,
                    };
                    dock.move_pane(
                        &key,
                        InsertTarget::Split {
                            node: group,
                            placement,
                            size: None,
                        },
                        window,
                        cx,
                    );
                } else if let Some(group) = origin_group {
                    dock.move_pane(
                        &key,
                        InsertTarget::Tabs {
                            node: group,
                            ix: None,
                            activate: true,
                        },
                        window,
                        cx,
                    );
                }
                Ok::<_, String>(())
            })?;
            existing.insert(win);
            added = true;
        }
        for win in existing.difference(&wanted).copied().collect::<Vec<_>>() {
            let key = pane_id(id, win);
            self.window.dock.update(cx, |dock, cx| {
                dock.remove(&key, window, cx);
            });
            self.window.neovim_panes.remove(&key);
        }
        if !wanted.is_empty() {
            if let Some(origin) = self.window.neovim_origins
                .remove(id)
                .filter(|key| key.starts_with("empty:"))
            {
                self.window.dock.update(cx, |dock, cx| {
                    dock.remove(&origin, window, cx);
                });
            }
        }
        if let Some(windows) = snapshot["windows"].as_array() {
            for row in windows {
                if let Some(win) = row["id"].as_i64() {
                    self.window.dock.update(cx, |dock, cx| {
                        dock.set_title(&pane_id(id, win), editor_title(row).into(), cx)
                    });
                }
            }
        }
        if let Some(current) = snapshot["currentWindow"]
            .as_i64()
            .filter(|win| wanted.contains(win))
        {
            let changed = self.window.neovim_focus_windows.get(id) != Some(&current);
            if session.read(cx).pending_focus.is_none() {
                self.window.neovim_focus_windows.insert(id.into(), current);
            }
            if (opening || (owns_focus && changed && session.read(cx).pending_focus.is_none()))
                && (self.window.neovim_layouts.get(id) != Some(&layout) || changed)
            {
                let key = pane_id(id, current);
                self.window.dock.update(cx, |dock, cx| {
                    dock.focus(&key, window, cx);
                    dock.set_shown(true, window, cx);
                });
                self.window.sidebar = Sidebar::Chats;
                self.window.dock_current = Some(key);
            }
        }
        // A redraw/plugin in an unfocused editor must never steal chat focus.
        if added && !owns_focus && !opening {
            if let Some(key) = focused {
                self.window.dock.update(cx, |dock, cx| {
                    dock.focus(&key, window, cx);
                });
            }
        }
        // Removing the final editor window can leave GPUI without a focused
        // pane. Hand focus back to the remaining workspace so navigation and
        // typing continue immediately after a successful guarded close.
        if owns_focus && self.window.dock.read(cx).focused_pane_id(window, cx).is_none() {
            self.window.dock.update(cx, |dock, cx| {
                dock.focus_relative_pane("first", window, cx);
            });
        }
        self.window.neovim_layouts.insert(id.into(), layout);
        Ok(())
    }

    fn remove_neovim_session_panes(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // Include retained panes in inactive workspaces, not just docked panes.
        let mut obsolete: HashSet<String> = self.window.neovim_panes.keys()
            .filter(|key| parse_id(key).is_some_and(|(session, _)| session == id))
            .cloned().collect();
        obsolete.extend(docking::DockWorkspace::saved_ids(&self.window.dock.read(cx).serialize(cx)?)?
            .into_iter().filter(|key| parse_id(key).is_some_and(|(session, _)| session == id)));
        obsolete.insert(files_id(id));
        let focused = self.window.dock.read(cx).focused_pane_id(window, cx);
        let owned_focus = focused.as_ref().is_some_and(|key| obsolete.contains(key));
        for key in &obsolete {
            self.window.dock.update(cx, |dock, cx| { dock.remove(key, window, cx); });
            self.window.neovim_panes.remove(key);
        }
        if let Some(set) = &mut self.named_workspaces {
            for item in &mut set.items {
                if let Some(layout) = &mut item.layout {
                    // Saved inactive layouts can contain additional window IDs.
                    let ids = docking::DockWorkspace::saved_ids(layout)?.into_iter()
                        .filter(|key| parse_id(key).is_some_and(|(session, _)| session == id)
                            || parse_files_id(key) == Some(id)).collect();
                    let cleaned = workspace_visibility::prune_native_pane_ids(layout, &ids)?;
                    if cleaned != *layout {
                        *layout = cleaned;
                        self.workspaces_dirty = true;
                    }
                }
            }
        }
        self.window.neovim_sessions.remove(id);
        self.window.neovim_layouts.remove(id);
        self.window.neovim_origins.remove(id);
        self.window.neovim_focus_windows.remove(id);
        if owned_focus {
            self.window.dock.update(cx, |dock, cx| { dock.focus_relative_pane("first", window, cx); });
        }
        self.window.dock_current = self.window.dock.read(cx).focused_pane_id(window, cx);
        self.preferences_dirty = true;
        self.queue_preferences_save(cx);
        cx.notify();
        Ok(())
    }

    pub(super) fn neovim_split_focused(
        &mut self,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let dock = self.window.dock.read(cx);
        let Some(key) = dock.focused_pane_id(window, cx).or_else(|| {
            self.window
                .dock_current
                .as_ref()
                .filter(|key| dock.group_for_pane(key, cx).is_some())
                .cloned()
        }) else {
            return false;
        };
        let Some((id, win)) = parse_id(&key) else {
            return false;
        };
        if let Some(session) = self.window.neovim_sessions.get(id) {
            session.update(cx, |session, cx| {
                session.command(win, if vertical { "vsplit" } else { "split" }, cx)
            });
        }
        true
    }
    pub(super) fn neovim_close_focused(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(key) = self.window.dock.read(cx).focused_pane_id(window, cx) else {
            return false;
        };
        let Some((id, win)) = parse_id(&key) else {
            return false;
        };
        if let Some(session) = self.window.neovim_sessions.get(id) {
            session.update(cx, |session, cx| session.close_window(win, cx));
        } else {
            // A stale restored identity has no editor that could acknowledge close.
            if let Err(error) = self.remove_neovim_session_panes(id, window, cx) {
                self.window.error = Some(error);
            }
        }
        true
    }
}

fn editor_title(row: &Value) -> String {
    let name = row["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .and_then(|name| std::path::Path::new(name).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "[No Name]".into());
    format!(
        "{}{}",
        name,
        if row["modified"] == true { " •" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    #[test]
    fn nested_split_uses_closest_retained_neighbor() {
        let tree = json!(["row", [["leaf", 1], ["col", [["leaf", 2], ["leaf", 3]]]]]);
        assert_eq!(
            insertion(&tree, 3, &HashSet::from([1, 2])),
            Some((2, "down"))
        );
        assert_eq!(insertion(&tree, 2, &HashSet::from([1, 3])), Some((3, "up")));
        assert_eq!(
            insertion(&tree, 1, &HashSet::from([2, 3])),
            Some((2, "left"))
        );
    }
    #[test]
    fn identities_are_session_scoped() {
        assert_eq!(parse_id("neovim:abc:1000"), Some(("abc", 1000)));
        assert!(parse_id("neovim::0").is_none());
        assert!(parse_id("terminal:abc").is_none());
        assert_eq!(parse_files_id("neovim-files:abc"), Some("abc"));
        assert!(parse_files_id("neovim-files:").is_none());
        assert!(parse_id("neovim-files:abc").is_none());
    }
}
