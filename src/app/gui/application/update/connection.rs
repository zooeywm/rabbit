use tracing::{error, info, warn};

use crate::app::{
    gui::{
        application::{
            RootApplication,
            message::{MessageSender, PendingHostSession, RootMessage},
        },
        state::{DirectConnectionCompletion, DirectTarget},
    },
    platform::ApplicationStack,
};
use crate::{
    infra::{ConnectionEndpoint, DirectConnectionOutcome, connect_transport},
    kernel::{
        connection_request::ConnectionRequest,
        screen_manager::ScreenLayoutManager,
        session::{Session, SessionRole},
        session_control::OutgoingScreenList,
    },
};

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) async fn handle_connection(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::ResetDirectConnection => {
                self.listener.direct_connection.reset();
                self.workspace.status_message.clear();
                Ok(true)
            }
            RootMessage::ConnectDirect(input) => {
                if self.listener.direct_connection.is_connecting() {
                    self.set_connection_status("Connection already in progress");
                    return Ok(true);
                }

                let target = match DirectTarget::parse(&input) {
                    Ok(target) => target,
                    Err(error) => {
                        self.set_connection_status(format!("Invalid address: {error}"));
                        return Ok(true);
                    }
                };
                if target.ip().is_some_and(|remote_ip| {
                    self.model.has_controller_session(remote_ip, target.port())
                }) {
                    self.set_connection_status("Session already connected");
                    return Ok(true);
                }
                if !self.listener.direct_connection.begin(target.clone()) {
                    self.set_connection_status("Connection already in progress");
                    return Ok(true);
                }
                let remote_host = target.host().to_string();
                let remote_port = target.port();
                let endpoint: &ConnectionEndpoint = self.model.app.as_ref();
                let endpoint = endpoint.clone();
                let screen_count = self.model.app.screens().len();
                let request = ConnectionRequest::local(
                    self.model.requester_name.clone(),
                    crate::kernel::connection_request::PeerCapabilities::local_host(screen_count),
                );
                let connection_sender = sender.clone();

                info!(
                    event = "direct_connection_started",
                    %remote_host,
                    ?remote_port,
                    protocol_major = request.protocol_major,
                    protocol_minor = request.protocol_minor,
                    "Direct connection started"
                );
                compio::runtime::spawn(async move {
                    let result =
                        connect_transport(&endpoint, remote_host, remote_port, request).await;
                    connection_sender.post(RootMessage::DirectConnectionFinished(result));
                })
                .detach();

                Ok(true)
            }
            RootMessage::DirectConnectionFinished(result) => {
                match result {
                    Ok(DirectConnectionOutcome::Connected {
                        transport,
                        host_capabilities,
                    }) => {
                        let peer_address = transport.remote_address();
                        self.listener
                            .direct_connection
                            .complete(DirectConnectionCompletion::Connected(peer_address));
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Controller, transport);
                        let (send, recv) = session.split();

                        self.start_session(
                            peer_address,
                            None,
                            host_capabilities,
                            send,
                            recv,
                            sender,
                        )
                        .await;
                    }
                    Ok(DirectConnectionOutcome::Rejected) => {
                        self.listener
                            .direct_connection
                            .complete(DirectConnectionCompletion::Rejected);
                    }
                    Ok(DirectConnectionOutcome::SelfConnection) => {
                        self.listener
                            .direct_connection
                            .complete(DirectConnectionCompletion::SelfRejected);
                    }
                    Ok(DirectConnectionOutcome::ProtocolMismatch {
                        peer_major,
                        peer_minor,
                    }) => {
                        self.listener.direct_connection.complete(
                            DirectConnectionCompletion::Failed(format!(
                                "Protocol mismatch (local {peer_major}.{peer_minor}; peer requires a compatible major version)"
                            )),
                        );
                        self.set_connection_status(format!(
                            "Protocol version incompatible with remote (local {peer_major}.{peer_minor})"
                        ));
                    }
                    Err(error) => {
                        self.listener
                            .direct_connection
                            .complete(DirectConnectionCompletion::Failed(error.to_string()));
                    }
                }
                Ok(true)
            }
            RootMessage::ConnectionRequest(request) => {
                self.model.pending_connection_requests.push(request);
                Ok(true)
            }
            RootMessage::AcceptConnectionRequest(index) => {
                let Some(request) = self.take_connection_request(index) else {
                    return Ok(false);
                };
                let peer_name = request.request().requester_name.clone();
                let peer_capabilities = request.request().capabilities.clone();
                let approval_sender = sender.clone();

                info!(
                    event = "connection_request_decided",
                    remote_address = %request.remote_address(),
                    requester_name = %request.request().requester_name,
                    decision = "accepted",
                    "Connection request decided"
                );
                let host_capabilities = self.model.local_capabilities.clone();
                compio::runtime::spawn(async move {
                    approval_sender.post(RootMessage::ConnectionAccepted {
                        peer_name,
                        peer_capabilities,
                        result: request.accept(host_capabilities).await,
                    });
                })
                .detach();

                Ok(true)
            }
            RootMessage::RejectConnectionRequest(index) => {
                let Some(request) = self.take_connection_request(index) else {
                    return Ok(false);
                };
                let approval_sender = sender.clone();

                info!(
                    event = "connection_request_decided",
                    remote_address = %request.remote_address(),
                    requester_name = %request.request().requester_name,
                    decision = "rejected",
                    "Connection request decided"
                );
                compio::runtime::spawn(async move {
                    approval_sender.post(RootMessage::ConnectionRejected(request.reject().await));
                })
                .detach();

                Ok(true)
            }
            RootMessage::ConnectionAccepted {
                peer_name,
                peer_capabilities,
                result,
            } => {
                match result {
                    Ok(transport) => {
                        let peer_address = transport.remote_address();
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Host, transport);
                        let (send, recv) = session.split();
                        let screen_list = OutgoingScreenList::try_from(self.model.app.screens())?;
                        let session = PendingHostSession {
                            peer_address,
                            peer_name,
                            peer_capabilities,
                            send,
                            recv,
                        };
                        let screen_list_sender = sender.clone();

                        compio::runtime::spawn(async move {
                            let result = session.send.send_screen_list(screen_list).await;
                            screen_list_sender
                                .post(RootMessage::InitialScreenListFinished { session, result });
                        })
                        .detach();
                    }
                    Err(error) => {
                        error!(error = ?error, "Failed to accept a QUIC connection request")
                    }
                }

                Ok(false)
            }
            RootMessage::InitialScreenListFinished { session, result } => {
                let changed = match result {
                    Ok(()) => {
                        self.start_session(
                            session.peer_address,
                            Some(session.peer_name),
                            session.peer_capabilities,
                            session.send,
                            session.recv,
                            sender,
                        )
                        .await
                    }
                    Err(error) => {
                        error!(error = ?error, "Failed to send the initial screen list");
                        false
                    }
                };

                Ok(changed)
            }
            RootMessage::ConnectionRejected(result) => {
                if let Err(error) = result {
                    error!(error = ?error, "Failed to reject a QUIC connection request");
                }

                Ok(false)
            }
            RootMessage::ConnectionRequestFailed(error) => {
                warn!(error = ?error, "Failed to receive a QUIC connection request");
                Ok(false)
            }
            RootMessage::ConnectionListenerFailed(error) => {
                error!(error = ?error, "QUIC connection listener stopped");
                self.listener.online = false;
                self.set_connection_status("The local connection listener stopped");
                Ok(true)
            }
            _ => unreachable!("message routed to the wrong connection handler"),
        }
    }
}
