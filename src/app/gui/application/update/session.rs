use std::rc::Rc;

use tracing::{error, info, trace, warn};

use crate::app::{
    gui::application::{
        RootApplication,
        message::{MessageSender, RootMessage},
    },
    platform::ApplicationStack,
    runtime::host_policy::HostStreamEvaluation,
};
use crate::kernel::{
    screen_configuration::ScreenResolutionStatus,
    session::{SessionMessage, SessionRole},
    session_control::ControlMessage,
};

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) async fn handle_session(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::DisconnectRemoteSession => {
                let Some(session_id) = self.controller_session_id() else {
                    return Ok(false);
                };
                self.disconnect_session(session_id)
            }
            RootMessage::DisconnectDevice(index) => {
                let Some(session_id) = self.host_session_ids().get(index).copied() else {
                    return Ok(false);
                };
                self.disconnect_session(session_id)
            }
            RootMessage::SessionMessageReceived(id, message) => {
                match message {
                    SessionMessage::Control(ControlMessage::ScreenList(screens)) => {
                        self.set_connection_status(format!(
                            "Session {} reported {} screens",
                            id.0,
                            screens.len()
                        ));
                        self.model.remote_screens.insert(id, screens);
                        self.refresh_remote_screen_list();
                    }
                    SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
                        let evaluation = match self.configure_preserved_screens(request, id) {
                            Ok(evaluation) => evaluation,
                            Err(error) => {
                                warn!(
                                    session_id = id.0,
                                    error = %error,
                                    "Host rejected screen stream request by policy"
                                );
                                self.set_connection_status(format!(
                                    "Session {} stream request rejected: {error}",
                                    id.0
                                ));
                                return Ok(true);
                            }
                        };
                        let HostStreamEvaluation { configured, plans } = evaluation;
                        let Some(session) = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == id)
                        else {
                            warn!(
                                session_id = id.0,
                                "Session closed before screen stream results could be sent"
                            );
                            return Ok(false);
                        };
                        let session_send = Rc::clone(&session.send);
                        self.host_stream
                            .pending_starts
                            .extend(plans.iter().map(|plan| (id, plan.screen_id)));
                        let configuration_sender = sender.clone();

                        compio::runtime::spawn(async move {
                            let result = session_send
                                .send_screen_streams_configured(configured)
                                .await;
                            configuration_sender.post(
                                RootMessage::ScreenStreamConfigurationFinished {
                                    session_id: id,
                                    streams: plans,
                                    result,
                                },
                            );
                        })
                        .detach();
                    }
                    SessionMessage::Control(ControlMessage::ScreenStreamsConfigured(
                        configured,
                    )) => {
                        self.remote_stream
                            .screen_stream
                            .apply_configuration(&configured);
                        let configured_count = configured
                            .outcomes
                            .iter()
                            .filter(|outcome| {
                                matches!(&outcome.status, ScreenResolutionStatus::Configured(_))
                            })
                            .count();
                        let failed_count = configured.outcomes.len() - configured_count;

                        self.set_connection_status(format!(
                            "Session {} request {}: {} configured, {} failed",
                            id.0, configured.request_id.0, configured_count, failed_count
                        ));
                        self.model.screen_stream_results.insert(id, configured);
                    }
                    SessionMessage::Control(ControlMessage::RequestKeyFrame(request)) => {
                        let Some(session) = self
                            .model
                            .sessions
                            .iter_mut()
                            .find(|session| session.send.id() == id)
                        else {
                            warn!(
                                event = "key_frame_request_session_missing",
                                session_id = id.0,
                                screen_id = request.screen_id.0,
                                "Key-frame request arrived after its Session closed"
                            );
                            return Ok(false);
                        };
                        let Some(stream) = session.screen_streams.get(&request.screen_id) else {
                            warn!(
                                event = "key_frame_request_stream_missing",
                                session_id = id.0,
                                screen_id = request.screen_id.0,
                                "Key-frame request has no running Host screen stream"
                            );
                            return Ok(false);
                        };

                        stream.request_key_frame();
                        trace!(
                            event = "key_frame_requested",
                            session_id = id.0,
                            screen_id = request.screen_id.0,
                            "Queued key-frame request for Host encoder"
                        );
                    }
                    SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
                        let role = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == id)
                            .map(|session| session.key.role());
                        match role {
                            Some(SessionRole::Host) => {
                                self.host_stream
                                    .pending_starts
                                    .remove(&(id, stop.screen_id));
                                if let Some(session) = self
                                    .model
                                    .sessions
                                    .iter_mut()
                                    .find(|session| session.send.id() == id)
                                {
                                    session.screen_streams.remove(&stop.screen_id);
                                }
                            }
                            Some(SessionRole::Controller) => {
                                if self.remote_stream.screen_stream.active_screen()
                                    == Some((id, stop.screen_id))
                                {
                                    self.stop_video_decoder()?;
                                    self.remote_stream.screen_stream.reset();
                                    self.model.selected_remote_screen = None;
                                    self.set_connection_status(format!(
                                        "The remote device stopped screen {} stream",
                                        stop.screen_id.0
                                    ));
                                }
                            }
                            None => return Ok(false),
                        }
                        info!(
                            event = "screen_stream_stop_received",
                            session_id = id.0,
                            screen_id = stop.screen_id.0,
                            "Screen stream stop received"
                        );
                    }
                    SessionMessage::Video(_) => {
                        eros::bail!("Video frame bypassed the latest-frame session queue")
                    }
                    SessionMessage::KeyFrameRequired(screen_id) => {
                        let Some(session) = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == id)
                        else {
                            warn!(
                                event = "key_frame_request_session_missing",
                                session_id = id.0,
                                screen_id = screen_id.0,
                                "Cannot request a key frame after the Session closed"
                            );
                            return Ok(false);
                        };
                        let session_send = Rc::clone(&session.send);
                        let request_sender = sender.clone();
                        compio::runtime::spawn(async move {
                            let result = session_send.request_key_frame(screen_id).await;
                            request_sender.post(RootMessage::KeyFrameRequestFinished {
                                session_id: id,
                                screen_id,
                                result,
                            });
                        })
                        .detach();
                    }
                }

                Ok(true)
            }
            RootMessage::SessionClosed(id) => {
                self.stop_session_video_decoder(id)?;
                self.remote_stream
                    .screen_stream
                    .fail_session(id, "The remote session closed".to_string());
                self.remove_session(id);
                info!(
                    event = "session_closed",
                    session_id = id.0,
                    "Session closed"
                );
                self.set_connection_status(format!("Session {} closed", id.0));
                Ok(true)
            }
            RootMessage::SessionFailed(id, error) => {
                self.stop_session_video_decoder(id)?;
                self.remote_stream
                    .screen_stream
                    .fail_session(id, format!("The remote session failed: {error}"));
                self.remove_session(id);
                error!(session_id = id.0, error = ?error, "Session receive loop failed");
                self.set_connection_status(format!("Session {} failed: {error}", id.0));
                Ok(true)
            }
            _ => unreachable!("message routed to the wrong session handler"),
        }
    }
}
