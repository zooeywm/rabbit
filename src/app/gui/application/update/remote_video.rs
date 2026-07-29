use std::rc::Rc;

use eros::Context as _;
use tracing::{debug, error, trace, warn};

use crate::app::runtime::controller_policy::evaluate_controller_set_screen_streams;
use crate::app::{
    config::{Config, PointerMode},
    gui::{
        application::{
            RootApplication,
            message::{MessageSender, RootMessage},
        },
        state::{ScreenStreamTarget, parse_stream_settings},
    },
    platform::ApplicationStack,
};
use crate::kernel::{
    input::{
        AbsolutePointerMove, KeyboardInput, MouseButtonInput, RelativePointerMove,
        RemoteInputEvent, map_viewport_position,
    },
    screen_configuration::{RemoteDisplayMode, ScreenStreamRequest, SetScreenStreams},
    video_encoder::VideoCodec,
};

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) async fn handle_remote_video(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::PointerMoved(event) => {
                let Some(target) = self.remote_stream.screen_stream.streaming_target() else {
                    return Ok(false);
                };
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == target.session_id)
                else {
                    return Ok(false);
                };
                let input = match <Stack::App as AsRef<Config>>::as_ref(&self.model.app)
                    .input
                    .pointer_mode
                {
                    PointerMode::Absolute => {
                        if !session.peer_capabilities.absolute_pointer {
                            return Ok(false);
                        }
                        let Some(position) = map_viewport_position(
                            event.x,
                            event.y,
                            event.viewport_width,
                            event.viewport_height,
                            target.frame_size,
                        ) else {
                            return Ok(false);
                        };
                        RemoteInputEvent::AbsolutePointerMove(AbsolutePointerMove {
                            screen_id: target.screen_id,
                            position,
                        })
                    }
                    PointerMode::Relative => {
                        if !session.peer_capabilities.reliable_input {
                            return Ok(false);
                        }
                        let Some((delta_x, delta_y)) = self
                            .relative_pointer_accumulator
                            .accumulate(event.delta_x, event.delta_y)
                        else {
                            return Ok(false);
                        };
                        RemoteInputEvent::RelativePointerMove(RelativePointerMove {
                            screen_id: target.screen_id,
                            delta_x,
                            delta_y,
                        })
                    }
                };
                session
                    .send
                    .send_remote_input(input)
                    .await
                    .with_context(|| format!("Failed to send {input:?}"))?;
                Ok(false)
            }
            RootMessage::Keyboard { key, state, repeat } => {
                let Some(target) = self.remote_stream.screen_stream.streaming_target() else {
                    return Ok(false);
                };
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == target.session_id)
                else {
                    return Ok(false);
                };
                if !session.peer_capabilities.reliable_input {
                    return Ok(false);
                }
                let input = RemoteInputEvent::Keyboard(KeyboardInput {
                    screen_id: target.screen_id,
                    key,
                    state,
                    repeat,
                });
                session
                    .send
                    .send_remote_input(input)
                    .await
                    .with_context(|| "Failed to send keyboard input")?;
                Ok(false)
            }
            RootMessage::MouseButton { button, state } => {
                let Some(target) = self.remote_stream.screen_stream.streaming_target() else {
                    return Ok(false);
                };
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == target.session_id)
                else {
                    return Ok(false);
                };
                if !session.peer_capabilities.reliable_input {
                    return Ok(false);
                }
                session
                    .send
                    .send_remote_input(RemoteInputEvent::MouseButton(MouseButtonInput {
                        screen_id: target.screen_id,
                        button,
                        state,
                    }))
                    .await
                    .with_context(|| "Failed to send mouse button input")?;
                Ok(false)
            }
            RootMessage::StopCurrentScreenStream => {
                let Some((session_id, screen_id)) =
                    self.remote_stream.screen_stream.active_screen()
                else {
                    return Ok(false);
                };
                self.stop_video_decoder()?;
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)
                else {
                    self.remote_stream.screen_stream.reset();
                    self.model.selected_remote_screen = None;
                    return Ok(true);
                };
                let session = Rc::clone(&session.send);
                let stop_sender = sender.clone();

                compio::runtime::spawn(async move {
                    let result = session.stop_screen_stream(screen_id).await;
                    stop_sender.post(RootMessage::ScreenStreamStopFinished {
                        session_id,
                        screen_id,
                        result,
                    });
                })
                .detach();

                Ok(false)
            }
            RootMessage::VideoFrameReceived(id, video) => {
                let screen_id = video.screen_id;
                let payload_size = video.packets.iter().map(bytes::Bytes::len).sum::<usize>();

                trace!(
                    session_id = id.0,
                    screen_id = video.screen_id.0,
                    packet_count = video.packets.len(),
                    payload_size,
                    "Received complete video RTP frame"
                );

                if self.remote_stream.screen_stream.active_screen() != Some((id, screen_id)) {
                    return Ok(false);
                }
                self.start_video_decoder(id, screen_id, sender)?;
                let decoder = self
                    .remote_stream
                    .video_decoder
                    .as_ref()
                    .context("Video decoder was not retained after startup")?;
                decoder.publish(video);
                Ok(false)
            }
            RootMessage::VideoFrameReady(id, screen_id) => {
                let received = self
                    .remote_stream
                    .screen_stream
                    .receive_video(id, screen_id);
                if received
                    && let Some(request_id) = self.remote_stream.screen_stream.active_request_id()
                {
                    self.remote_stream.key_frame_request.recovered(request_id);
                }
                Ok(received)
            }
            RootMessage::InitialVideoKeyFrameTimeout {
                request_id,
                session_id,
                screen_id,
            } => {
                let still_waiting = self
                    .remote_stream
                    .screen_stream
                    .waiting_target()
                    .is_some_and(|target| {
                        target.request_id == request_id
                            && target.session_id == session_id
                            && target.screen_id == screen_id
                    });
                if !still_waiting {
                    return Ok(false);
                }
                if !self.remote_stream.key_frame_request.begin(request_id) {
                    debug!(
                        event = "initial_video_key_frame_request_suppressed",
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        "A recent key-frame request already covers the initial video wait"
                    );
                    let retry_sender = sender.clone();
                    compio::runtime::spawn(async move {
                        compio::time::sleep(std::time::Duration::from_secs(2)).await;
                        retry_sender.post(RootMessage::InitialVideoKeyFrameTimeout {
                            request_id,
                            session_id,
                            screen_id,
                        });
                    })
                    .detach();
                    return Ok(false);
                }
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)
                else {
                    return Ok(false);
                };
                warn!(
                    event = "initial_video_key_frame_requested",
                    session_id = session_id.0,
                    screen_id = screen_id.0,
                    "Requesting a key frame because no decodable first frame arrived"
                );
                let session = Rc::clone(&session.send);
                let request_sender = sender.clone();
                compio::runtime::spawn(async move {
                    let result = session.request_key_frame(screen_id).await;
                    request_sender.post(RootMessage::KeyFrameRequestFinished {
                        session_id,
                        screen_id,
                        result,
                    });
                })
                .detach();
                Ok(false)
            }
            RootMessage::VideoDecoderFinished(id, screen_id, result) => {
                if !self
                    .remote_stream
                    .video_decoder
                    .as_ref()
                    .is_some_and(|decoder| decoder.matches(id, screen_id))
                {
                    return Ok(false);
                }
                self.remote_stream.video_decoder = None;
                self.view.clear_video()?;
                match result {
                    Ok(()) => {
                        if self.remote_stream.screen_stream.fail(
                            id,
                            screen_id,
                            "The hardware video decoder ended unexpectedly".to_string(),
                        ) {
                            self.set_connection_status(format!(
                                "Screen {} decoder ended unexpectedly",
                                screen_id.0
                            ));
                            return Ok(true);
                        }
                    }
                    Err(error) => {
                        error!(
                            event = "video_decoder_failed",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Hardware video decoder failed"
                        );
                        if self
                            .remote_stream
                            .screen_stream
                            .begin_recovery(id, screen_id)
                        {
                            let request_id = self
                                .remote_stream
                                .screen_stream
                                .active_request_id()
                                .context(
                                "Recovering screen stream has no request generation",
                            )?;
                            if !self.remote_stream.key_frame_request.begin(request_id) {
                                debug!(
                                    event = "decoder_recovery_key_frame_request_suppressed",
                                    session_id = id.0,
                                    screen_id = screen_id.0,
                                    "A recent key-frame request already covers decoder recovery"
                                );
                                let retry_sender = sender.clone();
                                compio::runtime::spawn(async move {
                                    compio::time::sleep(std::time::Duration::from_secs(2)).await;
                                    retry_sender.post(RootMessage::InitialVideoKeyFrameTimeout {
                                        request_id,
                                        session_id: id,
                                        screen_id,
                                    });
                                })
                                .detach();
                                return Ok(true);
                            }
                            let Some(session) = self
                                .model
                                .sessions
                                .iter()
                                .find(|session| session.send.id() == id)
                            else {
                                return Ok(false);
                            };
                            warn!(
                                event = "decoder_recovery_key_frame_requested",
                                session_id = id.0,
                                screen_id = screen_id.0,
                                "Reinitializing the decoder from a fresh key frame"
                            );
                            let session = Rc::clone(&session.send);
                            let request_sender = sender.clone();
                            compio::runtime::spawn(async move {
                                let result = session.request_key_frame(screen_id).await;
                                request_sender.post(RootMessage::KeyFrameRequestFinished {
                                    session_id: id,
                                    screen_id,
                                    result,
                                });
                            })
                            .detach();
                            self.set_connection_status(format!(
                                "Screen {} decoder is waiting for a recovery key frame",
                                screen_id.0
                            ));
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            RootMessage::VideoRendererFailed(message) => {
                let active = self.remote_stream.screen_stream.active_screen();
                self.remote_stream.video_decoder = None;
                if let Some((session_id, screen_id)) = active {
                    error!(
                        event = "video_renderer_failed",
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = %message,
                        "DMA-BUF video renderer failed"
                    );
                    if self.remote_stream.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("GPU video rendering failed: {message}"),
                    ) {
                        self.set_connection_status(format!(
                            "Screen {} renderer failed: {message}",
                            screen_id.0
                        ));
                        return Ok(true);
                    }
                }
                eros::bail!("Slint DMA-BUF video renderer failed: {}", message)
            }
            RootMessage::KeyFrameRequestFinished {
                session_id,
                screen_id,
                result,
            } => {
                if let Some(request_id) = self
                    .remote_stream
                    .screen_stream
                    .active_request_id()
                    .filter(|_| {
                        self.remote_stream.screen_stream.active_screen()
                            == Some((session_id, screen_id))
                    })
                {
                    self.remote_stream.key_frame_request.finish(request_id);
                }
                if let Err(error) = result {
                    warn!(
                        event = "key_frame_request_failed",
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to request the preferred on-demand key frame; stopping the Client screen stream"
                    );
                    if self.remote_stream.screen_stream.active_screen()
                        == Some((session_id, screen_id))
                    {
                        self.stop_video_decoder()?;
                        self.remote_stream.screen_stream.fail_session(
                            session_id,
                            format!("Failed to request a recovery key frame: {error}"),
                        );
                        self.set_connection_status(format!(
                            "Session {} screen {} key-frame request failed: {error}",
                            session_id.0, screen_id.0
                        ));
                        return Ok(true);
                    }
                } else if let Some(target) = self
                    .remote_stream
                    .screen_stream
                    .waiting_target()
                    .filter(|target| {
                        target.session_id == session_id && target.screen_id == screen_id
                    })
                    .cloned()
                {
                    let retry_sender = sender.clone();
                    compio::runtime::spawn(async move {
                        compio::time::sleep(std::time::Duration::from_secs(2)).await;
                        retry_sender.post(RootMessage::InitialVideoKeyFrameTimeout {
                            request_id: target.request_id,
                            session_id,
                            screen_id,
                        });
                    })
                    .detach();
                }

                Ok(false)
            }
            RootMessage::OpenRemoteScreen {
                selected_index,
                width,
                height,
                frame_rate,
                dynamic_frame_rate,
                bitrate_mbps,
            } => {
                let (frame_size, frame_rate, bitrate) =
                    match parse_stream_settings(&width, &height, &frame_rate, &bitrate_mbps) {
                        Ok(settings) => settings,
                        Err(error) => {
                            self.workspace.stream_settings_error = error.to_string();
                            return Ok(true);
                        }
                    };
                self.workspace.stream_settings_error.clear();
                let selected = self
                    .model
                    .remote_screen_entries
                    .get(selected_index)
                    .copied();

                self.stop_video_decoder()?;
                self.model.selected_remote_screen = selected;

                if let Some((session_id, screen_id)) = selected {
                    let Some(screen_name) =
                        self.model
                            .remote_screens
                            .get(&session_id)
                            .and_then(|screens| {
                                screens
                                    .iter()
                                    .find(|screen| screen.id == screen_id)
                                    .map(|screen| screen.name.clone())
                            })
                    else {
                        warn!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            "Selected remote screen is no longer available"
                        );
                        return Ok(false);
                    };
                    let (session_send, peer_capabilities, admits_streams) = {
                        let Some(session) = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == session_id)
                        else {
                            warn!(
                                session_id = session_id.0,
                                "Session closed before screen stream could be requested"
                            );
                            return Ok(false);
                        };
                        (
                            Rc::clone(&session.send),
                            session.peer_capabilities.clone(),
                            session.admits_new_streams(),
                        )
                    };
                    let request_id = self.model.next_screen_stream_request_id()?;
                    let request = SetScreenStreams {
                        request_id,
                        desired_streams: vec![ScreenStreamRequest {
                            screen_id,
                            remote_display: RemoteDisplayMode::Preserve,
                            frame_size,
                            frame_rate,
                            frame_rate_mode: if dynamic_frame_rate {
                                crate::kernel::video_encoder::VideoFrameRateMode::Dynamic
                            } else {
                                crate::kernel::video_encoder::VideoFrameRateMode::Fixed
                            },
                            codec: VideoCodec::H264,
                            bitrate,
                            fec_percentage:
                                crate::kernel::video_encoder::VideoFecPercentage::DEFAULT,
                        }],
                    };
                    if let Err(error) = evaluate_controller_set_screen_streams(
                        &request,
                        &self.model.local_capabilities,
                        &peer_capabilities,
                        admits_streams,
                    ) {
                        warn!(
                            session_id = session_id.0,
                            error = %error,
                            "Controller rejected local stream request by capability policy"
                        );
                        self.workspace.stream_settings_error = error.to_string();
                        self.set_connection_status(format!("Cannot open remote screen: {error}"));
                        return Ok(true);
                    }
                    self.remote_stream.screen_stream.begin(ScreenStreamTarget {
                        request_id,
                        session_id,
                        screen_id,
                        screen_name,
                        frame_size,
                        frame_rate,
                        frame_rate_mode: if dynamic_frame_rate {
                            crate::kernel::video_encoder::VideoFrameRateMode::Dynamic
                        } else {
                            crate::kernel::video_encoder::VideoFrameRateMode::Fixed
                        },
                        codec: VideoCodec::H264,
                        bitrate,
                    });

                    let request_sender = sender.clone();
                    compio::runtime::spawn(async move {
                        let result = session_send.send_screen_streams_request(request).await;
                        request_sender.post(RootMessage::ScreenStreamRequestFinished {
                            request_id,
                            session_id,
                            screen_id,
                            frame_size,
                            result,
                        });
                    })
                    .detach();
                }

                Ok(true)
            }
            RootMessage::ScreenStreamRequestFinished {
                request_id,
                session_id,
                screen_id,
                frame_size,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to request screen stream"
                    );
                    if self.remote_stream.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("Failed to request screen stream: {error}"),
                    ) {
                        self.remove_session(session_id);
                        return Ok(true);
                    }
                    self.remove_session(session_id);
                } else {
                    if !self
                        .model
                        .sessions
                        .iter()
                        .any(|session| session.send.id() == session_id)
                    {
                        return Ok(false);
                    }
                    trace!(
                        request_id = request_id.0,
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        width = frame_size.width,
                        height = frame_size.height,
                        "Screen stream request sent"
                    );
                }

                Ok(true)
            }
            RootMessage::ScreenStreamStopFinished {
                session_id,
                screen_id,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to stop screen stream"
                    );
                    self.remote_stream.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("Failed to stop screen stream: {error}"),
                    );
                    return Ok(true);
                }

                self.remote_stream.screen_stream.reset();
                self.stop_video_decoder()?;
                self.model.selected_remote_screen = None;
                self.set_connection_status(format!(
                    "Stopped screen {} stream; Session {} remains connected",
                    screen_id.0, session_id.0
                ));
                Ok(true)
            }
            _ => unreachable!("message routed to the wrong remote_video handler"),
        }
    }
}
