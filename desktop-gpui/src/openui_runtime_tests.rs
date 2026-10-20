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
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
        StyleRefinement, Styled as _, TestAppContext, VisualTestContext, Window, div, px, size,
    };
    use std::{panic::AssertUnwindSafe, rc::Rc, sync::Arc};

    struct TranscriptHarness(Entity<crate::chat::ChatPane>);
    impl Render for TranscriptHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                self.0.clone().cached(StyleRefinement::default().size_full()),
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
    }

    // The trusted renderer's actual Input has height 38, radius 6, border 1.
    // Observe its painted border, not a source-text sentinel: Markdown fallback
    // also contains all the source strings without mounting a native form.
    fn native_input_is_painted(visual: &mut VisualTestContext) -> bool {
        visual.update(|window, _| {
            let scale = window.scale_factor();
            window.painted_quads().iter().any(|quad| {
                (quad.bounds.size.height.as_f32() / scale - 38.).abs() < 0.5
                    && (quad.corner_radii.top_left.as_f32() / scale - 6.).abs() < 0.5
                    && (quad.border_widths.top.as_f32() / scale - 1.).abs() < 0.1
                    && quad.bounds.size.width.as_f32() / scale > 120.
            })
        })
    }

    /// Run this filter alone, with --test-threads=1 --nocapture. NSH's process
    /// queue belongs to its first GPUI test app, whereas its JS engine and the
    /// host presentation-state reader run on independent real threads.
    #[::gpui::test]
    fn native_openui_form_mounts_in_real_transcript(cx: &mut TestAppContext) {
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
            let task = "openui-mounted-task";
            let source = Rc::new(json!({
                "settings":{"theme":"dark","interfaceScale":100,"interfaceFontSize":14},
                "tasks":[{"id":task,"title":"Mounted OpenUI","agentId":"fixture-agent",
                    "provider":"codex","status":"idle","queue":[]}],
                "agents":[{"id":"fixture-agent","name":"Fixture","provider":"codex"}],
                "messages":[{"id":"openui-mounted-message","taskId":task,"role":"assistant",
                    "streamStatus":"complete","createdAt":1_700_000_000_000_i64,
                    "text":"```openui\nroot = Root([Form(\"reply\", [Input(\"colour\", \"Mounted colour\", \"blue\")], \"Prepare response\")])\n```"}],
                "events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]
            }));
            let (_, visual) = cx.add_window_view(move |window, cx| {
                let pane = cx.new(|cx| {
                    crate::chat::ChatPane::performance_fixture(task.into(), source, window, cx)
                });
                gpui_kit::component::Root::new(
                    cx.new(|_| TranscriptHarness(pane)), window, cx,
                )
            });
            visual.simulate_resize(size(px(1100.), px(900.)));
            visual.update(|window, _| window.activate_window());
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                // This FIRST draw traverses TranscriptViewport -> cached virtual
                // ListState row -> StreamingMessage -> MessageOutput. It catches
                // reentrant list access during OutputBlock::set_document.
                draw(visual);
                if native_input_is_painted(visual) {
                    break;
                }
                assert!(Instant::now() < deadline,
                    "actual NSH Input never painted; source fallback alone is insufficient");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(visual.debug_bounds("retained-transcript-viewport").is_some(),
                "test must mount the real transcript viewport");
            for _ in 0..4 {
                draw(visual);
                assert!(native_input_is_painted(visual),
                    "native form must survive cached transcript replay");
            }
        }));
        std::panic::set_hook(Box::new(move |info| previous(info)));
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
}
