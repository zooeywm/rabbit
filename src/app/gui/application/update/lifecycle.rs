use tracing::info;

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
                let retirements = self.model.begin_all_session_retirements();
                let shutdown_sender = sender.clone();

                info!(
                    event = "application_shutdown_started",
                    session_count = retirements.len(),
                    "Application shutdown started"
                );
                compio::runtime::spawn(async move {
                    for retirement in retirements {
                        retirement.finish().await;
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
