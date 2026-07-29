use eros::Context;

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    video_encoder::{VideoBitrate, VideoCodec},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ViewPage {
    #[default]
    Connect,
    Connecting,
    ConnectionError,
    Requests,
    Connected,
    StreamRequest,
    Streaming,
    StreamError,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkspaceSection {
    #[default]
    RemoteDevices,
    ThisDevice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConnectionRequestView {
    pub(crate) name: String,
    pub(crate) address: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConnectedDeviceView {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostedScreenStreamView {
    pub(crate) device_name: String,
    pub(crate) screen_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoteScreenView {
    pub(crate) name: String,
    pub(crate) original: String,
    pub(crate) selected_width: String,
    pub(crate) selected_height: String,
    pub(crate) selected_frame_rate: String,
    pub(crate) selected_bitrate_mbps: String,
}

pub(crate) fn format_frame_rate(frame_rate: FrameRate) -> String {
    let value = f64::from(frame_rate.numerator()) / f64::from(frame_rate.denominator());
    let formatted = format!("{value:.3}");

    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(crate) fn parse_stream_settings(
    width: &str,
    height: &str,
    frame_rate: &str,
    bitrate_mbps: &str,
) -> eros::Result<(PixelSize, FrameRate, VideoBitrate)> {
    let frame_size = parse_frame_size(width, height)?;
    let frame_rate = parse_frame_rate(frame_rate)?;
    let bitrate = parse_bitrate_mbps(bitrate_mbps)?;
    Ok((frame_size, frame_rate, bitrate))
}

pub(crate) fn recommended_bitrate_text(
    codec: VideoCodec,
    width: &str,
    height: &str,
    frame_rate: &str,
) -> Option<String> {
    let frame_size = parse_frame_size(width, height).ok()?;
    let frame_rate = parse_frame_rate(frame_rate).ok()?;
    Some(format_bitrate_mbps(
        codec.recommended_bitrate(frame_size, frame_rate),
    ))
}

pub(crate) fn format_bitrate_mbps(bitrate: VideoBitrate) -> String {
    let bits_per_second = bitrate.bits_per_second();
    let whole = bits_per_second / BITS_PER_MEGABIT;
    let fractional = bits_per_second % BITS_PER_MEGABIT;
    if fractional == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fractional:06}")
        .trim_end_matches('0')
        .to_string()
}

fn parse_frame_size(width: &str, height: &str) -> eros::Result<PixelSize> {
    let width = width
        .trim()
        .parse::<u32>()
        .with_context(|| format!("Invalid stream width {width:?}"))?;
    let height = height
        .trim()
        .parse::<u32>()
        .with_context(|| format!("Invalid stream height {height:?}"))?;
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        eros::bail!(
            "Stream resolution must use positive even dimensions, got {} × {}",
            width,
            height
        );
    }

    Ok(PixelSize { width, height })
}

fn parse_frame_rate(value: &str) -> eros::Result<FrameRate> {
    let value = value.trim();
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() || fractional.contains('.') || fractional.len() > 3 {
        eros::bail!("Invalid stream frame rate {:?}", value);
    }
    let whole = whole
        .parse::<u32>()
        .with_context(|| format!("Invalid stream frame rate {value:?}"))?;
    let denominator = match fractional.len() {
        0 => 1,
        1 => 10,
        2 => 100,
        3 => 1_000,
        _ => eros::bail!("Invalid stream frame rate {:?}", value),
    };
    let fractional = if fractional.is_empty() {
        0
    } else {
        fractional
            .parse::<u32>()
            .with_context(|| format!("Invalid stream frame rate {value:?}"))?
    };
    let numerator = whole
        .checked_mul(denominator)
        .and_then(|whole| whole.checked_add(fractional))
        .with_context(|| format!("Stream frame rate {value:?} is too large"))?;

    Ok(FrameRate::new(numerator, denominator)
        .with_context(|| format!("Stream frame rate must be positive, got {value:?}"))?)
}

fn parse_bitrate_mbps(value: &str) -> eros::Result<VideoBitrate> {
    let value = value.trim();
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() || fractional.contains('.') || fractional.len() > 3 {
        eros::bail!("Invalid stream bitrate {:?} Mbps", value);
    }
    let whole = whole
        .parse::<u32>()
        .with_context(|| format!("Invalid stream bitrate {value:?} Mbps"))?;
    let fractional_value = if fractional.is_empty() {
        0
    } else {
        fractional
            .parse::<u32>()
            .with_context(|| format!("Invalid stream bitrate {value:?} Mbps"))?
    };
    let fractional_scale = match fractional.len() {
        0 => 0,
        1 => 100_000,
        2 => 10_000,
        3 => 1_000,
        _ => unreachable!(),
    };
    let bits_per_second = whole
        .checked_mul(BITS_PER_MEGABIT)
        .and_then(|whole| {
            fractional_value
                .checked_mul(fractional_scale)
                .and_then(|fractional| whole.checked_add(fractional))
        })
        .with_context(|| format!("Stream bitrate {value:?} Mbps is too large"))?;
    VideoBitrate::new(bits_per_second)
        .with_context(|| format!("Stream bitrate must be positive, got {value:?} Mbps"))
}

const BITS_PER_MEGABIT: u32 = 1_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewState {
    pub(crate) section: WorkspaceSection,
    pub(crate) page: ViewPage,
    pub(crate) page_title: String,
    pub(crate) page_subtitle: String,
    pub(crate) status_text: String,
    pub(crate) stream_settings_error: String,
    pub(crate) local_protocol: String,
    pub(crate) local_port: String,
    pub(crate) local_server_online: bool,
    pub(crate) stream_title: String,
    pub(crate) stream_resolution: String,
    pub(crate) connection_requests: Vec<ConnectionRequestView>,
    pub(crate) connected_devices: Vec<ConnectedDeviceView>,
    pub(crate) hosted_screen_streams: Vec<HostedScreenStreamView>,
    pub(crate) remote_screens: Vec<RemoteScreenView>,
}
