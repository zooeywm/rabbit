use std::net::{IpAddr, SocketAddr};

use eros::Context;
use tracing::{error, info, warn};
use winio::prelude::*;

use crate::{
    app::{App, config::Config, init_logging},
    infra::{
        NiriScreenLayoutManagerState, PendingQuicConnectionRequest, QuicEndpoint,
        QuicTransport, RayonThreadPoolState, connect_transport,
        create_screen_layout_manager_state, receive_request,
    },
    kernel::{
        connection_request::ConnectionRequest,
        screen_manager::ScreenLayoutManager,
        session::{Session, SessionId, SessionRole},
    },
};

pub(crate) struct RootComponent {
    _app: App<NiriScreenLayoutManagerState>,
    window: Child<Window>,
    direct_address_input: Child<Edit>,
    connect_button: Child<Button>,
    connection_status: Child<Label>,
    requester_name: String,
    connection_request_title: Child<Label>,
    connection_request_list: Child<ListBox>,
    accept_connection_button: Child<Button>,
    reject_connection_button: Child<Button>,
    pending_connection_requests: Vec<PendingQuicConnectionRequest>,
    sessions: Vec<Session<QuicTransport>>,
    next_session_id: u32,
    _connection_listener: compio::runtime::JoinHandle<()>,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
    ConnectDirect,
    DirectConnectionFinished(eros::Result<Option<QuicTransport>>),
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
    fn next_session_id(&mut self) -> eros::Result<SessionId> {
        let id = SessionId(self.next_session_id);
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .context("Failed to allocate a Session ID")?;

        Ok(id)
    }

    fn parse_direct_target(&self) -> eros::Result<(IpAddr, Option<u16>)> {
        let input = self.direct_address_input.text()?;
        let input = input.trim();

        if let Ok(address) = input.parse::<SocketAddr>() {
            return Ok((address.ip(), Some(address.port())));
        }

        let ip = input
            .parse::<IpAddr>()
            .with_context(|| format!("Failed to parse direct connection IP {input:?}"))?;

        Ok((ip, None))
    }

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
        let requester_name = format!("{} ({})", config.app_name, local_address.port());

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
        let mut direct_address_input = Child::<Edit>::init(&window).await?;
        direct_address_input.set_text("127.0.0.1")?;
        let mut connect_button = Child::<Button>::init(&window).await?;
        connect_button.set_text("Connect")?;
        let mut connection_status = Child::<Label>::init(&window).await?;
        connection_status.set_text(format!("Listening on UDP {}", local_address.port()))?;
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
            direct_address_input,
            connect_button,
            connection_status,
            requester_name,
            connection_request_title,
            connection_request_list,
            accept_connection_button,
            reject_connection_button,
            pending_connection_requests: Vec::new(),
            sessions: Vec::new(),
            next_session_id: 0,
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
            self.connect_button => {
                ButtonEvent::Click => RootMessage::ConnectDirect,
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
        changed |= self.direct_address_input.update().await?;
        changed |= self.connect_button.update().await?;
        changed |= self.connection_status.update().await?;
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
            RootMessage::ConnectDirect => {
                let (remote_ip, remote_port) = match self.parse_direct_target() {
                    Ok(target) => target,
                    Err(error) => {
                        self.connection_status
                            .set_text(format!("Invalid address: {error}"))?;
                        return Ok(true);
                    }
                };
                let endpoint: &QuicEndpoint = self._app.as_ref();
                let endpoint = endpoint.clone();
                let request = ConnectionRequest {
                    requester_name: self.requester_name.clone(),
                };
                let connection_sender = sender.clone();

                self.connect_button.set_enabled(false)?;
                self.connection_status.set_text("Connecting...")?;

                compio::runtime::spawn(async move {
                    let result =
                        connect_transport(&endpoint, remote_ip, remote_port, request).await;
                    connection_sender.post(RootMessage::DirectConnectionFinished(result));
                })
                .detach();

                Ok(true)
            }
            RootMessage::DirectConnectionFinished(result) => {
                self.connect_button.set_enabled(true)?;

                match result {
                    Ok(Some(transport)) => {
                        let id = self.next_session_id()?;
                        self.sessions.push(Session::new(
                            id,
                            SessionRole::Controller,
                            transport,
                        ));
                        self.connection_status.set_text("Connection accepted")?;
                    }
                    Ok(None) => self.connection_status.set_text("Connection rejected")?,
                    Err(error) => self
                        .connection_status
                        .set_text(format!("Connection failed: {error}"))?,
                }

                Ok(true)
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
                    Ok(transport) => {
                        let id = self.next_session_id()?;
                        let session = Session::new(id, SessionRole::Host, transport);

                        match session.send_screen_list(self._app.screens()).await {
                            Ok(()) => self.sessions.push(session),
                            Err(error) => {
                                error!(%error, "Failed to send the initial screen list")
                            }
                        }
                    }
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
        let mut direct_connection = layout! {
            StackPanel::new(Orient::Horizontal),
            self.direct_address_input => { grow: true },
            self.connect_button,
        };
        let mut actions = layout! {
            StackPanel::new(Orient::Horizontal),
            self.reject_connection_button => { grow: true },
            self.accept_connection_button => { grow: true },
        };
        let mut panel = layout! {
            StackPanel::new(Orient::Vertical),
            direct_connection,
            self.connection_status,
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
