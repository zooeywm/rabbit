use eros::Context as _;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::gui::state::{
    ConnectedDeviceView, ConnectionRequestView, RemoteScreenView, ViewPage, ViewState,
};

slint::include_modules!();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuiIntent {
    Connect(String),
    DecideConnectionRequest { index: usize, accept: bool },
    OpenRemoteScreen(usize),
    RetryConnection,
    LeaveScreenStream,
    Close,
}

pub(crate) struct Gui {
    window: RabbitWindow,
    intent_sender: flume::Sender<GuiIntent>,
}

#[derive(Clone)]
pub(crate) struct ViewPublisher {
    window: slint::Weak<RabbitWindow>,
}

impl Gui {
    pub(crate) fn new() -> eros::Result<(Self, ViewPublisher, flume::Receiver<GuiIntent>)> {
        let window = RabbitWindow::new().context("Failed to create the Slint Rabbit window")?;
        let (sender, intents) = flume::unbounded();

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
            let sender = sender.clone();
            window.on_open_screen(move |index| {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                send_intent(&sender, GuiIntent::OpenRemoteScreen(index));
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
            window.on_leave_stream(move || {
                send_intent(&sender, GuiIntent::LeaveScreenStream);
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

        let publisher = ViewPublisher {
            window: window.as_weak(),
        };
        Ok((
            Self {
                window,
                intent_sender: sender,
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

impl ViewPublisher {
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
}

fn send_intent(sender: &flume::Sender<GuiIntent>, intent: GuiIntent) {
    if sender.send(intent).is_err()
        && let Err(error) = slint::quit_event_loop()
    {
        eprintln!("Failed to stop the Slint event loop after the App thread exited: {error}");
    }
}

fn apply_view_state(window: &RabbitWindow, state: ViewState) {
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
    window.set_local_port(state.local_port.into());
    window.set_local_server_online(state.local_server_online);
    window.set_stream_title(state.stream_title.into());
    window.set_stream_resolution(state.stream_resolution.into());
    window.set_connection_requests(connection_request_model(state.connection_requests));
    window.set_connected_devices(connected_device_model(state.connected_devices));
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

fn remote_screen_model(entries: Vec<RemoteScreenView>) -> ModelRc<RemoteScreenItem> {
    ModelRc::new(VecModel::from(
        entries
            .into_iter()
            .map(|entry| RemoteScreenItem {
                name: SharedString::from(entry.name),
                resolution: SharedString::from(entry.resolution),
            })
            .collect::<Vec<_>>(),
    ))
}
