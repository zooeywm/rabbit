use std::{
    collections::{HashSet, VecDeque},
    os::fd::AsFd as _,
    time::Duration,
};

use eros::Context as _;
use raw_window_handle::{
    HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle,
};
use smithay_client_toolkit::{
    delegate_dmabuf,
    dmabuf::{DmabufFeedback, DmabufHandler, DmabufState},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy as _, QueueHandle,
    backend::{Backend, ObjectId},
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_compositor, wl_region, wl_registry, wl_subcompositor, wl_subsurface,
        wl_surface,
    },
};
use wayland_protocols::wp::{
    linux_dmabuf::zv1::client::{zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};

use crate::{
    infra::platform::{
        client_video_probe::ClientVideoProbeReporter, video_decoder::GStreamerDecodedFrame,
        video_renderer::fit_viewport,
    },
    kernel::{geometry::PixelSize, video_renderer::VideoViewport},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WaylandVideoViewport {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[derive(Debug, Default)]
struct SupportedDmaBufFormats(HashSet<(u32, u64)>);

impl SupportedDmaBufFormats {
    fn from_feedback(feedback: &DmabufFeedback) -> Self {
        let table = feedback.format_table();
        let formats = feedback
            .tranches()
            .iter()
            .flat_map(|tranche| tranche.formats.iter())
            .filter_map(|index| table.get(usize::from(*index)))
            .map(|format| (format.format, format.modifier))
            .collect();
        Self(formats)
    }

    fn supports(&self, format: u32, modifier: u64) -> bool {
        self.0.contains(&(format, modifier))
    }
}

#[derive(Debug)]
struct SubmittedBuffer {
    buffer: wl_buffer::WlBuffer,
    _frame: GStreamerDecodedFrame,
}

#[derive(Debug)]
struct WaylandEventState {
    dmabuf: DmabufState,
    supported_formats: SupportedDmaBufFormats,
    released_buffers: Vec<ObjectId>,
}

impl DmabufHandler for WaylandEventState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf
    }

    fn dmabuf_feedback(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        feedback: DmabufFeedback,
    ) {
        self.supported_formats = SupportedDmaBufFormats::from_feedback(&feedback);
    }

    fn created(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        buffer: wl_buffer::WlBuffer,
    ) {
        buffer.destroy();
    }

    fn failed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
    ) {
    }

    fn released(&mut self, _: &Connection, _: &QueueHandle<Self>, buffer: &wl_buffer::WlBuffer) {
        self.released_buffers.push(buffer.id());
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandEventState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(WaylandEventState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandEventState: ignore wl_region::WlRegion);
delegate_noop!(WaylandEventState: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(WaylandEventState: ignore wl_subsurface::WlSubsurface);
delegate_noop!(WaylandEventState: ignore wl_surface::WlSurface);
delegate_noop!(WaylandEventState: ignore wp_viewport::WpViewport);
delegate_noop!(WaylandEventState: ignore wp_viewporter::WpViewporter);
delegate_dmabuf!(WaylandEventState);

pub(crate) struct WaylandVideoRenderer {
    connection: Connection,
    event_queue: EventQueue<WaylandEventState>,
    state: WaylandEventState,
    queue_handle: QueueHandle<WaylandEventState>,
    compositor: wl_compositor::WlCompositor,
    surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    viewport: wp_viewport::WpViewport,
    layout: Option<WaylandVideoViewport>,
    applied_viewport: Option<WaylandVideoViewport>,
    current_frame_size: Option<PixelSize>,
    pending_frame: Option<GStreamerDecodedFrame>,
    submitted: VecDeque<SubmittedBuffer>,
    probe_reporter: ClientVideoProbeReporter,
}

impl WaylandVideoRenderer {
    pub(crate) fn new(window: &slint::Window, probe_interval: Duration) -> eros::Result<Self> {
        let window_handle = window.window_handle();
        let display_handle = window_handle
            .display_handle()
            .with_context(|| "Slint did not expose a Wayland display handle")?;
        let surface_handle = window_handle
            .window_handle()
            .with_context(|| "Slint did not expose a Wayland window handle")?;
        let RawDisplayHandle::Wayland(display) = display_handle.as_raw() else {
            eros::bail!("Slint window is not using the Wayland display backend");
        };
        let RawWindowHandle::Wayland(window) = surface_handle.as_raw() else {
            eros::bail!("Slint window does not expose a Wayland parent surface");
        };

        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let parent_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                window.surface.as_ptr().cast(),
            )
        }
        .with_context(|| "Failed to import Slint's Wayland parent surface")?;
        let parent = wl_surface::WlSurface::from_id(&connection, parent_id)
            .with_context(|| "Failed to wrap Slint's Wayland parent surface")?;
        let (globals, mut event_queue) = registry_queue_init::<WaylandEventState>(&connection)
            .with_context(|| "Failed to discover Wayland globals for video display")?;
        let queue_handle = event_queue.handle();
        let compositor = globals
            .bind::<wl_compositor::WlCompositor, _, _>(&queue_handle, 1..=6, ())
            .with_context(|| "Wayland compositor global is unavailable")?;
        let subcompositor = globals
            .bind::<wl_subcompositor::WlSubcompositor, _, _>(&queue_handle, 1..=1, ())
            .with_context(|| "Wayland subcompositor global is unavailable")?;
        let viewporter = globals
            .bind::<wp_viewporter::WpViewporter, _, _>(&queue_handle, 1..=1, ())
            .with_context(|| "Wayland viewporter protocol is unavailable")?;
        let dmabuf = DmabufState::new(&globals, &queue_handle);
        let dmabuf_version = dmabuf
            .version()
            .with_context(|| "Wayland linux-dmabuf protocol version 3 or newer is unavailable")?;
        let mut state = WaylandEventState {
            dmabuf,
            supported_formats: SupportedDmaBufFormats::default(),
            released_buffers: Vec::new(),
        };

        let surface = compositor.create_surface(&queue_handle, ());
        let subsurface = subcompositor.get_subsurface(&surface, &parent, &queue_handle, ());
        subsurface.set_desync();
        let viewport = viewporter.get_viewport(&surface, &queue_handle, ());
        let input_region = compositor.create_region(&queue_handle, ());
        surface.set_input_region(Some(&input_region));
        input_region.destroy();

        if dmabuf_version >= 4 {
            let feedback = state
                .dmabuf
                .get_surface_feedback(&surface, &queue_handle)
                .with_context(|| "Failed to request Wayland surface DMA-BUF feedback")?;
            event_queue
                .roundtrip(&mut state)
                .with_context(|| "Failed to receive Wayland surface DMA-BUF feedback")?;
            feedback.destroy();
        } else {
            event_queue
                .roundtrip(&mut state)
                .with_context(|| "Failed to receive Wayland DMA-BUF modifiers")?;
            state.supported_formats.0.extend(
                state
                    .dmabuf
                    .modifiers()
                    .iter()
                    .map(|format| (format.format, format.modifier)),
            );
        }
        if state.supported_formats.0.is_empty() {
            eros::bail!("Wayland compositor advertised no usable DMA-BUF formats");
        }

        Ok(Self {
            connection,
            event_queue,
            state,
            queue_handle,
            compositor,
            surface,
            subsurface,
            viewport,
            layout: None,
            applied_viewport: None,
            current_frame_size: None,
            pending_frame: None,
            submitted: VecDeque::new(),
            probe_reporter: ClientVideoProbeReporter::new(probe_interval),
        })
    }

    pub(crate) fn set_viewport(&mut self, viewport: WaylandVideoViewport) -> eros::Result<()> {
        if viewport.width < 0 || viewport.height < 0 {
            eros::bail!(
                "Wayland video viewport has negative size {}x{}",
                viewport.width,
                viewport.height
            );
        }
        self.layout = Some(viewport);
        Ok(())
    }

    pub(crate) fn validate_frame(&self, frame: &GStreamerDecodedFrame) -> eros::Result<()> {
        let modifier: u64 = frame
            .buffer
            .planes
            .first()
            .with_context(|| "Decoded Wayland video frame has no DMA-BUF planes")?
            .modifier
            .into();
        let format = frame.buffer.format as u32;
        if !self.state.supported_formats.supports(format, modifier) {
            eros::bail!(
                "Wayland compositor does not support decoded DMA-BUF format {:?} modifier 0x{:016x}",
                frame.buffer.format,
                modifier
            );
        }
        Ok(())
    }

    pub(crate) fn present(&mut self, mut frame: GStreamerDecodedFrame) {
        if let Some(probe) = &mut frame.probe {
            probe.mark_gui_received();
        }
        self.pending_frame = Some(frame);
    }

    pub(crate) fn render(&mut self) -> eros::Result<()> {
        self.collect_released_buffers()?;
        let Some(layout) = self.layout else {
            return Ok(());
        };
        if layout.width == 0 || layout.height == 0 {
            self.hide_surface()?;
            return Ok(());
        }
        let frame_size = self
            .pending_frame
            .as_ref()
            .map(|frame| frame.buffer.size)
            .or(self.current_frame_size);
        let Some(frame_size) = frame_size else {
            return Ok(());
        };
        let fitted = fit_wayland_viewport(layout, frame_size)?;
        let geometry_changed = self.apply_viewport(fitted);

        let Some(mut frame) = self.pending_frame.take() else {
            if geometry_changed {
                self.surface.commit();
                self.connection
                    .flush()
                    .with_context(|| "Failed to flush resized Wayland video subsurface")?;
            }
            return Ok(());
        };
        if let Some(probe) = &mut frame.probe {
            probe.mark_dma_buf_import_started();
        }
        let params = self
            .state
            .dmabuf
            .create_params(&self.queue_handle)
            .with_context(|| "Failed to create Wayland DMA-BUF parameters")?;
        for (plane_index, plane) in frame.buffer.planes.iter().enumerate() {
            let object = frame
                .buffer
                .objects
                .get(plane.object_index)
                .with_context(|| {
                    format!(
                        "Decoded DMA-BUF plane {} references missing object {}",
                        plane_index, plane.object_index
                    )
                })?;
            params.add(
                object.fd.as_fd(),
                u32::try_from(plane_index)
                    .with_context(|| "Wayland DMA-BUF plane index exceeds u32")?,
                plane.offset,
                plane.stride,
                plane.modifier.into(),
            );
        }
        let width = i32::try_from(frame.buffer.size.width)
            .with_context(|| "Wayland DMA-BUF width exceeds i32")?;
        let height = i32::try_from(frame.buffer.size.height)
            .with_context(|| "Wayland DMA-BUF height exceeds i32")?;
        let (buffer, params) = params.create_immed(
            width,
            height,
            frame.buffer.format as u32,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &self.queue_handle,
        );
        params.destroy();
        if let Some(probe) = &mut frame.probe {
            probe.mark_dma_buf_import_completed();
            probe.mark_render_started();
        }
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
        self.connection
            .flush()
            .with_context(|| "Failed to flush Wayland video buffer commit")?;

        self.current_frame_size = Some(frame.buffer.size);
        let screen_id = frame.screen_id;
        if let Some(mut probe) = frame.probe.take() {
            probe.mark_render_completed();
            self.probe_reporter.record_frame(screen_id, probe);
        }
        self.submitted.push_back(SubmittedBuffer {
            buffer,
            _frame: frame,
        });
        Ok(())
    }

    pub(crate) fn clear(&mut self) -> eros::Result<()> {
        self.pending_frame = None;
        self.current_frame_size = None;
        self.applied_viewport = None;
        self.surface.set_opaque_region(None);
        self.surface.attach(None, 0, 0);
        self.surface.commit();
        self.connection
            .flush()
            .with_context(|| "Failed to flush cleared Wayland video subsurface")?;
        self.collect_released_buffers()?;
        self.probe_reporter.finish();
        Ok(())
    }

    fn apply_viewport(&mut self, viewport: WaylandVideoViewport) -> bool {
        if self.applied_viewport == Some(viewport) {
            return false;
        }
        self.subsurface.set_position(viewport.x, viewport.y);
        self.viewport
            .set_destination(viewport.width, viewport.height);
        let region = self.compositor.create_region(&self.queue_handle, ());
        region.add(0, 0, viewport.width, viewport.height);
        self.surface.set_opaque_region(Some(&region));
        region.destroy();
        self.applied_viewport = Some(viewport);
        true
    }

    fn hide_surface(&mut self) -> eros::Result<()> {
        if self.applied_viewport.take().is_none() {
            return Ok(());
        }
        self.surface.set_opaque_region(None);
        self.surface.attach(None, 0, 0);
        self.surface.commit();
        self.connection
            .flush()
            .with_context(|| "Failed to flush hidden Wayland video subsurface")?;
        Ok(())
    }

    pub(crate) fn teardown(&mut self) -> eros::Result<()> {
        self.clear()?;
        self.viewport.destroy();
        self.subsurface.destroy();
        self.surface.destroy();
        for submitted in self.submitted.drain(..) {
            submitted.buffer.destroy();
        }
        self.connection
            .flush()
            .with_context(|| "Failed to flush Wayland video display teardown")?;
        Ok(())
    }

    fn collect_released_buffers(&mut self) -> eros::Result<()> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .with_context(|| "Failed to dispatch Wayland video buffer events")?;
        if self.state.released_buffers.is_empty() {
            return Ok(());
        }
        let released = std::mem::take(&mut self.state.released_buffers);

        self.submitted.retain(|submitted| {
            if released.contains(&submitted.buffer.id()) {
                submitted.buffer.destroy();
                false
            } else {
                true
            }
        });
        Ok(())
    }
}

