//! Sanitized shapes from the installed workspace store, without task content.
use crate::docking::{DockWorkspace, PaneFactory, PaneSpec};
use gpui_kit::{
    div, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window,
};
use serde_json::{json, Value};
use std::rc::Rc;

struct Placeholder;
impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("Retained placeholder")
    }
}
fn tab(ids: &[&str], active: usize) -> Value {
    json!({"panel_name":"TabPanel","children":ids.iter().map(|id|json!({"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":id}}})).collect::<Vec<_>>(),"info":{"tabs":{"active_index":active}}})
}
fn layout(groups: Vec<Value>, focused: &str) -> Value {
    json!({"version":1,"focusedPaneId":focused,"center":{"panel_name":"StackPanel","info":{"stack":{"sizes":vec![610.75;groups.len()],"axis":0}},"children":groups}})
}
#[gpui_kit::test]
fn workspace_restore_retaining_accepts_four_columns_single_and_multi_tab_local_focus(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let mut dock = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|id, _, cx| {
            Some(PaneSpec {
                id: id.into(),
                title: "Placeholder".into(),
                content: cx.new(|_| Placeholder).into(),
                focus: cx.focus_handle(),
                lifecycle: None,
                closable: true,
            })
        });
        let workspace = cx.new(|cx| DockWorkspace::new("saved-shape-restore", factory, window, cx));
        dock = Some(workspace.clone());
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    let dock = dock.unwrap();
    for value in [
        layout(
            ["chat:one", "chat:two", "chat:three", "chat:four"]
                .iter()
                .map(|id| tab(&[id], 0))
                .collect(),
            "chat:two",
        ),
        layout(vec![tab(&["chat:five"], 0)], "chat:five"),
        layout(vec![tab(&["chat:six", "chat:one"], 1)], "chat:one"),
    ] {
        let expected = DockWorkspace::saved_ids(&value).unwrap();
        visual.update(|window, cx| {
            dock.update(cx, |dock, cx| {
                dock.restore_retaining(value, window, cx)
                    .expect("real saved shape restores");
                assert_eq!(
                    DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap(),
                    expected
                );
            })
        });
    }
}

#[gpui_kit::test]
fn workspace_restore_waits_for_matching_publication_without_consuming_initial_intent(
    cx: &mut TestAppContext,
) {
    use crate::Desktop;
    use monitter_gpui_desktop::native_publication::{self, PublicationChanges};
    use monitter_lib::experimental_desktop::{DesktopService, NativePreferences, UiDeltaResponse};
    use std::collections::HashSet;
    cx.update(gpui_kit::init);
    let mut desktop = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Desktop::new(window, cx));
        desktop = Some(view);
        // Exercise Desktop state transitions without rendering its unrelated
        // sidebar usage widgets, whose production workers use OS threads.
        let content = cx.new(|_| Placeholder);
        gpui_kit::component::Root::new(content, window, cx)
    });
    let desktop = desktop.unwrap();
    visual.update(|window, cx| {
        desktop.update(cx, |view, cx| {
            let mut snapshot = monitter_lib::model::default_snapshot();
            snapshot.tasks.push(serde_json::from_value(json!({
            "id":"fixture-task", "agentId":"fixture-agent", "title":"Fixture", "status":"idle",
            "createdAt":0,"updatedAt":0,"hostId":"fixture-host","cwd":"/tmp","provider":"codex",
            "model":"fixture","sandbox":"read-only"
        })).unwrap());
            view.native_state.apply(UiDeltaResponse {
                revision: "fixture:1".into(),
                snapshot: Some(snapshot.clone()),
                delta: None,
            });
            view.snapshot = Rc::new(json!({"tasks":[]}));
            view.publication_index = None;
            view.revision = None;
            view.service = Some(DesktopService::disconnected_for_ui_fixture());
            let mut preferences = NativePreferences::default();
            preferences.data.layout = Some(layout(
                vec![tab(&["chat:fixture-task"], 0)],
                "chat:fixture-task",
            ));
            view.preferences = Some(preferences);
            view.named_workspaces = None;
            view.preferences_restore = true;
            view.preferences_blocked = false;
            view.preferences_error = None;
            let generation = view.preferences_restore_generation;
            view.restore_preferences(window, cx);
            assert!(
                view.preferences_restore,
                "raw snapshot must not consume queued restore"
            );
            assert!(!view.preferences_restoring);
            assert!(!view.preferences_blocked);
            assert!(view.preferences_error.is_none());
            assert_eq!(view.preferences_restore_generation, generation);

            // Retain a placeholder for the task so restoring tests UI state only.
            let content = cx.new(|_| Placeholder);
            let focus = cx.focus_handle();
            view.dock
                .update(cx, |dock, cx| {
                    dock.add(
                        PaneSpec {
                            id: "chat:fixture-task".into(),
                            title: "Fixture".into(),
                            content: content.into(),
                            focus,
                            lifecycle: None,
                            closable: true,
                        },
                        gpui_kit::component::dock::DockPlacement::Center,
                        None,
                        window,
                        cx,
                    )
                })
                .unwrap();
            let changes = PublicationChanges {
                reset: true,
                ..Default::default()
            };
            let prepared =
                native_publication::prepare(snapshot, HashSet::new(), &changes, None).unwrap();
            let notifications = crate::notification_model::prepare_notifications(
                &prepared.snapshot, &prepared.index, &prepared.message_updates, &changes, None);
            view.commit_native_publication(prepared, Some("fixture:1".into()), changes, notifications, Default::default(), cx);
            view.revision = Some("fixture:older".into());
            view.restore_preferences(window, cx);
            assert!(view.preferences_restore, "stale committed publication must still defer restore");
            assert!(!view.preferences_blocked);
            assert!(view.preferences_error.is_none());
            view.revision = Some("fixture:1".into());
            view.restore_preferences(window, cx);
            assert!(!view.preferences_restore);
            assert!(!view.preferences_restoring);
            assert!(!view.preferences_blocked);
            assert!(view.preferences_error.is_none());
            assert_eq!(view.preferences_restore_generation, generation + 1);
            assert_eq!(
                DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap(),
                vec!["chat:fixture-task"]
            );
        })
    });
}

