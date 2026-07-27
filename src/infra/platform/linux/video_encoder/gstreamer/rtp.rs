//! H.264 RTP packet extraction from GStreamer samples.

use super::pipeline_util::h264_rtp_caps;

#[derive(Debug)]
pub(crate) struct GStreamerRtpPacket {
    payload: bytes::Bytes,
    marker: bool,
    pts_ns: Option<u64>,
}

impl TryFrom<gstreamer::Sample> for GStreamerRtpPacket {
    type Error = eros::ErrorUnion;

    fn try_from(sample: gstreamer::Sample) -> Result<Self, Self::Error> {
        let Some(caps) = sample.caps() else {
            eros::bail!("GStreamer encoded packet sample is missing caps");
        };

        if !caps.is_subset(&h264_rtp_caps()) {
            eros::bail!("GStreamer encoded packet has non-H.264 RTP caps {}", caps);
        }

        let Some(buffer) = sample.buffer_owned() else {
            eros::bail!("GStreamer H.264 RTP sample is missing its buffer");
        };
        let pts_ns = buffer.pts().map(gstreamer::ClockTime::nseconds);
        let Ok(buffer) = buffer.into_mapped_buffer_readable() else {
            eros::bail!("Failed to map GStreamer H.264 RTP packet for reading");
        };
        let payload = bytes::Bytes::from_owner(buffer);

        if payload.len() < 12 {
            eros::bail!("GStreamer H.264 RTP packet is shorter than its 12-byte fixed header");
        }
        if payload[0] >> 6 != 2 {
            eros::bail!(
                "GStreamer H.264 RTP packet has unsupported RTP version {}",
                payload[0] >> 6
            );
        }
        let marker = payload[1] & 0x80 != 0;

        Ok(Self {
            payload,
            marker,
            pts_ns,
        })
    }
}

impl GStreamerRtpPacket {
    pub(crate) fn is_frame_end(&self) -> bool {
        self.marker
    }

    pub(super) fn pts_ns(&self) -> Option<u64> {
        self.pts_ns
    }

    pub(super) fn payload_len(&self) -> usize {
        self.payload.len()
    }
}

impl From<GStreamerRtpPacket> for bytes::Bytes {
    fn from(packet: GStreamerRtpPacket) -> Self {
        packet.payload
    }
}
