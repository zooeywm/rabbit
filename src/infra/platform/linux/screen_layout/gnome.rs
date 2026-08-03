use eros::Context;
use smithay_client_toolkit::{
    delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init, protocol::wl_output};

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_manager::{
        Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect, ScreenTransform,
    },
};

#[derive(Debug, kudi::DepInj)]
#[target(GnomeScreenLayoutManager)]
pub(crate) struct GnomeScreenLayoutManagerState {
    screens: Vec<Screen>,
}

impl GnomeScreenLayoutManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            screens: Self::query_screens()?,
        })
    }

    fn query_screens() -> eros::Result<Vec<Screen>> {
        let connection = Connection::connect_to_env()
            .with_context(|| "Failed to connect to the GNOME Wayland compositor")?;
        let (globals, mut event_queue) = registry_queue_init::<GnomeOutputState>(&connection)
            .with_context(|| "Failed to initialize the GNOME Wayland output registry")?;
        let queue_handle = event_queue.handle();
        let mut state = GnomeOutputState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &queue_handle),
        };

        // OutputState binds wl_output and xdg-output globals during construction.
        // One additional roundtrip delivers their current mode and logical layout.
        event_queue
            .roundtrip(&mut state)
            .with_context(|| "Failed to enumerate GNOME Wayland outputs")?;

        let outputs = state
            .output_state
            .outputs()
            .map(|output| {
                let info = state
                    .output_state
                    .info(&output)
                    .with_context(|| "GNOME returned a Wayland output without information")?;
                let current_mode = info
                    .modes
                    .iter()
                    .find(|mode| mode.current)
                    .with_context(|| {
                        format!(
                            "GNOME Wayland output {} has no current mode",
                            info.name.as_deref().unwrap_or("<unnamed>")
                        )
                    })?;

                Ok(GnomeOutput {
                    name: info.name.clone().with_context(|| {
                        format!(
                            "GNOME Wayland output {} does not expose a connector name required for KMS capture",
                            info.id
                        )
                    })?,
                    mode_width: current_mode.dimensions.0,
                    mode_height: current_mode.dimensions.1,
                    refresh_rate_millihertz: current_mode.refresh_rate,
                    logical_position: info.logical_position.unwrap_or(info.location),
                    logical_size: info.logical_size,
                    integer_scale: info.scale_factor,
                    transform: info.transform,
                })
            })
            .collect::<eros::Result<Vec<_>>>()?;

        screens_from_outputs(outputs)
    }
}

impl<Deps> ScreenLayoutManager for GnomeScreenLayoutManager<Deps>
where
    Deps: AsRef<GnomeScreenLayoutManagerState> + AsMut<GnomeScreenLayoutManagerState>,
{
    fn refresh(&mut self) -> eros::Result<()> {
        // Keep the previous complete layout if querying the compositor fails.
        let screens = GnomeScreenLayoutManagerState::query_screens()?;
        self.screens = screens;
        Ok(())
    }

    fn screens(&self) -> &[Screen] {
        &self.screens
    }

    fn screen(&self, id: &ScreenId) -> Option<&Screen> {
        self.screens.iter().find(|screen| &screen.id == id)
    }

    fn primary_screen(&self) -> eros::Result<&Screen> {
        // Wayland does not expose GNOME's primary-monitor flag. The list is
        // stable and position-sorted, matching niri's deterministic fallback.
        Ok(self
            .screens
            .first()
            .context("No screen is currently available")?)
    }
}

#[derive(Debug, Clone)]
struct GnomeOutput {
    name: String,
    mode_width: i32,
    mode_height: i32,
    refresh_rate_millihertz: i32,
    logical_position: (i32, i32),
    logical_size: Option<(i32, i32)>,
    integer_scale: i32,
    transform: wl_output::Transform,
}

