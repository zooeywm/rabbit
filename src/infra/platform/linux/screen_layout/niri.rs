use eros::Context;
use niri_ipc::{Request, Response, Transform as NiriTransform, socket::Socket};

use crate::kernel::screen_manager::{
    Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect, ScreenTransform,
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
        let mapped_outputs = outputs
            .into_values()
            .filter_map(|output| {
                let logical = output.logical?;
                Some((output.name, logical))
            })
            .collect::<Vec<_>>();

        if mapped_outputs.is_empty() {
            return Ok(Vec::new());
        }

        let min_x = mapped_outputs
            .iter()
            .map(|(_, logical)| logical.x)
            .min()
            .expect("mapped_outputs is not empty");

        let min_y = mapped_outputs
            .iter()
            .map(|(_, logical)| logical.y)
            .min()
            .expect("mapped_outputs is not empty");

        let mut screens = Vec::with_capacity(mapped_outputs.len());

        for (name, logical) in mapped_outputs {
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

            screens.push(Screen {
                id: ScreenId(name),
                layout: ScreenLayout {
                    rect: ScreenRect {
                        x,
                        y,
                        width: logical.width,
                        height: logical.height,
                    },
                    scale: logical.scale,
                    transform: logical.transform.into(),
                },
            });
        }

        // Maintain deterministic ordering for enumeration and primary-screen
        // fallback selection.
        screens.sort_by(|left, right| {
            left.layout
                .rect
                .x
                .cmp(&right.layout.rect.x)
                .then_with(|| left.layout.rect.y.cmp(&right.layout.rect.y))
                .then_with(|| left.id.cmp(&right.id))
        });

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