fn fit_wayland_viewport(
    viewport: WaylandVideoViewport,
    frame_size: PixelSize,
) -> eros::Result<WaylandVideoViewport> {
    let fitted = fit_viewport(
        VideoViewport {
            x: u32::try_from(viewport.x).with_context(|| "Wayland video viewport x is negative")?,
            y: u32::try_from(viewport.y).with_context(|| "Wayland video viewport y is negative")?,
            width: u32::try_from(viewport.width)
                .with_context(|| "Wayland video viewport width is negative")?,
            height: u32::try_from(viewport.height)
                .with_context(|| "Wayland video viewport height is negative")?,
        },
        frame_size,
    )?;
    Ok(WaylandVideoViewport {
        x: i32::try_from(fitted.x).with_context(|| "Fitted Wayland video x exceeds i32")?,
        y: i32::try_from(fitted.y).with_context(|| "Fitted Wayland video y exceeds i32")?,
        width: i32::try_from(fitted.width)
            .with_context(|| "Fitted Wayland video width exceeds i32")?,
        height: i32::try_from(fitted.height)
            .with_context(|| "Fitted Wayland video height exceeds i32")?,
    })
}

// Focused test: cargo test infra::platform::video_renderer::wayland::tests --lib
#[cfg(test)]
mod tests {
    use crate::infra::platform::video_renderer::wayland::{
        SupportedDmaBufFormats, WaylandVideoViewport, fit_wayland_viewport,
    };
    use crate::kernel::geometry::PixelSize;

