//! Offline tests of retained workspace ownership, without rendering Desktop,
//! connecting a backend, creating a native browser, or writing preferences.
use super::*;
use ::core::prelude::v1::test;
use gpui_kit::{TestAppContext, VisualTestContext, component::Root};
use monitter_lib::experimental_desktop::{NativeWorkspace, NativeWorkspaceSet};

struct OfflineHost;
impl Render for OfflineHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

struct Fixture {
    desktop: Entity<Desktop>,
    drafts: [Entity<draft_creation::DraftCreationPane>; 3],
    docks: [Entity<docking::DockWorkspace>; 3],
    forms: [draft_creation::DraftFormState; 3],
    main: AnyWindowHandle,
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut fixture = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let mut retained = None;
        let desktop = cx.new(|cx| {
            let mut desktop = Desktop::new(window, cx);
            desktop.service = Some(DesktopService::disconnected_for_ui_fixture());
            desktop.preferences = Some(NativePreferences::default());
            // Keep all debounce continuations offline, including startup cache.
            desktop.preferences_busy = true;
            desktop.startup_pending = false;
            desktop.workspace_window_restore_pending = false;
            desktop.snapshot = Rc::new(serde_json::json!({
                "settings":{}, "agents":[{"id":"fixture-agent","sandbox":"read-only"}],
                "projects":[], "tasks":[], "hosts":[], "messages":[]
            }));
            desktop.window_hosts.insert(
                WorkspaceWindowId::main(),
                WorkspaceHost {
                    handle: window.window_handle(),
                    root: None,
                },
            );
            let mut drafts = Vec::new();
            let mut docks = Vec::new();
            let mut forms = Vec::new();
            let mut workspaces = Vec::new();
            for id in ["a", "b", "c"] {
                let service = desktop.service.clone().unwrap();
                let snapshot = (*desktop.snapshot).clone();
                let draft = cx.new(|cx| {
                    draft_creation::DraftCreationPane::new(service, snapshot, window, cx)
                });
                let mut form = draft.read(cx).export_state(cx).unwrap();
                form.title = format!("Retained {id}");
                form.message = format!("Unsent text in {id}");
                form.cwd = format!("/synthetic/{id}");
                draft
                    .update(cx, |draft, cx| {
                        draft.restore_state(form.clone(), window, cx)
                    })
                    .unwrap();
                let key = format!("draft:{id}");
                desktop
                    .draft_panes
                    .borrow_mut()
                    .insert(key.clone(), draft.clone());
                let mut view = Desktop::create_workspace_view(
                    desktop.chat_panes.clone(),
                    desktop.draft_panes.clone(),
                    Sidebar::Chats,
                    window,
                    cx,
                );
                view.active_workspace_id = Some(id.into());
                view.preferences_restore = false;
                view.preferences_restoring = false;
                view.preferences_restore_suppressed = false;
                view.dock_current = Some(key.clone());
                view.draft_creation = Some(draft.clone());
                view.draft_creation_open = true;
                let pane = draft.clone();
                view.dock
                    .update(cx, |dock, cx| {
                        dock.add(
                            docking::PaneSpec {
                                id: key,
                                title: format!("Draft {id}").into(),
                                content: pane.clone().into(),
                                focus: pane.focus_handle(cx),
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
                workspaces.push(NativeWorkspace {
                    id: id.into(),
                    name: format!("Workspace {id}"),
                    icon: "folder".into(),
                    color: "#38BFD2".into(),
                    url: None,
                    layout: Some(view.dock.read(cx).serialize(cx).unwrap()),
                });
                drafts.push(draft);
                docks.push(view.dock.clone());
                forms.push(form);
                if id == "a" {
                    desktop.window = view;
                } else {
                    view.dock
                        .update(cx, |dock, cx| dock.set_shown(false, window, cx));
                    desktop.workspace_view_cache.insert(id.into(), view);
                }
            }
            desktop.named_workspaces = Some(NativeWorkspaceSet {
                active_id: "a".into(),
                items: workspaces,
            });
            desktop.workspace_window_assignments =
                WorkspaceWindowState::new(&["a".into(), "b".into(), "c".into()]).unwrap();
            retained = Some((
                drafts.try_into().ok().unwrap(),
                docks.try_into().ok().unwrap(),
                forms.try_into().ok().unwrap(),
            ));
            desktop
        });
        let (drafts, docks, forms) = retained.unwrap();
        fixture = Some(Fixture {
            desktop,
            drafts,
            docks,
            forms,
            main: window.window_handle(),
        });
        Root::new(cx.new(|_| OfflineHost), window, cx)
    });
    (fixture.unwrap(), visual)
}

// Test windows retain the same component Root as production, including its
// per-window focus/dialog state. Only the application content is inert, avoiding
// Desktop's unrelated render workers while transfer/rehost runs unchanged.
fn destination(
    fixture: &Fixture,
    visual: &mut VisualTestContext,
    id: &WorkspaceWindowId,
) -> AnyWindowHandle {
    let handle: AnyWindowHandle = visual
        .add_window(|window, cx| Root::new(cx.new(|_| OfflineHost), window, cx))
        .into();
    visual.update(|_, cx| {
        fixture.desktop.update(cx, |desktop, _| {
            desktop
                .window_hosts
                .insert(id.clone(), WorkspaceHost { handle, root: None });
        })
    });
    handle
}

fn detach(
    fixture: &Fixture,
    visual: &mut VisualTestContext,
) -> (WorkspaceWindowId, AnyWindowHandle) {
    let (assignments, id) = fixture.desktop.read_with(visual, |desktop, _| {
        let mut assignments = desktop.workspace_window_assignments.clone();
        let id = assignments.detach_workspace("a").unwrap();
        (assignments, id)
    });
    let handle = destination(fixture, visual, &id);
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            desktop
                .transfer_current_workspace(&id, assignments, window, cx)
                .unwrap();
        })
    });
    (id, handle)
}

