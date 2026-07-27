use crate::{
    app::{App, config::Config},
    infra::ConnectionEndpoint,
};

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState> AsRef<Config>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    AsRef<ConnectionEndpoint>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &ConnectionEndpoint {
        &self.connection_endpoint
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::{
        app::App,
        infra::{
            GbmFramePipelineManager, GbmFramePipelineManagerState, KmsScreenCaptureManager,
            KmsScreenCaptureManagerState, NiriScreenLayoutManager, NiriScreenLayoutManagerState,
        },
        kernel::{
            frame_pipeline::FramePipelineManager,
            screen_capture::ScreenCaptureManager,
            screen_manager::{Screen, ScreenId, ScreenLayoutManager},
        },
    };

    impl<ScreenCaptureManagerState, FramePipelineManagerState> AsRef<NiriScreenLayoutManagerState>
        for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_ref(&self) -> &NiriScreenLayoutManagerState {
            &self.screen_layout_manager_state
        }
    }

    impl<ScreenCaptureManagerState, FramePipelineManagerState> AsMut<NiriScreenLayoutManagerState>
        for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_mut(&mut self) -> &mut NiriScreenLayoutManagerState {
            &mut self.screen_layout_manager_state
        }
    }

    impl<ScreenCaptureManagerState, FramePipelineManagerState> ScreenLayoutManager
        for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn refresh(&mut self) -> eros::Result<()> {
            NiriScreenLayoutManager::inj_ref_mut(self).refresh()
        }

        fn screens(&self) -> &[Screen] {
            NiriScreenLayoutManager::inj_ref(self).screens()
        }

        fn screen(&self, id: &ScreenId) -> Option<&Screen> {
            NiriScreenLayoutManager::inj_ref(self).screen(id)
        }

        fn primary_screen(&self) -> eros::Result<&Screen> {
            NiriScreenLayoutManager::inj_ref(self).primary_screen()
        }
    }

    impl<ScreenLayoutManagerState, FramePipelineManagerState> AsRef<KmsScreenCaptureManagerState>
        for App<ScreenLayoutManagerState, KmsScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_ref(&self) -> &KmsScreenCaptureManagerState {
            &self.screen_capture_manager_state
        }
    }

    impl<ScreenLayoutManagerState, FramePipelineManagerState> AsMut<KmsScreenCaptureManagerState>
        for App<ScreenLayoutManagerState, KmsScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_mut(&mut self) -> &mut KmsScreenCaptureManagerState {
            &mut self.screen_capture_manager_state
        }
    }

    impl<FramePipelineManagerState> ScreenCaptureManager
        for App<
            NiriScreenLayoutManagerState,
            KmsScreenCaptureManagerState,
            FramePipelineManagerState,
        >
    {
        type Lease = <KmsScreenCaptureManager<Self> as ScreenCaptureManager>::Lease;
        type Receiver = <KmsScreenCaptureManager<Self> as ScreenCaptureManager>::Receiver;

        fn acquire(
            &mut self,
            screen_id: &ScreenId,
        ) -> eros::Result<
            crate::kernel::screen_capture::ScreenCaptureSource<Self::Lease, Self::Receiver>,
        > {
            KmsScreenCaptureManager::inj_ref_mut(self).acquire(screen_id)
        }
    }

    impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<GbmFramePipelineManagerState>
        for App<ScreenLayoutManagerState, ScreenCaptureManagerState, GbmFramePipelineManagerState>
    {
        fn as_ref(&self) -> &GbmFramePipelineManagerState {
            &self.frame_pipeline_manager_state
        }
    }

    impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsMut<GbmFramePipelineManagerState>
        for App<ScreenLayoutManagerState, ScreenCaptureManagerState, GbmFramePipelineManagerState>
    {
        fn as_mut(&mut self) -> &mut GbmFramePipelineManagerState {
            &mut self.frame_pipeline_manager_state
        }
    }

    impl FramePipelineManager
        for App<
            NiriScreenLayoutManagerState,
            KmsScreenCaptureManagerState,
            GbmFramePipelineManagerState,
        >
    {
        type Frame = <GbmFramePipelineManager<Self> as FramePipelineManager>::Frame;
        type Subscription = <GbmFramePipelineManager<Self> as FramePipelineManager>::Subscription;

        fn subscribe(
            &mut self,
            screen_id: &ScreenId,
            parameters: crate::kernel::frame_pipeline::FramePipelineParameters,
            frame_rate: crate::kernel::geometry::FrameRate,
        ) -> eros::Result<Self::Subscription> {
            GbmFramePipelineManager::inj_ref_mut(self).subscribe(screen_id, parameters, frame_rate)
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use crate::{
        app::App,
        infra::{
            WgcFramePipelineManager, WgcFramePipelineManagerState, WgcScreenCaptureManager,
            WgcScreenCaptureManagerState, WindowsScreenLayoutManager,
            WindowsScreenLayoutManagerState,
        },
        kernel::{
            frame_pipeline::FramePipelineManager,
            screen_capture::ScreenCaptureManager,
            screen_manager::{Screen, ScreenId, ScreenLayoutManager},
        },
    };

    impl<ScreenCaptureManagerState, FramePipelineManagerState>
        AsRef<WindowsScreenLayoutManagerState>
        for App<
            WindowsScreenLayoutManagerState,
            ScreenCaptureManagerState,
            FramePipelineManagerState,
        >
    {
        fn as_ref(&self) -> &WindowsScreenLayoutManagerState {
            &self.screen_layout_manager_state
        }
    }

    impl<ScreenCaptureManagerState, FramePipelineManagerState>
        AsMut<WindowsScreenLayoutManagerState>
        for App<
            WindowsScreenLayoutManagerState,
            ScreenCaptureManagerState,
            FramePipelineManagerState,
        >
    {
        fn as_mut(&mut self) -> &mut WindowsScreenLayoutManagerState {
            &mut self.screen_layout_manager_state
        }
    }

    impl<ScreenCaptureManagerState, FramePipelineManagerState> ScreenLayoutManager
        for App<
            WindowsScreenLayoutManagerState,
            ScreenCaptureManagerState,
            FramePipelineManagerState,
        >
    {
        fn refresh(&mut self) -> eros::Result<()> {
            WindowsScreenLayoutManager::inj_ref_mut(self).refresh()
        }

        fn screens(&self) -> &[Screen] {
            WindowsScreenLayoutManager::inj_ref(self).screens()
        }

        fn screen(&self, id: &ScreenId) -> Option<&Screen> {
            WindowsScreenLayoutManager::inj_ref(self).screen(id)
        }

        fn primary_screen(&self) -> eros::Result<&Screen> {
            WindowsScreenLayoutManager::inj_ref(self).primary_screen()
        }
    }

    impl<ScreenLayoutManagerState, FramePipelineManagerState> AsRef<WgcScreenCaptureManagerState>
        for App<ScreenLayoutManagerState, WgcScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_ref(&self) -> &WgcScreenCaptureManagerState {
            &self.screen_capture_manager_state
        }
    }

    impl<ScreenLayoutManagerState, FramePipelineManagerState> AsMut<WgcScreenCaptureManagerState>
        for App<ScreenLayoutManagerState, WgcScreenCaptureManagerState, FramePipelineManagerState>
    {
        fn as_mut(&mut self) -> &mut WgcScreenCaptureManagerState {
            &mut self.screen_capture_manager_state
        }
    }

    impl<FramePipelineManagerState> ScreenCaptureManager
        for App<
            WindowsScreenLayoutManagerState,
            WgcScreenCaptureManagerState,
            FramePipelineManagerState,
        >
    {
        type Lease = <WgcScreenCaptureManager<Self> as ScreenCaptureManager>::Lease;
        type Receiver = <WgcScreenCaptureManager<Self> as ScreenCaptureManager>::Receiver;

        fn acquire(
            &mut self,
            screen_id: &ScreenId,
        ) -> eros::Result<
            crate::kernel::screen_capture::ScreenCaptureSource<Self::Lease, Self::Receiver>,
        > {
            WgcScreenCaptureManager::inj_ref_mut(self).acquire(screen_id)
        }
    }

    impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<WgcFramePipelineManagerState>
        for App<ScreenLayoutManagerState, ScreenCaptureManagerState, WgcFramePipelineManagerState>
    {
        fn as_ref(&self) -> &WgcFramePipelineManagerState {
            &self.frame_pipeline_manager_state
        }
    }

    impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsMut<WgcFramePipelineManagerState>
        for App<ScreenLayoutManagerState, ScreenCaptureManagerState, WgcFramePipelineManagerState>
    {
        fn as_mut(&mut self) -> &mut WgcFramePipelineManagerState {
            &mut self.frame_pipeline_manager_state
        }
    }

    impl FramePipelineManager
        for App<
            WindowsScreenLayoutManagerState,
            WgcScreenCaptureManagerState,
            WgcFramePipelineManagerState,
        >
    {
        type Frame = <WgcFramePipelineManager<Self> as FramePipelineManager>::Frame;
        type Subscription = <WgcFramePipelineManager<Self> as FramePipelineManager>::Subscription;

        fn subscribe(
            &mut self,
            screen_id: &ScreenId,
            parameters: crate::kernel::frame_pipeline::FramePipelineParameters,
            frame_rate: crate::kernel::geometry::FrameRate,
        ) -> eros::Result<Self::Subscription> {
            WgcFramePipelineManager::inj_ref_mut(self).subscribe(screen_id, parameters, frame_rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{app::App, kernel::frame_pipeline::FramePipelineManager};

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use crate::{
            infra::{
                GbmFramePipelineManagerState, KmsScreenCaptureManagerState,
                NiriScreenLayoutManagerState,
            },
            kernel::screen_capture::ScreenCaptureManager,
        };

        #[test]
        fn app_exposes_the_kms_screen_capture_manager() {
            fn assert_screen_capture_manager<Manager: ScreenCaptureManager>() {}

            assert_screen_capture_manager::<
                App<
                    NiriScreenLayoutManagerState,
                    KmsScreenCaptureManagerState,
                    GbmFramePipelineManagerState,
                >,
            >();
        }

        #[test]
        fn app_exposes_the_gbm_frame_pipeline_manager_state() {
            fn assert_frame_pipeline_manager_state<State: AsRef<GbmFramePipelineManagerState>>() {}

            assert_frame_pipeline_manager_state::<
                App<
                    NiriScreenLayoutManagerState,
                    KmsScreenCaptureManagerState,
                    GbmFramePipelineManagerState,
                >,
            >();
        }

        #[test]
        fn app_exposes_the_gbm_frame_pipeline_manager() {
            fn assert_frame_pipeline_manager<Manager: FramePipelineManager>() {}

            assert_frame_pipeline_manager::<
                App<
                    NiriScreenLayoutManagerState,
                    KmsScreenCaptureManagerState,
                    GbmFramePipelineManagerState,
                >,
            >();
        }
    }

    #[cfg(target_os = "windows")]
    mod windows {
        use super::*;
        use crate::{
            infra::{
                WgcFramePipelineManagerState, WgcScreenCaptureManagerState,
                WindowsScreenLayoutManagerState,
            },
            kernel::screen_capture::ScreenCaptureManager,
        };

        #[test]
        fn app_exposes_the_wgc_screen_capture_manager() {
            fn assert_screen_capture_manager<Manager: ScreenCaptureManager>() {}

            assert_screen_capture_manager::<
                App<
                    WindowsScreenLayoutManagerState,
                    WgcScreenCaptureManagerState,
                    WgcFramePipelineManagerState,
                >,
            >();
        }

        #[test]
        fn app_exposes_the_wgc_frame_pipeline_manager_state() {
            fn assert_frame_pipeline_manager_state<State: AsRef<WgcFramePipelineManagerState>>() {}

            assert_frame_pipeline_manager_state::<
                App<
                    WindowsScreenLayoutManagerState,
                    WgcScreenCaptureManagerState,
                    WgcFramePipelineManagerState,
                >,
            >();
        }

        #[test]
        fn app_exposes_the_wgc_frame_pipeline_manager() {
            fn assert_frame_pipeline_manager<Manager: FramePipelineManager>() {}

            assert_frame_pipeline_manager::<
                App<
                    WindowsScreenLayoutManagerState,
                    WgcScreenCaptureManagerState,
                    WgcFramePipelineManagerState,
                >,
            >();
        }
    }
}
