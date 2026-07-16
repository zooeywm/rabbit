use binrw::{
    BinRead, BinReaderExt, BinWrite, BinWriterExt, binread, binrw, io::Cursor,
};
use bytes::Bytes;
use eros::Context;

use crate::kernel::{
    screen_configuration::{
        PixelSize, RemoteDisplayMode, ResolutionResult, ScreenResolutionOutcome,
        ScreenResolutionStatus, ScreenStreamRequest, ScreenStreamRequestId,
        ScreenStreamsConfigured, SetScreenStreams,
    },
    screen_manager::{Screen, ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
    transport::{Delivery, TransportChannel, TransportMessage},
};

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

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
enum WireControlMessageTag {
    ScreenList = 0,
    SetScreenStreams = 1,
    ScreenStreamsConfigured = 2,
}

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
enum WireRemoteDisplayMode {
    Preserve = 0,
}

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
enum WireScreenTransform {
    Normal = 0,
    Rotate90 = 1,
    Rotate180 = 2,
    Rotate270 = 3,
    Flipped = 4,
    Flipped90 = 5,
    Flipped180 = 6,
    Flipped270 = 7,
}

#[derive(BinRead, BinWrite)]
struct WirePixelSize {
    width: u32,
    height: u32,
}

#[derive(BinRead, BinWrite)]
struct WireScreenRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(BinRead, BinWrite)]
struct WireScreenLayout {
    rect: WireScreenRect,
    scale: f64,
    transform: WireScreenTransform,
}

#[derive(BinWrite)]
struct WireScreenInfoRef<'a> {
    id: u8,
    name_length: u16,
    name: &'a [u8],
    layout: WireScreenLayout,
}

#[binread]
struct WireScreenInfo {
    id: u8,
    #[br(temp)]
    name_length: u16,
    #[br(count = name_length)]
    name: Vec<u8>,
    layout: WireScreenLayout,
}

#[binread]
struct WireScreenList {
    #[br(temp)]
    screen_count: u8,
    #[br(count = screen_count)]
    screens: Vec<WireScreenInfo>,
}

#[derive(BinRead, BinWrite)]
struct WireScreenStreamRequest {
    screen_id: u8,
    remote_display: WireRemoteDisplayMode,
    max_resolution: WirePixelSize,
}

#[binrw]
struct WireSetScreenStreams {
    request_id: u32,
    #[br(temp)]
    #[bw(try_calc(u8::try_from(changes.len())))]
    change_count: u8,
    #[br(count = change_count)]
    changes: Vec<WireScreenStreamRequest>,
}

#[derive(BinRead, BinWrite)]
#[br(return_unexpected_error)]
enum WireResolutionResult {
    #[brw(magic(0u8))]
    Exact { applied: WirePixelSize },
    #[brw(magic(1u8))]
    Fallback {
        requested: WirePixelSize,
        applied: WirePixelSize,
    },
    #[brw(magic(2u8))]
    Preserved {
        requested: WirePixelSize,
        actual: WirePixelSize,
    },
}

#[derive(BinRead, BinWrite)]
#[br(return_unexpected_error)]
enum WireOptionalPixelSize {
    #[brw(magic(0u8))]
    None,
    #[brw(magic(1u8))]
    Some(WirePixelSize),
}

#[derive(BinRead, BinWrite)]
#[br(return_unexpected_error)]
enum WireScreenResolutionStatus {
    #[brw(magic(0u8))]
    Configured(WireResolutionResult),
    #[brw(magic(1u8))]
    Failed {
        requested: WirePixelSize,
        actual: WireOptionalPixelSize,
    },
}

#[derive(BinRead, BinWrite)]
struct WireScreenResolutionOutcome {
    screen_id: u8,
    status: WireScreenResolutionStatus,
}

