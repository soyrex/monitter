//! Ordered downstream backdrop effects in each of Fast's scene encoders.
use std::ops::Range;
use gpui::{DevicePixels, Scene, Size};
use crate::metal_renderer::{InstanceBindings, MetalRenderer, new_command_encoder_for_texture};

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_quads<'a>(
    renderer: &mut MetalRenderer,
    scene: &Scene,
    range: Range<usize>,
    bindings: &InstanceBindings,
    viewport: Size<DevicePixels>,
    commands: &'a metal::CommandBufferRef,
    texture: &'a metal::TextureRef,
    mut encoder: &'a metal::RenderCommandEncoderRef,
    mut rebind: impl FnMut(&metal::RenderCommandEncoderRef, &metal::DeviceRef),
) -> &'a metal::RenderCommandEncoderRef {
    let mut normal_start = range.start;
    for index in range.clone() {
        if scene.quads[index].background.backdrop_blur_radius() == 0. { continue; }
        renderer.draw_quads(normal_start..index, bindings, viewport, encoder);
        encoder.end_encoding();
        let snapshot = renderer.snapshot_backdrop(commands, texture);
        encoder = new_command_encoder_for_texture(commands, texture, viewport, None);
        rebind(encoder, &renderer.device);
        encoder.set_fragment_texture(0, Some(&snapshot));
        renderer.draw_quads_with_pipeline(index..index + 1, bindings, viewport, encoder,
            &renderer.backdrop_quads_pipeline_state);
        normal_start = index + 1;
    }
    renderer.draw_quads(normal_start..range.end, bindings, viewport, encoder);
    encoder
}
