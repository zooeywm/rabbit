use eros::Context;
use tracing::{error, info, warn};
use winio::prelude::*;

use crate::{
    app::{App, config::Config, init_logging},
    infra::{
        NiriScreenLayoutManagerState, PendingQuicConnectionRequest, QuicEndpoint,
        QuicTransport, RayonThreadPoolState, create_screen_layout_manager_state, receive_request,
    },
};

pub(crate) struct RootComponent {
    _app: App<NiriScreenLayoutManagerState>,
    window: Child<Window>,
    connection_request_title: Child<Label>,
    connection_request_list: Child<ListBox>,
    accept_connection_button: Child<Button>,
    reject_connection_button: Child<Button>,
    pending_connection_requests: Vec<PendingQuicConnectionRequest>,
    accepted_transports: Vec<QuicTransport>,
    _connection_listener: compio::runtime::JoinHandle<()>,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
    ConnectionRequest(PendingQuicConnectionRequest),
    ConnectionRequestSelectionChanged,
    AcceptSelectedConnection,
    RejectSelectedConnection,
    ConnectionAccepted(eros::Result<QuicTransport>),
    ConnectionRejected(eros::Result<()>),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
}

impl RootComponent {
    fn set_connection_request_panel_visible(&mut self, visible: bool) -> eros::Result<()> {
        self.connection_request_title.set_visible(visible)?;
        self.connection_request_list.set_visible(visible)?;
        self.accept_connection_button.set_visible(visible)?;
        self.reject_connection_button.set_visible(visible)?;
        Ok(())
    }

    fn take_selected_connection_request(
        &mut self,
    ) -> eros::Result<Option<PendingQuicConnectionRequest>> {
        let mut selected = None;

        for index in 0..self.pending_connection_requests.len() {
            if self.connection_request_list.is_selected(index)? {
                selected = Some(index);
                break;
            }
        }

        let Some(selected) = selected else {
            return Ok(None);
        };

        self.connection_request_list.remove(selected)?;
        let request = self.pending_connection_requests.remove(selected);

        if self.pending_connection_requests.is_empty() {
            self.set_connection_request_panel_visible(false)?;
        } else {
            let next = selected.min(self.pending_connection_requests.len() - 1);
            self.connection_request_list.set_selected(next, true)?;
        }

        Ok(Some(request))
    }
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
        window.set_text(format!("Rabbit - UDP {}", local_address.port()))?;
        window.set_size(Size::new(800.0, 600.0))?;
        let mut connection_request_title = Child::<Label>::init(&window).await?;
        connection_request_title.set_text("Pending connection requests")?;
        connection_request_title.set_visible(false)?;
        let mut connection_request_list = Child::<ListBox>::init(&window).await?;
        connection_request_list.set_multiple(false)?;
        connection_request_list.set_visible(false)?;
        let mut accept_connection_button = Child::<Button>::init(&window).await?;
        accept_connection_button.set_text("Accept")?;
        accept_connection_button.set_visible(false)?;
        let mut reject_connection_button = Child::<Button>::init(&window).await?;
        reject_connection_button.set_text("Reject")?;
        reject_connection_button.set_visible(false)?;
        window.show()?;

        Ok(Self {
            _app: app,
            window,
            connection_request_title,
            connection_request_list,
            accept_connection_button,
            reject_connection_button,
            pending_connection_requests: Vec::new(),
            accepted_transports: Vec::new(),
            _connection_listener: compio::runtime::spawn(receive_connection_requests(
                quic_endpoint,
                sender.clone(),
            )),
        })
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        start! {
            sender, default: RootMessage::Noop,
            self.window => {
                WindowEvent::Close => RootMessage::Close,
            },
            self.connection_request_list => {
                ListBoxEvent::Select => RootMessage::ConnectionRequestSelectionChanged,
            },
            self.accept_connection_button => {
                ButtonEvent::Click => RootMessage::AcceptSelectedConnection,
            },
            self.reject_connection_button => {
                ButtonEvent::Click => RootMessage::RejectSelectedConnection,
            },
        }
    }

    async fn update_children(&mut self) -> eros::Result<bool> {
        let mut changed = self.window.update().await?;
        changed |= self.connection_request_title.update().await?;
        changed |= self.connection_request_list.update().await?;
        changed |= self.accept_connection_button.update().await?;
        changed |= self.reject_connection_button.update().await?;
        Ok(changed)
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
                let first_request = self.pending_connection_requests.is_empty();
                let item = format!(
                    "{} - {}",
                    request.request().requester_name,
                    request.remote_address(),
                );

                self.pending_connection_requests.push(request);
                self.connection_request_list.push(item)?;

                if first_request {
                    self.set_connection_request_panel_visible(true)?;
                    self.connection_request_list.set_selected(0, true)?;
                }

                Ok(true)
            }
            RootMessage::ConnectionRequestSelectionChanged => Ok(false),
            RootMessage::AcceptSelectedConnection => {
                let Some(request) = self.take_selected_connection_request()? else {
                    return Ok(false);
                };
                let approval_sender = sender.clone();

                compio::runtime::spawn(async move {
                    approval_sender
                        .post(RootMessage::ConnectionAccepted(request.accept().await));
                })
                .detach();

                Ok(true)
            }
            RootMessage::RejectSelectedConnection => {
                let Some(request) = self.take_selected_connection_request()? else {
                    return Ok(false);
                };
                let approval_sender = sender.clone();

                compio::runtime::spawn(async move {
                    approval_sender
                        .post(RootMessage::ConnectionRejected(request.reject().await));
                })
                .detach();

                Ok(true)
            }
            RootMessage::ConnectionAccepted(result) => {
                match result {
                    Ok(transport) => self.accepted_transports.push(transport),
                    Err(error) => error!(%error, "Failed to accept a QUIC connection request"),
                }

                Ok(false)
            }
            RootMessage::ConnectionRejected(result) => {
                if let Err(error) = result {
                    error!(%error, "Failed to reject a QUIC connection request");
                }

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

    fn render(&mut self, _sender: &ComponentSender<Self>) -> eros::Result<()> {
        let size = self.window.size()?;
        let mut actions = layout! {
            StackPanel::new(Orient::Horizontal),
            self.reject_connection_button => { grow: true },
            self.accept_connection_button => { grow: true },
        };
        let mut panel = layout! {
            StackPanel::new(Orient::Vertical),
            self.connection_request_title,
            self.connection_request_list => { grow: true },
            actions,
        };

        panel.set_size(size)?;
        Ok(())
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
