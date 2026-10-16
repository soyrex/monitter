//! Mounted native fixtures use a disconnected service; never inspect live work.
use super::{DiffDocument, GitPane, file_rows};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, ScrollDelta, ScrollWheelEvent, point, px, size};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::json;
use std::sync::Arc;
fn draw(v: &mut VisualTestContext) {
    v.run_until_parked();
    v.update(|w, cx| w.draw(cx).clear(cx));
}
fn check(cx: &mut TestAppContext, light: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
        Theme::change(
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            None,
            cx,
        );
    });
    let(pane,v)=cx.add_window_view(move|w,cx|{
        let status=json!({"repository":true,"branch":"fixture","truncated":true,"files":[
            {"path":"src/changes.rs","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":2,"deletions":1}},
            {"path":"Cargo.lock","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":999,"deletions":1}}]});
        let mut pane=GitPane::new(DesktopService::disconnected_for_ui_fixture(),&json!({"settings":{"theme":if light{"light"}else{"dark"}}}),w,cx);
        pane.task_id=Some("synthetic-git-design".into());pane.files=Arc::new(file_rows(&status));pane.file_list.reset(pane.files.len());pane.status=Some(status);
        pane.selected=Some(("src/changes.rs".into(),"unstaged".into()));
        let diff=json!({"repository":true,"text":"diff --git a/src/changes.rs b/src/changes.rs\n--- a/src/changes.rs\n+++ b/src/changes.rs\n@@ -40,3 +40,4 @@ fn review\n mod store;\n-mod previous;\n+mod changes;\n+mod plans;\n mod runner;\n@@ -80 +81 @@ fn later\n-old\n+new\n"});
        let mut diff = diff;
        diff["text"] = json!(diff["text"].as_str().unwrap().replace("+mod changes;", &format!("+{}", "LongToken".repeat(120))));
        pane.document=Arc::new(DiffDocument::parse(diff["text"].as_str().unwrap()));pane.diff=Some(diff);pane.rebuild_diff();pane
    });
    for width in [1000., 440., 360., 280.] {
        v.simulate_resize(size(px(width), px(700.)));
        draw(v);
        pane.read_with(v, |p, _| assert_eq!(p.compact_layout, width < 720.));
        for selector in ["git-summary-header", "git-mode-control", "git-diff-toolbar"] {
            let bounds = v.debug_bounds(selector).expect("responsive Git controls");
            assert!(bounds.left() >= px(0.), "{selector} left at {width}");
            assert!(
                bounds.right() <= px(width + 1.),
                "{selector} right at {width}"
            );
        }
        let status = v.debug_bounds("git-status-bar").expect("fixed bottom status bar");
        assert!(v.debug_bounds("git-status-truncated").is_some(), "truncated result qualifier stays visible");
        assert!(v.debug_bounds("git-status-partial").is_none(), "complete returned numstat is distinct from omitted files");
        assert!((status.bottom() - px(700.)).abs() <= px(1.), "status pinned to pane bottom at {width}: {status:?}");
        let mode = v.debug_bounds("git-mode-control").unwrap();
        assert!(mode.bottom() < status.top(), "mode remains above the scroll content");
        let files_mode = v.debug_bounds("git-files-mode").expect("Files label remains available");
        assert!(files_mode.size.height <= crate::design::ui_scaled_px(22.), "quiet compact Files control");
        let file = v
            .debug_bounds("git-file-row-0")
            .expect("file navigation is laid out");
        assert!(file.size.width > px(0.));
        assert!(file.right() <= px(width + 1.));
        let code = v
            .debug_bounds("git-code-row-1")
            .expect("native numbered code line");
        assert!(code.size.width > px(0.));
        assert!(code.right() <= px(width + 1.));
        v.simulate_event(ScrollWheelEvent { position: code.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-90.))), ..Default::default() });
        draw(v);
        assert_eq!(v.debug_bounds("git-status-bar").unwrap(), status, "diff scrolling must not move status at {width}");
        v.simulate_event(ScrollWheelEvent { position: code.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(90.))), ..Default::default() });
        draw(v);
        let wrapped = v.debug_bounds("git-code-row-3").expect("wrapped addition");
        assert!(wrapped.size.height > px(16.));
        assert!(wrapped.right() <= px(width + 1.));
        let collapse = v
            .debug_bounds("git-file-collapse")
            .expect("sticky file header");
        v.simulate_click(collapse.center(), Default::default());
        draw(v);
        pane.read_with(v, |p, _| assert!(!p.file_open));
        assert!(v.debug_bounds("git-code-row-1").is_none());
        let collapse = v.debug_bounds("git-file-collapse").unwrap();
        v.simulate_click(collapse.center(), Default::default());
        draw(v);
        pane.read_with(v, |p, _| assert!(p.file_open));
        assert!(v.debug_bounds("git-code-row-1").is_some());
        pane.update(v, |p, cx| {
            p.options.split = true;
            p.option_changed(cx);
        });
        draw(v);
        pane.read_with(v, |p, _| assert!(p.options.split));
        pane.update(v, |p, cx| {
            p.options.split = false;
            p.option_changed(cx);
        });
        draw(v);
    }
    pane.update(v, |p, cx| {
        p.changes_only = true;
        p.projection = Arc::new(super::collect_changes(&p.files, 24, |_, _| {
            Ok(json!({"repository":true,"text":"@@ -1 +1 @@\n-old\n+new\n"}))
        }));
        p.diff_list.reset(p.projection.rows.len());
        cx.notify();
    });
    draw(v);
    assert!(
        v.debug_bounds("git-file-row-0").is_none(),
        "whole-set view hides file navigation"
    );
    let code = v
        .debug_bounds("git-code-row-1")
        .expect("whole-set stream code");
    assert!(code.right() <= px(281.));
    pane.update(v, |p, cx| { p.selected = None; cx.notify(); });
    draw(v);
    let files_mode = v.debug_bounds("git-files-mode").expect("Files control also remains in whole-set mode");
    v.simulate_click(files_mode.center(), Default::default());
    draw(v);
    pane.read_with(v, |p, _| assert!(!p.changes_only, "real Files click returns to file navigation"));
    let lock = v
        .debug_bounds("git-file-row-1")
        .expect("generated file navigation");
    v.simulate_click(lock.center(), Default::default());
    draw(v);
    pane.read_with(v, |p, _| {
        assert_eq!(p.selected.as_ref().unwrap().0, "Cargo.lock");
        assert!(!p.generated_open);
        assert!(!p.diff_busy);
        assert!(p.diff.is_none());
    });
}
#[gpui_kit::test]
fn git_design_native_dark(cx: &mut TestAppContext) {
    check(cx, false);
}
#[gpui_kit::test]
fn git_design_native_light(cx: &mut TestAppContext) {
    check(cx, true);
}
