//! A fading tracked scroller must not lose its ramps when layers promote.
use crate::fast::layers::policy::Decision;
use crate::{
    AppContext as _, Axis, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, ScrollEdgeFade, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    TestAppContext, Window, decode_layer_tile, div, point, px,
};

struct FadingScroller {
    scroll: ScrollHandle,
    axis: Axis,
}

impl Render for FadingScroller {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let content = div().flex_shrink_0().bg(Hsla::red());
        let viewport = div()
            .id("fading-scroller")
            .size(px(100.))
            .track_scroll(&self.scroll);
        let viewport = match self.axis {
            Axis::Vertical => viewport
                .overflow_y_scroll()
                .child(content.w(px(100.)).h(px(1000.))),
            Axis::Horizontal => viewport
                .overflow_x_scroll()
                .child(content.w(px(1000.)).h(px(100.))),
        };
        div()
            .size_full()
            .bg(crate::white())
            .child(ScrollEdgeFade::new(
                viewport,
                self.scroll.clone(),
                self.axis,
                px(16.),
            ))
    }
}

#[crate::test]
fn fading_scrollers_bypass_promotion_and_keep_edge_masks(cx: &mut TestAppContext) {
    for axis in [Axis::Vertical, Axis::Horizontal] {
        let window = cx.add_window(|_, _| FadingScroller {
            scroll: ScrollHandle::new(),
            axis,
        });
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        // A first repaint would establish a scroll tile; the following
        // composites would drop its fade. Force those choices so the test
        // covers the layer decision without relying on promotion timing.
        for (decision, offset) in [
            (Decision::Repaint, 0.),
            (Decision::Composite, -50.),
            (Decision::Composite, -900.),
        ] {
            window
                .update(cx, |view, window, cx| {
                    view.scroll.set_offset(match axis {
                        Axis::Vertical => point(px(0.), px(offset)),
                        Axis::Horizontal => point(px(offset), px(0.)),
                    });
                    window.fast_layers.forced_decision = Some(decision);
                    cx.notify();
                })
                .unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
                let scene = &window.rendered_frame.scene;
                assert!(
                    scene.polychrome_sprites.iter().all(|sprite| {
                        decode_layer_tile(sprite.tile.texture_id, sprite.tile.tile_id).is_none()
                    }),
                    "faded scroll content must stay in the direct scene"
                );
                assert!(
                    window.fast_layers.layers.is_empty(),
                    "fade reaches layer eligibility during prepaint"
                );
                let content = scene
                    .quads
                    .iter()
                    .find(|quad| quad.background == Hsla::red().into())
                    .expect("the scroll content remains painted");
                let mask = content.content_mask;
                let (start, end) = match axis {
                    Axis::Vertical => (mask.fade_height.0, mask.fade_bottom_height.0),
                    Axis::Horizontal => (mask.fade_left_height.0, mask.fade_right_height.0),
                };
                assert_eq!(
                    start > 0.,
                    offset < -0.5,
                    "leading edge follows overflow position"
                );
                assert_eq!(
                    end > 0.,
                    offset > -899.5,
                    "trailing edge follows overflow position"
                );
            })
            .unwrap();
        }
    }
}
