use std::{
    collections::HashMap,
    io,
    thread::{self, JoinHandle},
};

use eros::Context;
use flume::{Receiver, RecvError, Selector, Sender, bounded, unbounded};
use gbm::{Format, Modifier};

use crate::{
    infra::WorkerReaperHandle,
    infra::platform::{
        dma_buf::DmaBufPool,
        frame_pipeline::{GbmFramePipelineFrame, SharedFramePipelineError},
        gpu::{GpuContext, GpuDevice, Nv12OutputStrategy},
        screen_capture::{EglDmaBufImage, KmsCapturedFrame, KmsFrameReceiver},
        video_encoder::{hardware_h264_encoder_for, va_vpp_input_modifiers},
    },
    kernel::{frame_pipeline::FramePipelineParameters, screen_manager::ScreenId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct FramePipelineId(pub(super) u64);

#[derive(Debug)]
enum GpuWorkerCommand {
    RegisterScreen {
        screen_id: ScreenId,
        frames: KmsFrameReceiver,
    },
    ReleaseScreen(ScreenId),
    RegisterPipeline {
        id: FramePipelineId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
        outputs: LatestSender<eros::Result<GbmFramePipelineFrame>>,
    },
    ReleasePipeline(FramePipelineId),
    Shutdown,
}

enum GpuWorkerEvent {
    Command(Result<GpuWorkerCommand, RecvError>),
    ScreenReady(ScreenId, Result<eros::Result<GpuDevice>, RecvError>),
    Frame(
        ScreenId,
        Result<eros::Result<Box<KmsCapturedFrame>>, RecvError>,
    ),
}

#[derive(Debug)]
pub(super) enum GpuWorkerNotification {
    ScreenFailed {
        screen_id: ScreenId,
        error: eros::ErrorUnion,
    },
}

#[derive(Debug)]
pub(super) struct GpuWorker {
    commands: Sender<GpuWorkerCommand>,
    thread: Option<JoinHandle<()>>,
    reaper: WorkerReaperHandle,
}

#[derive(Debug)]
pub(super) struct GpuPipelineRegistration {
    id: FramePipelineId,
    commands: Sender<GpuWorkerCommand>,
}

#[derive(Debug)]
pub(super) struct GpuPipelineSource {
    pub(super) registration: GpuPipelineRegistration,
    pub(super) frames: Receiver<eros::Result<GbmFramePipelineFrame>>,
}

#[derive(Debug)]
struct GpuPipeline {
    screen_id: ScreenId,
    parameters: FramePipelineParameters,
    outputs: LatestSender<eros::Result<GbmFramePipelineFrame>>,
    output_strategy: Option<FrameOutputStrategy>,
    output_pool: DmaBufPool,
}

const OUTPUT_POOL_CAPACITY: usize = 3;

#[derive(Debug, Clone, Copy)]
enum FrameOutputStrategy {
    PassthroughXrgb(Modifier),
    DirectNv12(Nv12OutputStrategy),
    VaapiXrgb(Modifier),
}

struct GpuScreen {
    device: Option<Receiver<eros::Result<GpuDevice>>>,
    frames: Receiver<eros::Result<KmsCapturedFrame>>,
}

#[derive(Debug)]
struct BoundGpu {
    device: GpuDevice,
    context: GpuContext,
}

#[derive(Debug)]
struct LatestSender<T> {
    sender: Sender<T>,
    overflow_receiver: Receiver<T>,
}

#[derive(Debug)]
pub(super) struct GpuScreenRegistration {
    screen_id: ScreenId,
    commands: Sender<GpuWorkerCommand>,
}

impl GpuWorker {
    pub(super) fn new(
        reaper: WorkerReaperHandle,
    ) -> io::Result<(Self, Receiver<GpuWorkerNotification>)> {
        let (commands, command_receiver) = unbounded();
        let (notification_sender, notifications) = bounded(1);
        let thread = thread::Builder::new()
            .name("rabbit-gpu".to_owned())
            .spawn(move || run_worker(command_receiver, notification_sender))?;

        Ok((
            Self {
                commands,
                thread: Some(thread),
                reaper,
            },
            notifications,
        ))
    }

    pub(super) fn register_screen(
        &self,
        screen_id: ScreenId,
        frames: KmsFrameReceiver,
    ) -> eros::Result<GpuScreenRegistration> {
        self.commands
            .send(GpuWorkerCommand::RegisterScreen { screen_id, frames })
            .with_context(|| "Failed to register a captured screen with the GPU worker")?;

        Ok(GpuScreenRegistration {
            screen_id,
            commands: self.commands.clone(),
        })
    }

    pub(super) fn register_pipeline(
        &self,
        id: FramePipelineId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
    ) -> eros::Result<GpuPipelineSource> {
        let (output_sender, frames) = bounded(1);
        let outputs = LatestSender {
            sender: output_sender,
            overflow_receiver: frames.clone(),
        };

        self.commands
            .send(GpuWorkerCommand::RegisterPipeline {
                id,
                screen_id,
                parameters,
                outputs,
            })
            .with_context(|| "Failed to register a frame pipeline with the GPU worker")?;

        Ok(GpuPipelineSource {
            registration: GpuPipelineRegistration {
                id,
                commands: self.commands.clone(),
            },
            frames,
        })
    }

    #[cfg(test)]
    pub(super) fn thread_id(&self) -> thread::ThreadId {
        self.thread
            .as_ref()
            .expect("GPU worker thread should exist during the test")
            .thread()
            .id()
    }
}

impl Drop for GpuWorker {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };

        let _ = self.commands.send(GpuWorkerCommand::Shutdown);
        self.reaper.reap(thread);
    }
}

