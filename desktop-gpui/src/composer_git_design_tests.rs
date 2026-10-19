use super::*;
use ::core::prelude::v1::test;
use gpui_kit::px as native_px;
use gpui_kit::{
    AppContext as _, TestAppContext,
    component::{Root, Theme, ThemeMode},
};
use std::{cell::RefCell, rc::Rc};

#[::core::prelude::v1::test]
fn line_totals_keep_unknown_binary_untracked_and_truncated_data_honest() {
    let status = json!({"repository":true,"truncated":true,"files":[
        {"indexStatus":"M","worktreeStatus":"M","stagedStats":{"additions":1200,"deletions":380},"unstagedStats":{"additions":4,"deletions":8}},
        {"indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":null,"deletions":null}},
        {"indexStatus":"?","worktreeStatus":"?","untracked":true}
    ]});
    assert_eq!(
        line_totals(&status),
        LineTotals {
            additions: 1204,
            deletions: 388,
            known: true,
            partial: true
        }
    );
    let legacy = json!({"files":[{"indexStatus":"M","worktreeStatus":" "}]});
    assert_eq!(
        line_totals(&legacy),
        LineTotals {
            partial: true,
            ..Default::default()
        }
    );
    assert_eq!(
        line_totals(&json!({"files":[]})),
        LineTotals {
            known: true,
            ..Default::default()
        }
    );
    assert_eq!(
        line_totals(&json!({"files":[],"truncated":true})),
        LineTotals {
            partial: true,
            ..Default::default()
        }
    );
    assert_eq!(grouped_count(1204), "1,204");
    assert_eq!(grouped_count(u64::MAX), "18,446,744,073,709,551,615");
}

fn mounted(cx: &mut TestAppContext, light: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
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
    let slot = Rc::new(RefCell::new(None));
    let output = slot.clone();
    let (_, visual) = cx.add_window_view(move |window, cx| {
        let snapshot = json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"original-task","cwd":"/fixture","hostId":"host"}]});
        let strip = cx.new(|cx| {
        let mut strip = ComposerGitStrip::new(DesktopService::disconnected_for_ui_fixture(), &snapshot, None, "original-task", window, cx);
        let palette = crate::design::palette(&json!({"theme":if light {"light"} else {"dark"}}), if light {WindowAppearance::Light} else {WindowAppearance::Dark});
        strip.composed_palette = Some(palette);
        let mut files = vec![json!({"indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":1204,"deletions":388}})];
        files.extend((0..41).map(|_| json!({"indexStatus":"M","worktreeStatus":" ","stagedStats":{"additions":0,"deletions":0}})));
        strip.status = Some(json!({"repository":true,"branch":"feature/a-very-long-branch-name-that-must-truncate","files":files}));
        strip
        });
        *output.borrow_mut() = Some(strip.clone());
        Root::new(strip, window, cx)
    });
    let strip = slot.borrow().as_ref().unwrap().clone();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let events = opened.clone();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&strip, move |_, event: &ComposerGitEvent, _| {
            let ComposerGitEvent::OpenGit { task_id } = event;
            events.borrow_mut().push(task_id.clone());
        })
    });
    for font in [14., 20., 32.] {
        visual.update(|_, cx| {
            strip.update(cx, |strip, cx| {
                strip.snapshot["settings"]["interfaceFontSize"] = json!(font);
                cx.notify();
            })
        });
        for width in [420., 280.] {
            visual.simulate_resize(size(native_px(width), native_px(180.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                visual
                    .debug_bounds("composer-git-changes-title")
                    .is_none(),
                "Git bar stays icon-and-branch only"
            );
            let branch = visual
                .debug_bounds("composer-git-branch")
                .expect("branch label mounted");
            let stats = visual
                .debug_bounds("composer-git-stats")
                .expect("real line totals mounted");
            let rail = visual
                .debug_bounds("composer-git-change-rail")
                .expect("full-width change rail");
            let button = visual
                .debug_bounds("composer-git-open")
                .expect("Git bar button");
            assert!(
                (rail.bottom() - button.bottom()).abs() <= native_px(0.5),
                "rail flush with Git bar bottom"
            );
            assert!(
                (rail.left() - button.left()).abs() <= native_px(0.5)
                    && (rail.right() - button.right()).abs() <= native_px(0.5),
                "rail spans the Git bar without side padding"
            );
            let add = visual
                .debug_bounds("composer-git-added-rail")
                .expect("added-line segment");
            let del = visual
                .debug_bounds("composer-git-deleted-rail")
                .expect("deleted-line segment");
            for bounds in [branch, stats, rail, add, del] {
                assert!(
                    bounds.left() >= native_px(-1.) && bounds.right() <= native_px(width + 1.),
                    "content stays in narrow pane: {bounds:?}"
                );
            }
            assert!(
                (add.size.width / rail.size.width - 1204. / 1592.).abs() < 0.02,
                "rail uses observed line totals"
            );
            assert!((del.right() - rail.right()).abs() <= px(1.));
            visual.simulate_click(branch.center(), Default::default());
        }
    }
    assert_eq!(
        &*opened.borrow(),
        &vec!["original-task".to_string(); 6],
        "every click keeps the original task scope"
    );
}
#[gpui_kit::test]
fn composer_git_design_dark(cx: &mut TestAppContext) {
    mounted(cx, false);
}
#[gpui_kit::test]
fn composer_git_design_light(cx: &mut TestAppContext) {
    mounted(cx, true);
}
