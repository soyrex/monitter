//! Executes the shipped renderer both headlessly and inside the real transcript.
use nsh_core::{ApproxTextEngine, EventKind, EventPayload, MountBatch, MountItem};
use nsh_engine::{Bundle, Engine, EngineConfig, MountSink};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

const RENDERER: &str = include_str!(concat!(env!("OUT_DIR"), "/openui-renderer.js"));

enum Observation {
    Batch(MountBatch),
    Error,
}
struct Sink(mpsc::Sender<Observation>);
impl MountSink for Sink {
    fn submit(&self, batch: MountBatch) {
        let _ = self.0.send(Observation::Batch(batch));
    }
    fn report_error(&self, _: &str) {
        let _ = self.0.send(Observation::Error);
    }
}

struct Runtime {
    engine: Engine,
    rx: mpsc::Receiver<Observation>,
    bridge: Option<u32>,
    props: HashMap<String, Value>,
    texts: HashMap<u32, String>,
}
impl Runtime {
    fn start() -> Self {
        let mut bundle = Bundle::source(RENDERER.to_owned());
        bundle.policy.declared = Some(nsh_core::access::Capabilities::default());
        bundle.policy.granted = Some(nsh_core::access::Capabilities::default());
        let mut config = EngineConfig::new(bundle);
        config.ui_sets = vec![nsh_base::registry()];
        config.limits = nsh_engine::Limits {
            memory_bytes: 32 * 1024 * 1024,
            load_budget: Duration::from_secs(3),
            call_budget: Duration::from_millis(100),
            max_nodes: 1000,
            max_op_values: 16_384,
            max_canvas_ops: 1000,
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let engine =
            Engine::start(config, || ApproxTextEngine, Sink(tx)).expect("start headless renderer");
        Self {
            engine,
            rx,
            bridge: None,
            props: HashMap::new(),
            texts: HashMap::new(),
        }
    }
    fn wait(&mut self, expected: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut operations = 0;
        loop {
            if expected(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "headless renderer did not reach expected state"
            );
            self.engine.control().frame();
            match self.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Observation::Error) => {
                    panic!("trusted renderer reported a QuickJS runtime error")
                }
                Ok(Observation::Batch(batch)) => {
                    operations += batch.items.len();
                    assert!(
                        operations <= 32_768,
                        "renderer exceeded smoke-test operation bound"
                    );
                    for item in batch.items {
                        match item {
                            MountItem::Component { id, name }
                                if name.as_ref() == "monitter.openui.bridge" =>
                            {
                                self.bridge = Some(id)
                            }
                            MountItem::ComponentProps { id, props } if Some(id) == self.bridge => {
                                for (name, value) in props {
                                    if let Some(value) = value {
                                        self.props.insert(name.to_string(), value);
                                    } else {
                                        self.props.remove(name.as_ref());
                                    }
                                }
                            }
                            MountItem::Text { id, text, .. } => {
                                self.texts.insert(id, text.text.clone());
                            }
                            MountItem::Delete { id } => {
                                self.texts.remove(&id);
                            }
                            _ => {}
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("renderer stopped before expected state")
                }
            }
        }
    }
    fn status(&self, expected: &str) -> bool {
        self.props.get("status").and_then(Value::as_str) == Some(expected)
    }
    fn shows(&self, expected: &str) -> bool {
        self.texts.values().any(|text| text == expected)
    }
    fn update(&mut self, source: &str, complete: bool) {
        // Clear the observed status, so a previous commit cannot satisfy a new turn.
        self.props.remove("status");
        self.engine.control().dispatch_event(
            self.bridge.expect("bridge ready"),
            EventKind::NativeEvent,
            EventPayload::Native {
                name: "Update".into(),
                data: json!({
                    "messageId":"runtime-smoke", "blockId":0, "source":source,
                    "complete":complete, "state":{}
                }),
            },
        );
    }
}

#[test]
fn trusted_renderer_quickjs_mount_update_stream_and_reject() {
    let mut runtime = Runtime::start();
    runtime.wait(|r| r.bridge.is_some() && r.status("ready"));
    runtime.update("root = Root([Text(\"QuickJS smoke\")])", false);
    runtime.wait(|r| r.status("rendered") && r.shows("QuickJS smoke"));
    // Append to a still-open program, then finish the same identity.
    runtime.update(
        "root = Root([Text(\"QuickJS smoke\"), Text(\"Appended\")])",
        true,
    );
    // Status may remain rendered and therefore need no fresh property write.
    runtime.wait(|r| r.shows("QuickJS smoke") && r.shows("Appended"));
    runtime.update("root = Root([Browser(\"https://example.invalid\")])", true);
    runtime.wait(|r| {
        r.status("error")
            && r.props
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
    });
    assert!(
        runtime.props.get("action").is_none_or(Value::is_null),
        "unsupported output prepared an action"
    );
}

#[cfg(feature = "performance-profiling")]
mod mounted {
    use super::*;
    use ::core::prelude::v1::test;
    use ::gpui::{
        AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
        StyleRefinement, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
        size,
    };
    use std::{panic::AssertUnwindSafe, rc::Rc, sync::Arc};

