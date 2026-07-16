use super::{App, config::Config};

use crate::{
    infra::{
        KmsScreenCaptureManagerState, NiriScreenLayoutManager,
        NiriScreenLayoutManagerState, QuicEndpoint, RayonThreadPool, RayonThreadPoolState,
    },
    kernel::screen_manager::{Screen, ScreenId, ScreenLayoutManager},
};

impl<ScreenLayoutManagerState, ScreenCaptureManagerState>
    App<ScreenLayoutManagerState, ScreenCaptureManagerState>
{
    pub(crate) fn spawn_cpu<Task, Output>(
        &self,
        task: Task,
    ) -> futures_channel::oneshot::Receiver<Output>
    where
        Task: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        RayonThreadPool::inj_ref(self).spawn(task)
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<Config>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState>
{
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<RayonThreadPoolState>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState>
{
    fn as_ref(&self) -> &RayonThreadPoolState {
        &self.rayon_thread_pool_state
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState> AsRef<QuicEndpoint>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState>
{
    fn as_ref(&self) -> &QuicEndpoint {
        &self.quic_endpoint
    }
}

impl<ScreenCaptureManagerState> AsRef<NiriScreenLayoutManagerState>
    for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState>
{
    fn as_ref(&self) -> &NiriScreenLayoutManagerState {
        &self.screen_layout_manager_state
    }
}

impl<ScreenCaptureManagerState> AsMut<NiriScreenLayoutManagerState>
    for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState>
{
    fn as_mut(&mut self) -> &mut NiriScreenLayoutManagerState {
        &mut self.screen_layout_manager_state
    }
}

impl<ScreenCaptureManagerState> ScreenLayoutManager
    for App<NiriScreenLayoutManagerState, ScreenCaptureManagerState>
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

impl<ScreenLayoutManagerState> AsRef<KmsScreenCaptureManagerState>
    for App<ScreenLayoutManagerState, KmsScreenCaptureManagerState>
{
    fn as_ref(&self) -> &KmsScreenCaptureManagerState {
        &self.screen_capture_manager_state
    }
}

impl<ScreenLayoutManagerState> AsMut<KmsScreenCaptureManagerState>
    for App<ScreenLayoutManagerState, KmsScreenCaptureManagerState>
{
    fn as_mut(&mut self) -> &mut KmsScreenCaptureManagerState {
        &mut self.screen_capture_manager_state
    }
}
