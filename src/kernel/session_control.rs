use bytes::{BufMut, BytesMut};
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
