use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

use gstreamer::prelude::{ElementExt as _, PadExtManual as _};

use crate::infra::platform::{
    video_encoder::gstreamer::GStreamerRtpPacket,
    video_probe::{VideoFrameProbe, VideoProbeReporter},
};

#[derive(Debug, Clone, Copy)]
enum VideoProbeEvent {
    PipelineInput { pts_ns: u64 },
    VppEntered { pts_ns: u64, at: Instant },
    VppCompleted { pts_ns: u64, at: Instant },
    EncoderEntered { pts_ns: u64, at: Instant },
    EncoderCompleted { pts_ns: u64, at: Instant },
}

#[derive(Debug, Default)]
struct RtpFrameStats {
    packets: u64,
    bytes: u64,
}

#[derive(Debug)]
pub(crate) struct GStreamerVideoProbe {
    submitted_probes: HashMap<u64, VideoFrameProbe>,
    encoding_probes: HashMap<u64, VideoFrameProbe>,
    events: flume::Receiver<VideoProbeEvent>,
    encoded_probe_order: VecDeque<u64>,
    encoder_completed_by_pts: HashMap<u64, Instant>,
    pending_rtp_frames: HashMap<u64, RtpFrameStats>,
    reporter: VideoProbeReporter,
}

impl GStreamerVideoProbe {
    pub(crate) fn new(
        source: &gstreamer_app::AppSrc,
        vpp: Option<&gstreamer::Element>,
        encoder: &gstreamer::Element,
    ) -> eros::Result<Self> {
        let (events, receiver) = flume::unbounded();
        install_pad_probes(source, vpp, encoder, events)?;

        Ok(Self {
            submitted_probes: HashMap::new(),
            encoding_probes: HashMap::new(),
            events: receiver,
            encoded_probe_order: VecDeque::new(),
            encoder_completed_by_pts: HashMap::new(),
            pending_rtp_frames: HashMap::new(),
            reporter: VideoProbeReporter::default(),
        })
    }

    pub(crate) fn submit_frame(&mut self, mut probe: VideoFrameProbe) {
        probe.mark_encoder_submitted();
        self.submitted_probes.insert(probe.pts_ns(), probe);
    }

    pub(crate) fn record_packet(&mut self, packet: &GStreamerRtpPacket) {
        self.collect_events();

        let Some(pts_ns) = packet.pts_ns() else {
            tracing::warn!(
                target: "rabbit::video_probe",
                "Encoded RTP packet has no PTS"
            );
            return;
        };
        let stats = self.pending_rtp_frames.entry(pts_ns).or_default();
        stats.packets += 1;
        stats.bytes += u64::try_from(packet.payload_len()).unwrap_or(u64::MAX);

        if !packet.is_frame_end() {
            return;
        }

        let stats = self.pending_rtp_frames.remove(&pts_ns).unwrap_or_default();
        let encoder_completed = self.encoder_completed_by_pts.remove(&pts_ns);
        let Some(input_pts_ns) = self.encoded_probe_order.pop_front() else {
            tracing::warn!(
                target: "rabbit::video_probe",
                output_pts_ns = pts_ns,
                "Encoded RTP frame has no matching input order entry"
            );
            return;
        };
        let Some(mut probe) = self.encoding_probes.remove(&input_pts_ns) else {
            tracing::warn!(
                target: "rabbit::video_probe",
                input_pts_ns,
                output_pts_ns = pts_ns,
                "Encoded RTP frame has no matching frame probe"
            );
            return;
        };

        probe.mark_encoder_completed(encoder_completed);
        self.reporter
            .record_frame(probe, stats.packets, stats.bytes);
    }

    pub(crate) fn finish(&mut self) {
        self.collect_events();
        self.reporter.finish();
    }

