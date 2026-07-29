use crate::kernel::video_color::{STREAM_VIDEO_COLORIMETRY, VideoColorimetry};

pub(crate) fn gstreamer_colorimetry() -> &'static str {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => "bt709",
    }
}

#[cfg(test)]
mod tests {
    use super::gstreamer_colorimetry;

    #[test]
    fn maps_stream_color_contract_to_linux_apis() {
        assert_eq!(gstreamer_colorimetry(), "bt709");
    }
}
