use std::collections::{HashMap, hash_map::Entry};

use eros::Context;

use crate::{
    infra::platform::screen_capture::kms::{
        subscription::{KmsFramePublisher, KmsFrameSubscription},
        types::{DmaBufFrame, KmsPlaneIssue},
        worker::KmsCaptureWorker,
    },
    kernel::{
        screen_capture::ScreenCaptureManager,
        screen_manager::{ScreenId, ScreenLayoutManager},
    },
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
    sources: HashMap<ScreenId, KmsCaptureSource>,
}

#[derive(Debug)]
struct KmsCaptureSource {
    worker: KmsCaptureWorker,
    frames: KmsFramePublisher,
}

impl KmsCaptureSource {
    fn new(screen_name: String) -> std::io::Result<Self> {
        Ok(Self {
            worker: KmsCaptureWorker::new(screen_name)?,
            frames: KmsFramePublisher::default(),
        })
    }

    fn subscribe(&mut self) -> KmsFrameSubscription {
        self.frames.subscribe()
    }

    async fn capture(&mut self) -> eros::Result<()> {
        let frame = self
            .worker
            .capture()
            .await
            .with_context(|| "Failed to capture a KMS source frame")?;
        self.frames.publish(frame);

        Ok(())
    }
}

impl KmsScreenCaptureManagerState {
    pub(crate) fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }
}

impl<Deps> KmsScreenCaptureManager<Deps>
where
    Deps: AsRef<KmsScreenCaptureManagerState>
        + AsMut<KmsScreenCaptureManagerState>
        + ScreenLayoutManager,
{
    fn source(&mut self, screen_id: &ScreenId) -> eros::Result<&mut KmsCaptureSource> {
        let screen_name = self
            .prj_ref()
            .screen(screen_id)
            .with_context(|| format!("Screen {} does not exist", screen_id.0))?
            .name
            .clone();

        match self.sources.entry(*screen_id) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let context =
                    format!("Failed to start KMS capture worker for screen {screen_name}");
                let source = KmsCaptureSource::new(screen_name).with_context(|| context)?;

                Ok(entry.insert(source))
            }
        }
    }
}

impl<Deps> ScreenCaptureManager for KmsScreenCaptureManager<Deps>
where
    Deps: AsRef<KmsScreenCaptureManagerState>
        + AsMut<KmsScreenCaptureManagerState>
        + ScreenLayoutManager,
{
    type Buffer = DmaBufFrame;
    type Issue = KmsPlaneIssue;
    type Subscription = KmsFrameSubscription;

    fn subscribe(&mut self, screen_id: &ScreenId) -> eros::Result<Self::Subscription> {
        Ok(self.source(screen_id)?.subscribe())
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
                .expect("test screen layout should not be empty"))
        }
    }

    #[test]
    fn subscriptions_reuse_one_source_per_physical_screen() {
        let mut deps = TestDeps {
            capture: KmsScreenCaptureManagerState::new(),
            screens: vec![screen(0, "eDP-1"), screen(1, "HDMI-A-1")],
        };
        let manager = KmsScreenCaptureManager::inj_ref_mut(&mut deps);

        let _first = manager
            .subscribe(&ScreenId(0))
            .expect("first KMS subscription should start");
        let _second = manager
            .subscribe(&ScreenId(0))
            .expect("second KMS subscription should reuse the source");

        assert_eq!(manager.sources.len(), 1);
        assert!(manager.subscribe(&ScreenId(2)).is_err());
        assert_eq!(manager.sources.len(), 1);
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
