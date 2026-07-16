use super::{App, config::Config};

use crate::{
    infra::{
        NiriScreenLayoutManager, NiriScreenLayoutManagerState, QuicEndpoint, RayonThreadPool,
        RayonThreadPoolState,
    },
    kernel::screen_manager::{Screen, ScreenId, ScreenLayoutManager},
};

impl<ScreenLayoutManagerState> App<ScreenLayoutManagerState> {
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

impl<ScreenLayoutManagerState> AsRef<Config> for App<ScreenLayoutManagerState> {
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl<ScreenLayoutManagerState> AsRef<RayonThreadPoolState> for App<ScreenLayoutManagerState> {
    fn as_ref(&self) -> &RayonThreadPoolState {
        &self.rayon_thread_pool_state
    }
}

impl<ScreenLayoutManagerState> AsRef<QuicEndpoint> for App<ScreenLayoutManagerState> {
    fn as_ref(&self) -> &QuicEndpoint {
        &self.quic_endpoint
    }
}

impl AsRef<NiriScreenLayoutManagerState> for App<NiriScreenLayoutManagerState> {
    fn as_ref(&self) -> &NiriScreenLayoutManagerState {
        &self.screen_layout_manager_state
    }
}

impl AsMut<NiriScreenLayoutManagerState> for App<NiriScreenLayoutManagerState> {
    fn as_mut(&mut self) -> &mut NiriScreenLayoutManagerState {
        &mut self.screen_layout_manager_state
    }
}

impl ScreenLayoutManager for App<NiriScreenLayoutManagerState> {
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
