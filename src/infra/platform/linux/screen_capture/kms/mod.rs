use eros::Context;

use crate::infra::WorkerReaperHandle;
use crate::kernel::{
    screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
    screen_manager::{ScreenId, ScreenLayoutManager},
};

mod capture;
mod composition;
mod device;
mod egl_context;
mod egl_ext;
mod framebuffer;
mod gbm_allocator;
mod gl_context;
mod output;
mod types;
mod worker;

#[cfg(test)]
pub(crate) use crate::infra::platform::screen_capture::kms::worker::empty_kms_frame;
pub(crate) use crate::infra::platform::screen_capture::kms::worker::{
    KmsCaptureLease, KmsCapturedFrame, KmsFrameReceiver,
};
pub(crate) use egl_context::{EglContext, EglDmaBufImage};

#[derive(Debug, kudi::DepInj)]
#[target(KmsScreenCaptureManager)]
pub(crate) struct KmsScreenCaptureManagerState {
    enable_probing: bool,
    worker_reaper: WorkerReaperHandle,
    composition_modifiers: Vec<drm::buffer::DrmModifier>,
}

impl KmsScreenCaptureManagerState {
    pub(crate) fn new(
        enable_probing: bool,
        worker_reaper: WorkerReaperHandle,
        composition_modifiers: Vec<drm::buffer::DrmModifier>,
    ) -> Self {
        Self {
            enable_probing,
            worker_reaper,
            composition_modifiers,
        }
    }
}

impl<Deps> ScreenCaptureManager for KmsScreenCaptureManager<Deps>
where
    Deps: AsRef<KmsScreenCaptureManagerState>
        + AsMut<KmsScreenCaptureManagerState>
        + ScreenLayoutManager,
{
    type Lease = KmsCaptureLease;
    type Receiver = KmsFrameReceiver;

    fn acquire(
        &mut self,
        screen_id: &ScreenId,
    ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>> {
        let screen_name = self
            .prj_ref()
            .screen(screen_id)
            .with_context(|| format!("Screen {} does not exist", screen_id.0))?
            .name
            .clone();
        let context = format!("Failed to start KMS capture worker for screen {screen_name}");

        let state = <Deps as AsRef<KmsScreenCaptureManagerState>>::as_ref(self.prj_ref());

        Ok(KmsCaptureLease::new(
            screen_name,
            state.enable_probing,
            state.worker_reaper.clone(),
            state.composition_modifiers.clone(),
        )
        .with_context(|| context)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::{
            WorkerReaper,
            platform::screen_capture::kms::{
                KmsScreenCaptureManager, KmsScreenCaptureManagerState,
            },
        },
        kernel::{
            geometry::PixelSize,
            screen_capture::ScreenCaptureManager,
            screen_manager::{
                Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect, ScreenTransform,
            },
        },
    };

    struct TestDeps {
        capture: KmsScreenCaptureManagerState,
        screens: Vec<Screen>,
    }

    impl AsRef<KmsScreenCaptureManagerState> for TestDeps {
        fn as_ref(&self) -> &KmsScreenCaptureManagerState {
            &self.capture
        }
    }

    impl AsMut<KmsScreenCaptureManagerState> for TestDeps {
        fn as_mut(&mut self) -> &mut KmsScreenCaptureManagerState {
            &mut self.capture
        }
    }

    impl ScreenLayoutManager for TestDeps {
        fn refresh(&mut self) -> eros::Result<()> {
            Ok(())
        }

        fn screens(&self) -> &[Screen] {
            &self.screens
        }

        fn screen(&self, id: &ScreenId) -> Option<&Screen> {
            self.screens.iter().find(|screen| screen.id == *id)
        }

        fn primary_screen(&self) -> eros::Result<&Screen> {
            Ok(self
                .screens
                .first()
                .expect("Test screen layout should not be empty"))
        }
    }

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn acquires_one_owned_source_for_an_existing_screen() {
        let (reaper, reaper_handle) = WorkerReaper::new().expect("Test worker reaper should start");
        let mut deps = TestDeps {
            capture: KmsScreenCaptureManagerState::new(false, reaper_handle, Vec::new()),
            screens: vec![screen(0, "eDP-1"), screen(1, "HDMI-A-1")],
        };
        let manager = KmsScreenCaptureManager::inj_ref_mut(&mut deps);

        let source = manager
            .acquire(&ScreenId(0))
            .expect("KMS capture source should start");

        assert!(manager.acquire(&ScreenId(2)).is_err());
        drop(source);
        drop(deps);
        drop(reaper);
    }

    fn screen(id: u8, name: &str) -> Screen {
        Screen {
            id: ScreenId(id),
            name: name.to_owned(),
            resolution: PixelSize {
                width: 1920,
                height: 1080,
            },
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }
}
