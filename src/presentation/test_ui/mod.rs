use gpui::{
    AppContext, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowOptions, div,
};
use gpui_component::{
    Disableable as _, Root, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::{
    app::AppHandle,
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

#[derive(Clone, Copy)]
enum StreamState {
    Stopped,
    Starting,
    Running(StreamId),
    Stopping,
    Restarting,
}

#[derive(Clone, Copy)]
enum CaptureOnlyState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

struct TestUi {
    app_handle: AppHandle,
    stream_state: StreamState,
    capture_only_state: CaptureOnlyState,
}

impl Render for TestUi {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .child("Rabbit Test UI")
            .child(
                Button::new("capture-only-action")
                    .label(self.capture_only_action_label())
                    .disabled(self.capture_only_action_disabled())
                    .on_click(cx.listener(|this, _, _, cx| match this.capture_only_state {
                        CaptureOnlyState::Stopped => this.start_capture_only(cx),
                        CaptureOnlyState::Running => this.stop_capture_only(cx),
                        CaptureOnlyState::Starting | CaptureOnlyState::Stopping => {}
                    })),
            )
            .child(
                Button::new("stream-action")
                    .primary()
                    .label(self.stream_action_label())
                    .disabled(self.stream_action_disabled())
                    .on_click(cx.listener(|this, _, _, cx| match this.stream_state {
                        StreamState::Stopped => this.start_stream(cx),
                        StreamState::Running(_) => this.stop_stream(cx),
                        StreamState::Starting | StreamState::Stopping | StreamState::Restarting => {
                        }
                    })),
            )
            .child(
                Button::new("restart")
                    .label("Restart")
                    .disabled(self.restart_disabled())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.restart_stream(cx);
                    })),
            )
    }
}

impl TestUi {
    fn stream_action_label(&self) -> &'static str {
        match self.stream_state {
            StreamState::Stopped => "Start",
            StreamState::Starting => "Starting...",
            StreamState::Running(_) => "Stop",
            StreamState::Stopping => "Stopping...",
            StreamState::Restarting => "Restarting...",
        }
    }

    fn stream_action_disabled(&self) -> bool {
        !matches!(self.capture_only_state, CaptureOnlyState::Stopped)
            || matches!(
                self.stream_state,
                StreamState::Starting | StreamState::Stopping | StreamState::Restarting
            )
    }

    fn restart_disabled(&self) -> bool {
        !matches!(self.capture_only_state, CaptureOnlyState::Stopped)
            || !matches!(self.stream_state, StreamState::Running(_))
    }

    fn capture_only_action_label(&self) -> &'static str {
        match self.capture_only_state {
            CaptureOnlyState::Stopped => "Start Capture Only",
            CaptureOnlyState::Starting => "Starting Capture...",
            CaptureOnlyState::Running => "Stop Capture Only",
            CaptureOnlyState::Stopping => "Stopping Capture...",
        }
    }

    fn capture_only_action_disabled(&self) -> bool {
        !matches!(self.stream_state, StreamState::Stopped)
            || matches!(
                self.capture_only_state,
                CaptureOnlyState::Starting | CaptureOnlyState::Stopping
            )
    }

    fn start_capture_only(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.capture_only_state, CaptureOnlyState::Stopped)
            || !matches!(self.stream_state, StreamState::Stopped)
        {
            return;
        }

        self.capture_only_state = CaptureOnlyState::Starting;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle
                .start_capture_only(CaptureSourceId::new(0))
                .await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("Test capture-only source started");
                        this.capture_only_state = CaptureOnlyState::Running;
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to start test capture-only source");
                        this.capture_only_state = CaptureOnlyState::Stopped;
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_capture_only(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.capture_only_state, CaptureOnlyState::Running) {
            return;
        }

        self.capture_only_state = CaptureOnlyState::Stopping;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle
                .stop_capture_only(CaptureSourceId::new(0))
                .await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("Test capture-only source stopped");
                        this.capture_only_state = CaptureOnlyState::Stopped;
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to stop test capture-only source");
                        this.capture_only_state = CaptureOnlyState::Running;
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_stream(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.stream_state, StreamState::Stopped) {
            return;
        }

        self.stream_state = StreamState::Starting;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle.start_stream(CaptureSourceId::new(0)).await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(stream_id) => {
                        tracing::info!("Test stream {} started", stream_id.value());
                        this.stream_state = StreamState::Running(stream_id);
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
        let StreamState::Running(stream_id) = self.stream_state else {
            return;
        };

        self.stream_state = StreamState::Stopping;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            let result = app_handle.remove_stream(stream_id).await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("Test stream {} stopped", stream_id.value());
                        this.stream_state = StreamState::Stopped;
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to stop test stream");
                        this.stream_state = StreamState::Running(stream_id);
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn restart_stream(&mut self, cx: &mut Context<Self>) {
        let StreamState::Running(stream_id) = self.stream_state else {
            return;
        };

        self.stream_state = StreamState::Restarting;
        cx.notify();

        let app_handle = self.app_handle.clone();

        cx.spawn(async move |this, cx| {
            if let Err(error) = app_handle.remove_stream(stream_id).await {
                this.update(cx, |this, cx| {
                    tracing::error!(?error, "Failed to stop test stream for restart");
                    this.stream_state = StreamState::Running(stream_id);
                    cx.notify();
                })
                .ok();

                return;
            }

            let result = app_handle.start_stream(CaptureSourceId::new(0)).await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(new_stream_id) => {
                        tracing::info!(
                            "Test stream {} restarted as {}",
                            stream_id.value(),
                            new_stream_id.value()
                        );
                        this.stream_state = StreamState::Running(new_stream_id);
                    }
                    Err(error) => {
                        tracing::error!(?error, "Failed to restart test stream");
                        this.stream_state = StreamState::Stopped;
                    }
                }

                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn run(app_handle: AppHandle) -> eros::Result<()> {
    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| TestUi {
                    app_handle,
                    stream_state: StreamState::Stopped,
                    capture_only_state: CaptureOnlyState::Stopped,
                });

                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open test UI window");
        })
        .detach();
    });
    Ok(())
}
