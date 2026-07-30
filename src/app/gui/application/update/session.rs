use std::rc::Rc;

use tracing::{error, info, warn};

use crate::app::{
    gui::application::{
        RootApplication,
        message::{MessageSender, RootMessage},
    },
    platform::ApplicationStack,
    runtime::host_control::{HostControlEffect, apply_host_session_message},
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
                self.disconnect_session(session_id).await
            }
            RootMessage::DisconnectDevice(index) => {
                let Some(session_id) = self.host_session_ids().get(index).copied() else {
                    return Ok(false);
                };
                self.disconnect_session(session_id).await
            }
            RootMessage::SessionMessageReceived(id, message) => {
                let role = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == id)
                    .map(|session| session.key.role());

                if role == Some(SessionRole::Host) {
                    if self
                        .handle_host_control_message(id, message, sender)
                        .await?
                    {
                        return Ok(true);
                    }
                    // Fall through only for Ignore decisions that may still be
                    // meaningful as generic control (should be rare on host).
                    return Ok(true);
                }

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
                    SessionMessage::Control(ControlMessage::ScreenStreamsConfigured(
                        configured,
                    )) => {
                        self.remote_stream
                            .screen_stream
                            .apply_configuration(&configured);
                        if let Some(target) =
                            self.remote_stream.screen_stream.waiting_target().cloned()
                        {
                            let timeout_sender = sender.clone();
                            compio::runtime::spawn(async move {
                                compio::time::sleep(std::time::Duration::from_secs(1)).await;
                                timeout_sender.post(RootMessage::InitialVideoKeyFrameTimeout {
                                    request_id: target.request_id,
                                    session_id: target.session_id,
                                    screen_id: target.screen_id,
                                });
                            })
                            .detach();
                        }
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
                    SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
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
                        info!(
                            event = "screen_stream_stop_received",
                            session_id = id.0,
                            screen_id = stop.screen_id.0,
                            "Screen stream stop received"
                        );
                    }
                    SessionMessage::Control(ControlMessage::SetScreenStreams(_))
                    | SessionMessage::Control(ControlMessage::RequestKeyFrame(_))
                    | SessionMessage::Control(ControlMessage::RemoteInput(_)) => {
                        warn!(
                            session_id = id.0,
                            "Controller role received host-only control message"
                        );
                    }
                    SessionMessage::Video(_) => {
                        eros::bail!("Video frame bypassed the latest-frame session queue")
                    }
                    SessionMessage::KeyFrameRequired(screen_id) => {
                        let Some(request_id) = self
                            .remote_stream
                            .screen_stream
                            .active_request_id()
                            .filter(|_| {
                                self.remote_stream.screen_stream.active_screen()
                                    == Some((id, screen_id))
                            })
                        else {
                            return Ok(false);
                        };
                        if !self.remote_stream.key_frame_request.begin(request_id) {
                            return Ok(false);
                        }
                        warn!(
                            event = "video_recovery_key_frame_requested",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            "Requesting a recovery key frame after incomplete RTP video"
                        );
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
                self.remove_session(id).await;
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
                self.remove_session(id).await;
                error!(session_id = id.0, error = ?error, "Session receive loop failed");
                self.set_connection_status(format!("Session {} failed: {error}", id.0));
                Ok(true)
            }
            _ => unreachable!("message routed to the wrong session handler"),
        }
    }

    /// Adapts shared Host control effects to the GUI message queue.
    async fn handle_host_control_message(
        &mut self,
        id: crate::kernel::session::SessionId,
        message: SessionMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        let effect = apply_host_session_message(
            &mut self.model,
            &mut self.remote_input_injector,
            id,
            message,
        );

        match effect {
            HostControlEffect::ConfigureStreams(configuration) => {
                self.host_stream.pending_starts.extend(
                    configuration
                        .plans()
                        .iter()
                        .map(|plan| (id, plan.screen_id)),
                );
                let configuration_sender = sender.clone();

                compio::runtime::spawn(async move {
                    let completion = configuration.send().await;
                    configuration_sender.post(RootMessage::ScreenStreamConfigurationFinished {
                        session_id: completion.session_id,
                        streams: completion.plans,
                        result: completion.result,
                    });
                })
                .detach();
                Ok(true)
            }
            HostControlEffect::StreamRequestRejected(error) => {
                self.set_connection_status(format!(
                    "Session {} stream request rejected: {error}",
                    id.0
                ));
                Ok(true)
            }
            HostControlEffect::StreamStopped { screen_id, task } => {
                if let Some(task) = task
                    && let Err(join_error) = task.await
                {
                    error!(
                        event = "stopped_screen_stream_join_failed",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        error = ?join_error,
                        "Stopped screen stream task failed while releasing its resources"
                    );
                }
                self.host_stream.pending_starts.remove(&(id, screen_id));
                Ok(true)
            }
            HostControlEffect::NoShellAction => Ok(true),
        }
    }
}