impl Drop for GpuPipelineRegistration {
    fn drop(&mut self) {
        let _ = self
            .commands
            .send(GpuWorkerCommand::ReleasePipeline(self.id));
    }
}

impl Drop for GpuScreenRegistration {
    fn drop(&mut self) {
        let _ = self
            .commands
            .send(GpuWorkerCommand::ReleaseScreen(self.screen_id));
    }
}

fn run_worker(commands: Receiver<GpuWorkerCommand>, notifications: Sender<GpuWorkerNotification>) {
    let mut screens = HashMap::new();
    let mut pipelines = HashMap::new();
    let mut gpu = None;

    loop {
        match wait_for_event(&commands, &screens) {
            GpuWorkerEvent::Command(Ok(GpuWorkerCommand::RegisterScreen { screen_id, frames })) => {
                let (device, frames) = frames.into_parts();
                screens.insert(
                    screen_id,
                    GpuScreen {
                        device: Some(device),
                        frames,
                    },
                );
            }
            GpuWorkerEvent::Command(Ok(GpuWorkerCommand::ReleaseScreen(screen_id))) => {
                screens.remove(&screen_id);
            }
            GpuWorkerEvent::Command(Ok(GpuWorkerCommand::RegisterPipeline {
                id,
                screen_id,
                parameters,
                outputs,
            })) => {
                pipelines.insert(
                    id,
                    GpuPipeline {
                        screen_id,
                        parameters,
                        outputs,
                        output_strategy: None,
                        output_pool: DmaBufPool::new(OUTPUT_POOL_CAPACITY),
                    },
                );
            }
            GpuWorkerEvent::Command(Ok(GpuWorkerCommand::ReleasePipeline(id))) => {
                pipelines.remove(&id);
            }
            GpuWorkerEvent::Command(Ok(GpuWorkerCommand::Shutdown) | Err(_)) => return,
            GpuWorkerEvent::ScreenReady(screen_id, Ok(Ok(device))) => {
                let mismatch = gpu
                    .as_ref()
                    .and_then(|bound: &BoundGpu| gpu_mismatch(&bound.device, screen_id, &device));

                if let Some(error) = mismatch {
                    screens.remove(&screen_id);
                    if !notify_screen_failed(&notifications, screen_id, error) {
                        return;
                    }
                    continue;
                }

                if gpu.is_none() {
                    let context = match GpuContext::new(&device).with_context(|| {
                        format!(
                            "Failed to initialize GPU processing for screen {} on {}",
                            screen_id.0,
                            device.render_node_path().display()
                        )
                    }) {
                        Ok(context) => context,
                        Err(error) => {
                            screens.remove(&screen_id);
                            if !notify_screen_failed(&notifications, screen_id, error) {
                                return;
                            }
                            continue;
                        }
                    };
                    gpu = Some(BoundGpu { device, context });
                }
                if let Some(screen) = screens.get_mut(&screen_id) {
                    screen.device = None;
                }
            }
            GpuWorkerEvent::ScreenReady(screen_id, Ok(Err(error))) => {
                screens.remove(&screen_id);
                if !notify_screen_failed(&notifications, screen_id, error) {
                    return;
                }
            }
            GpuWorkerEvent::ScreenReady(screen_id, Err(_)) => {
                screens.remove(&screen_id);
            }
            GpuWorkerEvent::Frame(screen_id, Ok(Ok(frame))) => {
                let Some(gpu) = &gpu else {
                    screens.remove(&screen_id);
                    if !notify_screen_failed(
                        &notifications,
                        screen_id,
                        eros::error!(
                            "GPU context is missing while processing screen {}",
                            screen_id.0
                        ),
                    ) {
                        return;
                    }
                    continue;
                };

                process_screen_frame(&gpu.context, screen_id, *frame, &mut pipelines);
            }
            GpuWorkerEvent::Frame(screen_id, Ok(Err(error))) => {
                screens.remove(&screen_id);
                if !notify_screen_failed(&notifications, screen_id, error) {
                    return;
                }
            }
            GpuWorkerEvent::Frame(screen_id, Err(_)) => {
                screens.remove(&screen_id);
            }
        }
    }
}

