use eros::Context;
use niri_ipc::{Request, Response, Transform as NiriTransform, socket::Socket};

use crate::kernel::{
    geometry::PixelSize,
    screen_manager::{
        Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect,
        ScreenTransform,
    },
};

#[derive(Debug, kudi::DepInj)]
#[target(NiriScreenLayoutManager)]
pub(crate) struct NiriScreenLayoutManagerState {
    screens: Vec<Screen>,
}

impl NiriScreenLayoutManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            screens: Self::query_screens()?,
        })
    }

    fn query_screens() -> eros::Result<Vec<Screen>> {
        let mut socket = Socket::connect().context("Failed to connect to the Niri IPC socket")?;

        let reply = socket
            .send(Request::Outputs)
            .context("Failed to request outputs from Niri")?;

        let response = match reply {
            Ok(response) => response,
            Err(message) => {
                eros::bail!("Niri rejected the outputs request: {}", message);
            }
        };

        let outputs = match response {
            Response::Outputs(outputs) => outputs,
            response => {
                eros::bail!(
                    "Niri returned an unexpected response to the \
                     outputs request: {:?}",
                    response
                );
            }
        };

        // Outputs without a logical layout are disabled or unmapped.
        let mut mapped_outputs = Vec::new();

        for output in outputs.into_values() {
            let Some(logical) = output.logical else {
                continue;
            };
            let mode_index = output.current_mode.with_context(|| {
                format!("Niri mapped output {} without a current mode", output.name)
            })?;
            let mode = output.modes.get(mode_index).with_context(|| {
                format!(
                    "Niri returned invalid current mode index {mode_index} for output {}",
                    output.name
                )
            })?;

            mapped_outputs.push((
                output.name,
                logical,
                PixelSize {
                    width: u32::from(mode.width),
                    height: u32::from(mode.height),
                },
            ));
        }

        if mapped_outputs.is_empty() {
            return Ok(Vec::new());
        }

        let min_x = mapped_outputs
            .iter()
            .map(|(_, logical, _)| logical.x)
            .min()
            .expect("mapped_outputs is not empty");

        let min_y = mapped_outputs
            .iter()
            .map(|(_, logical, _)| logical.y)
            .min()
            .expect("mapped_outputs is not empty");

        let mut mapped_screens = Vec::with_capacity(mapped_outputs.len());

        for (name, logical, resolution) in mapped_outputs {
            if !logical.scale.is_finite() || logical.scale <= 0.0 {
                eros::bail!(
                    "Niri returned an invalid scale for screen \
                     {name}: {}",
                    logical.scale,
                );
            }

            let x = u32::try_from(i64::from(logical.x) - i64::from(min_x)).with_context(|| {
                format!("Failed to normalize the x coordinate of screen {name}")
            })?;

            let y = u32::try_from(i64::from(logical.y) - i64::from(min_y)).with_context(|| {
                format!("Failed to normalize the y coordinate of screen {name}")
            })?;

            mapped_screens.push((
                name,
                resolution,
                ScreenLayout {
                    rect: ScreenRect {
                        x,
                        y,
                        width: logical.width,
                        height: logical.height,
                    },
                    scale: logical.scale,
                    transform: logical.transform.into(),
                },
            ));
        }

        // Maintain deterministic ordering for enumeration and primary-screen
        // fallback selection.
        mapped_screens.sort_by(|left, right| {
            left.2
                .rect
                .x
                .cmp(&right.2.rect.x)
                .then_with(|| left.2.rect.y.cmp(&right.2.rect.y))
                .then_with(|| left.0.cmp(&right.0))
        });

        if mapped_screens.len() > usize::from(u8::MAX) {
            eros::bail!("Niri exposed more than 255 mapped screens");
        }

        let mut screens = Vec::with_capacity(mapped_screens.len());

        for (index, (name, resolution, layout)) in mapped_screens.into_iter().enumerate() {
            let id = u8::try_from(index)
                .with_context(|| "Failed to assign a logical Niri screen ID")?;

            screens.push(Screen {
                id: ScreenId(id),
                name,
                resolution,
                layout,
            });
        }

        Ok(screens)
    }
}

impl<Deps> ScreenLayoutManager for NiriScreenLayoutManager<Deps>
where
    Deps: AsRef<NiriScreenLayoutManagerState> + AsMut<NiriScreenLayoutManagerState>,
{
    fn refresh(&mut self) -> eros::Result<()> {
        // Build the complete replacement first so a failed refresh leaves
        // the previous valid layout untouched.
        let screens = NiriScreenLayoutManagerState::query_screens()?;
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
        // Niri does not expose a primary output. The list is sorted by x,
        // then y, then id, so the first entry is the deterministic fallback.
        Ok(self
            .screens
            .first()
            .context("No screen is currently available")?)
    }
}

impl From<NiriTransform> for ScreenTransform {
    fn from(transform: NiriTransform) -> Self {
        match transform {
            NiriTransform::Normal => Self::Normal,
            NiriTransform::_90 => Self::Rotate270,
            NiriTransform::_180 => Self::Rotate180,
            NiriTransform::_270 => Self::Rotate90,
            NiriTransform::Flipped => Self::Flipped,
            NiriTransform::Flipped90 => Self::Flipped270,
            NiriTransform::Flipped180 => Self::Flipped180,
            NiriTransform::Flipped270 => Self::Flipped90,
        }
    }
}
