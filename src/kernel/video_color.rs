//! Fixed color contract for encoded screen video.

/// Rabbit screen streams always carry SDR BT.709 YCbCr in limited range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoColorimetry {
    Bt709Limited,
}

pub const STREAM_VIDEO_COLORIMETRY: VideoColorimetry = VideoColorimetry::Bt709Limited;

#[cfg(test)]
mod tests {
    use super::{STREAM_VIDEO_COLORIMETRY, VideoColorimetry};

    #[test]
    fn stream_colorimetry_is_bt709_limited() {
        assert_eq!(STREAM_VIDEO_COLORIMETRY, VideoColorimetry::Bt709Limited);
    }
}