fn gpu_mismatch(
    bound: &GpuDevice,
    screen_id: ScreenId,
    device: &GpuDevice,
) -> Option<eros::ErrorUnion> {
    (bound != device).then(|| {
        eros::error!(
            "GPU worker for {} cannot capture screen {} from {}",
            bound.render_node_path().display(),
            screen_id.0,
            device.render_node_path().display()
        )
    })
}

fn notify_screen_failed(
    notifications: &Sender<GpuWorkerNotification>,
    screen_id: ScreenId,
    error: eros::ErrorUnion,
) -> bool {
    notifications
        .send(GpuWorkerNotification::ScreenFailed { screen_id, error })
        .is_ok()
}

fn wait_for_event(
    commands: &Receiver<GpuWorkerCommand>,
    screens: &HashMap<ScreenId, GpuScreen>,
) -> GpuWorkerEvent {
    let mut selector = Selector::new().recv(commands, GpuWorkerEvent::Command);

    for (&screen_id, screen) in screens {
        if let Some(device) = &screen.device {
            selector = selector.recv(device, move |device| {
                GpuWorkerEvent::ScreenReady(screen_id, device)
            });
        } else {
            selector = selector.recv(&screen.frames, move |frame| {
                GpuWorkerEvent::Frame(screen_id, frame.map(|result| result.map(Box::new)))
            });
        }
    }

    selector.wait()
}

fn process_screen_frame(
    context: &GpuContext,
    screen_id: ScreenId,
    mut frame: KmsCapturedFrame,
    pipelines: &mut HashMap<FramePipelineId, GpuPipeline>,
) {
    if let Some(probe) = &mut frame.probe {
        probe.mark_gpu_received();
    }

    let frame = match publish_single_pipeline_passthrough(screen_id, frame, pipelines) {
        Ok(()) => return,
        Err(frame) => frame,
    };

    let mut frame = frame;
    let source = match prepare_pipeline_source(context, screen_id, &mut frame) {
        Ok(source) => source,
        Err(error) => {
            let failure = SharedFramePipelineError::from(error);
            route_screen_frame(screen_id, frame, pipelines, |_, _| {
                Err(eros::error!(failure.clone()))
            });
            return;
        }
    };

    route_screen_frame(screen_id, frame, pipelines, |parameters, frame| {
        process_pipeline_frame(context, &source, parameters, frame)
    });
}

