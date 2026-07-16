use bytes::{Buf, BufMut, Bytes, BytesMut};
use eros::Context;

use crate::kernel::{
    screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
    screen_manager::{Screen, ScreenId, ScreenLayout, ScreenTransform},
    transport::{Delivery, TransportChannel, TransportMessage},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
enum ControlMessageTag {
    ScreenList = 0,
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
            tag => eros::bail!("Unknown Control message tag {tag}"),
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
