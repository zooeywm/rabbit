use crate::{
    app::App,
    infra::{
        WgcFramePipelineManager, WgcFramePipelineManagerState, WgcScreenCaptureManager,
        WgcScreenCaptureManagerState, WindowsScreenLayoutManager, WindowsScreenLayoutManagerState,
    },
    kernel::{
        frame_pipeline::FramePipelineManager,
        screen_capture::ScreenCaptureManager,
        screen_manager::{Screen, ScreenId, ScreenLayoutManager},
    },
};

impl<ScreenCaptureManagerState, FramePipelineManagerState> AsRef<WindowsScreenLayoutManagerState>
    for App<WindowsScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &WindowsScreenLayoutManagerState {
        &self.screen_layout_manager_state
    }
}

impl<ScreenCaptureManagerState, FramePipelineManagerState> AsMut<WindowsScreenLayoutManagerState>
    for App<WindowsScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_mut(&mut self) -> &mut WindowsScreenLayoutManagerState {
        &mut self.screen_layout_manager_state
    }
}

impl<ScreenCaptureManagerState, FramePipelineManagerState> ScreenLayoutManager
    for App<WindowsScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
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
    ) -> eros::Result<crate::kernel::screen_capture::ScreenCaptureSource<Self::Lease, Self::Receiver>>
    {
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
        delivery: crate::kernel::frame_pipeline::FrameDelivery,
    ) -> eros::Result<Self::Subscription> {
        WgcFramePipelineManager::inj_ref_mut(self)
            .subscribe(screen_id, parameters, frame_rate, delivery)
    }
}