fn publish_single_pipeline_passthrough(
    screen_id: ScreenId,
    mut frame: KmsCapturedFrame,
    pipelines: &mut HashMap<FramePipelineId, GpuPipeline>,
) -> Result<(), KmsCapturedFrame> {
    let mut matching = pipelines
        .iter()
        .filter(|(_, pipeline)| pipeline.screen_id == screen_id)
        .map(|(id, _)| *id);
    let Some(pipeline_id) = matching.next() else {
        return Err(frame);
    };
    if matching.next().is_some() {
        return Err(frame);
    }

    let Some(pipeline) = pipelines.get_mut(&pipeline_id) else {
        return Err(frame);
    };
    if pipeline.parameters.frame_size != frame.buffer.size
        || frame.buffer.format != Format::Xrgb8888
        || frame.buffer.planes.len() != 1
    {
        return Err(frame);
    }
    let modifier = frame.buffer.planes[0].modifier;

    match pipeline.output_strategy {
        None => {
            if let Err(error) = hardware_h264_encoder_for(&frame.buffer) {
                tracing::debug!(
                    target: "rabbit::frame_pipeline",
                    screen_id = screen_id.0,
                    modifier = ?modifier,
                    error = ?error,
                    "Hardware H.264 pipeline rejected KMS XRGB pass-through"
                );
                return Err(frame);
            }
            pipeline.output_strategy = Some(FrameOutputStrategy::PassthroughXrgb(modifier));
            tracing::info!(
                target: "rabbit::frame_pipeline",
                screen_id = screen_id.0,
                width = frame.buffer.size.width,
                height = frame.buffer.size.height,
                strategy = FrameOutputStrategy::PassthroughXrgb(modifier).name(),
                "Selected frame-pipeline output strategy"
            );
        }
        Some(FrameOutputStrategy::PassthroughXrgb(expected)) if expected == modifier => {}
        Some(FrameOutputStrategy::PassthroughXrgb(expected)) => {
            pipeline.outputs.publish(Err(eros::error!(
                "Screen {} XRGB pass-through modifier changed from {:?} to {:?}",
                screen_id.0,
                expected,
                modifier
            )));
            pipelines.remove(&pipeline_id);
            return Ok(());
        }
        Some(FrameOutputStrategy::DirectNv12(_) | FrameOutputStrategy::VaapiXrgb(_)) => {
            return Err(frame);
        }
    }

    let probe = frame.probe.take().map(|mut probe| {
        probe.mark_gpu_submitted();
        probe
    });
    pipeline.outputs.publish(Ok(GbmFramePipelineFrame {
        buffer: frame.buffer,
        probe,
    }));

    Ok(())
}

fn prepare_pipeline_source<'context>(
    context: &'context GpuContext,
    screen_id: ScreenId,
    frame: &mut KmsCapturedFrame,
) -> eros::Result<EglDmaBufImage<'context>> {
    if frame.buffer.format != Format::Xrgb8888 {
        eros::bail!(
            "First-version frame pipeline requires an XRGB8888 source for screen {}, got {:?}",
            screen_id.0,
            frame.buffer.format
        );
    }

    if let Some(fence) = frame.buffer.readiness_fence.take() {
        context.egl().wait_on_native_fence(fence).with_context(|| {
            format!(
                "Failed to wait for screen {} {:?} source readiness",
                screen_id.0, frame.buffer.format
            )
        })?;
    }

    context
        .egl()
        .import_dma_buf_frame(&frame.buffer)
        .with_context(|| {
            format!(
                "Failed to import screen {} {:?} source frame",
                screen_id.0, frame.buffer.format
            )
        })
}

