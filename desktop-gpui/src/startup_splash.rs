//! Retained startup cover. The owner supplies readiness; this view does no I/O.
use crate::{
    activity_sparkles::{SparkleField, SparkleLayout},
    branding, design,
    design::ui_scaled_px as px,
};
use gpui_kit::{prelude::*, *};
use serde_json::Value;
use std::time::Duration;

const FADE_DURATION: Duration = Duration::from_millis(350);

pub struct NativeStartupSplash {
    sparkles: Entity<SparkleField>,
    ready: bool,
    finished: bool,
}

impl NativeStartupSplash {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut palette = design::palette(&Value::Null, WindowAppearance::Dark);
        palette.paper = 0;
        palette.panel = 0;
        let sparkles = cx.new(|_| {
            let mut sparkles = SparkleField::new(
                "startup-logo-sparkles".into(),
                SparkleLayout::Tab,
                false,
                palette,
                crate::native_motion::motion_allowed(),
            );
            sparkles.set_visible(true);
            sparkles
        });
        Self {
            sparkles,
            ready: false,
            finished: false,
        }
    }

    /// Readiness is monotonic: after the interface is prepared, the startup
    /// cover never returns during subsequent backend refreshes.
    pub fn set_ready(&mut self, ready: bool, cx: &mut Context<Self>) {
        if !ready || self.ready || self.finished {
            return;
        }
        self.ready = true;
        if !crate::native_motion::motion_allowed() {
            self.finished = true;
            cx.notify();
            return;
        }
        cx.spawn(async move |splash, cx| {
            cx.background_executor().timer(FADE_DURATION).await;
            let _ = splash.update(cx, |splash, cx| {
                splash.finished = true;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

impl Render for NativeStartupSplash {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.finished {
            return div().into_any_element();
        }
        let cover = div()
            .id("native-startup-splash")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgb(0x000000))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .w(px(520.))
                    .max_w_full()
                    .h(px(280.))
                    .flex()
                    .items_center()
                    .justify_center()
                    // The repeating animation belongs to this retained child,
                    // keeping the preparing application shell out of its clock.
                    .child(div().absolute().inset_0().child(self.sparkles.clone()))
                    .child(img(branding::wordmark()).w(px(340.)).max_w_full()),
            );
        if self.ready {
            cover
                .with_animation(
                    "startup-cover-exit",
                    Animation::new(FADE_DURATION).with_max_fps(30.),
                    |cover, progress| cover.opacity(1. - progress),
                )
                .into_any_element()
        } else {
            cover.into_any_element()
        }
    }
}
