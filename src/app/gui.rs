use std::collections::VecDeque;

use eros::Context;
use tracing::{error, info, warn};
use winio::prelude::*;

use crate::{
    app::{App, config::Config, init_logging},
    infra::{
        NiriScreenLayoutManagerState, PendingQuicConnectionRequest, QuicEndpoint,
        RayonThreadPoolState, create_screen_layout_manager_state, receive_request,
    },
};

pub(crate) struct RootComponent {
    _app: App<NiriScreenLayoutManagerState>,
    window: Child<Window>,
    pending_connection_requests: VecDeque<PendingQuicConnectionRequest>,
    _connection_listener: compio::runtime::JoinHandle<()>,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
    ConnectionRequest(PendingQuicConnectionRequest),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
}

impl Component for RootComponent {
    type Error = eros::ErrorUnion;
    type Event = ();
    type Init<'a> = ();
    type Message = RootMessage;

    async fn init(_init: Self::Init<'_>, sender: &ComponentSender<Self>) -> eros::Result<Self> {
        let config = Config::new()?;
        init_logging(&config)?;
        let screen_layout_manager_state = create_screen_layout_manager_state()
            .context("Failed to create the screen layout manager state")?;
        let rayon_thread_pool_state =
            RayonThreadPoolState::new().context("Failed to create the Rayon thread pool state")?;
        let quic_endpoint = QuicEndpoint::new()
            .await
            .context("Failed to create the QUIC endpoint")?;
        let local_address = quic_endpoint.local_address()?;

        info!(%local_address, "Listening for direct QUIC connections");

        let mut app = App::new(
            config,
            screen_layout_manager_state,
            rayon_thread_pool_state,
            quic_endpoint.clone(),
        );
        app.run().await?;

        let mut window = Child::<Window>::init(()).await?;
        window.set_text("Rabbit")?;
        window.set_size(Size::new(800.0, 600.0))?;
        window.show()?;

        Ok(Self {
            _app: app,
            window,
            pending_connection_requests: VecDeque::new(),
            _connection_listener: compio::runtime::spawn(receive_connection_requests(
                quic_endpoint,
                sender.clone(),
            )),
        })
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
            RootMessage::ConnectionRequest(request) => {
                self.pending_connection_requests.push_back(request);
                Ok(false)
            }
            RootMessage::ConnectionRequestFailed(error) => {
                warn!(%error, "Failed to receive a QUIC connection request");
                Ok(false)
            }
            RootMessage::ConnectionListenerFailed(error) => {
                error!(%error, "QUIC connection listener stopped");
                Ok(false)
            }
        }
    }
}

async fn receive_connection_requests(
    endpoint: QuicEndpoint,
    sender: ComponentSender<RootComponent>,
) {
    loop {
        let connection = match endpoint.accept_connection().await {
            Ok(Some(connection)) => connection,
            Ok(None) => return,
            Err(error) => {
                sender.post(RootMessage::ConnectionListenerFailed(error));
                return;
            }
        };

        match receive_request(connection).await {
            Ok(request) => sender.post(RootMessage::ConnectionRequest(request)),
            Err(error) => sender.post(RootMessage::ConnectionRequestFailed(error)),
        }
    }
}
