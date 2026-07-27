use std::rc::Rc;

use tracing::{error, info};

use crate::app::{
    gui::application::{
        RootApplication,
        message::{MessageSender, RootMessage},
    },
    platform::ApplicationStack,
};

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) async fn handle_host_stream(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::StopHostedScreenStream(index) => {
                let Some((session_id, screen_id)) =
                    self.hosted_screen_stream_entries().get(index).copied()
                else {
                    return Ok(false);
                };
                if !self
                    .host_stream
                    .pending_stops
                    .insert((session_id, screen_id))
                {
                    return Ok(false);
                }
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)
                else {
                    self.host_stream
                        .pending_stops
                        .remove(&(session_id, screen_id));
                    return Ok(false);
                };
                let session_send = Rc::clone(&session.send);
                let stop_sender = sender.clone();

                compio::runtime::spawn(async move {
                    let result = session_send.stop_screen_stream(screen_id).await;
                    stop_sender.post(RootMessage::HostScreenStreamStopFinished {
                        session_id,
                        screen_id,
                        result,
                    });
                })
                .detach();
                self.set_connection_status(format!(
                    "Stopping Session {} screen {} stream",
                    session_id.0, screen_id.0
                ));
                Ok(true)
            }
            RootMessage::ScreenStreamConfigurationFinished {
                session_id,
                streams,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        error = ?error,
                        "Failed to send screen stream results"
                    );
                    self.remove_session(session_id);
                    return Ok(true);
                }

                if !self
                    .model
                    .sessions
                    .iter()
                    .any(|session| session.send.id() == session_id)
                {
                    return Ok(false);
                }

                let mut changed = false;
                for plan in streams {
                    if !self
                        .host_stream
                        .pending_starts
                        .remove(&(session_id, plan.screen_id))
                    {
                        continue;
                    }
                    if let Err(error) = self.replace_screen_stream(
                        session_id,
                        plan.screen_id,
                        plan.parameters,
                        plan.frame_rate,
                        sender,
                    ) {
                        error!(
                            session_id = session_id.0,
                            screen_id = plan.screen_id.0,
                            error = ?error,
                            "Failed to start screen stream"
                        );
                    } else {
                        changed = true;
                    }
                }

                Ok(changed)
            }
            RootMessage::ScreenStreamFinished(id, screen_id, stream_id, result) => {
                let Some(session) = self
                    .model
                    .sessions
                    .iter_mut()
                    .find(|session| session.send.id() == id)
                else {
                    return Ok(false);
                };
                let is_current = session
                    .screen_streams
                    .get(&screen_id)
                    .is_some_and(|stream| stream.id == stream_id);

                if !is_current {
                    return Ok(false);
                }

                let session_closed_normally = session.send.is_closed_normally();
                session.screen_streams.remove(&screen_id);

                match result {
                    Ok(()) => info!(
                        event = "screen_stream_finished",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        "Screen stream finished"
                    ),
                    Err(_) if session_closed_normally => info!(
                        event = "screen_stream_finished",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        "Screen stream finished during normal Session close"
                    ),
                    Err(error) => {
                        error!(
                            event = "screen_stream_failed",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Screen stream failed"
                        );
                        self.set_connection_status(format!(
                            "Session {} screen {} failed: {error}",
                            id.0, screen_id.0
                        ));
                    }
                }

                Ok(true)
            }
            RootMessage::HostScreenStreamStopFinished {
                session_id,
                screen_id,
                result,
            } => {
                self.host_stream
                    .pending_stops
                    .remove(&(session_id, screen_id));
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to notify the remote device that its screen stream was stopped"
                    );
                    self.set_connection_status(format!(
                        "Failed to stop Session {} screen {}: {error}",
                        session_id.0, screen_id.0
                    ));
                    return Ok(true);
                }

                let Some(session) = self
                    .model
                    .sessions
                    .iter_mut()
                    .find(|session| session.send.id() == session_id)
                else {
                    return Ok(false);
                };
                session.screen_streams.remove(&screen_id);
                info!(
                    event = "host_screen_stream_stopped",
                    session_id = session_id.0,
                    screen_id = screen_id.0,
                    "Host stopped screen stream"
                );
                self.set_connection_status(format!(
                    "Stopped Session {} screen {} stream",
                    session_id.0, screen_id.0
                ));
                Ok(true)
            }
            _ => unreachable!("message routed to the wrong host_stream handler"),
        }
    }
}