    struct TranscriptHarness(Entity<crate::chat::ChatPane>);
    impl Render for TranscriptHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                self.0
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        }
    }

    fn draw(visual: &mut VisualTestContext) {
        visual.executor().advance_clock(Duration::from_millis(16));
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        assert_a11y_tree(visual, false);
    }

    // Inspect the full tree emitted by the real NSH surface, including its
    // synthetic descendants and focused-input proxy. Duplicate children also
    // fail when repeated under the SAME parent (AccessKit rejects both cases).
    fn assert_a11y_tree(visual: &mut VisualTestContext, expect_form: bool) {
        let tree: Value = visual.update(|window, _| {
            assert!(window.is_a11y_active(), "fixture must build accessibility");
            serde_json::from_str(
                &window
                    .debug_a11y_tree_json()
                    .expect("draw emitted an accessibility tree"),
            )
            .expect("valid accessibility debug JSON")
        });
        let nodes = tree["nodes"].as_object().expect("accessibility nodes");
        assert_eq!(
            tree["frame"]["node_count"].as_u64(),
            Some(nodes.len() as u64),
            "duplicate node IDs must not disappear during debug serialization"
        );
        let root = tree["root"].as_str().expect("accessibility root");
        let mut parents = HashMap::new();
        let mut ids = std::collections::HashSet::new();
        for (parent, node) in nodes {
            assert!(ids.insert(node["accesskit_id"].as_str().expect("node ID")));
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    let child = child.as_str().expect("child ID");
                    assert!(nodes.contains_key(child), "missing child {child}");
                    assert!(
                        parents.insert(child, parent.as_str()).is_none(),
                        "accessibility child {child} appears more than once; parent {parent}; tree {tree}"
                    );
                }
            }
        }
        assert!(!parents.contains_key(root), "root cannot have a parent");
        let mut seen = std::collections::HashSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            assert!(seen.insert(id), "accessibility cycle at {id}");
            if let Some(children) = nodes[id]["children"].as_array() {
                pending.extend(children.iter().map(|child| child.as_str().unwrap()));
            }
        }
        assert_eq!(seen.len(), nodes.len(), "unreachable accessibility nodes");
        if expect_form {
            for (role, label) in [
                ("TextInput", "Mounted colour"),
                ("Button", "Prepare response"),
            ] {
                assert!(
                    nodes
                        .values()
                        .any(|node| node["aria"]["role"] == role && node["aria"]["label"] == label),
                    "actual native form accessibility control missing: {role} {label}"
                );
            }
        }
    }

    // The trusted renderer's actual Input has height 38, radius 6, border 1.
    // Observe its painted border, not a source-text sentinel: Markdown fallback
    // also contains all the source strings without mounting a native form.
    fn native_input_bounds(visual: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        visual.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_quads()
                .iter()
                .find(|quad| {
                    (quad.bounds.size.height.as_f32() / scale - 38.).abs() < 0.5
                        && (quad.corner_radii.top_left.as_f32() / scale - 6.).abs() < 0.5
                        && (quad.border_widths.top.as_f32() / scale - 1.).abs() < 0.1
                        && quad.bounds.size.width.as_f32() / scale > 120.
                })
                .map(|quad| {
                    Bounds::new(
                        point(
                            px(quad.bounds.origin.x.as_f32() / scale),
                            px(quad.bounds.origin.y.as_f32() / scale),
                        ),
                        size(
                            px(quad.bounds.size.width.as_f32() / scale),
                            px(quad.bounds.size.height.as_f32() / scale),
                        ),
                    )
                })
        })
    }

    fn wait_for(
        visual: &mut VisualTestContext,
        description: &str,
        mut observed: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            draw(visual);
            if observed(visual) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "mounted OpenUI did not reach {description}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_field(
        visual: &mut VisualTestContext,
        pane: &Entity<crate::chat::ChatPane>,
        state_key: &str,
        expected: &str,
    ) {
        wait_for(visual, "edited native field bridge state", |visual| {
            assert!(
                pane.read_with(visual, |pane, cx| pane.draft(cx)).is_empty(),
                "native form typing or source updates must not alter the composer"
            );
            match crate::openui_state::read(state_key) {
                crate::openui_state::Read::Ready(Some(state)) => {
                    state["$name"].as_str() == Some(expected)
                }
                crate::openui_state::Read::Failed(error) => {
                    panic!("fixture form state failed: {error}")
                }
                _ => false,
            }
        });
    }

    fn settled_body_height(visual: &mut VisualTestContext, selector: &'static str) -> f32 {
        let mut height = 0.;
        let mut stable = 0;
        wait_for(visual, "stable native message layout", |visual| {
            let Some(bounds) = visual.debug_bounds(selector) else {
                return false;
            };
            let next = bounds.size.height.as_f32();
            if (height - next).abs() < 0.5 {
                stable += 1;
            } else {
                stable = 0;
            }
            height = next;
            // Real sleeps let the independent engine thread deliver layout commits.
            stable >= 6 && native_input_bounds(visual).is_some()
        });
        height
    }

    fn publish_stream_revision(
        pane: &Entity<crate::chat::ChatPane>,
        visual: &mut VisualTestContext,
        snapshot: &Value,
        revision: &str,
    ) {
        pane.update(visual, |pane, cx| {
            pane.sync_prepared(
                Rc::new(snapshot.clone()),
                Some(revision.into()),
                Arc::new(vec![]),
                Arc::new(HashMap::new()),
                true,
                cx,
            );
            pane.reconcile_history(snapshot["messages"].as_array().unwrap().clone(), false, cx);
        });
    }

    /// NSH's process queue belongs to its first GPUI test app. Run this fixture
    /// in a fresh process even when the parent runs every OpenUI test together.
    #[::gpui::test]
    fn native_openui_form_mounts_in_real_transcript(cx: &mut TestAppContext) {
        const CHILD_GUARD: &str = "MONITTER_OPENUI_MOUNTED_TEST_CHILD";
        match std::env::var_os(CHILD_GUARD) {
            None => {
                // Editing the mounted form writes only to this disposable profile.
                let profile = std::env::temp_dir()
                    .join(format!("monitter-openui-mounted-{}", uuid::Uuid::new_v4(),));
                std::fs::create_dir(&profile).expect("create isolated form-state profile");
                let output = std::process::Command::new(
                    std::env::current_exe().expect("locate mounted-test executable"),
                )
                .args([
                    "--exact",
                    "openui_runtime_tests::mounted::native_openui_form_mounts_in_real_transcript",
                    "--test-threads=1",
                    "--nocapture",
                ])
                .env(CHILD_GUARD, "1")
                // Test-support activation occurs before the child creates any window.
                .env("GPUI_A11Y_ACTIVE", "1")
                .env("MONITTER_GPUI_PROFILE_DIR", &profile)
                .output();
                let _ = std::fs::remove_dir_all(&profile);
                let output = output.expect("start isolated mounted OpenUI test");
                if !output.status.success() {
                    println!(
                        "mounted OpenUI child stdout:\n{}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                    println!(
                        "mounted OpenUI child stderr:\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                assert!(
                    output.status.success(),
                    "mounted OpenUI child failed: {}",
                    output.status
                );
                return;
            }
            Some(value) => assert_eq!(value, "1", "invalid mounted-test child guard"),
        }
        // Selection group markers catch/rethrow descendant panics. Preserve the
        // ORIGINAL panic site/backtrace in stdout before that unwind loses it.
        let previous: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync> =
            Arc::from(std::panic::take_hook());
        let forward = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            println!("mounted OpenUI original panic: {info}");
            println!("{}", std::backtrace::Backtrace::force_capture());
            forward(info);
        }));
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            cx.executor().allow_parking();
            cx.update(|cx| {
                gpui_kit::init(cx);
                crate::surface_blend::initialize_for_ui_fixture();
                crate::native_motion::initialize_for_ui_fixture(cx);
            });
            let task = format!("openui-mounted-task-{}", uuid::Uuid::new_v4());
            let message_id = format!("openui-mounted-message-{}", uuid::Uuid::new_v4());
            // A usable form inside an unfinished Root array. The official parser
            // can render this prefix while both the program and fence remain open.
            let prefix = "$name = \"blue\"\nroot = Root([Form(\"reply\", [Input(\"colour\", \"Mounted colour\", $name)], \"Prepare response\")";
            let appended = format!("{prefix}, Text(\"Appended content acknowledged\")])");
            let mut snapshot = json!({
                "settings":{"theme":"dark","interfaceScale":100,"interfaceFontSize":14},
                "tasks":[{"id":task,"title":"Mounted OpenUI","agentId":"fixture-agent",
                    "provider":"codex","status":"idle","queue":[]}],
                "agents":[{"id":"fixture-agent","name":"Fixture","provider":"codex"}],
                "messages":[{"id":message_id,"taskId":task,"role":"assistant",
                    "streamStatus":"streaming","createdAt":1_700_000_000_000_i64,
                    "text":format!("```openui\n{prefix}")}],
                "events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]
            });
            let state_key = json!([
                "monitter-openui-v1/monitter-native-v1/lang-core-1.0.1",
                task,
                message_id,
                0,
            ])
            .to_string();
            // One leaked selector per disposable child process matches GPUI's
            // &'static str debug lookup without introducing a production hook.
            let body_selector: &'static str =
                Box::leak(format!("message-body-{message_id}").into_boxed_str());
            let mut retained = None;
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    crate::chat::ChatPane::performance_fixture(
                        task.clone(),
                        Rc::new(snapshot.clone()),
                        window,
                        cx,
                    )
                });
                retained = Some(pane.clone());
                gpui_kit::component::Root::new(cx.new(|_| TranscriptHarness(pane)), window, cx)
            });
            let pane = retained.unwrap();
            visual.simulate_resize(size(px(1100.), px(900.)));
            visual.update(|window, _| window.activate_window());
            // This FIRST draw traverses TranscriptViewport -> cached virtual
            // ListState row -> StreamingMessage -> MessageOutput. It catches
            // reentrant list access during OutputBlock::set_document.
            wait_for(visual, "actual native Input paint", |visual| {
                native_input_bounds(visual).is_some()
            });
            assert!(
                visual
                    .debug_bounds("retained-transcript-viewport")
                    .is_some(),
                "test must mount the real transcript viewport"
            );
            assert_a11y_tree(visual, true);
            let input = native_input_bounds(visual).unwrap();
            visual.simulate_click(input.center(), Default::default());
            visual.simulate_keystrokes("cmd-a");
            visual.simulate_input("User edited");
            wait_for_field(visual, &pane, &state_key, "User edited");
            let initial_height = settled_body_height(visual, body_selector);

            // This is a true append, not replacement with a completed program.
            // Wait for its extra Text to change native layout before typing.
            snapshot["messages"][0]["text"] = json!(format!("```openui\n{appended}"));
            publish_stream_revision(&pane, visual, &snapshot, "mounted-append");
            wait_for(visual, "appended native content layout", |visual| {
                native_input_bounds(visual).is_some()
                    && visual
                        .debug_bounds(body_selector)
                        .is_some_and(|bounds| bounds.size.height.as_f32() > initial_height + 8.)
            });
            settled_body_height(visual, body_selector);
            // Do not click, Tab or move the caret again. If the entity/focus or
            // edited value was lost, this suffix will not reach the same field.
            visual.simulate_input(" after append");
            wait_for_field(visual, &pane, &state_key, "User edited after append");

            snapshot["messages"][0]["text"] = json!(format!("```openui\n{appended}\n```"));
            snapshot["messages"][0]["streamStatus"] = json!("complete");
            publish_stream_revision(&pane, visual, &snapshot, "mounted-complete");
            settled_body_height(visual, body_selector);
            visual.simulate_input(" after complete");
            wait_for_field(
                visual,
                &pane,
                &state_key,
                "User edited after append after complete",
            );
            for _ in 0..4 {
                draw(visual);
                assert_a11y_tree(visual, true);
                assert!(
                    native_input_bounds(visual).is_some(),
                    "streamed native form must survive cached transcript replay"
                );
            }
            assert!(
                pane.read_with(visual, |pane, cx| pane.draft(cx)).is_empty(),
                "streaming, native field edits and completion must never prepare/send a composer draft"
            );
            // Exercise the same two-card rows used by generated company cards.
            // Narrowing the pane must reflow cards and increase measured height.
            let prose = "Editable interfaces with long descriptions that must wrap within the available card width. ".repeat(4);
            let cards = format!(
                "root = Root([Row([Card(\"First\", [Text({0})]), Card(\"Second\", [Text({0})])]), Row([Card(\"Third\", [Text({0})]), Card(\"Fourth\", [Text({0})])])])",
                serde_json::to_string(&prose).unwrap()
            );
            snapshot["messages"][0]["text"] = json!(format!("```openui\n{cards}\n```"));
            publish_stream_revision(&pane, visual, &snapshot, "responsive-cards");
            // This stage has no Input; settle independently from the form helper.
            for _ in 0..60 {
                draw(visual);
                std::thread::sleep(Duration::from_millis(10));
            }
            let wide_height = visual.debug_bounds(body_selector).unwrap().size.height;
            visual.simulate_resize(size(px(430.), px(900.)));
            wait_for(visual, "narrow cards grow transcript height", |visual| {
                visual
                    .debug_bounds(body_selector)
                    .is_some_and(|b| b.size.height > wide_height + px(40.))
            });
            let list = pane.read_with(visual, |pane, _| {
                pane.transcripts.get(&task).unwrap().list.clone()
            });
            let before = list.logical_scroll_top();
            let viewport = visual.debug_bounds("retained-transcript-viewport").unwrap();
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: viewport.center(),
                delta: gpui::ScrollDelta::Pixels(point(px(0.), px(120.))),
                ..Default::default()
            });
            draw(visual);
            let after = list.logical_scroll_top();
            assert_ne!(
                (before.item_ix, before.offset_in_item),
                (after.item_ix, after.offset_in_item),
                "wheel over native OpenUI must move the host transcript"
            );
        }));
        std::panic::set_hook(Box::new(move |info| previous(info)));
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
}
