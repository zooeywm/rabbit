use gpui::{
    AppContext, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowOptions, div,
};
use gpui_component::{
    Disableable as _, Root, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::{
    app::{AppHandle, RunningApp},
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

#[derive(Clone, Copy)]
enum StreamMode {
    Full,
    HostOnly,
}

#[derive(Clone, Copy)]
enum StreamState {
    Stopped,
    Starting(StreamMode),
    Running(StreamId, StreamMode),
    Stopping(StreamMode),
}

struct TestUi {
    app_handle: AppHandle,
    stream_state: StreamState,
    next_stream_id: u16,
}

impl Render for TestUi {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .child("Rabbit Test UI")
            .child(
                Button::new("stream-action")
                    .primary()
                    .label(self.stream_action_label(StreamMode::Full))
                    .disabled(self.stream_action_disabled(StreamMode::Full))
                    .on_click(cx.listener(|this, _, _, cx| match this.stream_state {
                        StreamState::Stopped => this.start_stream(cx),
                        StreamState::Running(_, StreamMode::Full) => this.stop_stream(cx),
                        _ => {}
                    })),
            )
            .child(
                Button::new("host-only-stream-action")
                    .label(self.stream_action_label(StreamMode::HostOnly))
                    .disabled(self.stream_action_disabled(StreamMode::HostOnly))
                    .on_click(cx.listener(|this, _, _, cx| match this.stream_state {
                        StreamState::Stopped => this.start_host_only_stream(cx),
                        StreamState::Running(_, StreamMode::HostOnly) => this.stop_stream(cx),
                        _ => {}
                    })),
            )
    }
}

impl TestUi {
    fn stream_action_label(&self, mode: StreamMode) -> &'static str {
        match (self.stream_state, mode) {
            (StreamState::Starting(StreamMode::Full), StreamMode::Full) => "Starting...",
            (StreamState::Starting(StreamMode::HostOnly), StreamMode::HostOnly) => {
                "Starting Host Only..."
            }
            (StreamState::Running(_, StreamMode::Full), StreamMode::Full) => "Stop",
            (StreamState::Running(_, StreamMode::HostOnly), StreamMode::HostOnly) => {
                "Stop Host Only"
            }
            (StreamState::Stopping(StreamMode::Full), StreamMode::Full) => "Stopping...",
            (StreamState::Stopping(StreamMode::HostOnly), StreamMode::HostOnly) => {
                "Stopping Host Only..."
            }
            (_, StreamMode::Full) => "Start",
            (_, StreamMode::HostOnly) => "Start Host Only",
        }
    }

    fn stream_action_disabled(&self, mode: StreamMode) -> bool {
        !matches!(
            (self.stream_state, mode),
            (StreamState::Stopped, _)
                | (StreamState::Running(_, StreamMode::Full), StreamMode::Full)
                | (
                    StreamState::Running(_, StreamMode::HostOnly),
                    StreamMode::HostOnly
                )
        )
    }

    fn start_stream(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.stream_state, StreamState::Stopped) {
            return;
        }

        self.stream_state = StreamState::Starting(StreamMode::Full);
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle.start_stream(CaptureSourceId::new(0)).await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(stream_id) => {
                        tracing::info!("Test stream {} started", stream_id.value());
                        this.stream_state = StreamState::Running(stream_id, StreamMode::Full);
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to start test stream");
                        this.stream_state = StreamState::Stopped;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_host_only_stream(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.stream_state, StreamState::Stopped) {
            return;
        }

        self.stream_state = StreamState::Starting(StreamMode::HostOnly);
        let stream_id = StreamId::new(self.next_stream_id);
        let Some(next_stream_id) = self.next_stream_id.checked_add(1) else {
            tracing::error!("Test stream ID space is exhausted");
            self.stream_state = StreamState::Stopped;
            cx.notify();
            return;
        };
        self.next_stream_id = next_stream_id;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle
                .simulate_remote_start_stream(CaptureSourceId::new(0), stream_id)
                .await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("Test stream {} started", stream_id.value());
                        this.stream_state = StreamState::Running(stream_id, StreamMode::HostOnly);
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to start test stream");
                        this.stream_state = StreamState::Stopped;
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_stream(&mut self, cx: &mut Context<Self>) {
        let StreamState::Running(stream_id, mode) = self.stream_state else {
            return;
        };

        self.stream_state = StreamState::Stopping(mode);
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = match mode {
                StreamMode::Full => app_handle.remove_stream(stream_id).await,
                StreamMode::HostOnly => app_handle.simulate_remote_remove_stream(stream_id).await,
            };

            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("Test stream {} stopped", stream_id.value());
                        this.stream_state = StreamState::Stopped;
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to stop test stream");
                        this.stream_state = StreamState::Running(stream_id, mode);
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn run(app: RunningApp) -> eros::Result<()> {
    let app_handle = app.handle();

    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| TestUi {
                    app_handle,
                    stream_state: StreamState::Stopped,
                    next_stream_id: 0,
                });

                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open test UI window");
        })
        .detach();
    });

    drop(app);
    Ok(())
}
