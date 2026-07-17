use eros::Context as _;

#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder;

impl GStreamerVideoEncoder {
    pub(crate) fn new() -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;

        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::GStreamerVideoEncoder;

    #[test]
    fn gstreamer_video_encoder_boundary_can_be_created() {
        let _encoder = GStreamerVideoEncoder::new()
            .expect("GStreamer should initialize before creating an encoder");
    }
}
