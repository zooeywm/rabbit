use winio::prelude::*;

pub(crate) struct RootView {
    window: Child<Window>,
    direct_address_input: Child<Edit>,
    connect_button: Child<Button>,
    connection_status: Child<Label>,
    remote_screen_title: Child<Label>,
    remote_screen_list: Child<ListBox>,
    connection_request_title: Child<Label>,
    connection_request_list: Child<ListBox>,
    accept_connection_button: Child<Button>,
    reject_connection_button: Child<Button>,
}

pub(crate) struct RootViewInit {
    pub(crate) local_port: u16,
}

pub(crate) enum RootViewEvent {
    Close,
    ConnectDirect(String),
    ConnectionRequestSelected(Option<usize>),
    AcceptConnection(Option<usize>),
    RejectConnection(Option<usize>),
    RemoteScreenSelected(Option<usize>),
}

pub(crate) enum RootViewMessage {
    Noop,
    Close,
    ConnectDirect,
    ConnectionRequestSelectionChanged,
    AcceptConnection,
    RejectConnection,
    RemoteScreenSelectionChanged,
    SetConnecting(bool),
    SetConnectionStatus(String),
    SetConnectionRequests {
        entries: Vec<String>,
        selected: Option<usize>,
    },
    SetRemoteScreens(Vec<String>),
}

impl RootView {
    fn selected_index(list: &Child<ListBox>) -> eros::Result<Option<usize>> {
        for index in 0..list.len()? {
            if list.is_selected(index)? {
                return Ok(Some(index));
            }
        }

        Ok(None)
    }

    fn set_connection_request_panel_visible(&mut self, visible: bool) -> eros::Result<()> {
        self.connection_request_title.set_visible(visible)?;
        self.connection_request_list.set_visible(visible)?;
        self.accept_connection_button.set_visible(visible)?;
        self.reject_connection_button.set_visible(visible)?;
        Ok(())
    }
}

impl Component for RootView {
    type Error = eros::ErrorUnion;
    type Event = RootViewEvent;
    type Init<'a> = RootViewInit;
    type Message = RootViewMessage;

    async fn init(init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> eros::Result<Self> {
        init! {
            window: Window = (()) => {
                text: format!("Rabbit - UDP {}", init.local_port),
                size: Size::new(800.0, 600.0),
            },
            direct_address_input: Edit = (&window) => {
                text: "127.0.0.1",
            },
            connect_button: Button = (&window) => {
                text: "Connect",
            },
            connection_status: Label = (&window) => {
                text: format!("Listening on UDP {}", init.local_port),
            },
            remote_screen_title: Label = (&window) => {
                text: "Remote screens",
                visible: false,
            },
            remote_screen_list: ListBox = (&window) => {
                multiple: false,
                visible: false,
            },
            connection_request_title: Label = (&window) => {
                text: "Pending connection requests",
                visible: false,
            },
            connection_request_list: ListBox = (&window) => {
                multiple: false,
                visible: false,
            },
            accept_connection_button: Button = (&window) => {
                text: "Accept",
                visible: false,
            },
            reject_connection_button: Button = (&window) => {
                text: "Reject",
                visible: false,
            },
        }
        window.show()?;

        Ok(Self {
            window,
            direct_address_input,
            connect_button,
            connection_status,
            remote_screen_title,
            remote_screen_list,
            connection_request_title,
            connection_request_list,
            accept_connection_button,
            reject_connection_button,
        })
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        start! {
            sender, default: RootViewMessage::Noop,
            self.window => {
                WindowEvent::Close => RootViewMessage::Close,
            },
            self.connect_button => {
                ButtonEvent::Click => RootViewMessage::ConnectDirect,
            },
            self.connection_request_list => {
                ListBoxEvent::Select => RootViewMessage::ConnectionRequestSelectionChanged,
            },
            self.remote_screen_list => {
                ListBoxEvent::Select => RootViewMessage::RemoteScreenSelectionChanged,
            },
            self.accept_connection_button => {
                ButtonEvent::Click => RootViewMessage::AcceptConnection,
            },
            self.reject_connection_button => {
                ButtonEvent::Click => RootViewMessage::RejectConnection,
            },
        }
    }

    async fn update_children(&mut self) -> eros::Result<bool> {
        update_children!(
            self.window,
            self.direct_address_input,
            self.connect_button,
            self.connection_status,
            self.remote_screen_title,
            self.remote_screen_list,
            self.connection_request_title,
            self.connection_request_list,
            self.accept_connection_button,
            self.reject_connection_button,
        )
    }

    async fn update(
        &mut self,
        message: Self::Message,
        sender: &ComponentSender<Self>,
    ) -> eros::Result<bool> {
        match message {
            RootViewMessage::Noop => Ok(false),
            RootViewMessage::Close => {
                sender.output(RootViewEvent::Close);
                Ok(false)
            }
            RootViewMessage::ConnectDirect => {
                sender.output(RootViewEvent::ConnectDirect(
                    self.direct_address_input.text()?,
                ));
                Ok(false)
            }
            RootViewMessage::ConnectionRequestSelectionChanged => {
                sender.output(RootViewEvent::ConnectionRequestSelected(
                    Self::selected_index(&self.connection_request_list)?,
                ));
                Ok(false)
            }
            RootViewMessage::AcceptConnection => {
                sender.output(RootViewEvent::AcceptConnection(Self::selected_index(
                    &self.connection_request_list,
                )?));
                Ok(false)
            }
            RootViewMessage::RejectConnection => {
                sender.output(RootViewEvent::RejectConnection(Self::selected_index(
                    &self.connection_request_list,
                )?));
                Ok(false)
            }
            RootViewMessage::RemoteScreenSelectionChanged => {
                sender.output(RootViewEvent::RemoteScreenSelected(Self::selected_index(
                    &self.remote_screen_list,
                )?));
                Ok(false)
            }
            RootViewMessage::SetConnecting(connecting) => {
                self.connect_button.set_enabled(!connecting)?;
                Ok(false)
            }
            RootViewMessage::SetConnectionStatus(status) => {
                self.connection_status.set_text(status)?;
                Ok(true)
            }
            RootViewMessage::SetConnectionRequests { entries, selected } => {
                let visible = !entries.is_empty();
                self.connection_request_list.set_items(entries)?;
                self.set_connection_request_panel_visible(visible)?;

                if let Some(selected) = selected {
                    self.connection_request_list.set_selected(selected, true)?;
                }

                Ok(true)
            }
            RootViewMessage::SetRemoteScreens(entries) => {
                let visible = !entries.is_empty();
                self.remote_screen_list.set_items(entries)?;
                self.remote_screen_title.set_visible(visible)?;
                self.remote_screen_list.set_visible(visible)?;
                Ok(true)
            }
        }
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> eros::Result<()> {
        let size = self.window.size()?;
        let mut direct_connection = layout! {
            StackPanel::new(Orient::Horizontal),
            self.direct_address_input => { grow: true },
            self.connect_button,
        };
        let mut actions = layout! {
            StackPanel::new(Orient::Horizontal),
            self.reject_connection_button => { grow: true },
            self.accept_connection_button => { grow: true },
        };
        let mut panel = layout! {
            StackPanel::new(Orient::Vertical),
            direct_connection,
            self.connection_status,
            self.remote_screen_title,
            self.remote_screen_list => { grow: true },
            self.connection_request_title,
            self.connection_request_list => { grow: true },
            actions,
        };

        panel.set_size(size)?;
        Ok(())
    }
}