#[binrw]
struct WireScreenStreamsConfigured {
    request_id: u32,
    #[br(temp)]
    #[bw(try_calc(u8::try_from(outcomes.len())))]
    outcome_count: u8,
    #[br(count = outcome_count)]
    outcomes: Vec<WireScreenResolutionOutcome>,
}

impl From<RemoteDisplayMode> for WireRemoteDisplayMode {
    fn from(mode: RemoteDisplayMode) -> Self {
        match mode {
            RemoteDisplayMode::Preserve => Self::Preserve,
        }
    }
}

impl From<WireRemoteDisplayMode> for RemoteDisplayMode {
    fn from(mode: WireRemoteDisplayMode) -> Self {
        match mode {
            WireRemoteDisplayMode::Preserve => Self::Preserve,
        }
    }
}

impl From<ScreenTransform> for WireScreenTransform {
    fn from(transform: ScreenTransform) -> Self {
        match transform {
            ScreenTransform::Normal => Self::Normal,
            ScreenTransform::Rotate90 => Self::Rotate90,
            ScreenTransform::Rotate180 => Self::Rotate180,
            ScreenTransform::Rotate270 => Self::Rotate270,
            ScreenTransform::Flipped => Self::Flipped,
            ScreenTransform::Flipped90 => Self::Flipped90,
            ScreenTransform::Flipped180 => Self::Flipped180,
            ScreenTransform::Flipped270 => Self::Flipped270,
        }
    }
}

impl From<WireScreenTransform> for ScreenTransform {
    fn from(transform: WireScreenTransform) -> Self {
        match transform {
            WireScreenTransform::Normal => Self::Normal,
            WireScreenTransform::Rotate90 => Self::Rotate90,
            WireScreenTransform::Rotate180 => Self::Rotate180,
            WireScreenTransform::Rotate270 => Self::Rotate270,
            WireScreenTransform::Flipped => Self::Flipped,
            WireScreenTransform::Flipped90 => Self::Flipped90,
            WireScreenTransform::Flipped180 => Self::Flipped180,
            WireScreenTransform::Flipped270 => Self::Flipped270,
        }
    }
}

impl From<PixelSize> for WirePixelSize {
    fn from(size: PixelSize) -> Self {
        Self {
            width: size.width,
            height: size.height,
        }
    }
}

