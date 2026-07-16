use bytes::{Buf, BufMut, Bytes, BytesMut};
use eros::Context;

use crate::kernel::{
    screen_configuration::{
        PixelSize, RemoteDisplayMode, ResolutionResult, ScreenResolutionStatus,
        ScreenStreamRequest, ScreenStreamRequestId, ScreenStreamsConfigured, SetScreenStreams,
    },
    screen_manager::{Screen, ScreenId, ScreenLayout, ScreenTransform},
    transport::{Delivery, TransportChannel, TransportMessage},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
enum ControlMessageTag {
    ScreenList = 0,
    SetScreenStreams = 1,
    ScreenStreamsConfigured = 2,
}

impl From<ControlMessageTag> for u8 {
    fn from(tag: ControlMessageTag) -> Self {
        tag as Self
    }
}

impl TryFrom<u8> for ControlMessageTag {
    type Error = eros::ErrorUnion;

    fn try_from(tag: u8) -> eros::Result<Self> {
        match tag {
            0 => Ok(Self::ScreenList),
            1 => Ok(Self::SetScreenStreams),
            2 => Ok(Self::ScreenStreamsConfigured),
            tag => eros::bail!("Unknown Control message tag {tag}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
enum ScreenResolutionStatusTag {
    Configured = 0,
    Failed = 1,
}

impl From<ScreenResolutionStatusTag> for u8 {
    fn from(tag: ScreenResolutionStatusTag) -> Self {
        tag as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
enum ResolutionResultTag {
    Exact = 0,
    Fallback = 1,
    Preserved = 2,
}

impl From<ResolutionResultTag> for u8 {
    fn from(tag: ResolutionResultTag) -> Self {
        tag as Self
    }
}

impl From<RemoteDisplayMode> for u8 {
    fn from(mode: RemoteDisplayMode) -> Self {
        match mode {
            RemoteDisplayMode::Preserve => 0,
        }
    }
}

impl TryFrom<u8> for RemoteDisplayMode {
    type Error = eros::ErrorUnion;

    fn try_from(mode: u8) -> eros::Result<Self> {
        match mode {
            0 => Ok(Self::Preserve),
            mode => eros::bail!("Unknown RemoteDisplayMode tag {mode}"),
        }
    }
}

impl From<ScreenTransform> for u8 {
    fn from(transform: ScreenTransform) -> Self {
        match transform {
            ScreenTransform::Normal => 0,
            ScreenTransform::Rotate90 => 1,
            ScreenTransform::Rotate180 => 2,
            ScreenTransform::Rotate270 => 3,
            ScreenTransform::Flipped => 4,
            ScreenTransform::Flipped90 => 5,
            ScreenTransform::Flipped180 => 6,
            ScreenTransform::Flipped270 => 7,
        }
    }
}

impl TryFrom<u8> for ScreenTransform {
    type Error = eros::ErrorUnion;

    fn try_from(transform: u8) -> eros::Result<Self> {
        match transform {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Rotate90),
            2 => Ok(Self::Rotate180),
            3 => Ok(Self::Rotate270),
            4 => Ok(Self::Flipped),
            5 => Ok(Self::Flipped90),
            6 => Ok(Self::Flipped180),
            7 => Ok(Self::Flipped270),
            transform => eros::bail!("Unknown ScreenTransform tag {transform}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub id: ScreenId,
    pub name: String,
    pub layout: ScreenLayout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    ScreenList(Vec<ScreenInfo>),
    SetScreenStreams(SetScreenStreams),
    ScreenStreamsConfigured(ScreenStreamsConfigured),
}

impl TryFrom<&[Screen]> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(screens: &[Screen]) -> eros::Result<Self> {
        let screen_count = u8::try_from(screens.len())
            .with_context(|| "Failed to encode ScreenList screen count")?;
        let mut payload = BytesMut::new();

        payload.put_u8(ControlMessageTag::ScreenList.into());
        payload.put_u8(screen_count);

        for screen in screens {
            let name = screen.name.as_bytes();
            let name_length = u16::try_from(name.len())
                .with_context(|| "Failed to encode ScreenInfo name length")?;
            let ScreenLayout {
                rect,
                scale,
                transform,
            } = screen.layout;

            payload.put_u8(screen.id.0);
            payload.put_u16(name_length);
            payload.extend_from_slice(name);
            payload.put_u32(rect.x);
            payload.put_u32(rect.y);
            payload.put_u32(rect.width);
            payload.put_u32(rect.height);
            payload.put_f64(scale);
            payload.put_u8(transform.into());
        }

        Ok(Self {
            channel: TransportChannel::Control,
            delivery: Delivery::ReliableOrdered,
            payload: payload.freeze(),
        })
    }
}

impl TryFrom<SetScreenStreams> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(request: SetScreenStreams) -> eros::Result<Self> {
        let change_count = u8::try_from(request.changes.len())
            .with_context(|| "Failed to encode SetScreenStreams change count")?;
        let mut payload = BytesMut::new();

        payload.put_u8(ControlMessageTag::SetScreenStreams.into());
        payload.put_u32(request.request_id.0);
        payload.put_u8(change_count);

        for change in request.changes {
            payload.put_u8(change.screen_id.0);
            payload.put_u8(change.remote_display.into());
            payload.put_u32(change.max_resolution.width);
            payload.put_u32(change.max_resolution.height);
        }

        Ok(Self {
            channel: TransportChannel::Control,
            delivery: Delivery::ReliableOrdered,
            payload: payload.freeze(),
        })
    }
}

impl TryFrom<ScreenStreamsConfigured> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(configured: ScreenStreamsConfigured) -> eros::Result<Self> {
        let outcome_count = u8::try_from(configured.outcomes.len())
            .with_context(|| "Failed to encode ScreenStreamsConfigured outcome count")?;
        let mut payload = BytesMut::new();

        payload.put_u8(ControlMessageTag::ScreenStreamsConfigured.into());
        payload.put_u32(configured.request_id.0);
        payload.put_u8(outcome_count);

        for outcome in configured.outcomes {
            payload.put_u8(outcome.screen_id.0);

            match outcome.status {
                ScreenResolutionStatus::Configured(result) => {
                    payload.put_u8(ScreenResolutionStatusTag::Configured.into());

                    match result {
                        ResolutionResult::Exact { applied } => {
                            payload.put_u8(ResolutionResultTag::Exact.into());
                            put_pixel_size(&mut payload, applied);
                        }
                        ResolutionResult::Fallback { requested, applied } => {
                            payload.put_u8(ResolutionResultTag::Fallback.into());
                            put_pixel_size(&mut payload, requested);
                            put_pixel_size(&mut payload, applied);
                        }
                        ResolutionResult::Preserved { requested, actual } => {
                            payload.put_u8(ResolutionResultTag::Preserved.into());
                            put_pixel_size(&mut payload, requested);
                            put_pixel_size(&mut payload, actual);
                        }
                    }
                }
                ScreenResolutionStatus::Failed { requested, actual } => {
                    payload.put_u8(ScreenResolutionStatusTag::Failed.into());
                    put_pixel_size(&mut payload, requested);

                    if let Some(actual) = actual {
                        payload.put_u8(1);
                        put_pixel_size(&mut payload, actual);
                    } else {
                        payload.put_u8(0);
                    }
                }
            }
        }

        Ok(Self {
            channel: TransportChannel::Control,
            delivery: Delivery::ReliableOrdered,
            payload: payload.freeze(),
        })
    }
}

fn put_pixel_size(payload: &mut BytesMut, size: PixelSize) {
    payload.put_u32(size.width);
    payload.put_u32(size.height);
}

impl TryFrom<TransportMessage> for ControlMessage {
    type Error = eros::ErrorUnion;

    fn try_from(message: TransportMessage) -> eros::Result<Self> {
        if message.channel != TransportChannel::Control {
            eros::bail!(
                "Cannot decode Control message from channel {:?}",
                message.channel
            );
        }

        if message.delivery != Delivery::ReliableOrdered {
            eros::bail!(
                "Cannot decode Control message with delivery {:?}",
                message.delivery
            );
        }

        let mut reader = ControlPayloadReader::from(message.payload);
        let tag = ControlMessageTag::try_from(reader.read_u8("Control message tag")?)
            .with_context(|| "Failed to decode Control message tag")?;
        let message = match tag {
            ControlMessageTag::ScreenList => Self::ScreenList(reader.read_screen_list()?),
            ControlMessageTag::SetScreenStreams => {
                Self::SetScreenStreams(reader.read_set_screen_streams()?)
            }
            ControlMessageTag::ScreenStreamsConfigured => {
                eros::bail!("ScreenStreamsConfigured decoding is not implemented")
            }
        };

        if reader.has_remaining() {
            eros::bail!("Control message contains trailing payload bytes");
        }

        Ok(message)
    }
}

struct ControlPayloadReader {
    payload: Bytes,
}

impl From<Bytes> for ControlPayloadReader {
    fn from(payload: Bytes) -> Self {
        Self { payload }
    }
}

impl ControlPayloadReader {
    fn has_remaining(&self) -> bool {
        self.payload.has_remaining()
    }

    fn read_screen_list(&mut self) -> eros::Result<Vec<ScreenInfo>> {
        let screen_count = usize::from(self.read_u8("ScreenList screen count")?);
        let mut screens = Vec::with_capacity(screen_count);

        for _ in 0..screen_count {
            screens.push(self.read_screen_info()?);
        }

        Ok(screens)
    }

    fn read_screen_info(&mut self) -> eros::Result<ScreenInfo> {
        let id = ScreenId(self.read_u8("ScreenInfo ID")?);
        let name_length = usize::from(self.read_u16("ScreenInfo name length")?);
        let name = self.read_bytes(name_length, "ScreenInfo name")?;
        let name = std::str::from_utf8(&name)
            .with_context(|| "Failed to decode ScreenInfo name as UTF-8")?
            .to_owned();
        let rect = crate::kernel::screen_manager::ScreenRect {
            x: self.read_u32("ScreenInfo layout x")?,
            y: self.read_u32("ScreenInfo layout y")?,
            width: self.read_u32("ScreenInfo layout width")?,
            height: self.read_u32("ScreenInfo layout height")?,
        };
        let scale = self.read_f64("ScreenInfo scale")?;
        let transform = ScreenTransform::try_from(self.read_u8("ScreenInfo transform")?)
            .with_context(|| "Failed to decode ScreenInfo transform")?;

        Ok(ScreenInfo {
            id,
            name,
            layout: ScreenLayout {
                rect,
                scale,
                transform,
            },
        })
    }

    fn read_set_screen_streams(&mut self) -> eros::Result<SetScreenStreams> {
        let request_id = ScreenStreamRequestId(self.read_u32("SetScreenStreams request ID")?);
        let change_count = usize::from(self.read_u8("SetScreenStreams change count")?);
        let mut changes = Vec::with_capacity(change_count);

        for _ in 0..change_count {
            let screen_id = ScreenId(self.read_u8("ScreenStreamRequest screen ID")?);
            let remote_display =
                RemoteDisplayMode::try_from(self.read_u8("ScreenStreamRequest display mode")?)
                    .with_context(|| "Failed to decode ScreenStreamRequest display mode")?;
            let max_resolution = PixelSize {
                width: self.read_u32("ScreenStreamRequest maximum width")?,
                height: self.read_u32("ScreenStreamRequest maximum height")?,
            };

            changes.push(ScreenStreamRequest {
                screen_id,
                remote_display,
                max_resolution,
            });
        }

        Ok(SetScreenStreams {
            request_id,
            changes,
        })
    }

    fn read_u8(&mut self, field: &str) -> eros::Result<u8> {
        self.require(size_of::<u8>(), field)?;
        Ok(self.payload.get_u8())
    }

    fn read_u16(&mut self, field: &str) -> eros::Result<u16> {
        self.require(size_of::<u16>(), field)?;
        Ok(self.payload.get_u16())
    }

    fn read_u32(&mut self, field: &str) -> eros::Result<u32> {
        self.require(size_of::<u32>(), field)?;
        Ok(self.payload.get_u32())
    }

    fn read_f64(&mut self, field: &str) -> eros::Result<f64> {
        self.require(size_of::<f64>(), field)?;
        Ok(self.payload.get_f64())
    }

    fn read_bytes(&mut self, length: usize, field: &str) -> eros::Result<Bytes> {
        self.require(length, field)?;
        Ok(self.payload.split_to(length))
    }

    fn require(&self, length: usize, field: &str) -> eros::Result<()> {
        if self.payload.remaining() < length {
            eros::bail!("{field} is truncated");
        }

        Ok(())
    }
}
