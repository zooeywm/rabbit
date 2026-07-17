use std::collections::{HashMap, hash_map::Entry};

use eros::Context;

use crate::{
    infra::platform::screen_capture::kms::worker::KmsCaptureWorker,
    kernel::screen_manager::{ScreenId, ScreenLayoutManager},
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
mod subscription;
mod types;
mod worker;

#[derive(Debug, kudi::DepInj)]
#[target(KmsScreenCaptureManager)]
pub(crate) struct KmsScreenCaptureManagerState {
    workers: HashMap<ScreenId, KmsCaptureWorker>,
}

impl KmsScreenCaptureManagerState {
    pub(crate) fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }
}

impl<Deps> KmsScreenCaptureManager<Deps>
where
    Deps: AsRef<KmsScreenCaptureManagerState>
        + AsMut<KmsScreenCaptureManagerState>
        + ScreenLayoutManager,
{
    pub(crate) fn worker(&mut self, screen_id: &ScreenId) -> eros::Result<&mut KmsCaptureWorker> {
        let screen_name = self
            .prj_ref()
            .screen(screen_id)
            .with_context(|| format!("Screen {} does not exist", screen_id.0))?
            .name
            .clone();

        match self.workers.entry(*screen_id) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let context =
                    format!("Failed to start KMS capture worker for screen {screen_name}");
                let worker = KmsCaptureWorker::new(screen_name).with_context(|| context)?;

                Ok(entry.insert(worker))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::platform::screen_capture::kms::{
            KmsScreenCaptureManager, KmsScreenCaptureManagerState,
        },
        kernel::{
            geometry::PixelSize,
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
                .expect("test screen layout should not be empty"))
        }
    }

    #[test]
    fn reuses_one_worker_per_physical_screen() {
        let mut deps = TestDeps {
            capture: KmsScreenCaptureManagerState::new(),
            screens: vec![screen(0, "eDP-1"), screen(1, "HDMI-A-1")],
        };
        let manager = KmsScreenCaptureManager::inj_ref_mut(&mut deps);

        let first_worker = manager
            .worker(&ScreenId(0))
            .expect("first KMS worker should start") as *const _;
        let reused_worker = manager
            .worker(&ScreenId(0))
            .expect("first KMS worker should be reused") as *const _;
        let second_worker = manager
            .worker(&ScreenId(1))
            .expect("second KMS worker should start") as *const _;

        assert_eq!(first_worker, reused_worker);
        assert_ne!(first_worker, second_worker);
        assert_eq!(manager.workers.len(), 2);
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