fn desktop_restore_fixture(
    cx: &mut TestAppContext,
) -> (gpui_kit::Entity<crate::Desktop>, &mut gpui_kit::VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut desktop = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            let mut view = crate::Desktop::new(window, cx);
            view.service = Some(monitter_lib::experimental_desktop::DesktopService::disconnected_for_ui_fixture());
            view.preferences = Some(Default::default());
            view.preferences_busy = false;
            view.preferences_restore = false;
            view.preferences_restoring = false;
            view.preferences_restore_suppressed = false;
            view.preferences_blocked = false;
            view.snapshot = Rc::new(json!({"tasks":[{"id":"fixture-task"}]}));
            view
        });
        desktop = Some(view);
        gpui_kit::component::Root::new(cx.new(|_| Placeholder), window, cx)
    });
    (desktop.unwrap(), visual)
}

fn retained_placeholder(id: &str, cx: &mut Context<DockWorkspace>) -> PaneSpec {
    PaneSpec {
        id: id.into(), title: id.into(), content: cx.new(|_| Placeholder).into(),
        focus: cx.focus_handle(), lifecycle: None, closable: true,
    }
}

const MISSING_TERMINAL: &str = "terminal:37fffedc-3272-4039-8d23-31a493a849ae";

#[gpui_kit::test]
fn ideas_pane_opens_beside_agent_and_reuses_retained_entity(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        view.snapshot = Rc::new(json!({"projects":[{"id":"project","name":"Project","ideas":[],"ideasRevision":0}],"tasks":[{"id":"fixture-task","projectId":"project"}]}));
        view.selected = Some("fixture-task".into());
        view.dock.update(cx, |dock, cx| {
            dock.add(retained_placeholder("chat:fixture-task", cx), gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
            dock.focus("chat:fixture-task", window, cx);
        });
        view.open_ideas(window, cx).unwrap();
        let pane = view.ideas.as_ref().unwrap().entity_id();
        let dock = view.dock.read(cx);
        assert_ne!(dock.group_for_pane("route:ideas", cx), dock.group_for_pane("chat:fixture-task", cx), "ideas must split beside the agent, not hide its tab");
        let saved = dock.serialize(cx).unwrap();
        assert!(DockWorkspace::saved_ids(&saved).unwrap().contains(&"route:ideas".into()));
        view.open_ideas(window, cx).unwrap();
        assert_eq!(view.ideas.as_ref().unwrap().entity_id(), pane);
        assert_eq!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap().iter().filter(|id| *id == "route:ideas").count(), 1);
        view.dock.update(cx, |dock, cx| dock.remove("route:ideas", window, cx));
        view.open_ideas(window, cx).unwrap();
        assert_eq!(view.ideas.as_ref().unwrap().entity_id(), pane, "closing and reopening keeps the same editor");
        view.finish_workspace_restore(saved, vec![], vec![], 0, window, cx);
        assert!(view.preferences_error.is_none(), "Idea List is a supported saved route");
    }));
}

