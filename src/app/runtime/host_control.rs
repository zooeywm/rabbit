//! Shared Host control execution used by both GUI and headless shells.

use std::rc::Rc;

use tracing::{info, trace, warn};

use crate::{
    app::{
        model::ApplicationModel,
        platform::ApplicationStack,
        runtime::{
            host_policy::{HostStreamEvaluation, evaluate_set_screen_streams},
            host_stream_lifecycle::apply_host_stop_screen_stream,
        },
        services::host_stream::HostStreamPlan,
    },
    infra::SessionTransportSend,
    kernel::{
        connection_request::PeerCapabilities,
        domain_error::DomainError,
        input::{RemoteInputEvent, RemoteInputInjector as _},
        screen_manager::{Screen, ScreenId, ScreenLayoutManager},
        session::{SessionId, SessionMessage, SessionSend},
        session_control::ControlMessage,
    },
};

#[derive(Debug)]
enum HostControlDecision {
    SetScreenStreams(Result<HostStreamEvaluation, DomainError>),
    RequestKeyFrame(ScreenId),
    StopScreenStream(ScreenId),
    RemoteInput(RemoteInputEvent),
    Ignore,
}

/// Shell-visible result after the shared Host runtime applies a control message.
pub enum HostControlEffect {
    ConfigureStreams(HostStreamConfiguration),
    StreamRequestRejected(DomainError),
    StreamStopped(ScreenId),
    NoShellAction,
}

/// Deferred configuration response. Shells only decide where completion is posted.
pub struct HostStreamConfiguration {
    session_id: SessionId,
    session: Rc<SessionSend<SessionTransportSend>>,
    evaluation: HostStreamEvaluation,
}

impl HostStreamConfiguration {
    pub fn plans(&self) -> &[HostStreamPlan] {
        &self.evaluation.plans
    }

    pub async fn send(self) -> HostStreamConfigurationResult {
        let HostStreamEvaluation { configured, plans } = self.evaluation;
        let result = self
            .session
            .send_screen_streams_configured(configured)
            .await;
        HostStreamConfigurationResult {
            session_id: self.session_id,
            plans,
            result,
        }
    }
}

pub struct HostStreamConfigurationResult {
    pub session_id: SessionId,
    pub plans: Vec<HostStreamPlan>,
    pub result: eros::Result<()>,
}

/// Classifies and applies one Host-role message.
///
/// GUI and headless code must only adapt the returned effect to their message queues.
pub fn apply_host_session_message<Stack>(
    model: &mut ApplicationModel<Stack>,
    remote_input_injector: &mut Stack::RemoteInputInjector,
    session_id: SessionId,
    message: SessionMessage,
) -> HostControlEffect
where
    Stack: ApplicationStack,
{
    let Some((peer_capabilities, admits_streams, session_send)) = model
        .sessions
        .iter()
        .find(|session| session.send.id() == session_id)
        .map(|session| {
            (
                session.peer_capabilities.clone(),
                session.admits_new_streams(),
                Rc::clone(&session.send),
            )
        })
    else {
        warn!(
            session_id = session_id.0,
            "Ignored Host control message for a missing Session"
        );
        return HostControlEffect::NoShellAction;
    };

    let decision = classify_host_session_message(
        message,
        &model.local_capabilities,
        &peer_capabilities,
        admits_streams,
        |screen_id| model.app.screen(screen_id).cloned(),
    );

    match decision {
        HostControlDecision::SetScreenStreams(Ok(evaluation)) => {
            HostControlEffect::ConfigureStreams(HostStreamConfiguration {
                session_id,
                session: session_send,
                evaluation,
            })
        }
        HostControlDecision::SetScreenStreams(Err(error)) => {
            warn!(
                session_id = session_id.0,
                error = %error,
                "Host rejected screen stream request by policy"
            );
            HostControlEffect::StreamRequestRejected(error)
        }
        HostControlDecision::RequestKeyFrame(screen_id) => {
            let Some(stream) = model
                .sessions
                .iter()
                .find(|session| session.send.id() == session_id)
                .and_then(|session| session.screen_streams.get(&screen_id))
            else {
                warn!(
                    event = "key_frame_request_stream_missing",
                    session_id = session_id.0,
                    screen_id = screen_id.0,
                    "Key-frame request has no running Host screen stream"
                );
                return HostControlEffect::NoShellAction;
            };

            stream.request_key_frame();
            trace!(
                event = "key_frame_requested",
                session_id = session_id.0,
                screen_id = screen_id.0,
                "Queued key-frame request for Host encoder"
            );
            HostControlEffect::NoShellAction
        }
        HostControlDecision::StopScreenStream(screen_id) => {
            if let Some(session) = model
                .sessions
                .iter_mut()
                .find(|session| session.send.id() == session_id)
            {
                apply_host_stop_screen_stream(&mut session.screen_streams, screen_id);
            }
            info!(
                event = "screen_stream_stop_received",
                session_id = session_id.0,
                screen_id = screen_id.0,
                "Screen stream stop received"
            );
            HostControlEffect::StreamStopped(screen_id)
        }
        HostControlDecision::RemoteInput(input) => {
            apply_remote_input(model, remote_input_injector, session_id, input);
            HostControlEffect::NoShellAction
        }
        HostControlDecision::Ignore => {
            warn!(
                session_id = session_id.0,
                "Host ignored an inapplicable session message"
            );
            HostControlEffect::NoShellAction
        }
    }
}

