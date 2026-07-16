use eros::Context;
use winio::prelude::*;

use crate::{
    app::{App, config::Config, init_logging},
    infra::{
        NiriScreenLayoutManagerState, RayonThreadPoolState, create_screen_layout_manager_state,
    },
};

pub(crate) struct RootComponent {
    _app: App<NiriScreenLayoutManagerState>,
    window: Child<Window>,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
}

impl Component for RootComponent {
    type Error = eros::ErrorUnion;
    type Event = ();
    type Init<'a> = ();
    type Message = RootMessage;

    async fn init(_init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> eros::Result<Self> {
        let config = Config::new()?;
        init_logging(&config)?;
        let screen_layout_manager_state = create_screen_layout_manager_state()
            .context("Failed to create the screen layout manager state")?;
        let rayon_thread_pool_state =
            RayonThreadPoolState::new().context("Failed to create the Rayon thread pool state")?;
        let mut app = App::new(config, screen_layout_manager_state, rayon_thread_pool_state);
        app.run().await?;

        let mut window = Child::<Window>::init(()).await?;
        window.set_text("Rabbit")?;
        window.set_size(Size::new(800.0, 600.0))?;
        window.show()?;

        Ok(Self { _app: app, window })
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        self.window
            .start(
                sender,
                |event| match event {
                    WindowEvent::Close => Some(RootMessage::Close),
                    _ => None,
                },
                || RootMessage::Noop,
            )
            .await
    }

    async fn update(
        &mut self,
        message: Self::Message,
        sender: &ComponentSender<Self>,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::Noop => Ok(false),
            RootMessage::Close => {
                sender.output(());
                Ok(false)
            }
        }
    }
}