fn route_screen_frame(
    screen_id: ScreenId,
    frame: KmsCapturedFrame,
    pipelines: &mut HashMap<FramePipelineId, GpuPipeline>,
    mut process: impl FnMut(
        &mut GpuPipeline,
        &KmsCapturedFrame,
    ) -> eros::Result<Option<GbmFramePipelineFrame>>,
) {
    pipelines.retain(|_, pipeline| {
        if pipeline.screen_id != screen_id {
            return true;
        }

        let output = process(pipeline, &frame);
        if output.is_err() {
            frame.buffer.invalidate_lease();
        }
        let succeeded = output.is_ok();
        if let Some(output) = output.transpose() {
            pipeline.outputs.publish(output);
        }
        succeeded
    });
}

fn process_pipeline_frame(
    context: &GpuContext,
    source: &EglDmaBufImage<'_>,
    pipeline: &mut GpuPipeline,
    frame: &KmsCapturedFrame,
) -> eros::Result<Option<GbmFramePipelineFrame>> {
    let parameters = pipeline.parameters;
    validate_nv12_size(parameters.frame_size)?;
    let source_texture = context
        .egl()
        .create_dma_buf_texture(source)
        .with_context(|| "Failed to bind the frame-pipeline source texture")?;
    let mut first = None;
    let strategy = match pipeline.output_strategy {
        Some(strategy) => strategy,
        None => {
            let (buffer, strategy) = select_output(context, parameters.frame_size)?;
            first = Some(buffer);
            pipeline.output_strategy = Some(strategy);
            tracing::info!(
                target: "rabbit::frame_pipeline",
                screen_id = pipeline.screen_id.0,
                width = parameters.frame_size.width,
                height = parameters.frame_size.height,
                strategy = strategy.name(),
                "Selected frame-pipeline output strategy"
            );
            strategy
        }
    };
    let buffer = pipeline.output_pool.acquire(
        || match first.take() {
            Some(buffer) => Ok(buffer),
            None => allocate_output(context, parameters.frame_size, strategy),
        },
        |fence| {
            context
                .egl()
                .wait_on_native_fence(fence)
                .with_context(|| "Failed to enqueue frame-pipeline output reuse fence")
        },
    )?;
    let Some(mut buffer) = buffer else {
        return Ok(None);
    };
    match strategy {
        FrameOutputStrategy::PassthroughXrgb(_) => {
            eros::bail!("XRGB pass-through reached the GPU processing path")
        }
        FrameOutputStrategy::DirectNv12(_) => {
            let target_image = context
                .egl()
                .import_nv12_target(&buffer)
                .with_context(|| "Failed to import the frame-pipeline NV12 output planes")?;
            let target = context
                .egl()
                .create_nv12_target(&target_image)
                .with_context(|| "Failed to bind the frame-pipeline NV12 output targets")?;
            context
                .egl()
                .convert_to_nv12(&source_texture, &target)
                .with_context(|| "Failed to convert the source frame to NV12")?;
        }
        FrameOutputStrategy::VaapiXrgb(_) => {
            let target_image = context
                .egl()
                .import_composition_target(&buffer)
                .with_context(|| "Failed to import the frame-pipeline XRGB output")?;
            let target = context
                .egl()
                .create_composition_target(&target_image)
                .with_context(|| "Failed to bind the frame-pipeline XRGB output")?;
            context
                .egl()
                .copy_frame(&source_texture, &target)
                .with_context(|| "Failed to copy the source frame to the VAAPI XRGB output")?;
        }
    }
    let readiness_fence = context
        .egl()
        .finish_frame_pipeline()
        .with_context(|| "Failed to export frame-pipeline output readiness")?;
    frame.buffer.set_release_fence(
        readiness_fence
            .try_clone()
            .with_context(|| "Failed to duplicate the source-frame release fence")?,
    );
    buffer.readiness_fence = Some(readiness_fence);

    Ok(Some(GbmFramePipelineFrame {
        buffer,
        probe: frame.probe.clone().map(|mut probe| {
            probe.mark_gpu_submitted();
            probe
        }),
    }))
}

