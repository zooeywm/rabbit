use eros::Context;

use crate::kernel::geometry::{FrameRate, PixelSize};

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
) -> eros::Result<(PixelSize, FrameRate)> {
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

    let frame_rate = parse_frame_rate(frame_rate)?;
    Ok((PixelSize { width, height }, frame_rate))
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
