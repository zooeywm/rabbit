//! Binary wire codecs for control-plane messages (`binrw`).

use binrw::{BinRead, BinReaderExt, BinWrite, BinWriterExt, binread, binrw, io::Cursor};
use bytes::Bytes;
use eros::Context;

use super::{ControlMessage, OutgoingRemoteInput, OutgoingScreenList, ScreenInfo};
use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    input::{
        AbsolutePointerMove, InputState, KeyboardInput, KeyboardKey, MouseButton, MouseButtonInput,
        NormalizedPosition, RelativePointerMove, RemoteInputEvent,
    },
    screen_configuration::{
        RemoteDisplayMode, RequestKeyFrame, ResolutionResult, ScreenResolutionOutcome,
        ScreenResolutionStatus, ScreenStreamRequest, ScreenStreamRequestId,
        ScreenStreamsConfigured, SetScreenStreams, StopScreenStream,
    },
    screen_manager::{Screen, ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
    transport::{Delivery, TransportChannel, TransportMessage},
};

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
pub(super) enum WireControlMessageTag {
    ScreenList = 0,
    SetScreenStreams = 1,
    ScreenStreamsConfigured = 2,
    StopScreenStream = 3,
    RequestKeyFrame = 4,
    AbsolutePointerMove = 5,
    KeyboardInput = 6,
    MouseButtonInput = 7,
    RelativePointerMove = 8,
}

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
pub(super) enum WireRemoteDisplayMode {
    Preserve = 0,
}

