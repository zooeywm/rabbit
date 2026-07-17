use crate::{
    infra::platform::screen_capture::kms::types::{
        KmsPlanePlacement, KmsPlaneTransform, KmsRotation,
    },
    kernel::geometry::PixelSize,
};

const FIXED_POINT_SCALE: f64 = 65_536.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct KmsCompositionTransform {
    pub position: [f32; 9],
    pub texture: [f32; 9],
}

impl KmsCompositionTransform {
    pub(crate) fn new(
        output_size: PixelSize,
        framebuffer_size: PixelSize,
        placement: KmsPlanePlacement,
    ) -> Self {
        Self {
            position: position_matrix(output_size, placement),
            texture: texture_matrix(framebuffer_size, placement),
        }
    }
}

fn position_matrix(output_size: PixelSize, placement: KmsPlanePlacement) -> [f32; 9] {
    let output_width = f64::from(output_size.width);
    let output_height = f64::from(output_size.height);
    let destination = placement.destination;
    let width = f64::from(destination.width);
    let height = f64::from(destination.height);
    let scale_x = width / output_width;
    let scale_y = height / output_height;
    let offset_x = 2.0 * f64::from(destination.x) / output_width + scale_x - 1.0;
    let offset_y = 1.0 - 2.0 * f64::from(destination.y) / output_height - scale_y;

    [
        scale_x as f32,
        0.0,
        0.0,
        0.0,
        scale_y as f32,
        0.0,
        offset_x as f32,
        offset_y as f32,
        1.0,
    ]
}

fn texture_matrix(framebuffer_size: PixelSize, placement: KmsPlanePlacement) -> [f32; 9] {
    let origin = texture_coordinate(framebuffer_size, placement, 0.0, 0.0);
    let x_axis = texture_coordinate(framebuffer_size, placement, 1.0, 0.0);
    let y_axis = texture_coordinate(framebuffer_size, placement, 0.0, 1.0);

    [
        (x_axis.0 - origin.0) as f32,
        (x_axis.1 - origin.1) as f32,
        0.0,
        (y_axis.0 - origin.0) as f32,
        (y_axis.1 - origin.1) as f32,
        0.0,
        origin.0 as f32,
        origin.1 as f32,
        1.0,
    ]
}

fn texture_coordinate(
    framebuffer_size: PixelSize,
    placement: KmsPlanePlacement,
    texture_x: f64,
    texture_y: f64,
) -> (f64, f64) {
    let (source_x, source_y) = source_coordinate(
        placement.transform,
        texture_x,
        1.0 - texture_y,
    );
    let source = placement.source;
    let framebuffer_width = f64::from(framebuffer_size.width) * FIXED_POINT_SCALE;
    let framebuffer_height = f64::from(framebuffer_size.height) * FIXED_POINT_SCALE;

    (
        (f64::from(source.x) + source_x * f64::from(source.width)) / framebuffer_width,
        (f64::from(source.y) + source_y * f64::from(source.height)) / framebuffer_height,
    )
}

fn source_coordinate(
    transform: KmsPlaneTransform,
    destination_x: f64,
    destination_y: f64,
) -> (f64, f64) {
    let (mut source_x, mut source_y) = match transform.rotation {
        KmsRotation::Rotate0 => (destination_x, destination_y),
        KmsRotation::Rotate90 => (1.0 - destination_y, destination_x),
        KmsRotation::Rotate180 => (1.0 - destination_x, 1.0 - destination_y),
        KmsRotation::Rotate270 => (destination_y, 1.0 - destination_x),
    };

    if transform.reflect_x {
        source_x = 1.0 - source_x;
    }
    if transform.reflect_y {
        source_y = 1.0 - source_y;
    }

    (source_x, source_y)
}