fn screens_from_outputs(outputs: Vec<GnomeOutput>) -> eros::Result<Vec<Screen>> {
    if outputs.is_empty() {
        return Ok(Vec::new());
    }

    let min_x = outputs
        .iter()
        .map(|output| output.logical_position.0)
        .min()
        .context("GNOME output list became empty while calculating its layout")?;
    let min_y = outputs
        .iter()
        .map(|output| output.logical_position.1)
        .min()
        .context("GNOME output list became empty while calculating its layout")?;

    let mut mapped = outputs
        .into_iter()
        .map(|output| screen_from_output(output, min_x, min_y))
        .collect::<eros::Result<Vec<_>>>()?;

    mapped.sort_by(|left, right| {
        left.layout
            .rect
            .x
            .cmp(&right.layout.rect.x)
            .then_with(|| left.layout.rect.y.cmp(&right.layout.rect.y))
            .then_with(|| left.name.cmp(&right.name))
    });

    let supported_screen_count = usize::from(ScreenId::MAX) + 1;
    if mapped.len() > supported_screen_count {
        eros::bail!(
            "GNOME exposed more than {} mapped screens",
            supported_screen_count
        );
    }

    for (index, screen) in mapped.iter_mut().enumerate() {
        let id = u8::try_from(index).with_context(|| "Failed to assign a GNOME screen ID")?;
        screen.id = ScreenId::try_from(id)
            .with_context(|| format!("Failed to validate GNOME screen ID {id}"))?;
    }

    Ok(mapped)
}

fn screen_from_output(output: GnomeOutput, min_x: i32, min_y: i32) -> eros::Result<Screen> {
    let mode_width = u32::try_from(output.mode_width).with_context(|| {
        format!(
            "GNOME returned an invalid mode width for {}: {}",
            output.name, output.mode_width
        )
    })?;
    let mode_height = u32::try_from(output.mode_height).with_context(|| {
        format!(
            "GNOME returned an invalid mode height for {}: {}",
            output.name, output.mode_height
        )
    })?;
    if mode_width == 0 || mode_height == 0 {
        eros::bail!(
            "GNOME returned a zero-sized current mode for {}",
            output.name
        );
    }

    let refresh_rate = u32::try_from(output.refresh_rate_millihertz)
        .ok()
        .and_then(|rate| FrameRate::new(rate, 1_000))
        .with_context(|| {
            format!(
                "GNOME returned an invalid current refresh rate for {}: {} mHz",
                output.name, output.refresh_rate_millihertz
            )
        })?;
    let transform = screen_transform(output.transform)
        .with_context(|| format!("GNOME returned an unknown transform for {}", output.name))?;
    let rotated = matches!(
        transform,
        ScreenTransform::Rotate90
            | ScreenTransform::Rotate270
            | ScreenTransform::Flipped90
            | ScreenTransform::Flipped270
    );
    let transformed_size = if rotated {
        (mode_height, mode_width)
    } else {
        (mode_width, mode_height)
    };
    let (logical_width, logical_height) = logical_size(&output, transformed_size)?;
    let scale = logical_scale(&output, transformed_size, (logical_width, logical_height))?;
    let x = u32::try_from(i64::from(output.logical_position.0) - i64::from(min_x))
        .with_context(|| format!("Failed to normalize the x coordinate of {}", output.name))?;
    let y = u32::try_from(i64::from(output.logical_position.1) - i64::from(min_y))
        .with_context(|| format!("Failed to normalize the y coordinate of {}", output.name))?;

    Ok(Screen {
        // Assigned after position/name sorting.
        id: ScreenId::try_from(0).expect("zero is a valid screen ID"),
        name: output.name,
        resolution: PixelSize {
            width: mode_width,
            height: mode_height,
        },
        frame_rate: refresh_rate,
        layout: ScreenLayout {
            rect: ScreenRect {
                x,
                y,
                width: logical_width,
                height: logical_height,
            },
            scale,
            transform,
        },
    })
}

fn logical_size(output: &GnomeOutput, transformed_size: (u32, u32)) -> eros::Result<(u32, u32)> {
    if let Some((width, height)) = output.logical_size {
        let width = u32::try_from(width).with_context(|| {
            format!(
                "GNOME returned an invalid logical width for {}",
                output.name
            )
        })?;
        let height = u32::try_from(height).with_context(|| {
            format!(
                "GNOME returned an invalid logical height for {}",
                output.name
            )
        })?;
        if width == 0 || height == 0 {
            eros::bail!(
                "GNOME returned a zero-sized logical mode for {}",
                output.name
            );
        }
        return Ok((width, height));
    }

    let scale = u32::try_from(output.integer_scale)
        .ok()
        .filter(|scale| *scale > 0)
        .with_context(|| format!("GNOME returned an invalid scale for {}", output.name))?;
    Ok((
        transformed_size.0.div_ceil(scale),
        transformed_size.1.div_ceil(scale),
    ))
}