fn assert_form(fixture: &Fixture, index: usize, cx: &App) {
    assert_eq!(
        fixture.drafts[index].read(cx).export_state(cx).unwrap(),
        fixture.forms[index]
    );
}

#[gpui_kit::test]
fn workspace_window_native_host_initializes_component_state_before_backdrop(
    cx: &mut TestAppContext,
) {
    let (fixture, visual) = fixture(cx);
    visual.update(|_, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            let mut assignments = desktop.workspace_window_assignments.clone();
            let id = assignments.detach_workspace("a").unwrap();
            let handle = desktop.create_workspace_host(&id, &id, cx).unwrap();
            assert_eq!(desktop.window_hosts[&id].handle, handle);
            assert!(!desktop.window_hosts[&id].root.as_ref().unwrap().read(cx).ready.get());
            // Opening the host has already performed its synchronous first
            // draw while Desktop is leased. Tear it down before deferred
            // readiness mounts Desktop's unrelated native metrics workers.
            handle.update(cx, |_, window, cx| {
                dialog_backdrop::install(window, cx);
                window.remove_window();
            }).unwrap();
            desktop.window_hosts.remove(&id);
        })
    });
}

#[gpui_kit::test]
fn workspace_window_focus_waits_for_ready_owner_without_mutating_outgoing_dock(
    cx: &mut TestAppContext,
) {
    let (fixture, visual) = fixture(cx);
    let (id, handle) = detach(&fixture, visual);
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            desktop
                .activate_retained_workspace("c", window, cx)
                .unwrap();
            let mut assignments = desktop.workspace_window_assignments.clone();
            assignments
                .transfer_workspace("c", &id, usize::MAX)
                .unwrap();
            desktop
                .transfer_current_workspace(&id, assignments, window, cx)
                .unwrap();
            assert!(desktop.focus_pane_owner("draft:a", cx));
            let host = &desktop.parked_window_views[&id];
            assert_eq!(host.active_workspace_id.as_deref(), Some("c"));
            assert_eq!(host.dock_current.as_deref(), Some("draft:c"));
            assert_eq!(
                host.pending_pane_focus,
                Some(("a".into(), "draft:a".into()))
            );
        });
    });
    handle
        .update(visual, |_, window, cx| {
            fixture.desktop.update(cx, |desktop, cx| {
                desktop.preferences_busy = false;
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    desktop.process_workspace_window_intent(window, cx);
                    assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("a"));
                    assert_eq!(
                        desktop.window.dock.entity_id(),
                        fixture.docks[0].entity_id()
                    );
                    assert_eq!(desktop.window.dock_current.as_deref(), Some("draft:a"));
                    assert!(desktop.window.pending_pane_focus.is_none());
                });
                desktop.preferences_busy = true;
            });
        })
        .unwrap();
    handle
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn workspace_window_detach_return_retains_docks_drafts_and_one_controller(cx: &mut TestAppContext) {
    let (fixture, visual) = fixture(cx);
    let (controller, panes, drafts, snapshot, lock) =
        fixture.desktop.read_with(visual, |desktop, _| {
            (
                fixture.desktop.entity_id(),
                desktop.chat_panes.clone(),
                desktop.draft_panes.clone(),
                desktop.snapshot.clone(),
                desktop.session_lock.entity_id(),
            )
        });
    let (id, handle) = detach(&fixture, visual);
    visual.update(|_, cx| {
        fixture.desktop.read_with(cx, |desktop, cx| {
            assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("b"));
            assert_eq!(
                desktop.window.dock.entity_id(),
                fixture.docks[1].entity_id()
            );
            assert_eq!(
                desktop.parked_window_views[&id].dock.entity_id(),
                fixture.docks[0].entity_id()
            );
            assert_eq!(
                desktop.workspace_view_cache["c"].dock.entity_id(),
                fixture.docks[2].entity_id()
            );
            assert_eq!(
                Desktop::active_layout_ids(&desktop.parked_window_views[&id], cx).unwrap(),
                HashSet::from(["draft:a".into()])
            );
            assert_form(&fixture, 0, cx);
        });
        assert_eq!(
            cx.with_window(fixture.drafts[0].entity_id(), |window, _| window
                .window_handle()),
            Some(handle)
        );
    });
    handle
        .update(visual, |_, window, cx| {
            fixture.desktop.update(cx, |desktop, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    let mut assignments = desktop.workspace_window_assignments.clone();
                    assignments.reattach_workspace("a").unwrap();
                    desktop
                        .transfer_current_workspace(
                            &WorkspaceWindowId::main(),
                            assignments,
                            window,
                            cx,
                        )
                        .unwrap();
                });
            })
        })
        .unwrap();
    handle
        .update(visual, |_, _, cx| {
            assert_eq!(
                cx.with_window(fixture.drafts[0].entity_id(), |window, _| window
                    .window_handle()),
                Some(fixture.main)
            );
            fixture.desktop.read_with(cx, |desktop, cx| {
                assert_eq!(fixture.desktop.entity_id(), controller);
                assert!(Rc::ptr_eq(&desktop.chat_panes, &panes));
                assert!(Rc::ptr_eq(&desktop.draft_panes, &drafts));
                assert!(Rc::ptr_eq(&desktop.snapshot, &snapshot));
                assert_eq!(desktop.session_lock.entity_id(), lock);
                assert_eq!(
                    desktop.service.as_ref().unwrap().server_instance_id(),
                    "synthetic-ui-fixture"
                );
                assert!(
                    desktop._changes.is_none(),
                    "moving a view must not subscribe another backend"
                );
                assert!(
                    desktop.preferences_save_timer.is_some(),
                    "preference debounce stays on the single controller"
                );
                assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("a"));
                assert_eq!(
                    desktop.window.dock.entity_id(),
                    fixture.docks[0].entity_id()
                );
                assert_eq!(
                    desktop.workspace_view_cache["b"].dock.entity_id(),
                    fixture.docks[1].entity_id()
                );
                for index in 0..3 {
                    assert_form(&fixture, index, cx);
                }
            });
        })
        .unwrap();
    handle
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn workspace_window_async_callback_follows_moved_inactive_workspace(cx: &mut TestAppContext) {
    let (fixture, visual) = fixture(cx);
    let (release, wait) = futures::channel::oneshot::channel::<()>();
    let observed = Rc::new(RefCell::new(None));
    let output = observed.clone();
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            desktop
                .spawn_workspace(window, cx, async move |desktop, cx| {
                    wait.await.unwrap();
                    let result = desktop.update_in(cx, |desktop, window, cx| {
                        desktop.window.error = Some("callback for a".into());
                        *output.borrow_mut() = Some((
                            desktop.window.active_workspace_id.clone(),
                            window.window_handle(),
                        ));
                        cx.notify();
                    });
                    assert!(
                        result.is_ok(),
                        "captured workspace stays available after moving"
                    );
                })
                .detach();
        })
    });
    let (id, handle) = detach(&fixture, visual);
    // Move C to the same secondary host, making A retained but inactive there.
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            assert!(
                desktop
                    .activate_retained_workspace("c", window, cx)
                    .unwrap()
            );
            let mut assignments = desktop.workspace_window_assignments.clone();
            assignments
                .transfer_workspace("c", &id, usize::MAX)
                .unwrap();
            desktop
                .transfer_current_workspace(&id, assignments, window, cx)
                .unwrap();
        })
    });
    release.send(()).unwrap();
    visual.run_until_parked();
    assert_eq!(*observed.borrow(), Some((Some("a".into()), handle)));
    fixture.desktop.read_with(visual, |desktop, cx| {
        assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("b"));
        assert!(desktop.window.error.is_none());
        assert_eq!(
            desktop.parked_window_views[&id]
                .active_workspace_id
                .as_deref(),
            Some("c")
        );
        assert!(desktop.parked_window_views[&id].error.is_none());
        assert_eq!(
            desktop.workspace_view_cache["a"].error.as_deref(),
            Some("callback for a")
        );
        for index in 0..3 {
            assert_form(&fixture, index, cx);
        }
    });
    handle
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn workspace_window_close_returns_active_and_cached_views_before_native_parent_closes(
    cx: &mut TestAppContext,
) {
    let (fixture, visual) = fixture(cx);
    let (id, handle) = detach(&fixture, visual);
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            assert!(
                desktop
                    .activate_retained_workspace("c", window, cx)
                    .unwrap()
            );
            let mut assignments = desktop.workspace_window_assignments.clone();
            assignments
                .transfer_workspace("c", &id, usize::MAX)
                .unwrap();
            desktop
                .transfer_current_workspace(&id, assignments, window, cx)
                .unwrap();
        })
    });
    handle
        .update(visual, |_, window, cx| {
            fixture.desktop.update(cx, |desktop, cx| {
                // Close's readiness check is synchronous. Restore the offline debounce
                // guard before returning control to any executor task.
                desktop.preferences_busy = false;
                assert!(!desktop.request_workspace_window_close(&id, window, cx));
                desktop.preferences_busy = true;
            })
        })
        .unwrap();
    visual.update(|_, cx| {
        fixture.desktop.read_with(cx, |desktop, cx| {
            assert!(!desktop.window_hosts.contains_key(&id));
            assert!(!desktop.parked_window_views.contains_key(&id));
            assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("c"));
            assert_eq!(
                desktop.window.dock.entity_id(),
                fixture.docks[2].entity_id()
            );
            assert_eq!(
                desktop.workspace_view_cache["a"].dock.entity_id(),
                fixture.docks[0].entity_id()
            );
            assert_eq!(
                desktop.workspace_view_cache["b"].dock.entity_id(),
                fixture.docks[1].entity_id()
            );
            for id in ["a", "b", "c"] {
                assert_eq!(
                    desktop
                        .workspace_window_assignments
                        .window_for_workspace(id),
                    Some(&WorkspaceWindowId::main())
                );
            }
            for index in 0..3 {
                assert_form(&fixture, index, cx);
            }
        });
    });
    // Query from a separate live window: the primary cannot be entered twice.
    let probe = visual.add_window(|window, cx| Root::new(cx.new(|_| OfflineHost), window, cx));
    probe
        .update(visual, |_, _, cx| {
            for draft in &fixture.drafts {
                assert_eq!(
                    cx.with_window(draft.entity_id(), |window, _| window.window_handle()),
                    Some(fixture.main)
                );
            }
        })
        .unwrap();
    probe
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn workspace_window_duplicate_preflight_preserves_source_destination_and_cached_layouts(
    cx: &mut TestAppContext,
) {
    let (fixture, visual) = fixture(cx);
    let (assignments, id) = fixture.desktop.read_with(visual, |desktop, _| {
        let mut assignments = desktop.workspace_window_assignments.clone();
        let id = assignments.detach_workspace("b").unwrap();
        (assignments, id)
    });
    let handle = destination(&fixture, visual, &id);
    visual.update(|window, cx| {
        fixture.desktop.update(cx, |desktop, cx| {
            let mut destination = desktop.workspace_view_cache.remove("b").unwrap();
            handle
                .update(cx, |_, target, cx| {
                    Desktop::rehost_workspace_view(&mut destination, None, target, cx)
                })
                .unwrap()
                .unwrap();
            desktop.parked_window_views.insert(id.clone(), destination);
            desktop.workspace_window_assignments = assignments;
            let duplicate = fixture.drafts[0].clone();
            desktop.workspace_view_cache["c"]
                .dock
                .update(cx, |dock, cx| {
                    dock.add(
                        docking::PaneSpec {
                            id: "draft:a".into(),
                            title: "Same retained page".into(),
                            content: duplicate.clone().into(),
                            focus: duplicate.focus_handle(cx),
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
            let before = desktop.workspace_window_assignments.clone();
            let mut attempted = before.clone();
            attempted.transfer_workspace("a", &id, usize::MAX).unwrap();
            let layouts: Vec<_> = fixture
                .docks
                .iter()
                .map(|dock| dock.read(cx).serialize(cx).unwrap())
                .collect();
            let generation = desktop.preferences_generation;
            assert!(
                desktop
                    .transfer_current_workspace(&id, attempted, window, cx)
                    .is_err()
            );
            assert_eq!(desktop.workspace_window_assignments, before);
            assert_eq!(desktop.preferences_generation, generation);
            assert_eq!(desktop.window.active_workspace_id.as_deref(), Some("a"));
            assert_eq!(
                desktop.window.dock.entity_id(),
                fixture.docks[0].entity_id()
            );
            assert_eq!(
                desktop.parked_window_views[&id]
                    .active_workspace_id
                    .as_deref(),
                Some("b")
            );
            assert_eq!(
                desktop.parked_window_views[&id].dock.entity_id(),
                fixture.docks[1].entity_id()
            );
            assert!(
                desktop.window_hosts.contains_key(&id),
                "rejected transfer must not destroy a preexisting destination"
            );
            assert!(!desktop.workspace_window_transfer);
            for (dock, before) in fixture.docks.iter().zip(layouts) {
                assert_eq!(dock.read(cx).serialize(cx).unwrap(), before);
            }
            for index in 0..3 {
                assert_form(&fixture, index, cx);
            }
        })
    });
    handle
        .update(visual, |_, _, cx| {
            assert_eq!(
                cx.with_window(fixture.drafts[0].entity_id(), |window, _| window
                    .window_handle()),
                Some(fixture.main)
            );
        })
        .unwrap();
    handle
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}


// Exercise the production component through Kit, without compiling unrelated
// upstream component unit fixtures or duplicating context-menu behavior.
mod context_menu_focus_regression {
    use gpui_kit::{
        AppContext as _, Context, FocusHandle, Focusable as _, InteractiveElement as _,
        IntoElement, MouseButton, MouseDownEvent, ParentElement as _, PlatformInput, Render,
        StatefulInteractiveElement as _, Styled as _, TestAppContext, VisualTestContext, Window,
        component::{Root, menu::ContextMenuExt as _}, div, point, px,
    };
    use std::{cell::{Cell, RefCell}, rc::Rc};

    gpui_kit::actions!(workspace_window_context_menu, [CloseFixtureTab]);

    struct MenuHost {
        content_focus: FocusHandle,
        menu_focus: Rc<RefCell<Option<FocusHandle>>>,
        render_focus: Rc<RefCell<Vec<bool>>>,
        actions: Rc<Cell<usize>>,
    }

    impl Render for MenuHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Some(menu_focus) = self.menu_focus.borrow().as_ref() {
                self.render_focus.borrow_mut().push(window.focused(cx).as_ref() == Some(menu_focus));
            }
            let menu_focus = self.menu_focus.clone();
            let actions = self.actions.clone();
            div().size_full().flex().flex_col()
                .child(div().id("focused-content").role(gpui_kit::Role::TextInput)
                    .h(px(40.)).track_focus(&self.content_focus))
                .child(div().id("tab-action-bar").h(px(60.))
                    .on_action(move |_: &CloseFixtureTab, _, _| actions.set(actions.get() + 1))
                    .child(div().id("workspace-context-tab").size_full()
                        .debug_selector(|| "workspace-window-menu-trigger".into())
                        .context_menu(move |menu, _, cx| {
                            *menu_focus.borrow_mut() = Some(menu.focus_handle(cx));
                            menu.menu("Close", Box::new(CloseFixtureTab))
                        })))
        }
    }

    #[gpui_kit::test]
    fn workspace_window_context_menu_focus_precedes_draw_and_keyboard_restores_content(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let menu_focus = Rc::new(RefCell::new(None));
        let render_focus = Rc::new(RefCell::new(Vec::new()));
        let actions = Rc::new(Cell::new(0));
        let mut content_focus = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let host = cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                content_focus = Some(focus.clone());
                MenuHost {
                    content_focus: focus,
                    menu_focus: menu_focus.clone(),
                    render_focus: render_focus.clone(),
                    actions: actions.clone(),
                }
            });
            Root::new(host, window, cx)
        });
        let content_focus = content_focus.unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let press = visual.debug_bounds("workspace-window-menu-trigger")
            .expect("production context-menu trigger rendered").center();

        let open_before_draw = |visual: &mut VisualTestContext| {
            render_focus.borrow_mut().clear();
            *menu_focus.borrow_mut() = None;
            // Observe the actual focus at render entry. The test framework may
            // draw while dispatching; focus must already be settled then.
            visual.update(|window, cx| {
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Right,
                    position: point(press.x, press.y),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }), cx);
            });
            let focus = menu_focus.borrow().clone().expect("deferred menu was built");
            visual.update(|window, cx| {
                assert_eq!(window.focused(cx).as_ref(), Some(&focus));
                window.draw(cx).clear(cx);
                assert_eq!(window.focused(cx).as_ref(), Some(&focus));
            });
            let observed = render_focus.borrow();
            assert!(!observed.is_empty(), "menu frame was rendered");
            assert!(observed.iter().all(|focused| *focused), "menu must own focus before rendering starts");
        };
        open_before_draw(visual);
        visual.simulate_keystrokes("escape");
        assert_eq!(actions.get(), 0, "Escape must dismiss without running an item");
        visual.update(|window, cx| {
            assert_eq!(window.focused(cx).as_ref(), Some(&content_focus));
            window.draw(cx).clear(cx);
        });
        open_before_draw(visual);
        visual.simulate_keystrokes("down enter");
        assert_eq!(actions.get(), 1, "confirm must reach the trigger's action ancestor once");
        visual.update(|window, cx| {
            assert_eq!(window.focused(cx).as_ref(), Some(&content_focus));
        });
    }
}