impl FrameOutputStrategy {
    fn name(self) -> &'static str {
        match self {
            Self::PassthroughXrgb(_) => "kms_xrgb_passthrough",
            Self::DirectNv12(strategy) => strategy.name(),
            Self::VaapiXrgb(_) => "vaapi_xrgb",
        }
    }
}

fn select_output(
    context: &GpuContext,
    size: crate::kernel::geometry::PixelSize,
) -> eros::Result<(
    crate::infra::platform::dma_buf::DmaBufFrame,
    FrameOutputStrategy,
)> {
    for strategy in Nv12OutputStrategy::ALL {
        if !context.supports_nv12_output(strategy) {
            continue;
        }

        let buffer = match context.allocate_nv12_output(size, strategy) {
            Ok(buffer) => buffer,
            Err(error) => {
                tracing::debug!(
                    target: "rabbit::frame_pipeline",
                    strategy = strategy.name(),
                    error = ?error,
                    "Failed to allocate direct NV12 output candidate"
                );
                continue;
            }
        };
        let renderable = context
            .egl()
            .import_nv12_target(&buffer)
            .and_then(|image| context.egl().create_nv12_target(&image));
        if let Err(error) = renderable {
            tracing::debug!(
                target: "rabbit::frame_pipeline",
                strategy = strategy.name(),
                error = ?error,
                "EGL rejected direct NV12 output candidate"
            );
            continue;
        }
        if let Err(error) = hardware_h264_encoder_for(&buffer) {
            tracing::debug!(
                target: "rabbit::frame_pipeline",
                strategy = strategy.name(),
                error = ?error,
                "Hardware H.264 encoder rejected direct NV12 output candidate"
            );
            continue;
        }

        return Ok((buffer, FrameOutputStrategy::DirectNv12(strategy)));
    }

    for modifier in va_vpp_input_modifiers(Format::Xrgb8888)
        .with_context(|| "Failed to find VAAPI-compatible XRGB DMA-BUF modifiers")?
    {
        let strategy = FrameOutputStrategy::VaapiXrgb(modifier);
        let buffer = match allocate_output(context, size, strategy) {
            Ok(buffer) => buffer,
            Err(error) => {
                tracing::debug!(
                    target: "rabbit::frame_pipeline",
                    modifier = ?modifier,
                    error = ?error,
                    "Failed to allocate VAAPI XRGB output candidate"
                );
                continue;
            }
        };
        let renderable = context
            .egl()
            .import_composition_target(&buffer)
            .and_then(|image| context.egl().create_composition_target(&image));
        if let Err(error) = renderable {
            tracing::debug!(
                target: "rabbit::frame_pipeline",
                modifier = ?modifier,
                error = ?error,
                "EGL rejected VAAPI XRGB output candidate"
            );
            continue;
        }
        if let Err(error) = hardware_h264_encoder_for(&buffer) {
            tracing::debug!(
                target: "rabbit::frame_pipeline",
                modifier = ?modifier,
                error = ?error,
                "Hardware H.264 pipeline rejected VAAPI XRGB output candidate"
            );
            continue;
        }

        return Ok((buffer, strategy));
    }

    eros::bail!("No VAAPI XRGB DMA-BUF output candidate is usable")
}

fn allocate_output(
    context: &GpuContext,
    size: crate::kernel::geometry::PixelSize,
    strategy: FrameOutputStrategy,
) -> eros::Result<crate::infra::platform::dma_buf::DmaBufFrame> {
    match strategy {
        FrameOutputStrategy::PassthroughXrgb(_) => {
            eros::bail!("XRGB pass-through does not allocate a frame-pipeline output")
        }
        FrameOutputStrategy::DirectNv12(strategy) => context.allocate_nv12_output(size, strategy),
        FrameOutputStrategy::VaapiXrgb(modifier) => context.allocate_dma_buf_with_modifier(
            size,
            Format::Xrgb8888,
            modifier,
            gbm::BufferObjectFlags::RENDERING,
        ),
    }
}