fn logical_scale(
    output: &GnomeOutput,
    transformed_size: (u32, u32),
    logical_size: (u32, u32),
) -> eros::Result<f64> {
    let scale_x = f64::from(transformed_size.0) / f64::from(logical_size.0);
    let scale_y = f64::from(transformed_size.1) / f64::from(logical_size.1);
    let tolerance = scale_x.max(scale_y) * 0.02;
    if !scale_x.is_finite()
        || !scale_y.is_finite()
        || scale_x <= 0.0
        || scale_y <= 0.0
        || (scale_x - scale_y).abs() > tolerance
    {
        eros::bail!(
            "GNOME returned inconsistent physical and logical sizes for {}: {}x{} pixels, {}x{} logical",
            output.name,
            transformed_size.0,
            transformed_size.1,
            logical_size.0,
            logical_size.1,
        );
    }
    Ok((scale_x + scale_y) / 2.0)
}

fn screen_transform(transform: wl_output::Transform) -> Option<ScreenTransform> {
    match transform {
        wl_output::Transform::Normal => Some(ScreenTransform::Normal),
        wl_output::Transform::_90 => Some(ScreenTransform::Rotate270),
        wl_output::Transform::_180 => Some(ScreenTransform::Rotate180),
        wl_output::Transform::_270 => Some(ScreenTransform::Rotate90),
        wl_output::Transform::Flipped => Some(ScreenTransform::Flipped),
        wl_output::Transform::Flipped90 => Some(ScreenTransform::Flipped270),
        wl_output::Transform::Flipped180 => Some(ScreenTransform::Flipped180),
        wl_output::Transform::Flipped270 => Some(ScreenTransform::Flipped90),
        _ => None,
    }
}

struct GnomeOutputState {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for GnomeOutputState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ProvidesRegistryState for GnomeOutputState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_output!(GnomeOutputState);
delegate_registry!(GnomeOutputState);

#[cfg(test)]
mod tests {
    use super::{GnomeOutput, screens_from_outputs};
    use crate::kernel::screen_manager::ScreenTransform;
    use wayland_client::protocol::wl_output;

    fn output(
        name: &str,
        mode: (i32, i32),
        logical_position: (i32, i32),
        logical_size: (i32, i32),
        transform: wl_output::Transform,
    ) -> GnomeOutput {
        GnomeOutput {
            name: name.to_owned(),
            mode_width: mode.0,
            mode_height: mode.1,
            refresh_rate_millihertz: 60_000,
            logical_position,
            logical_size: Some(logical_size),
            integer_scale: 1,
            transform,
        }
    }

    #[test]
    fn normalizes_negative_coordinates_and_preserves_fractional_scale() {
        let screens = screens_from_outputs(vec![
            output(
                "HDMI-A-1",
                (2560, 1440),
                (-2048, 0),
                (2048, 1152),
                wl_output::Transform::Normal,
            ),
            output(
                "eDP-1",
                (1920, 1080),
                (0, 72),
                (1536, 864),
                wl_output::Transform::Normal,
            ),
        ])
        .expect("GNOME outputs should convert");

        assert_eq!(screens[0].name, "HDMI-A-1");
        assert_eq!(screens[0].layout.rect.x, 0);
        assert_eq!(screens[0].layout.scale, 1.25);
        assert_eq!(screens[1].name, "eDP-1");
        assert_eq!(screens[1].layout.rect.x, 2048);
        assert_eq!(screens[1].layout.rect.y, 72);
        assert_eq!(screens[1].layout.scale, 1.25);
    }

    #[test]
    fn swaps_physical_axes_when_deriving_rotated_scale() {
        let screens = screens_from_outputs(vec![output(
            "DP-1",
            (2560, 1440),
            (0, 0),
            (1152, 2048),
            wl_output::Transform::_90,
        )])
        .expect("rotated GNOME output should convert");

        assert_eq!(screens[0].layout.rect.width, 1152);
        assert_eq!(screens[0].layout.rect.height, 2048);
        assert_eq!(screens[0].layout.scale, 1.25);
        assert_eq!(screens[0].layout.transform, ScreenTransform::Rotate270);
    }
}
