//! RTP video frame assembly for unreliable session video channels.
//!
//! Controllers reassemble H.264 RTP packets into complete frames, request
//! keyframes after sequence gaps, and discard dependent frames until an IDR
//! restores the stream. Hosts never receive this path.

use std::collections::HashMap;

use eros::Context as _;

use crate::kernel::{screen_manager::ScreenId, session::ReceivedVideoFrame};

pub(super) struct RtpVideoStream {
    pub(super) next_sequence: Option<u16>,
    pub(super) frame: Option<RtpFrameAssembly>,
    pub(super) waiting_for_keyframe: bool,
    pub(super) keyframe_request_pending: bool,
}

impl Default for RtpVideoStream {
    fn default() -> Self {
        Self {
            next_sequence: None,
            frame: None,
            waiting_for_keyframe: true,
            keyframe_request_pending: false,
        }
    }
}

pub(super) struct RtpFrameAssembly {
    pub(super) timestamp: u32,
    pub(super) packets: Vec<bytes::Bytes>,
    pub(super) payload_size: usize,
    pub(super) valid: bool,
    pub(super) keyframe: bool,
}

struct RtpPacketMetadata {
    sequence: u16,
    timestamp: u32,
    marker: bool,
    keyframe: bool,
}

pub(super) struct VideoAssemblyResult {
    pub(super) frame: Option<ReceivedVideoFrame>,
    pub(super) request_key_frame: bool,
}

const RTP_FIXED_HEADER_SIZE: usize = 12;
const MAX_ENCODED_VIDEO_FRAME_SIZE: usize = 16 * 1024 * 1024;

pub(super) fn assemble_video_frame(
    streams: &mut HashMap<ScreenId, RtpVideoStream>,
    screen_id: ScreenId,
    packet: bytes::Bytes,
) -> eros::Result<VideoAssemblyResult> {
    let metadata = decode_rtp_metadata(&packet)?;
    let packet_size = packet.len();
    let stream = streams.entry(screen_id).or_default();
    let sequence_is_contiguous = stream
        .next_sequence
        .is_none_or(|expected| metadata.sequence == expected);
    stream.next_sequence = Some(metadata.sequence.wrapping_add(1));
    let starts_new_frame = stream
        .frame
        .as_ref()
        .is_none_or(|frame| frame.timestamp != metadata.timestamp);

    if starts_new_frame {
        stream.frame = Some(RtpFrameAssembly {
            timestamp: metadata.timestamp,
            packets: Vec::new(),
            payload_size: 0,
            valid: sequence_is_contiguous || metadata.keyframe,
            keyframe: metadata.keyframe,
        });
    }
    let frame = stream
        .frame
        .as_mut()
        .with_context(|| format!("RTP frame for screen {} is missing", screen_id.0))?;

    let mut request_key_frame = false;
    if !sequence_is_contiguous {
        stream.waiting_for_keyframe = true;
        if !starts_new_frame || !metadata.keyframe {
            frame.valid = false;
            request_key_frame = true;
            stream.keyframe_request_pending = true;
        }
    }
    frame.keyframe |= metadata.keyframe;
    frame.payload_size = frame
        .payload_size
        .checked_add(packet_size)
        .with_context(|| format!("RTP frame size overflow for screen {}", screen_id.0))?;
    if frame.payload_size > MAX_ENCODED_VIDEO_FRAME_SIZE {
        eros::bail!(
            "RTP frame for screen {} exceeds {} bytes",
            screen_id.0,
            MAX_ENCODED_VIDEO_FRAME_SIZE
        );
    }
    frame.packets.push(packet);

    if !metadata.marker {
        return Ok(VideoAssemblyResult {
            frame: None,
            request_key_frame,
        });
    }

    let frame = stream
        .frame
        .take()
        .with_context(|| format!("Completed RTP frame for screen {} is missing", screen_id.0))?;
    if !frame.valid {
        return Ok(VideoAssemblyResult {
            frame: None,
            request_key_frame,
        });
    }
    if stream.waiting_for_keyframe {
        if !frame.keyframe {
            if !stream.keyframe_request_pending {
                request_key_frame = true;
                stream.keyframe_request_pending = true;
            }
            return Ok(VideoAssemblyResult {
                frame: None,
                request_key_frame,
            });
        }
        stream.waiting_for_keyframe = false;
        stream.keyframe_request_pending = false;
    }

    Ok(VideoAssemblyResult {
        frame: Some(ReceivedVideoFrame {
            screen_id,
            packets: frame.packets,
        }),
        request_key_frame,
    })
}

fn decode_rtp_metadata(packet: &bytes::Bytes) -> eros::Result<RtpPacketMetadata> {
    if packet.len() < RTP_FIXED_HEADER_SIZE {
        eros::bail!(
            "Video RTP packet is {} bytes, shorter than the fixed {}-byte header",
            packet.len(),
            RTP_FIXED_HEADER_SIZE
        );
    }
    let version = packet[0] >> 6;
    if version != 2 {
        eros::bail!("Video RTP packet has unsupported version {version}");
    }

    Ok(RtpPacketMetadata {
        sequence: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        marker: packet[1] & 0x80 != 0,
        keyframe: h264_rtp_payload_contains_idr(&packet[RTP_FIXED_HEADER_SIZE..]),
    })
}

fn h264_rtp_payload_contains_idr(payload: &[u8]) -> bool {
    let Some(&nal_header) = payload.first() else {
        return false;
    };

    match nal_header & 0x1f {
        5 => true,
        24 => stap_a_contains_idr(&payload[1..]),
        28 => payload
            .get(1)
            .is_some_and(|fu_header| fu_header & 0x80 != 0 && fu_header & 0x1f == 5),
        _ => false,
    }
}

fn stap_a_contains_idr(mut payload: &[u8]) -> bool {
    while payload.len() >= 2 {
        let nal_size = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
        payload = &payload[2..];
        let Some(nal) = payload.get(..nal_size) else {
            return false;
        };
        if nal.first().is_some_and(|header| header & 0x1f == 5) {
            return true;
        }
        payload = &payload[nal_size..];
    }

    false
}
