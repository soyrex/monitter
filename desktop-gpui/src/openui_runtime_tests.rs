//! Executes the shipped renderer in hardened QuickJS without GPUI or a window.
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