fn apply_remote_input<Stack>(
    model: &ApplicationModel<Stack>,
    remote_input_injector: &mut Stack::RemoteInputInjector,
    session_id: SessionId,
    input: RemoteInputEvent,
) where
    Stack: ApplicationStack,
{
    let screen_id = input.screen_id();
    let capability_enabled = if input.is_reliable() {
        model.local_capabilities.reliable_input
    } else {
        model.local_capabilities.absolute_pointer
    };
    if !capability_enabled {
        warn!(
            event = "remote_input_capability_disabled",
            session_id = session_id.0,
            screen_id = screen_id.get(),
            "Ignored remote input that is disabled by local capabilities"
        );
        return;
    }
    let has_active_stream = model
        .sessions
        .iter()
        .find(|session| session.send.id() == session_id)
        .is_some_and(|session| session.screen_streams.contains_key(&screen_id));
    if !has_active_stream {
        warn!(
            event = "remote_input_stream_missing",
            session_id = session_id.0,
            screen_id = screen_id.get(),
            "Ignored remote input without an active screen stream"
        );
        return;
    }
    let Some(screen) = model.app.screen(&screen_id) else {
        warn!(
            event = "remote_input_screen_missing",
            session_id = session_id.0,
            screen_id = screen_id.get(),
            "Ignored remote input for an unavailable screen"
        );
        return;
    };
    if let Err(error) = remote_input_injector.inject(input, screen, model.app.screens()) {
        warn!(
            event = "remote_input_injection_failed",
            session_id = session_id.0,
            screen_id = screen_id.get(),
            error = ?error,
            "Failed to inject remote input"
        );
    }
}

fn classify_host_session_message(
    message: SessionMessage,
    local: &PeerCapabilities,
    peer: &PeerCapabilities,
    admits_streams: bool,
    lookup: impl Fn(&ScreenId) -> Option<Screen>,
) -> HostControlDecision {
    match message {
        SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
            HostControlDecision::SetScreenStreams(evaluate_set_screen_streams(
                request,
                lookup,
                local,
                peer,
                admits_streams,
            ))
        }
        SessionMessage::Control(ControlMessage::RequestKeyFrame(request)) => {
            HostControlDecision::RequestKeyFrame(request.screen_id)
        }
        SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
            HostControlDecision::StopScreenStream(stop.screen_id)
        }
        SessionMessage::Control(ControlMessage::RemoteInput(input)) => {
            HostControlDecision::RemoteInput(input)
        }
        SessionMessage::Control(_)
        | SessionMessage::Video(_)
        | SessionMessage::KeyFrameRequired(_) => HostControlDecision::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::{
            RemoteDisplayMode, RequestKeyFrame, ScreenStreamRequest, ScreenStreamRequestId,
            SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
    };

    fn screen(id: u8) -> Screen {
        Screen {
            id: ScreenId(id),
            name: "s".into(),
            resolution: PixelSize {
                width: 100,
                height: 100,
            },
            frame_rate: FrameRate::new(60, 1).expect("fps"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }

    #[test]
    fn classifies_key_frame_and_set_streams() {
        let decision = classify_host_session_message(
            SessionMessage::Control(ControlMessage::RequestKeyFrame(RequestKeyFrame {
                screen_id: ScreenId(3),
            })),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
            |_| None,
        );
        assert!(matches!(
            decision,
            HostControlDecision::RequestKeyFrame(ScreenId(3))
        ));

        let request = SetScreenStreams {
            request_id: ScreenStreamRequestId(1),
            desired_streams: vec![ScreenStreamRequest {
                screen_id: ScreenId(0),
                remote_display: RemoteDisplayMode::Preserve,
                frame_size: PixelSize {
                    width: 100,
                    height: 100,
                },
                frame_rate: FrameRate::new(60, 1).expect("fps"),
            }],
        };
        let decision = classify_host_session_message(
            SessionMessage::Control(ControlMessage::SetScreenStreams(request)),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
            |id| (id.get() == 0).then(|| screen(0)),
        );
        assert!(matches!(
            decision,
            HostControlDecision::SetScreenStreams(Ok(_))
        ));
    }
}
