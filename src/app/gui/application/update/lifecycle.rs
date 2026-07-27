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
    pub(super) async fn handle_lifecycle(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::SelectSection(section) => {
                if self.workspace.active_section == section {
                    return Ok(false);
                }
                self.workspace.active_section = section;
                Ok(true)
            }
            RootMessage::Close => {
                if self.lifecycle.closing {
                    return Ok(false);
                }
                self.lifecycle.closing = true;
                crate::app::shutdown::request();
                self.stop_video_decoder()?;
                let tasks = self.model.begin_screen_stream_shutdown();
                let sessions = self
                    .model
                    .sessions
                    .iter()
                    .map(|session| Rc::clone(&session.send))
                    .collect::<Vec<_>>();
                let shutdown_sender = sender.clone();

                info!(
                    event = "application_shutdown_started",
                    screen_stream_count = tasks.len(),
                    "Application shutdown started"
                );
                compio::runtime::spawn(async move {
                    for task in tasks {
                        if let Err(error) = task.await {
                            error!(
                                error = ?error,
                                "Screen stream task failed during application shutdown"
                            );
                        }
                    }

                    for session in sessions {
                        session.close().await;
                    }

                    shutdown_sender.post(RootMessage::ShutdownFinished);
                })
                .detach();

                Ok(false)
            }
            RootMessage::ShutdownFinished => {
                info!(
                    event = "application_shutdown_finished",
                    "Application shutdown finished"
                );
                self.lifecycle.finished = true;
                Ok(false)
            }
            _ => unreachable!("message routed to the wrong lifecycle handler"),
        }
    }
}