#[derive(BinRead, BinWrite)]
#[brw(repr = u8)]
pub(super) enum WireScreenTransform {
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
pub(super) struct WirePixelSize {
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireFrameRate {
    pub(super) numerator: u32,
    pub(super) denominator: u32,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireScreenRect {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireScreenLayout {
    pub(super) rect: WireScreenRect,
    pub(super) scale: f64,
    pub(super) transform: WireScreenTransform,
}

#[derive(BinWrite)]
pub(super) struct WireScreenInfoRef<'a> {
    pub(super) id: u8,
    pub(super) name_length: u16,
    pub(super) name: &'a [u8],
    pub(super) resolution: WirePixelSize,
    pub(super) frame_rate: WireFrameRate,
    pub(super) layout: WireScreenLayout,
}

#[binread]
pub(super) struct WireScreenInfo {
    pub(super) id: u8,
    #[br(temp)]
    name_length: u16,
    #[br(count = name_length)]
    pub(super) name: Vec<u8>,
    pub(super) resolution: WirePixelSize,
    pub(super) frame_rate: WireFrameRate,
    pub(super) layout: WireScreenLayout,
}

#[binread]
pub(super) struct WireScreenList {
    #[br(temp)]
    screen_count: u8,
    #[br(count = screen_count)]
    screens: Vec<WireScreenInfo>,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireScreenStreamRequest {
    screen_id: u8,
    remote_display: WireRemoteDisplayMode,
    frame_size: WirePixelSize,
    frame_rate: WireFrameRate,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireStopScreenStream {
    screen_id: u8,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireRequestKeyFrame {
    screen_id: u8,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireAbsolutePointerMove {
    screen_id: u8,
    x: u16,
    y: u16,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireKeyboardInput {
    screen_id: u8,
    key: u16,
    state: u8,
    repeat: u8,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireMouseButtonInput {
    screen_id: u8,
    button: u8,
    state: u8,
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireRelativePointerMove {
    screen_id: u8,
    delta_x: i32,
    delta_y: i32,
}

#[binrw]
pub(super) struct WireSetScreenStreams {
    request_id: u32,
    #[br(temp)]
    #[bw(try_calc(u8::try_from(desired_streams.len())))]
    desired_stream_count: u8,
    #[br(count = desired_stream_count)]
    desired_streams: Vec<WireScreenStreamRequest>,
}

#[derive(BinRead, BinWrite)]
#[br(return_unexpected_error)]
pub(super) enum WireResolutionResult {
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
pub(super) enum WireOptionalPixelSize {
    #[brw(magic(0u8))]
    None,
    #[brw(magic(1u8))]
    Some(WirePixelSize),
}

#[derive(BinRead, BinWrite)]
#[br(return_unexpected_error)]
pub(super) enum WireScreenResolutionStatus {
    #[brw(magic(0u8))]
    Configured(WireResolutionResult),
    #[brw(magic(1u8))]
    Failed {
        requested: WirePixelSize,
        actual: WireOptionalPixelSize,
    },
}

#[derive(BinRead, BinWrite)]
pub(super) struct WireScreenResolutionOutcome {
    screen_id: u8,
    status: WireScreenResolutionStatus,
}

#[binrw]
pub(super) struct WireScreenStreamsConfigured {
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

impl From<FrameRate> for WireFrameRate {
    fn from(frame_rate: FrameRate) -> Self {
        Self {
            numerator: frame_rate.numerator(),
            denominator: frame_rate.denominator(),
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
        let frame_rate = FrameRate::new(screen.frame_rate.numerator, screen.frame_rate.denominator)
            .with_context(|| format!("Failed to decode ScreenInfo {} frame rate", screen.id))?;

        Ok(Self {
            id: ScreenId::try_from(screen.id)
                .with_context(|| format!("Failed to decode ScreenInfo {} screen ID", screen.id))?,
            name,
            resolution: screen.resolution.into(),
            frame_rate,
            layout: screen.layout.into(),
        })
    }
}

impl TryFrom<WireScreenStreamRequest> for ScreenStreamRequest {
    type Error = eros::ErrorUnion;

    fn try_from(request: WireScreenStreamRequest) -> eros::Result<Self> {
        let frame_rate =
            FrameRate::new(request.frame_rate.numerator, request.frame_rate.denominator)
                .with_context(|| {
                    format!(
                        "Failed to decode SetScreenStreams frame rate for screen {}",
                        request.screen_id
                    )
                })?;

        Ok(Self {
            screen_id: ScreenId::try_from(request.screen_id).with_context(|| {
                format!(
                    "Failed to decode SetScreenStreams screen ID {}",
                    request.screen_id
                )
            })?,
            remote_display: request.remote_display.into(),
            frame_size: request.frame_size.into(),
            frame_rate,
        })
    }
}

impl From<ScreenStreamRequest> for WireScreenStreamRequest {
    fn from(request: ScreenStreamRequest) -> Self {
        Self {
            screen_id: request.screen_id.0,
            remote_display: request.remote_display.into(),
            frame_size: request.frame_size.into(),
            frame_rate: request.frame_rate.into(),
        }
    }
}

impl From<SetScreenStreams> for WireSetScreenStreams {
    fn from(request: SetScreenStreams) -> Self {
        Self {
            request_id: request.request_id.0,
            desired_streams: request
                .desired_streams
                .into_iter()
                .map(Into::into)
                .collect(),
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

impl TryFrom<WireScreenResolutionOutcome> for ScreenResolutionOutcome {
    type Error = eros::ErrorUnion;

    fn try_from(outcome: WireScreenResolutionOutcome) -> eros::Result<Self> {
        Ok(Self {
            screen_id: ScreenId::try_from(outcome.screen_id).with_context(|| {
                format!(
                    "Failed to decode ScreenStreamsConfigured screen ID {}",
                    outcome.screen_id
                )
            })?,
            status: outcome.status.into(),
        })
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

impl TryFrom<&[Screen]> for OutgoingScreenList {
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

        Ok(Self(finish_control_message(writer)))
    }
}

impl From<OutgoingScreenList> for TransportMessage {
    fn from(screen_list: OutgoingScreenList) -> Self {
        screen_list.0
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

impl TryFrom<StopScreenStream> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(stop: StopScreenStream) -> eros::Result<Self> {
        let mut writer = begin_control_message(WireControlMessageTag::StopScreenStream)?;

        writer
            .write_be(&WireStopScreenStream {
                screen_id: stop.screen_id.0,
            })
            .with_context(|| {
                format!(
                    "Failed to encode StopScreenStream for screen {}",
                    stop.screen_id.0
                )
            })?;

        Ok(finish_control_message(writer))
    }
}

impl TryFrom<RequestKeyFrame> for TransportMessage {
    type Error = eros::ErrorUnion;

    fn try_from(request: RequestKeyFrame) -> eros::Result<Self> {
        let mut writer = begin_control_message(WireControlMessageTag::RequestKeyFrame)?;

        writer
            .write_be(&WireRequestKeyFrame {
                screen_id: request.screen_id.0,
            })
            .with_context(|| {
                format!(
                    "Failed to encode key-frame request for screen {}",
                    request.screen_id.0
                )
            })?;

        Ok(finish_control_message(writer))
    }
}

impl TryFrom<RemoteInputEvent> for OutgoingRemoteInput {
    type Error = eros::ErrorUnion;

    fn try_from(input: RemoteInputEvent) -> eros::Result<Self> {
        let writer = match input {
            RemoteInputEvent::AbsolutePointerMove(movement) => {
                let mut writer = begin_control_message(WireControlMessageTag::AbsolutePointerMove)?;
                writer
                    .write_be(&WireAbsolutePointerMove {
                        screen_id: movement.screen_id.0,
                        x: movement.position.x,
                        y: movement.position.y,
                    })
                    .with_context(|| "Failed to encode absolute pointer movement")?;
                writer
            }
            RemoteInputEvent::Keyboard(input) => {
                let mut writer = begin_control_message(WireControlMessageTag::KeyboardInput)?;
                writer
                    .write_be(&WireKeyboardInput {
                        screen_id: input.screen_id.0,
                        key: input.key.wire_code(),
                        state: input.state.wire_code(),
                        repeat: u8::from(input.repeat),
                    })
                    .with_context(|| "Failed to encode keyboard input")?;
                writer
            }
            RemoteInputEvent::MouseButton(input) => {
                let mut writer = begin_control_message(WireControlMessageTag::MouseButtonInput)?;
                writer
                    .write_be(&WireMouseButtonInput {
                        screen_id: input.screen_id.0,
                        button: input.button.wire_code(),
                        state: input.state.wire_code(),
                    })
                    .with_context(|| "Failed to encode mouse button input")?;
                writer
            }
            RemoteInputEvent::RelativePointerMove(movement) => {
                let mut writer = begin_control_message(WireControlMessageTag::RelativePointerMove)?;
                writer
                    .write_be(&WireRelativePointerMove {
                        screen_id: movement.screen_id.0,
                        delta_x: movement.delta_x,
                        delta_y: movement.delta_y,
                    })
                    .with_context(|| "Failed to encode relative pointer movement")?;
                writer
            }
        };
        let mut message = finish_control_message(writer);
        if !input.is_reliable() {
            message.delivery = Delivery::Unreliable;
        }
        Ok(Self(message))
    }
}

impl From<OutgoingRemoteInput> for TransportMessage {
    fn from(input: OutgoingRemoteInput) -> Self {
        input.0
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

        let delivery = message.delivery;
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
                    desired_streams: wire
                        .desired_streams
                        .into_iter()
                        .map(ScreenStreamRequest::try_from)
                        .collect::<eros::Result<Vec<_>>>()?,
                })
            }
            WireControlMessageTag::ScreenStreamsConfigured => {
                let wire = reader
                    .read_be::<WireScreenStreamsConfigured>()
                    .with_context(|| "Failed to decode ScreenStreamsConfigured")?;

                Self::ScreenStreamsConfigured(ScreenStreamsConfigured {
                    request_id: ScreenStreamRequestId(wire.request_id),
                    outcomes: wire
                        .outcomes
                        .into_iter()
                        .map(ScreenResolutionOutcome::try_from)
                        .collect::<eros::Result<Vec<_>>>()?,
                })
            }
            WireControlMessageTag::StopScreenStream => {
                let wire = reader
                    .read_be::<WireStopScreenStream>()
                    .with_context(|| "Failed to decode StopScreenStream")?;

                Self::StopScreenStream(StopScreenStream {
                    screen_id: ScreenId(wire.screen_id),
                })
            }
            WireControlMessageTag::RequestKeyFrame => {
                let wire = reader
                    .read_be::<WireRequestKeyFrame>()
                    .with_context(|| "Failed to decode key-frame request")?;

                Self::RequestKeyFrame(RequestKeyFrame {
                    screen_id: ScreenId::try_from(wire.screen_id).with_context(|| {
                        format!(
                            "Failed to decode key-frame request screen ID {}",
                            wire.screen_id
                        )
                    })?,
                })
            }
            WireControlMessageTag::AbsolutePointerMove => {
                let wire = reader
                    .read_be::<WireAbsolutePointerMove>()
                    .with_context(|| "Failed to decode absolute pointer movement")?;
                Self::RemoteInput(RemoteInputEvent::AbsolutePointerMove(AbsolutePointerMove {
                    screen_id: ScreenId::try_from(wire.screen_id).with_context(|| {
                        format!(
                            "Failed to decode absolute pointer movement screen ID {}",
                            wire.screen_id
                        )
                    })?,
                    position: NormalizedPosition {
                        x: wire.x,
                        y: wire.y,
                    },
                }))
            }
            WireControlMessageTag::KeyboardInput => {
                let wire = reader
                    .read_be::<WireKeyboardInput>()
                    .with_context(|| "Failed to decode keyboard input")?;
                let state = InputState::try_from(wire.state)?;
                let repeat = match wire.repeat {
                    0 => false,
                    1 => true,
                    value => eros::bail!("Invalid keyboard repeat flag {}", value),
                };
                if state == InputState::Released && repeat {
                    eros::bail!("Released keyboard input cannot be marked as repeated");
                }
                Self::RemoteInput(RemoteInputEvent::Keyboard(KeyboardInput {
                    screen_id: ScreenId::try_from(wire.screen_id)?,
                    key: KeyboardKey::try_from(wire.key)?,
                    state,
                    repeat,
                }))
            }
            WireControlMessageTag::MouseButtonInput => {
                let wire = reader
                    .read_be::<WireMouseButtonInput>()
                    .with_context(|| "Failed to decode mouse button input")?;
                Self::RemoteInput(RemoteInputEvent::MouseButton(MouseButtonInput {
                    screen_id: ScreenId::try_from(wire.screen_id)?,
                    button: MouseButton::try_from(wire.button)?,
                    state: InputState::try_from(wire.state)?,
                }))
            }
            WireControlMessageTag::RelativePointerMove => {
                let wire = reader
                    .read_be::<WireRelativePointerMove>()
                    .with_context(|| "Failed to decode relative pointer movement")?;
                Self::RemoteInput(RemoteInputEvent::RelativePointerMove(RelativePointerMove {
                    screen_id: ScreenId::try_from(wire.screen_id)?,
                    delta_x: wire.delta_x,
                    delta_y: wire.delta_y,
                }))
            }
        };
        let expected_delivery = match &message {
            Self::RemoteInput(input) if !input.is_reliable() => Delivery::Unreliable,
            _ => Delivery::ReliableOrdered,
        };
        if delivery != expected_delivery {
            eros::bail!(
                "Control message has delivery {:?}, expected {:?}",
                delivery,
                expected_delivery
            );
        }
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
    let name_length =
        u16::try_from(name.len()).with_context(|| "Failed to encode ScreenInfo name length")?;
    let wire = WireScreenInfoRef {
        id: screen.id.0,
        name_length,
        name,
        resolution: screen.resolution.into(),
        frame_rate: screen.frame_rate.into(),
        layout: screen.layout.into(),
    };

    writer
        .write_be(&wire)
        .with_context(|| format!("Failed to encode ScreenInfo {}", screen.id.0))?;

    Ok(())
}

// Focused test: cargo test kernel::session_control::tests --lib
