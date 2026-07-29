use std::{
    os::fd::{AsFd as _, AsRawFd as _},
    time::Duration,
};

use eros::Context as _;
use smithay_client_toolkit::{
    delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, Dispatch, Proxy as _, QueueHandle, WEnum,
    globals::{GlobalList, registry_queue_init},
    protocol::wl_output,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

pub(crate) struct WaylandDamageNotifier {
    connection: Connection,
    event_queue: wayland_client::EventQueue<WaylandDamageState>,
    state: WaylandDamageState,
    queue_handle: QueueHandle<WaylandDamageState>,
}

struct WaylandDamageState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    output: Option<wl_output::WlOutput>,
    buffer: Option<smithay_client_toolkit::shm::slot::Buffer>,
    request_pending: bool,
    baseline_ready: bool,
    damaged: bool,
    failed: bool,
}

impl WaylandDamageNotifier {
    pub(crate) fn new(output_name: &str) -> eros::Result<Self> {
        let connection = Connection::connect_to_env()
            .with_context(|| "Failed to connect Wayland damage notifier")?;
        let (globals, mut event_queue) = registry_queue_init::<WaylandDamageState>(&connection)
            .with_context(|| "Failed to initialize Wayland damage registry")?;
        ensure_screencopy_v2(&globals)?;
        let queue_handle = event_queue.handle();
        let registry_state = RegistryState::new(&globals);
        let output_state = OutputState::new(&globals, &queue_handle);
        let shm = Shm::bind(&globals, &queue_handle)
            .with_context(|| "Wayland compositor does not expose wl_shm")?;
        let pool = SlotPool::new(1, &shm)
            .with_context(|| "Failed to allocate Wayland damage notification pool")?;
        let manager = globals
            .bind(&queue_handle, 2..=2, ())
            .with_context(|| "Wayland compositor does not expose screencopy v2 damage events")?;
        let mut state = WaylandDamageState {
            registry_state,
            output_state,
            shm,
            pool,
            manager,
            output: None,
            buffer: None,
            request_pending: false,
            baseline_ready: false,
            damaged: false,
            failed: false,
        };
        event_queue
            .roundtrip(&mut state)
            .with_context(|| "Failed to enumerate Wayland outputs for damage notification")?;
        state.output = state.output_state.outputs().find(|output| {
            state
                .output_state
                .info(output)
                .and_then(|info| info.name)
                .is_some_and(|name| name == output_name)
        });
        if state.output.is_none() {
            eros::bail!("Wayland output {output_name:?} is unavailable for dynamic frame rate");
        }
        tracing::info!(
            target: "rabbit::screen_capture::wayland_damage",
            event = "wayland_damage_notifier_ready",
            output = output_name,
            "Using Wayland screencopy damage to trigger dynamic KMS capture"
        );
        Ok(Self {
            connection,
            event_queue,
            state,
            queue_handle,
        })
    }

    pub(crate) fn wait(&mut self, timeout: Duration) -> eros::Result<bool> {
        if !self.state.request_pending {
            let output = self
                .state
                .output
                .as_ref()
                .with_context(|| "Wayland damage notifier lost its output")?;
            self.state
                .manager
                .capture_output(0, output, &self.queue_handle, ());
            self.state.request_pending = true;
            self.state.damaged = false;
            self.state.failed = false;
            self.connection
                .flush()
                .with_context(|| "Failed to flush Wayland damage request")?;
        }

        self.event_queue
            .dispatch_pending(&mut self.state)
            .with_context(|| "Failed to dispatch pending Wayland damage events")?;
        self.connection
            .flush()
            .with_context(|| "Failed to flush Wayland screencopy buffer request")?;
        if self.finish_request()? {
            return Ok(true);
        }
        let Some(read_guard) = self.event_queue.prepare_read() else {
            return Ok(false);
        };
        let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        let mut descriptor = libc::pollfd {
            fd: read_guard.connection_fd().as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready < 0 {
            Err(std::io::Error::last_os_error())
                .with_context(|| "Failed to poll Wayland damage events")?;
        }
        if ready == 0 {
            drop(read_guard);
            return Ok(false);
        }
        read_guard
            .read()
            .with_context(|| "Failed to read Wayland damage events")?;
        self.event_queue
            .dispatch_pending(&mut self.state)
            .with_context(|| "Failed to dispatch Wayland damage events")?;
        self.connection
            .flush()
            .with_context(|| "Failed to flush Wayland damage event responses")?;
        self.finish_request()
    }

    fn finish_request(&mut self) -> eros::Result<bool> {
        if self.state.failed {
            eros::bail!("Wayland screencopy damage request failed");
        }
        if !self.state.damaged {
            return Ok(false);
        }
        self.state.request_pending = false;
        self.state.buffer = None;
        Ok(true)
    }
}

fn ensure_screencopy_v2(globals: &GlobalList) -> eros::Result<()> {
    Ok(globals
        .contents()
        .with_list(|globals| {
            globals.iter().any(|global| {
                global.interface
                    == zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1::interface().name
                    && global.version >= 2
            })
        })
        .then_some(())
        .with_context(|| "Wayland compositor does not expose screencopy v2 damage events")?)
}

impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for WaylandDamageState {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _event: zwlr_screencopy_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for WaylandDamageState {
    fn event(
        state: &mut Self,
        frame: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let WEnum::Value(format) = format else {
                    state.failed = true;
                    return;
                };
                let dimensions = (
                    i32::try_from(width),
                    i32::try_from(height),
                    i32::try_from(stride),
                );
                let (Ok(width), Ok(height), Ok(stride)) = dimensions else {
                    state.failed = true;
                    return;
                };
                match state.pool.create_buffer(width, height, stride, format) {
                    Ok((buffer, _)) => {
                        if state.baseline_ready {
                            frame.copy_with_damage(buffer.wl_buffer());
                        } else {
                            frame.copy(buffer.wl_buffer());
                        }
                        state.buffer = Some(buffer);
                    }
                    Err(_) => state.failed = true,
                }
            }
            zwlr_screencopy_frame_v1::Event::Damage { .. } => {
                state.damaged = true;
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                frame.destroy();
                state.baseline_ready = true;
                state.damaged = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.destroy();
                state.failed = true;
            }
            _ => {}
        }
    }
}

impl OutputHandler for WaylandDamageState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.output.as_ref() == Some(&output) {
            self.output = None;
            self.failed = true;
        }
    }
}

impl ShmHandler for WaylandDamageState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for WaylandDamageState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_output!(WaylandDamageState);
delegate_registry!(WaylandDamageState);
delegate_shm!(WaylandDamageState);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::WaylandDamageNotifier;

    #[test]
    #[ignore = "requires a Wayland compositor with wlr-screencopy v2"]
    fn receives_output_damage() {
        let output =
            std::env::var("RABBIT_KMS_SCREEN").expect("RABBIT_KMS_SCREEN must select an output");
        let mut notifier =
            WaylandDamageNotifier::new(&output).expect("Damage notifier should initialize");
        while !notifier
            .wait(Duration::from_millis(100))
            .expect("Initial screencopy should remain healthy")
        {}
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if notifier
                .wait(Duration::from_millis(100))
                .expect("Damage notification should remain healthy")
            {
                return;
            }
        }
        panic!("Expected compositor damage within three seconds");
    }
}