#[gpui_kit::test]
fn ideas_split_actions_target_the_owning_pane_and_retain_editors(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        view.snapshot = Rc::new(json!({"projects":[{"id":"project","name":"Project","ideas":[],"ideasRevision":0}],"tasks":[{"id":"fixture-task","projectId":"project"}]}));
        view.selected = Some("fixture-task".into());
        view.dock.update(cx, |dock, cx| {
            dock.add(retained_placeholder("chat:fixture-task", cx), gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
            dock.focus("chat:fixture-task", window, cx);
        });
        view.open_ideas(window, cx).unwrap();
    }));
    for vertical in [true, false] {
        let (owner_group, agent_group, editor, before) = visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.dock.update(cx, |dock, cx| assert!(dock.focus("chat:fixture-task", window, cx)));
            let dock = view.dock.read(cx);
            (dock.group_for_pane("route:ideas", cx), dock.group_for_pane("chat:fixture-task", cx),
                view.ideas.as_ref().unwrap().entity_id(), DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap().len())
        }));
        visual.update(|_, cx| desktop.update(cx, |view, cx| {
            view.ideas.as_ref().unwrap().update(cx, |_, cx| cx.emit(crate::ideas::IdeasEvent::Split { vertical }));
        }));
        visual.run_until_parked();
        visual.update(|_, cx| desktop.update(cx, |view, cx| {
            let dock = view.dock.read(cx);
            assert_eq!(dock.group_for_pane("route:ideas", cx), owner_group);
            assert_eq!(dock.group_for_pane("chat:fixture-task", cx), agent_group);
            assert_eq!(view.ideas.as_ref().unwrap().entity_id(), editor);
            assert_eq!(DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap().len(), before + 1);
        }));
    }
}

#[gpui_kit::test]
fn desktop_workspace_restore_prunes_missing_terminal_and_retains_chat_and_outgoing_pane(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        view.dock.update(cx, |dock, cx| {
            dock.add(retained_placeholder("chat:outgoing", cx), gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
            dock.register(retained_placeholder("chat:fixture-task", cx), cx).unwrap();
        });
        let outgoing_key = view.dock.read(cx).panel_key("chat:outgoing").unwrap();
        view.finish_workspace_restore(layout(vec![tab(&["chat:fixture-task", MISSING_TERMINAL], 1)], MISSING_TERMINAL), vec![], vec![], 0, window, cx);
        assert!(!view.preferences_blocked);
        assert!(view.preferences_error.is_none());
        assert_eq!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap(), vec!["chat:fixture-task"]);
        assert_eq!(view.dock.read(cx).panel_key("chat:outgoing"), Some(outgoing_key), "switching retains the outgoing pane entity");
    }));
}

#[gpui_kit::test]
fn desktop_workspace_restore_missing_terminal_only_restores_empty_layout(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        view.dock.update(cx, |dock, cx| {
            dock.add(retained_placeholder("chat:outgoing", cx), gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
        });
        view.finish_workspace_restore(layout(vec![tab(&[MISSING_TERMINAL], 0)], MISSING_TERMINAL), vec![], vec![], 0, window, cx);
        assert!(view.preferences_error.is_none());
        assert!(!view.preferences_blocked);
        assert!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap().is_empty());
        assert!(view.dock.read(cx).contains("chat:outgoing"));
    }));
}

#[gpui_kit::test]
fn desktop_workspace_restore_prunes_missing_neovim_session(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        let missing_neovim = "neovim:session-no-longer-running:1001";
        view.finish_workspace_restore(
            layout(vec![tab(&[missing_neovim], 0)], missing_neovim),
            vec![],
            vec![],
            0,
            window,
            cx,
        );
        assert!(view.preferences_error.is_none());
        assert!(!view.preferences_blocked);
        assert!(!view.dock.read(cx).contains(missing_neovim));
        assert!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap())
            .unwrap()
            .is_empty());
    }));
}

