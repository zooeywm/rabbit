use crate::app::{
    gui::{
        application::RootApplication,
        state::{
            ConnectedDeviceView, ConnectionRequestView, DirectConnectionState,
            HostedScreenStreamView, RemoteScreenView, ScreenStreamState, ViewPage, ViewState,
            WorkspaceSection, format_frame_rate,
        },
    },
    platform::ApplicationStack,
};
use crate::kernel::screen_manager::ScreenLayoutManager;

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) fn publish_view_state(&self) -> eros::Result<()> {
        self.view.publish(self.view_state())
    }

    pub(super) fn view_state(&self) -> ViewState {
        let connection_requests = self
            .model
            .pending_connection_requests
            .iter()
            .map(|request| ConnectionRequestView {
                name: request.request().requester_name.clone(),
                address: request.remote_address().to_string(),
            })
            .collect::<Vec<_>>();

        let connected_devices = self
            .host_session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let session = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)?;
                let streaming = !session.screen_streams.is_empty();
                Some(ConnectedDeviceView {
                    name: session
                        .peer_name
                        .clone()
                        .unwrap_or_else(|| "Rabbit".to_string()),
                    address: session.key.peer_address().to_string(),
                    status: if streaming {
                        "Streaming".to_string()
                    } else {
                        "Connected".to_string()
                    },
                })
            })
            .collect::<Vec<_>>();

        let hosted_screen_streams = self
            .hosted_screen_stream_entries()
            .into_iter()
            .filter_map(|(session_id, screen_id)| {
                let session = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)?;
                let screen = self.model.app.screen(&screen_id)?;
                Some(HostedScreenStreamView {
                    device_name: session
                        .peer_name
                        .clone()
                        .unwrap_or_else(|| session.key.peer_address().to_string()),
                    screen_name: screen.name.clone(),
                })
            })
            .collect::<Vec<_>>();

        let remote_screens = self
            .model
            .remote_screen_entries
            .iter()
            .filter_map(|(session_id, screen_id)| {
                self.model
                    .remote_screens
                    .get(session_id)?
                    .iter()
                    .find(|screen| screen.id == *screen_id)
                    .map(|screen| {
                        let frame_rate = format_frame_rate(screen.frame_rate);
                        RemoteScreenView {
                            name: format!("Session {} · {}", session_id.0, screen.name),
                            original: format!(
                                "Original: {} × {} @ {} Hz",
                                screen.resolution.width, screen.resolution.height, frame_rate
                            ),
                            selected_width: screen.resolution.width.to_string(),
                            selected_height: screen.resolution.height.to_string(),
                            selected_frame_rate: frame_rate,
                        }
                    })
            })
            .collect::<Vec<_>>();
        let (page, page_title, page_subtitle, status_text, stream_title, stream_resolution) =
            match self.workspace.active_section {
                WorkspaceSection::ThisDevice if !connection_requests.is_empty() => (
                    ViewPage::Requests,
                    "Connection requests".to_string(),
                    "Devices are requesting access to this Rabbit instance".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                WorkspaceSection::ThisDevice => (
                    ViewPage::Connected,
                    "This device".to_string(),
                    if connected_devices.is_empty() {
                        "Waiting for clients to connect".to_string()
                    } else {
                        "Clients currently accessing this Rabbit instance".to_string()
                    },
                    self.workspace.status_message.clone(),
                    String::new(),
                    String::new(),
                ),
                WorkspaceSection::RemoteDevices => match &self.remote_stream.screen_stream {
                    ScreenStreamState::Requesting(target) => (
                        ViewPage::StreamRequest,
                        "Requesting screen stream...".to_string(),
                        format!(
                            "Requesting {} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Waiting for the remote device to configure the stream".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::WaitingForVideo(target) => (
                        ViewPage::StreamRequest,
                        "Starting screen stream...".to_string(),
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Waiting for the first video frame".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Streaming(target) => (
                        ViewPage::Streaming,
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Connected to the remote screen".to_string(),
                        "Receiving video frames".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Failed { target, message } => (
                        ViewPage::StreamError,
                        "Screen stream failed".to_string(),
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        message.clone(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Idle => match &self.listener.direct_connection {
                        DirectConnectionState::Connecting { target } => (
                            ViewPage::Connecting,
                            "Connecting...".to_string(),
                            format!("Connecting to {target}"),
                            "Waiting for the remote device to respond".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Rejected { target } => (
                            ViewPage::ConnectionError,
                            "Connection rejected".to_string(),
                            format!("Unable to connect to {target}"),
                            "The remote device rejected the connection request".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::SelfRejected { target } => (
                            ViewPage::ConnectionError,
                            "Cannot connect to this device".to_string(),
                            format!("{target} is this Rabbit instance"),
                            "Enter the address of another Rabbit instance".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Failed { target, message } => (
                            ViewPage::ConnectionError,
                            "Connection failed".to_string(),
                            format!("Unable to connect to {target}"),
                            message.clone(),
                            String::new(),
                            String::new(),
                        ),
                        _ if !remote_screens.is_empty() => (
                            ViewPage::Connected,
                            if let DirectConnectionState::Connected { peer } =
                                &self.listener.direct_connection
                            {
                                format!("Connected to {peer}")
                            } else {
                                "Remote devices".to_string()
                            },
                            "Select a remote screen to open".to_string(),
                            self.workspace.status_message.clone(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Connected { peer } => (
                            ViewPage::Connecting,
                            format!("Connected to {peer}"),
                            "Loading the remote screen list".to_string(),
                            "Connection established".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Idle => (
                            ViewPage::Connect,
                            "Connect to a device".to_string(),
                            "Enter the server IP address, hostname, or either with a port"
                                .to_string(),
                            self.workspace.status_message.clone(),
                            String::new(),
                            String::new(),
                        ),
                    },
                },
            };

        let (connection_requests, connected_devices, hosted_screen_streams, remote_screens) =
            match self.workspace.active_section {
                WorkspaceSection::RemoteDevices => {
                    (Vec::new(), Vec::new(), Vec::new(), remote_screens)
                }
                WorkspaceSection::ThisDevice => (
                    connection_requests,
                    connected_devices,
                    hosted_screen_streams,
                    Vec::new(),
                ),
            };

        ViewState {
            section: self.workspace.active_section,
            page,
            page_title,
            page_subtitle,
            status_text,
            stream_settings_error: self.workspace.stream_settings_error.clone(),
            local_protocol: self.listener.local_protocol.to_string(),
            local_port: self.listener.local_port.to_string(),
            local_server_online: self.listener.online,
            stream_title,
            stream_resolution,
            connection_requests,
            connected_devices,
            hosted_screen_streams,
            remote_screens,
        }
    }
}
