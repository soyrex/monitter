//! Keep decorative provider motion independent of chat and composer rendering.
use gpui_kit::{
    Animation, AnimationExt as _, Context, Image, IntoElement, Pixels, Render, Styled as _,
    Transformation, Window, div, ease_in_out, img, percentage,
};
#[cfg(feature = "performance-profiling")]
use gpui_kit::EntityId;
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub(super) struct WorkingLogo {
    image: Option<Arc<Image>>,
    size: Pixels,
    animate: bool,
    #[cfg(feature = "performance-profiling")]
    owner_pane: Option<EntityId>,
}

impl WorkingLogo {
    pub(super) fn set_content(
        &mut self,
        image: Option<Arc<Image>>,
        size: Pixels,
        animate: bool,
        cx: &mut Context<Self>,
        #[cfg(feature = "performance-profiling")]
        owner_pane: EntityId,
    ) {
        #[cfg(feature = "performance-profiling")]
        { self.owner_pane = Some(owner_pane); }
        let same_image = match (&self.image, &image) {
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            (None, None) => true,
            _ => false,
        };
        if same_image && self.size == size && self.animate == animate {
            return;
        }
        self.image = image;
        self.size = size;
        self.animate = animate;
        cx.notify();
    }
}

impl Render for WorkingLogo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "performance-profiling")]
        let (window_id, logo_id, owner_pane) = (window.window_handle().window_id(), cx.entity_id(), self.owner_pane);
        #[cfg(feature = "performance-profiling")]
        let static_override = crate::performance_chat::static_working_logo_override();
        #[cfg(not(feature = "performance-profiling"))]
        let static_override = false;
        let image_present = self.image.is_some();
        let animation_requested = self.animate;
        let animation_enabled = image_present && animation_requested && !static_override
            && !cx.reduce_motion() && crate::native_motion::motion_allowed();
        #[cfg(feature = "performance-profiling")]
        if let Some(owner) = owner_pane {
            crate::performance_chat::working_logo_render(
                window_id, owner, logo_id, animation_requested, animation_enabled, image_present,
            );
        }
        let Some(image) = self.image.clone() else {
            return div().into_any_element();
        };
        let image = img(image).w(self.size).h(self.size).flex_shrink_0();
        if animation_enabled {
            image
                .with_animation(
                    "pane-header-logo-spin",
                    Animation::new(Duration::from_millis(1500))
                        .repeat()
                        .with_max_fps(30.)
                        .with_easing(ease_in_out),
                    move |image, progress| {
                        #[cfg(feature = "performance-profiling")]
                        if let Some(owner) = owner_pane {
                            crate::performance_chat::animation_evaluation(window_id, owner, logo_id);
                        }
                        image.with_transformation(Transformation::rotate(percentage(progress)))
                    },
                )
                .into_any_element()
        } else {
            image.into_any_element()
        }
    }
}