#[gpui_kit::test]
fn exited_neovim_removes_moved_pane_and_retains_neighboring_chat(cx: &mut TestAppContext) {
    use monitter_lib::experimental_desktop::{NativeWorkspace, NativeWorkspaceSet};

    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        let editor_id = "neovim:fixture-nvim:1001";
        let spec = view
            .neovim_spec("fixture-nvim", 1001, window, cx)
            .expect("fixture session should create its retained editor pane");
        let chat = cx.new(|_| Placeholder);
        let chat_spec = PaneSpec {
            id: "chat:neighbor".into(),
            title: "Chat neighbor".into(),
            content: chat.into(),
            focus: cx.focus_handle(),
            lifecycle: None,
            closable: true,
        };
        view.dock.update(cx, |dock, cx| {
            dock.add(chat_spec, gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
            dock.add(spec, gpui_kit::component::dock::DockPlacement::Center, None, window, cx).unwrap();
        });
        let chat_group = view.dock.read(cx).group_for_pane("chat:neighbor", cx).unwrap();
        assert!(view.dock.update(cx, |dock, cx| dock.move_pane(
            editor_id,
            gpui_kit::component::dock::InsertTarget::Split {
                node: chat_group,
                placement: gpui_kit::component::Placement::Right,
                size: None,
            },
            window,
            cx,
        )));
        assert!(view.dock.read(cx).contains(editor_id));
        assert!(view.dock.read(cx).contains("chat:neighbor"));

        view.neovim_layouts.insert("fixture-nvim".into(), json!(["leaf", 1001]));
        view.neovim_origins.insert("fixture-nvim".into(), "chat:neighbor".into());
        view.neovim_focus_windows.insert("fixture-nvim".into(), 1001);
        view.named_workspaces = Some(NativeWorkspaceSet {
            active_id: "active".into(),
            items: vec![NativeWorkspace {
                id: "active".into(),
                name: "Active".into(),
                icon: "folder".into(),
                color: "#38BFD2".into(),
                url: None, layout: Some(layout(vec![tab(&[editor_id, "chat:neighbor"], 0)], editor_id)),
            }],
        });
        view.neovim_sessions
            .get("fixture-nvim")
            .expect("Neovim session retained")
            .update(cx, |session, _| session.snapshot["exited"] = json!(true));

        view.reconcile_neovim("fixture-nvim", window, cx).unwrap();

        assert!(!view.dock.read(cx).contains(editor_id));
        assert!(view.dock.read(cx).contains("chat:neighbor"));
        assert!(!view.neovim_sessions.contains_key("fixture-nvim"));
        assert!(!view.neovim_panes.contains_key(editor_id));
        assert!(!view.neovim_layouts.contains_key("fixture-nvim"));
        assert!(!view.neovim_origins.contains_key("fixture-nvim"));
        assert!(!view.neovim_focus_windows.contains_key("fixture-nvim"));
        let inactive_ids = DockWorkspace::saved_ids(
            view.named_workspaces.as_ref().unwrap().items[0].layout.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(inactive_ids, vec!["chat:neighbor"]);
    }));
}

#[gpui_kit::test]
fn desktop_workspace_restore_keeps_running_and_exited_terminals(cx: &mut TestAppContext) {
    let (desktop, visual) = desktop_restore_fixture(cx);
    let running = "terminal:137fffed-3272-4039-8d23-31a493a849ae";
    let exited = "terminal:237fffed-3272-4039-8d23-31a493a849ae";
    visual.update(|window, cx| desktop.update(cx, |view, cx| {
        view.dock.update(cx, |dock, cx| {
            for id in [running, exited] { dock.register(retained_placeholder(id, cx), cx).unwrap(); }
        });
        let sessions = [(running, "running"), (exited, "exited")].into_iter().map(|(id, status)| json!({"id":id.strip_prefix("terminal:").unwrap(), "status":status})).collect();
        view.finish_workspace_restore(layout(vec![tab(&[running, MISSING_TERMINAL, exited], 2)], exited), sessions, vec![], 0, window, cx);
        assert!(view.preferences_error.is_none());
        assert!(!view.preferences_blocked);
        assert_eq!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap(), vec![running, exited]);
    }));
}

