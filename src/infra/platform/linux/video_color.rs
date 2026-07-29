use khronos_egl as egl;

use crate::{
    infra::platform::egl_dma_buf::{
        ITU_REC709_EXT, SAMPLE_RANGE_HINT_EXT, YUV_COLOR_SPACE_HINT_EXT, YUV_NARROW_RANGE_EXT,
    },
    kernel::video_color::{STREAM_VIDEO_COLORIMETRY, VideoColorimetry},
};

pub(crate) fn gstreamer_colorimetry() -> &'static str {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => "bt709",
    }
}

pub(crate) fn egl_yuv_color_attributes() -> [egl::Attrib; 4] {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => [
            YUV_COLOR_SPACE_HINT_EXT,
            ITU_REC709_EXT,
            SAMPLE_RANGE_HINT_EXT,
            YUV_NARROW_RANGE_EXT,
        ],
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::egl_dma_buf::{
        ITU_REC709_EXT, SAMPLE_RANGE_HINT_EXT, YUV_COLOR_SPACE_HINT_EXT, YUV_NARROW_RANGE_EXT,
    };

    use super::{egl_yuv_color_attributes, gstreamer_colorimetry};

    #[test]
    fn maps_stream_color_contract_to_linux_apis() {
        assert_eq!(gstreamer_colorimetry(), "bt709");
        assert_eq!(
            egl_yuv_color_attributes(),
            [
                YUV_COLOR_SPACE_HINT_EXT,
                ITU_REC709_EXT,
                SAMPLE_RANGE_HINT_EXT,
                YUV_NARROW_RANGE_EXT,
            ]
        );
    }
}