    fn collect_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                VideoProbeEvent::PipelineInput { pts_ns } => {
                    if let Some(probe) = self.submitted_probes.remove(&pts_ns) {
                        self.encoding_probes.insert(pts_ns, probe);
                        self.encoded_probe_order.push_back(pts_ns);
                    }
                    self.submitted_probes
                        .retain(|pending_pts_ns, _| *pending_pts_ns > pts_ns);
                }
                VideoProbeEvent::VppEntered { pts_ns, at } => {
                    if let Some(probe) = self.encoding_probes.get_mut(&pts_ns) {
                        probe.mark_vpp_entered(at);
                    }
                }
                VideoProbeEvent::VppCompleted { pts_ns, at } => {
                    if let Some(probe) = self.encoding_probes.get_mut(&pts_ns) {
                        probe.mark_vpp_completed(at);
                    }
                }
                VideoProbeEvent::EncoderEntered { pts_ns, at } => {
                    if let Some(probe) = self.encoding_probes.get_mut(&pts_ns) {
                        probe.mark_encoder_entered(at);
                    }
                }
                VideoProbeEvent::EncoderCompleted { pts_ns, at } => {
                    self.encoder_completed_by_pts.insert(pts_ns, at);
                }
            }
        }
    }
}

fn install_pad_probes(
    source: &gstreamer_app::AppSrc,
    vpp: Option<&gstreamer::Element>,
    encoder: &gstreamer::Element,
    events: flume::Sender<VideoProbeEvent>,
) -> eros::Result<()> {
    let Some(source_pad) = source.static_pad("src") else {
        eros::bail!("GStreamer appsrc does not expose a source pad");
    };
    let pipeline_events = events.clone();
    source_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, info| {
        if let Some(pts_ns) = buffer_pts(info) {
            let _ = pipeline_events.send(VideoProbeEvent::PipelineInput { pts_ns });
        }
        gstreamer::PadProbeReturn::Ok
    });

    if let Some(vpp) = vpp {
        let Some(vpp_sink_pad) = vpp.static_pad("sink") else {
            eros::bail!("GStreamer VAAPI VPP does not expose a sink pad");
        };
        let entered_events = events.clone();
        vpp_sink_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, info| {
            if let Some(pts_ns) = buffer_pts(info) {
                let _ = entered_events.send(VideoProbeEvent::VppEntered {
                    pts_ns,
                    at: Instant::now(),
                });
            }
            gstreamer::PadProbeReturn::Ok
        });

        let Some(vpp_source_pad) = vpp.static_pad("src") else {
            eros::bail!("GStreamer VAAPI VPP does not expose a source pad");
        };
        let completed_events = events.clone();
        vpp_source_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, info| {
            if let Some(pts_ns) = buffer_pts(info) {
                let _ = completed_events.send(VideoProbeEvent::VppCompleted {
                    pts_ns,
                    at: Instant::now(),
                });
            }
            gstreamer::PadProbeReturn::Ok
        });
    }

    let Some(encoder_sink_pad) = encoder.static_pad("sink") else {
        eros::bail!("GStreamer H.264 encoder does not expose a sink pad");
    };
    let entered_events = events.clone();
    encoder_sink_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, info| {
        if let Some(pts_ns) = buffer_pts(info) {
            let _ = entered_events.send(VideoProbeEvent::EncoderEntered {
                pts_ns,
                at: Instant::now(),
            });
        }
        gstreamer::PadProbeReturn::Ok
    });

    let Some(encoder_source_pad) = encoder.static_pad("src") else {
        eros::bail!("GStreamer H.264 encoder does not expose a source pad");
    };
    encoder_source_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, info| {
        if let Some(pts_ns) = buffer_pts(info) {
            let _ = events.send(VideoProbeEvent::EncoderCompleted {
                pts_ns,
                at: Instant::now(),
            });
        }
        gstreamer::PadProbeReturn::Ok
    });

    Ok(())
}

fn buffer_pts(info: &gstreamer::PadProbeInfo) -> Option<u64> {
    info.buffer()
        .and_then(|buffer| buffer.pts())
        .map(gstreamer::ClockTime::nseconds)
}