#[gpui_kit::test]
fn desktop_workspace_restore_explicit_failure_preserves_active_workspace_and_allows_next_switch(
    cx: &mut TestAppContext,
) {
    use monitter_lib::experimental_desktop::{NativeWorkspace, NativeWorkspaceSet};
    let (desktop, visual) = desktop_restore_fixture(cx);
    visual.update(|window, cx| {
        desktop.update(cx, |view, cx| {
            view.dock.update(cx, |dock, cx| {
                dock.add(
                    retained_placeholder("chat:outgoing", cx),
                    gpui_kit::component::dock::DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            });
            let outgoing = view.window.dock.clone();
            let outgoing_key = outgoing.read(cx).panel_key("chat:outgoing").unwrap();
            let current = outgoing.read(cx).serialize(cx).unwrap();
            let valid = layout(vec![tab(&["chat:fixture-task"], 0)], "chat:fixture-task");
            view.named_workspaces = Some(NativeWorkspaceSet {
                active_id: "current".into(),
                items: [
                    ("current", current.clone()),
                    ("invalid", json!({"version":999})),
                    ("valid", valid.clone()),
                ]
                .into_iter()
                .map(|(id, layout)| NativeWorkspace {
                    id: id.into(),
                    name: id.into(),
                    icon: "folder".into(),
                    color: "#38BFD2".into(),
                    url: None,
                    layout: Some(layout),
                })
                .collect(),
            });
            // Startup normally establishes these identities. Exercise an actual
            // explicit switch, which parks the outgoing full view before restore.
            view.window.active_workspace_id = Some("current".into());
            view.workspace_window_restore_pending = false;
            view.workspace_window_assignments =
                crate::workspace_window_state::WorkspaceWindowState::new(&[
                    "current".into(),
                    "invalid".into(),
                    "valid".into(),
                ])
                .unwrap();
            for invalid in [
                json!({"version":999}),
                layout(vec![tab(&["terminal:malformed"], 0)], "terminal:malformed"),
            ] {
                view.named_workspaces
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|item| item.id == "invalid")
                    .unwrap()
                    .layout = Some(invalid.clone());
                view.switch_named_workspace("invalid".into(), window, cx);
                assert_eq!(view.window.workspace_switch.as_deref(), Some("invalid"));
                assert_ne!(
                    view.window.dock.entity_id(),
                    outgoing.entity_id(),
                    "uncached restore has its own destination dock"
                );
                assert_eq!(
                    view.workspace_view_cache["current"].dock.entity_id(),
                    outgoing.entity_id()
                );
                view.finish_workspace_restore(invalid.clone(), vec![], vec![], 0, window, cx);
                assert_eq!(view.named_workspaces.as_ref().unwrap().active_id, "current");
                assert_eq!(view.window.active_workspace_id.as_deref(), Some("current"));
                assert_eq!(
                    view.window.dock.entity_id(),
                    outgoing.entity_id(),
                    "both failure paths reactivate the exact retained source"
                );
                assert_eq!(
                    view.dock.read(cx).panel_key("chat:outgoing"),
                    Some(outgoing_key)
                );
                assert_eq!(view.dock.read(cx).serialize(cx).unwrap(), current);
                assert_eq!(
                    view.named_workspaces
                        .as_ref()
                        .unwrap()
                        .items
                        .iter()
                        .find(|item| item.id == "invalid")
                        .unwrap()
                        .layout
                        .as_ref(),
                    Some(&invalid),
                    "failed transient UI never replaces the saved target"
                );
                assert!(
                    !view.workspace_view_cache.contains_key("invalid"),
                    "a failed target cannot be selected as a successful retained view"
                );
                assert!(view.preferences_error.is_some());
                assert!(
                    !view.preferences_blocked,
                    "an explicit failure must leave workspace controls usable"
                );
                assert!(
                    !view.window.preferences_restore
                        && !view.window.preferences_restoring
                        && !view.window.preferences_restore_suppressed
                );
                assert!(view.workspace_switch.is_none());
            }
            view.switch_named_workspace("valid".into(), window, cx);
            assert_eq!(
                view.workspace_switch.as_deref(),
                Some("valid"),
                "next workspace click queues a new restore"
            );
            // The offline fixture provides its placeholder to the new dock, as
            // prepare_saved_panes normally constructs content from the snapshot.
            view.window
                .dock
                .update(cx, |dock, cx| {
                    dock.register(retained_placeholder("chat:fixture-task", cx), cx)
                })
                .unwrap();
            view.finish_workspace_restore(valid, vec![], vec![], 0, window, cx);
            assert_eq!(view.named_workspaces.as_ref().unwrap().active_id, "valid");
            assert_eq!(view.window.active_workspace_id.as_deref(), Some("valid"));
            assert_eq!(
                view.workspace_view_cache["current"].dock.entity_id(),
                outgoing.entity_id()
            );
            assert_eq!(
                outgoing.read(cx).serialize(cx).unwrap(),
                current,
                "successful next restore still retains the source unchanged"
            );
            assert!(view.preferences_error.is_none());
            assert!(!view.preferences_blocked);
        })
    });
}
