use eros::Context;

use crate::infra::{WorkerReaperHandle, platform::dma_buf::DmaBufProfile};
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
    encoder_profile_provider: EncoderProfileProvider,
    encoder_profiles: Option<Vec<DmaBufProfile>>,
}

type EncoderProfileProvider = fn(drm::buffer::DrmFourcc) -> eros::Result<Vec<DmaBufProfile>>;

impl KmsScreenCaptureManagerState {
    pub(crate) fn new(
        enable_probing: bool,
        worker_reaper: WorkerReaperHandle,
        encoder_profile_provider: EncoderProfileProvider,
    ) -> Self {
        Self {
            enable_probing,
            worker_reaper,
            encoder_profile_provider,
            encoder_profiles: None,
        }
    }

    fn encoder_profiles(&mut self) -> Vec<DmaBufProfile> {
        if let Some(profiles) = &self.encoder_profiles {
            return profiles.clone();
        }

        let profiles = match (self.encoder_profile_provider)(drm::buffer::DrmFourcc::Xrgb8888) {
            Ok(profiles) => profiles,
            Err(error) => {
                tracing::debug!(
                    target: "rabbit::screen_capture::kms",
                    error = ?error,
                    "Encoder-compatible KMS capture profiles are unavailable"
                );
                Vec::new()
            }
        };
        self.encoder_profiles = Some(profiles.clone());
        profiles
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

        let state = <Deps as AsMut<KmsScreenCaptureManagerState>>::as_mut(self.prj_ref_mut());
        let enable_probing = state.enable_probing;
        let worker_reaper = state.worker_reaper.clone();
        let encoder_profiles = state.encoder_profiles();

        Ok(
            KmsCaptureLease::new(screen_name, enable_probing, worker_reaper, encoder_profiles)
                .with_context(|| context)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    static PROFILE_QUERIES: AtomicUsize = AtomicUsize::new(0);

    fn counted_empty_profiles(
        _: drm::buffer::DrmFourcc,
    ) -> eros::Result<Vec<crate::infra::platform::dma_buf::DmaBufProfile>> {
        PROFILE_QUERIES.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn resolves_encoder_profiles_lazily_once() {
        PROFILE_QUERIES.store(0, Ordering::Relaxed);
        let (reaper, reaper_handle) = WorkerReaper::new().expect("Test worker reaper should start");
        let mut state =
            KmsScreenCaptureManagerState::new(false, reaper_handle, counted_empty_profiles);

        assert_eq!(PROFILE_QUERIES.load(Ordering::Relaxed), 0);
        assert!(state.encoder_profiles().is_empty());
        assert!(state.encoder_profiles().is_empty());
        assert_eq!(PROFILE_QUERIES.load(Ordering::Relaxed), 1);

        drop(state);
        drop(reaper);
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
            capture: KmsScreenCaptureManagerState::new(false, reaper_handle, |_| Ok(Vec::new())),
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