impl From<ScreenRect> for WireScreenRect {
    fn from(rect: ScreenRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

impl From<ScreenLayout> for WireScreenLayout {
    fn from(layout: ScreenLayout) -> Self {
        Self {
            rect: layout.rect.into(),
            scale: layout.scale,
            transform: layout.transform.into(),
        }
    }
}

impl From<WirePixelSize> for PixelSize {
    fn from(size: WirePixelSize) -> Self {
        Self {
            width: size.width,
            height: size.height,
        }
    }
}

impl From<WireScreenRect> for ScreenRect {
    fn from(rect: WireScreenRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

impl From<WireScreenLayout> for ScreenLayout {
    fn from(layout: WireScreenLayout) -> Self {
        Self {
            rect: layout.rect.into(),
            scale: layout.scale,
            transform: layout.transform.into(),
        }
    }
}

impl TryFrom<WireScreenInfo> for ScreenInfo {
    type Error = eros::ErrorUnion;

    fn try_from(screen: WireScreenInfo) -> eros::Result<Self> {
        let name = String::from_utf8(screen.name)
            .with_context(|| format!("Failed to decode name for ScreenInfo {}", screen.id))?;

        Ok(Self {
            id: ScreenId(screen.id),
            name,
            layout: screen.layout.into(),
        })
    }
}

impl From<WireScreenStreamRequest> for ScreenStreamRequest {
    fn from(request: WireScreenStreamRequest) -> Self {
        Self {
            screen_id: ScreenId(request.screen_id),
            remote_display: request.remote_display.into(),
            max_resolution: request.max_resolution.into(),
        }
    }
}

impl From<ScreenStreamRequest> for WireScreenStreamRequest {
    fn from(request: ScreenStreamRequest) -> Self {
        Self {
            screen_id: request.screen_id.0,
            remote_display: request.remote_display.into(),
            max_resolution: request.max_resolution.into(),
        }
    }
}

impl From<SetScreenStreams> for WireSetScreenStreams {
    fn from(request: SetScreenStreams) -> Self {
        Self {
            request_id: request.request_id.0,
            changes: request.changes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<WireResolutionResult> for ResolutionResult {
    fn from(result: WireResolutionResult) -> Self {
        match result {
            WireResolutionResult::Exact { applied } => Self::Exact {
                applied: applied.into(),
            },
            WireResolutionResult::Fallback { requested, applied } => Self::Fallback {
                requested: requested.into(),
                applied: applied.into(),
            },
            WireResolutionResult::Preserved { requested, actual } => Self::Preserved {
                requested: requested.into(),
                actual: actual.into(),
            },
        }
    }
}

impl From<ResolutionResult> for WireResolutionResult {
    fn from(result: ResolutionResult) -> Self {
        match result {
            ResolutionResult::Exact { applied } => Self::Exact {
                applied: applied.into(),
            },
            ResolutionResult::Fallback { requested, applied } => Self::Fallback {
                requested: requested.into(),
                applied: applied.into(),
            },
            ResolutionResult::Preserved { requested, actual } => Self::Preserved {
                requested: requested.into(),
                actual: actual.into(),
            },
        }
    }
}

impl From<WireOptionalPixelSize> for Option<PixelSize> {
    fn from(size: WireOptionalPixelSize) -> Self {
        match size {
            WireOptionalPixelSize::None => None,
            WireOptionalPixelSize::Some(size) => Some(size.into()),
        }
    }
}

impl From<Option<PixelSize>> for WireOptionalPixelSize {
    fn from(size: Option<PixelSize>) -> Self {
        match size {
            Some(size) => Self::Some(size.into()),
            None => Self::None,
        }
    }
}

impl From<WireScreenResolutionStatus> for ScreenResolutionStatus {
    fn from(status: WireScreenResolutionStatus) -> Self {
        match status {
            WireScreenResolutionStatus::Configured(result) => Self::Configured(result.into()),
            WireScreenResolutionStatus::Failed { requested, actual } => Self::Failed {
                requested: requested.into(),
                actual: actual.into(),
            },
        }
    }
}

impl From<ScreenResolutionStatus> for WireScreenResolutionStatus {
    fn from(status: ScreenResolutionStatus) -> Self {
        match status {
            ScreenResolutionStatus::Configured(result) => Self::Configured(result.into()),
            ScreenResolutionStatus::Failed { requested, actual } => Self::Failed {
                requested: requested.into(),
                actual: actual.into(),
            },
        }
    }
}

impl From<WireScreenResolutionOutcome> for ScreenResolutionOutcome {
    fn from(outcome: WireScreenResolutionOutcome) -> Self {
        Self {
            screen_id: ScreenId(outcome.screen_id),
            status: outcome.status.into(),
        }
    }
}

impl From<ScreenResolutionOutcome> for WireScreenResolutionOutcome {
    fn from(outcome: ScreenResolutionOutcome) -> Self {
        Self {
            screen_id: outcome.screen_id.0,
            status: outcome.status.into(),
        }
    }
}

impl From<ScreenStreamsConfigured> for WireScreenStreamsConfigured {
    fn from(configured: ScreenStreamsConfigured) -> Self {
        Self {
            request_id: configured.request_id.0,
            outcomes: configured.outcomes.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<&[Screen]> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(screens: &[Screen]) -> eros::Result<Self> {
        let screen_count = u8::try_from(screens.len())
            .with_context(|| "Failed to encode ScreenList screen count")?;
        let mut writer = begin_control_message(WireControlMessageTag::ScreenList)?;

        writer
            .write_be(&screen_count)
            .with_context(|| "Failed to encode ScreenList screen count")?;

        for screen in screens {
            write_screen_info(&mut writer, screen)?;
        }

        Ok(finish_control_message(writer))
    }
}

impl TryFrom<SetScreenStreams> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(request: SetScreenStreams) -> eros::Result<Self> {
        let mut writer = begin_control_message(WireControlMessageTag::SetScreenStreams)?;
        let wire = WireSetScreenStreams::from(request);

        writer
            .write_be(&wire)
            .with_context(|| "Failed to encode SetScreenStreams")?;

        Ok(finish_control_message(writer))
    }
}

impl TryFrom<ScreenStreamsConfigured> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(configured: ScreenStreamsConfigured) -> eros::Result<Self> {
        let mut writer = begin_control_message(WireControlMessageTag::ScreenStreamsConfigured)?;
        let wire = WireScreenStreamsConfigured::from(configured);

        writer
            .write_be(&wire)
            .with_context(|| "Failed to encode ScreenStreamsConfigured")?;

        Ok(finish_control_message(writer))
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

        let mut reader = Cursor::new(message.payload);
        let tag = reader
            .read_be::<WireControlMessageTag>()
            .with_context(|| "Failed to decode Control message tag")?;
        let message = match tag {
            WireControlMessageTag::ScreenList => {
                let wire = reader
                    .read_be::<WireScreenList>()
                    .with_context(|| "Failed to decode ScreenList")?;
                let screens = wire
                    .screens
                    .into_iter()
                    .map(ScreenInfo::try_from)
                    .collect::<eros::Result<Vec<_>>>()?;

                Self::ScreenList(screens)
            }
            WireControlMessageTag::SetScreenStreams => {
                let wire = reader
                    .read_be::<WireSetScreenStreams>()
                    .with_context(|| "Failed to decode SetScreenStreams")?;

                Self::SetScreenStreams(SetScreenStreams {
                    request_id: ScreenStreamRequestId(wire.request_id),
                    changes: wire.changes.into_iter().map(Into::into).collect(),
                })
            }
            WireControlMessageTag::ScreenStreamsConfigured => {
                let wire = reader
                    .read_be::<WireScreenStreamsConfigured>()
                    .with_context(|| "Failed to decode ScreenStreamsConfigured")?;

                Self::ScreenStreamsConfigured(ScreenStreamsConfigured {
                    request_id: ScreenStreamRequestId(wire.request_id),
                    outcomes: wire.outcomes.into_iter().map(Into::into).collect(),
                })
            }
        };
        let payload_length = u64::try_from(reader.get_ref().len())
            .with_context(|| "Failed to validate decoded Control payload length")?;

        if reader.position() != payload_length {
            eros::bail!(
                "Control message contains {} trailing payload bytes",
                payload_length - reader.position()
            );
        }

        Ok(message)
    }
}

fn begin_control_message(tag: WireControlMessageTag) -> eros::Result<Cursor<Vec<u8>>> {
    let mut writer = Cursor::new(Vec::new());

    writer
        .write_be(&tag)
        .with_context(|| "Failed to encode Control message tag")?;

    Ok(writer)
}

fn finish_control_message(writer: Cursor<Vec<u8>>) -> TransportMessage {
    TransportMessage {
        channel: TransportChannel::Control,
        delivery: Delivery::ReliableOrdered,
        payload: Bytes::from(writer.into_inner()),
    }
}

fn write_screen_info(writer: &mut Cursor<Vec<u8>>, screen: &Screen) -> eros::Result<()> {
    let name = screen.name.as_bytes();
    let name_length = u16::try_from(name.len())
        .with_context(|| "Failed to encode ScreenInfo name length")?;
    let wire = WireScreenInfoRef {
        id: screen.id.0,
        name_length,
        name,
        layout: screen.layout.into(),
    };

    writer
        .write_be(&wire)
        .with_context(|| format!("Failed to encode ScreenInfo {}", screen.id.0))?;

    Ok(())
}
