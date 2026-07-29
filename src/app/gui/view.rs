use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use eros::Context as _;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::gui::{
    input::{keyboard_key_from_slint, mouse_button_from_slint},
    video_view::{self, VideoViewPublisher},
};
use crate::app::{
    config::{APP_ID, PointerMode},
    gui::state::{
        ConnectedDeviceView, ConnectionRequestView, HostedScreenStreamView, RemoteScreenView,
        ViewPage, ViewState, WorkspaceSection, recommended_bitrate_text,
    },
};
use crate::kernel::{
    input::{InputState, KeyboardKey, MouseButton},
    video_encoder::VideoCodec,
};

slint::slint! {
    export {
        AppPage,
        ConnectedDeviceItem,
        ConnectionRequestItem,
        HostedScreenStreamItem,
        NavigationSection,
        RabbitWindow,
        RemoteScreenItem,
    } from "../../../ui/app.slint";
}

#[derive(Clone, Debug)]
pub(crate) enum GuiIntent {
    SelectSection(WorkspaceSection),
    Connect(String),
    DecideConnectionRequest {
        index: usize,
        accept: bool,
    },
    OpenRemoteScreen {
        index: usize,
        width: String,
        height: String,
        frame_rate: String,
        bitrate_mbps: String,
    },
    DisconnectRemoteSession,
    StopHostedScreenStream(usize),
    DisconnectDevice(usize),
    RetryConnection,
    StopScreenStream,
    VideoFrameReady {
        session_id: crate::kernel::session::SessionId,
        screen_id: crate::kernel::screen_manager::ScreenId,
    },
    VideoRendererFailed(String),
    PointerMoved(PointerIntent),
    Keyboard {
        key: KeyboardKey,
        state: InputState,
        repeat: bool,
    },
    MouseButton {
        button: MouseButton,
        state: InputState,
    },
    Close,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PointerViewportEvent {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) delta_x: f32,
    pub(crate) delta_y: f32,
    pub(crate) viewport_width: f32,
    pub(crate) viewport_height: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum PointerIntent {
    Direct(PointerViewportEvent),
    Coalesced(PointerMailbox),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PointerMailbox {
    state: Arc<Mutex<PointerMailboxState>>,
}

#[derive(Debug, Default)]
struct PointerMailboxState {
    latest: Option<PointerViewportEvent>,
    notification_pending: bool,
}

impl PointerMailbox {
    fn submit(&self, event: PointerViewportEvent, sender: &flume::Sender<GuiIntent>) {
        let notify = {
            let mut state = self.state.lock().expect("pointer mailbox poisoned");
            state.latest = Some(event);
            if state.notification_pending {
                false
            } else {
                state.notification_pending = true;
                true
            }
        };
        if notify
            && sender
                .send(GuiIntent::PointerMoved(PointerIntent::Coalesced(
                    self.clone(),
                )))
                .is_err()
        {
            self.state
                .lock()
                .expect("pointer mailbox poisoned")
                .notification_pending = false;
        }
    }

    pub(crate) fn take(&self) -> Option<PointerViewportEvent> {
        let mut state = self.state.lock().expect("pointer mailbox poisoned");
        state.notification_pending = false;
        state.latest.take()
    }
}

pub(crate) struct Gui<VideoView>
where
    VideoView: video_view::VideoViewStack,
{
    window: RabbitWindow,
    intent_sender: flume::Sender<GuiIntent>,
    video_stack: std::marker::PhantomData<VideoView>,
}

pub(crate) struct ViewPublisher<VideoView>
where
    VideoView: video_view::VideoViewStack,
{
    window: slint::Weak<RabbitWindow>,
    video: VideoViewPublisher<VideoView>,
}

impl<VideoView> Clone for ViewPublisher<VideoView>
where
    VideoView: video_view::VideoViewStack,
{
    fn clone(&self) -> Self {
        Self {
            window: self.window.clone(),
            video: self.video.clone(),
        }
    }
}

impl<VideoView> Gui<VideoView>
where
    VideoView: video_view::VideoViewStack,
{
    pub(crate) fn new(
        probe_interval: Duration,
        pointer_mode: PointerMode,
    ) -> eros::Result<(Self, ViewPublisher<VideoView>, flume::Receiver<GuiIntent>)> {
        VideoView::select_slint_backend()?;
        let window = RabbitWindow::new().context("Failed to create the Slint Rabbit window")?;
        slint::set_xdg_app_id(APP_ID).context("Failed to set the Rabbit XDG application ID")?;
        let (sender, intents) = flume::unbounded();

        {
            let sender = sender.clone();
            window.on_select_section(move |section| {
                let section = match section {
                    NavigationSection::RemoteDevices => WorkspaceSection::RemoteDevices,
                    NavigationSection::ThisDevice => WorkspaceSection::ThisDevice,
                };
                send_intent(&sender, GuiIntent::SelectSection(section));
            });
        }
        {
            let sender = sender.clone();
            window.on_connect(move |address| {
                send_intent(&sender, GuiIntent::Connect(address.to_string()));
            });
        }
        {
            let sender = sender.clone();
            window.on_decide_request(move |index, accept| {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                send_intent(
                    &sender,
                    GuiIntent::DecideConnectionRequest { index, accept },
                );
            });
        }
        {
            window.on_recommended_bitrate(move |width, height, frame_rate| {
                recommended_bitrate_text(VideoCodec::H264, &width, &height, &frame_rate)
                    .unwrap_or_default()
                    .into()
            });
        }
        {
            let sender = sender.clone();
            window.on_open_screen(move |index, width, height, frame_rate, bitrate_mbps| {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                send_intent(
                    &sender,
                    GuiIntent::OpenRemoteScreen {
                        index,
                        width: width.to_string(),
                        height: height.to_string(),
                        frame_rate: frame_rate.to_string(),
                        bitrate_mbps: bitrate_mbps.to_string(),
                    },
                );
            });
        }
        {
            let sender = sender.clone();
            window.on_disconnect_remote_session(move || {
                send_intent(&sender, GuiIntent::DisconnectRemoteSession);
            });
        }
        {
            let sender = sender.clone();
            window.on_stop_hosted_screen_stream(move |index| {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                send_intent(&sender, GuiIntent::StopHostedScreenStream(index));
            });
        }
        {
            let sender = sender.clone();
            window.on_disconnect_device(move |index| {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                send_intent(&sender, GuiIntent::DisconnectDevice(index));
            });
        }
        {
            let sender = sender.clone();
            window.on_retry_connect(move || {
                send_intent(&sender, GuiIntent::RetryConnection);
            });
        }
        {
            let sender = sender.clone();
            window.on_stop_stream(move || {
                send_intent(&sender, GuiIntent::StopScreenStream);
            });
        }
        {
            let sender = sender.clone();
            let mailbox = PointerMailbox::default();
            window.on_remote_pointer_moved(
                move |x, y, delta_x, delta_y, viewport_width, viewport_height| {
                    let event = PointerViewportEvent {
                        x,
                        y,
                        delta_x,
                        delta_y,
                        viewport_width,
                        viewport_height,
                    };
                    match pointer_mode {
                        PointerMode::Absolute => mailbox.submit(event, &sender),
                        PointerMode::Relative => send_intent(
                            &sender,
                            GuiIntent::PointerMoved(PointerIntent::Direct(event)),
                        ),
                    }
                },
            );
        }
        {
            let sender = sender.clone();
            window.on_remote_mouse_button(move |button, pressed| {
                let Some(button) = mouse_button_from_slint(button) else {
                    return;
                };
                send_intent(
                    &sender,
                    GuiIntent::MouseButton {
                        button,
                        state: if pressed {
                            InputState::Pressed
                        } else {
                            InputState::Released
                        },
                    },
                );
            });
        }
        {
            let sender = sender.clone();
            window.on_remote_keyboard(move |text, pressed, repeat| {
                let Some(key) = keyboard_key_from_slint(text.as_str()) else {
                    return;
                };
                send_intent(
                    &sender,
                    GuiIntent::Keyboard {
                        key,
                        state: if pressed {
                            InputState::Pressed
                        } else {
                            InputState::Released
                        },
                        repeat,
                    },
                );
            });
        }
        let close_sender = sender.clone();
        window.window().on_close_requested(move || {
            if close_sender.send(GuiIntent::Close).is_ok() {
                CloseRequestResponse::KeepWindowShown
            } else {
                CloseRequestResponse::HideWindow
            }
        });

        let video = video_view::install::<VideoView>(&window, sender.clone(), probe_interval)?;
        let publisher = ViewPublisher {
            window: window.as_weak(),
            video,
        };
        Ok((
            Self {
                window,
                intent_sender: sender,
                video_stack: std::marker::PhantomData,
            },
            publisher,
            intents,
        ))
    }

    pub(crate) fn run(&self) -> eros::Result<()> {
        self.window
            .run()
            .context("Failed to run the Slint event loop")?;
        Ok(())
    }

    pub(crate) fn request_close(&self) {
        if self.intent_sender.send(GuiIntent::Close).is_err() {
            // The App thread has already completed, so there is nothing left to close.
        }
    }
}

impl<VideoView> ViewPublisher<VideoView>
where
    VideoView: video_view::VideoViewStack,
{
    pub(crate) fn publish(&self, state: ViewState) -> eros::Result<()> {
        let window = self.window.clone();
        slint::invoke_from_event_loop(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            apply_view_state(&window, state);
        })
        .context("Failed to publish Rabbit state to the Slint event loop")?;
        Ok(())
    }

    pub(crate) fn quit(&self) -> eros::Result<()> {
        slint::quit_event_loop().context("Failed to stop the Slint event loop")?;
        Ok(())
    }

    pub(crate) fn present_video(
        &self,
        session_id: crate::kernel::session::SessionId,
        screen_id: crate::kernel::screen_manager::ScreenId,
        frame: VideoView::Frame,
    ) -> eros::Result<()>
    where
        VideoView::Frame: Send,
    {
        self.video.present(session_id, screen_id, frame)
    }

    pub(crate) fn clear_video(&self) -> eros::Result<()> {
        self.video.clear()
    }
}

fn send_intent(sender: &flume::Sender<GuiIntent>, intent: GuiIntent) {
    if sender.send(intent).is_err()
        && let Err(error) = slint::quit_event_loop()
    {
        eprintln!("Failed to stop the Slint event loop after the App thread exited: {error}");
    }
}

fn apply_view_state(window: &RabbitWindow, state: ViewState) {
    window.set_section(match state.section {
        WorkspaceSection::RemoteDevices => NavigationSection::RemoteDevices,
        WorkspaceSection::ThisDevice => NavigationSection::ThisDevice,
    });
    window.set_page(match state.page {
        ViewPage::Connect => AppPage::Connect,
        ViewPage::Connecting => AppPage::Connecting,
        ViewPage::ConnectionError => AppPage::ConnectionError,
        ViewPage::Requests => AppPage::Requests,
        ViewPage::Connected => AppPage::Connected,
        ViewPage::StreamRequest => AppPage::StreamRequest,
        ViewPage::Streaming => AppPage::Streaming,
        ViewPage::StreamError => AppPage::StreamError,
    });
    window.set_page_title(state.page_title.into());
    window.set_page_subtitle(state.page_subtitle.into());
    window.set_status_text(state.status_text.into());
    window.set_stream_settings_error(state.stream_settings_error.into());
    window.set_local_protocol(state.local_protocol.into());
    window.set_local_port(state.local_port.into());
    window.set_local_server_online(state.local_server_online);
    window.set_stream_title(state.stream_title.into());
    window.set_stream_resolution(state.stream_resolution.into());
    window.set_connection_requests(connection_request_model(state.connection_requests));
    window.set_connected_devices(connected_device_model(state.connected_devices));
    window.set_hosted_screen_streams(hosted_screen_stream_model(state.hosted_screen_streams));
    window.set_remote_screens(remote_screen_model(state.remote_screens));
}

fn connection_request_model(entries: Vec<ConnectionRequestView>) -> ModelRc<ConnectionRequestItem> {
    ModelRc::new(VecModel::from(
        entries
            .into_iter()
            .map(|entry| ConnectionRequestItem {
                name: SharedString::from(entry.name),
                address: SharedString::from(entry.address),
            })
            .collect::<Vec<_>>(),
    ))
}

fn connected_device_model(entries: Vec<ConnectedDeviceView>) -> ModelRc<ConnectedDeviceItem> {
    ModelRc::new(VecModel::from(
        entries
            .into_iter()
            .map(|entry| ConnectedDeviceItem {
                name: SharedString::from(entry.name),
                address: SharedString::from(entry.address),
                status: SharedString::from(entry.status),
            })
            .collect::<Vec<_>>(),
    ))
}

fn hosted_screen_stream_model(
    entries: Vec<HostedScreenStreamView>,
) -> ModelRc<HostedScreenStreamItem> {
    ModelRc::new(VecModel::from(
        entries
            .into_iter()
            .map(|entry| HostedScreenStreamItem {
                device_name: SharedString::from(entry.device_name),
                screen_name: SharedString::from(entry.screen_name),
            })
            .collect::<Vec<_>>(),
    ))
}

fn remote_screen_model(entries: Vec<RemoteScreenView>) -> ModelRc<RemoteScreenItem> {
    ModelRc::new(VecModel::from(
        entries
            .into_iter()
            .map(|entry| RemoteScreenItem {
                name: SharedString::from(entry.name),
                original: SharedString::from(entry.original),
                selected_width: SharedString::from(entry.selected_width),
                selected_height: SharedString::from(entry.selected_height),
                selected_frame_rate: SharedString::from(entry.selected_frame_rate),
                selected_bitrate_mbps: SharedString::from(entry.selected_bitrate_mbps),
            })
            .collect::<Vec<_>>(),
    ))
}
