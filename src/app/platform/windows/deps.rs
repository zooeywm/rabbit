use crate::{
    app::App,
    infra::{
        WindowsFramePipelineManager, WindowsFramePipelineManagerState, WindowsScreenCaptureManager,
        WindowsScreenCaptureManagerState, WindowsScreenLayoutManager,
        WindowsScreenLayoutManagerState,
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

impl<ScreenLayoutManagerState, FramePipelineManagerState> AsRef<WindowsScreenCaptureManagerState>
    for App<ScreenLayoutManagerState, WindowsScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &WindowsScreenCaptureManagerState {
        &self.screen_capture_manager_state
    }
}

impl<ScreenLayoutManagerState, FramePipelineManagerState> AsMut<WindowsScreenCaptureManagerState>
    for App<ScreenLayoutManagerState, WindowsScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_mut(&mut self) -> &mut WindowsScreenCaptureManagerState {
        &mut self.screen_capture_manager_state
    }
}

impl<FramePipelineManagerState> ScreenCaptureManager
    for App<
        WindowsScreenLayoutManagerState,
        WindowsScreenCaptureManagerState,
        FramePipelineManagerState,
    >
{
    type Lease = <WindowsScreenCaptureManager<Self> as ScreenCaptureManager>::Lease;
    type Receiver = <WindowsScreenCaptureManager<Self> as ScreenCaptureManager>::Receiver;

    fn acquire(
        &mut self,
        screen_id: &ScreenId,
    ) -> eros::Result<crate::kernel::screen_capture::ScreenCaptureSource<Self::Lease, Self::Receiver>>
    {
        WindowsScreenCaptureManager::inj_ref_mut(self).acquire(screen_id)
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<WindowsFramePipelineManagerState>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, WindowsFramePipelineManagerState>
{
    fn as_ref(&self) -> &WindowsFramePipelineManagerState {
        &self.frame_pipeline_manager_state
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsMut<WindowsFramePipelineManagerState>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, WindowsFramePipelineManagerState>
{
    fn as_mut(&mut self) -> &mut WindowsFramePipelineManagerState {
        &mut self.frame_pipeline_manager_state
    }
}

impl FramePipelineManager
    for App<
        WindowsScreenLayoutManagerState,
        WindowsScreenCaptureManagerState,
        WindowsFramePipelineManagerState,
    >
{
    type Frame = <WindowsFramePipelineManager<Self> as FramePipelineManager>::Frame;
    type Subscription = <WindowsFramePipelineManager<Self> as FramePipelineManager>::Subscription;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: crate::kernel::frame_pipeline::FramePipelineParameters,
        frame_rate: crate::kernel::geometry::FrameRate,
        delivery: crate::kernel::frame_pipeline::FrameDelivery,
    ) -> eros::Result<Self::Subscription> {
        WindowsFramePipelineManager::inj_ref_mut(self)
            .subscribe(screen_id, parameters, frame_rate, delivery)
    }
}
