#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder;

impl GStreamerVideoEncoder {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::GStreamerVideoEncoder;

    #[test]
    fn gstreamer_video_encoder_boundary_can_be_created() {
        let _encoder = GStreamerVideoEncoder::new();
    }
}