    #[test]
    fn dma_buf_support_requires_an_exact_format_modifier_pair() {
        let formats = SupportedDmaBufFormats([(0x3231_564e, 7)].into_iter().collect());

        assert!(formats.supports(0x3231_564e, 7));
        assert!(!formats.supports(0x3231_564e, 8));
        assert!(!formats.supports(0x3432_5258, 7));
    }

    #[test]
    fn viewport_keeps_slint_logical_coordinates() {
        let viewport = WaylandVideoViewport {
            x: 12,
            y: 24,
            width: 960,
            height: 600,
        };

        assert_eq!(viewport.x, 12);
        assert_eq!(viewport.y, 24);
        assert_eq!(viewport.width, 960);
        assert_eq!(viewport.height, 600);
    }

    #[test]
    fn fits_video_inside_wayland_region_with_centered_letterbox() {
        let fitted = fit_wayland_viewport(
            WaylandVideoViewport {
                x: 12,
                y: 24,
                width: 960,
                height: 600,
            },
            PixelSize {
                width: 1920,
                height: 1080,
            },
        )
        .expect("Wayland video viewport should fit");

        assert_eq!(
            fitted,
            WaylandVideoViewport {
                x: 12,
                y: 54,
                width: 960,
                height: 540,
            }
        );
    }
}