fn validate_nv12_size(size: crate::kernel::geometry::PixelSize) -> eros::Result<()> {
    if size.width == 0 || size.height == 0 {
        eros::bail!(
            "NV12 frame size must be non-zero, got {}x{}",
            size.width,
            size.height
        );
    }

    if !size.width.is_multiple_of(2) || !size.height.is_multiple_of(2) {
        eros::bail!(
            "NV12 frame size must use even dimensions, got {}x{}",
            size.width,
            size.height
        );
    }

    Ok(())
}

impl<T> LatestSender<T> {
    fn publish(&self, mut item: T) {
        loop {
            match self.sender.try_send(item) {
                Ok(()) | Err(flume::TrySendError::Disconnected(_)) => return,
                Err(flume::TrySendError::Full(returned_item)) => {
                    item = returned_item;
                    match self.overflow_receiver.try_recv() {
                        Ok(_) | Err(flume::TryRecvError::Empty) => {}
                        Err(flume::TryRecvError::Disconnected) => return,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use flume::unbounded;

    use crate::{
        infra::platform::{
            dma_buf::DmaBufPool,
            frame_pipeline::worker::{
                FramePipelineId, GpuPipeline, GpuScreen, GpuWorker, GpuWorkerEvent,
                GpuWorkerNotification, LatestSender, OUTPUT_POOL_CAPACITY, gpu_mismatch,
                route_screen_frame, validate_nv12_size, wait_for_event,
            },
            gpu::GpuDevice,
            screen_capture::{KmsFrameReceiver, empty_kms_frame},
        },
        kernel::{
            frame_pipeline::FramePipelineParameters, geometry::PixelSize, screen_manager::ScreenId,
        },
    };

    #[test]
    fn worker_registers_pipelines_without_opening_a_gpu() {
        let (_reaper, reaper_handle) =
            crate::infra::WorkerReaper::new().expect("Test worker reaper should start");
        let (worker, _notifications) =
            GpuWorker::new(reaper_handle).expect("Empty GPU worker should start");
        let screen = worker
            .register_screen(ScreenId(0), KmsFrameReceiver::empty())
            .expect("Captured screen should register");
        let parameters = FramePipelineParameters {
            frame_size: PixelSize {
                width: 1920,
                height: 1080,
            },
        };
        let first = worker
            .register_pipeline(FramePipelineId(0), ScreenId(0), parameters)
            .expect("First frame pipeline should register");
        let second = worker
            .register_pipeline(FramePipelineId(1), ScreenId(0), parameters)
            .expect("Second frame pipeline should register");

        drop(first);
        drop(second);
        drop(screen);
        drop(worker);
    }

    #[test]
    fn worker_event_loop_receives_screen_gpu_identity_before_frames() {
        let (_command_sender, command_receiver) = unbounded();
        let (_frame_sender, receiver) = KmsFrameReceiver::channel();
        let (device, frames) = receiver.into_parts();
        let screens = HashMap::from([(
            ScreenId(3),
            GpuScreen {
                device: Some(device),
                frames,
            },
        )]);

        match wait_for_event(&command_receiver, &screens) {
            GpuWorkerEvent::ScreenReady(screen_id, Ok(Ok(device))) => {
                assert_eq!(screen_id, ScreenId(3));
                assert_eq!(
                    device.render_node_path(),
                    std::path::Path::new("/dev/dri/renderD128")
                );
            }
            _ => panic!("GPU worker should receive the screen GPU identity"),
        }
    }

    #[test]
    fn gpu_binding_rejects_a_different_render_node() {
        let bound = GpuDevice::from(PathBuf::from("/dev/dri/renderD128"));
        let different = GpuDevice::from(PathBuf::from("/dev/dri/renderD129"));

        let error = gpu_mismatch(&bound, ScreenId(2), &different)
            .expect("A different render node should be rejected");

        assert!(error.to_string().contains("cannot capture screen 2"));
    }

    #[test]
    fn worker_reports_a_capture_failure_for_its_screen() {
        let (_reaper, reaper_handle) =
            crate::infra::WorkerReaper::new().expect("Test worker reaper should start");
        let (worker, notifications) =
            GpuWorker::new(reaper_handle).expect("GPU worker should start");
        let (frame_sender, frame_receiver) = KmsFrameReceiver::channel();
        let screen = worker
            .register_screen(ScreenId(4), frame_receiver)
            .expect("Captured screen should register");

        frame_sender
            .send(Err(eros::error!("test capture failure")))
            .expect("Capture failure should be sent");

        match notifications
            .recv()
            .expect("GPU worker should report the capture failure")
        {
            GpuWorkerNotification::ScreenFailed { screen_id, error } => {
                assert_eq!(screen_id, ScreenId(4));
                assert_eq!(error.to_string(), "test capture failure");
            }
        }

        drop(screen);
        drop(worker);
    }

    #[test]
    fn worker_routes_processing_result_only_to_pipelines_for_the_same_screen() {
        let (matching_sender, matching_frames) = flume::bounded(1);
        let (other_sender, other_frames) = flume::bounded(1);
        let mut pipelines = HashMap::from([
            (
                FramePipelineId(0),
                GpuPipeline {
                    screen_id: ScreenId(2),
                    parameters: parameters(1280, 720),
                    outputs: LatestSender {
                        sender: matching_sender,
                        overflow_receiver: matching_frames.clone(),
                    },
                    output_strategy: None,
                    output_pool: DmaBufPool::new(OUTPUT_POOL_CAPACITY),
                },
            ),
            (
                FramePipelineId(1),
                GpuPipeline {
                    screen_id: ScreenId(3),
                    parameters: parameters(1920, 1080),
                    outputs: LatestSender {
                        sender: other_sender,
                        overflow_receiver: other_frames.clone(),
                    },
                    output_strategy: None,
                    output_pool: DmaBufPool::new(OUTPUT_POOL_CAPACITY),
                },
            ),
        ]);

        route_screen_frame(
            ScreenId(2),
            empty_kms_frame(PixelSize {
                width: 2560,
                height: 1440,
            }),
            &mut pipelines,
            |_, _| Err(eros::error!("test processing failure")),
        );

        let error = matching_frames
            .recv()
            .expect("Matching pipeline should receive a processing result")
            .expect_err("Test processing should fail");
        assert_eq!(error.to_string(), "test processing failure");
        assert!(other_frames.try_recv().is_err());
    }

    #[test]
    fn nv12_output_requires_non_zero_even_dimensions() {
        validate_nv12_size(PixelSize {
            width: 1920,
            height: 1080,
        })
        .expect("Even NV12 dimensions should be accepted");

        let zero = validate_nv12_size(PixelSize {
            width: 0,
            height: 1080,
        })
        .expect_err("Zero NV12 dimensions should be rejected");
        let odd = validate_nv12_size(PixelSize {
            width: 1919,
            height: 1080,
        })
        .expect_err("Odd NV12 dimensions should be rejected");

        assert!(zero.to_string().contains("non-zero"));
        assert!(odd.to_string().contains("even dimensions"));
    }

    #[test]
    fn pipeline_output_keeps_only_the_latest_unconsumed_frame() {
        let (sender, receiver) = flume::bounded(1);
        let outputs = LatestSender {
            sender,
            overflow_receiver: receiver.clone(),
        };

        outputs.publish(1_u8);
        outputs.publish(2_u8);

        assert_eq!(receiver.recv(), Ok(2));
    }

    fn parameters(width: u32, height: u32) -> FramePipelineParameters {
        FramePipelineParameters {
            frame_size: PixelSize { width, height },
        }
    }
}
