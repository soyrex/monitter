//! Device-local native window placement. Never changes the shared workspace profile.
use gpui_kit::*;
use serde_json::{Value, json};
const FILE: &str = "window-state.json";
fn path() -> Option<std::path::PathBuf> {
    monitter_lib::experimental_desktop::profile_dir()
        .ok()
        .map(|p| p.join(FILE))
}
fn cached() -> &'static std::sync::Mutex<Option<Value>> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<Option<Value>>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        std::sync::Mutex::new(
            path()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice(&b).ok()),
        )
    })
}
fn read() -> Option<Value> {
    cached().lock().ok()?.clone()
}
fn enqueue(value: Value, flush: bool) {
    type Write = (Value, Option<std::sync::mpsc::Sender<()>>);
    static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<Write>> = std::sync::OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Write>();
        std::thread::spawn(move || {
            while let Ok((mut value, ack)) = rx.recv() {
                let mut acknowledgements = ack.into_iter().collect::<Vec<_>>();
                for (newer, ack) in rx.try_iter() {
                    value = newer;
                    acknowledgements.extend(ack);
                }
                if let Some(path) = path() {
                    if let Some(parent) = path.parent() {
                        if std::fs::create_dir_all(parent).is_ok() {
                            let temporary = path.with_extension("json.tmp");
                            if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
                                if std::fs::write(&temporary, bytes).is_ok() {
                                    let _ = std::fs::rename(temporary, path);
                                }
                            }
                        }
                    }
                }
                for ack in acknowledgements {
                    let _ = ack.send(());
                }
            }
        });
        tx
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = sender.send((value, if flush { Some(tx) } else { None }));
    if flush {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
    }
}
pub fn restore_native(window: &Window, cx: &App) {
    let Some(saved) = read() else {
        return;
    };
    if saved["state"] == "fullscreen"
        || saved["displays"] != displays(cx)
        || saved["nativeStateBounds"] != saved["bounds"]
        || saved["nativeStateWindowMode"] != saved["state"]
    {
        return;
    }
    if let Some(bytes) = saved["nativeState"].as_array().and_then(|a| {
        a.iter()
            .map(|n| n.as_u64().filter(|n| *n <= 255).map(|n| n as u8))
            .collect::<Option<Vec<_>>>()
    }) {
        window.restore_native_window_state(&bytes);
    }
}
pub fn save_native(window: &Window, cx: &App) {
    save(window, cx);
    if let Some(bytes) = window.native_window_state() {
        if let Ok(mut saved) = cached().lock() {
            if let Some(v) = saved.as_mut() {
                v["nativeState"] = json!(bytes);
                v["nativeStateBounds"] = v["bounds"].clone();
                v["nativeStateWindowMode"] = v["state"].clone();
            }
        }
    }
    if let Some(saved) = read() {
        enqueue(saved, true);
    }
}
fn encode(b: Bounds<Pixels>) -> Value {
    json!([
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height)
    ])
}
fn decode(v: &Value) -> Option<Bounds<Pixels>> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut n = [0_f32; 4];
    for (i, v) in a.iter().enumerate() {
        n[i] = v.as_f64()? as f32;
        if !n[i].is_finite() {
            return None;
        }
    }
    if n[2] < 100. || n[3] < 100. {
        return None;
    }
    Some(Bounds::new(
        point(px(n[0]), px(n[1])),
        size(px(n[2]), px(n[3])),
    ))
}
fn displays(cx: &App) -> Value {
    let mut rows: Vec<Value> = cx.displays().iter().map(|d| json!({
        "uuid": d.uuid().ok().map(|u|u.to_string()), "bounds": encode(d.bounds()), "visible": encode(d.visible_bounds())
    })).collect();
    rows.sort_by_key(|v| v["uuid"].to_string());
    Value::Array(rows)
}
pub fn display_id(cx: &App) -> Option<DisplayId> {
    let saved = read()?;
    cx.displays()
        .iter()
        .find(|d| {
            d.uuid().ok().map(|u| u.to_string()) == saved["displayUuid"].as_str().map(str::to_owned)
        })
        .map(|d| d.id())
}
pub fn initial(cx: &App) -> Option<WindowBounds> {
    let saved = read()?;
    let mut b = decode(&saved["restoreBounds"]).or_else(|| decode(&saved["bounds"]))?;
    if saved["displays"] != displays(cx) {
        let ds = cx.displays();
        let target = ds
            .iter()
            .find(|d| {
                d.uuid().ok().map(|u| u.to_string())
                    == saved["displayUuid"].as_str().map(str::to_owned)
            })
            .cloned()
            .or_else(|| cx.primary_display())?;
        let v = target.visible_bounds();
        b.size.width = b.size.width.min(v.size.width);
        b.size.height = b.size.height.min(v.size.height);
        b.origin.x = b.origin.x.max(v.origin.x).min(v.right() - b.size.width);
        b.origin.y = b.origin.y.max(v.origin.y).min(v.bottom() - b.size.height);
    }
    Some(match saved["state"].as_str() {
        Some("fullscreen") => WindowBounds::Fullscreen(b),
        Some("maximized") => WindowBounds::Maximized(b),
        _ => WindowBounds::Windowed(b),
    })
}
pub fn save(window: &Window, cx: &App) {
    let old = read();
    let state = if window.is_fullscreen() {
        "fullscreen"
    } else if window.is_maximized() {
        "maximized"
    } else {
        "windowed"
    };
    let bounds = window.bounds();
    let restore = match window.window_bounds() {
        WindowBounds::Fullscreen(b) => b,
        _ if state == "maximized" => old
            .as_ref()
            .and_then(|s| decode(&s["restoreBounds"]))
            .unwrap_or(bounds),
        _ => bounds,
    };
    let mut value = json!({"version":1,"state":state,"bounds":encode(bounds),"restoreBounds":encode(restore),
        "displayUuid":window.display(cx).and_then(|d|d.uuid().ok()).map(|u|u.to_string()),"displays":displays(cx)});
    if old.as_ref() == Some(&value) {
        return;
    }
    if let Some(old) = old.as_ref() {
        if old["bounds"] == value["bounds"]
            && old["state"] == value["state"]
            && old["displays"] == value["displays"]
        {
            for key in ["nativeState", "nativeStateBounds", "nativeStateWindowMode"] {
                if let Some(v) = old.get(key) {
                    value[key] = v.clone();
                }
            }
        }
    }
    if let Ok(mut state) = cached().lock() {
        *state = Some(value.clone());
    }
    enqueue(value, false);
}
