use eros::Context as _;

#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder;

impl GStreamerVideoEncoder {
    pub(crate) fn new() -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;

        Ok(Self)
    }

    fn find_hardware_h264_encoders() -> eros::Result<Vec<gstreamer::ElementFactory>> {
        let h264_caps = gstreamer::Caps::builder("video/x-h264").build();

        let factories = gstreamer::ElementFactory::factories_with_type(
            gstreamer::ElementFactoryType::ENCODER | gstreamer::ElementFactoryType::HARDWARE,
            gstreamer::Rank::NONE,
        )
        .into_iter()
        .filter(|factory| {
            is_hardware_video_encoder(factory) && factory.can_src_any_caps(&h264_caps)
        })
        .collect::<Vec<_>>();

        if factories.is_empty() {
            eros::bail!("No GStreamer hardware H.264 encoder is available");
        }

        Ok(factories)
    }

    fn select_hardware_h264_encoder(
        input_caps: &gstreamer::CapsRef,
    ) -> eros::Result<gstreamer::ElementFactory> {
        if !Self::is_nv12_dmabuf_input_caps(input_caps) {
            eros::bail!(
                "First-version H.264 encoding requires NV12 DMA-BUF input caps, got {}",
                input_caps
            );
        }

        let factory = Self::find_hardware_h264_encoders()?
            .into_iter()
            .find(|factory| factory.can_sink_all_caps(input_caps));

        let Some(factory) = factory else {
            eros::bail!(
                "No GStreamer hardware H.264 encoder accepts input caps {}",
                input_caps
            );
        };

        Ok(factory)
    }

    fn is_nv12_dmabuf_input_caps(caps: &gstreamer::CapsRef) -> bool {
        if caps.size() != 1 {
            return false;
        }

        let Some((structure, features)) = caps.iter_with_features().next() else {
            return false;
        };

        features.contains("memory:DMABuf")
            && structure
                .get::<&str>("format")
                .is_ok_and(|format| format == "DMA_DRM")
            && structure
                .get::<&str>("drm-format")
                .is_ok_and(|format| format == "NV12" || format.starts_with("NV12:"))
    }
}

fn is_hardware_video_encoder(factory: &gstreamer::ElementFactory) -> bool {
    let Some(class) = factory.metadata("klass") else {
        return false;
    };

    ["Encoder", "Video", "Hardware"]
        .into_iter()
        .all(|required| class.split('/').any(|component| component == required))
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::GStreamerVideoEncoder;

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn finds_a_registered_hardware_h264_encoder() {
        let _encoder = GStreamerVideoEncoder::new()
            .expect("GStreamer should initialize before inspecting encoders");
        let factories = GStreamerVideoEncoder::find_hardware_h264_encoders()
            .expect("At least one hardware H.264 encoder should be registered");

        for factory in factories {
            let class = factory
                .metadata("klass")
                .expect("Hardware encoder factory should expose klass metadata");
            assert!(class.split('/').any(|component| component == "Encoder"));
            assert!(class.split('/').any(|component| component == "Video"));
            assert!(class.split('/').any(|component| component == "Hardware"));
            assert!(factory.can_src_any_caps(&h264_caps()));
        }
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn selects_a_hardware_h264_encoder_for_nv12_dmabuf_input() {
        let _encoder = GStreamerVideoEncoder::new()
            .expect("GStreamer should initialize before selecting an encoder");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let factory = GStreamerVideoEncoder::select_hardware_h264_encoder(&input_caps)
            .expect("A hardware H.264 encoder should accept its advertised DMA-BUF input caps");

        assert!(factory.can_sink_all_caps(&input_caps));
        assert!(factory.can_src_any_caps(&h264_caps()));
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn rejects_p010_dmabuf_input() {
        let _encoder = GStreamerVideoEncoder::new()
            .expect("GStreamer should initialize before validating encoder input caps");
        let input_caps = gstreamer::Caps::builder("video/x-raw")
            .features(["memory:DMABuf"])
            .field("format", "DMA_DRM")
            .field("drm-format", "P010")
            .build();

        GStreamerVideoEncoder::select_hardware_h264_encoder(&input_caps)
            .expect_err("The first-version encoder should reject P010 input");
    }

    fn registered_nv12_dmabuf_input_caps() -> gstreamer::Caps {
        GStreamerVideoEncoder::find_hardware_h264_encoders()
            .expect("At least one hardware H.264 encoder should be registered")
            .into_iter()
            .flat_map(|factory| factory.static_pad_templates())
            .filter(|template| template.direction() == gstreamer::PadDirection::Sink)
            .find_map(|template| {
                let caps = template.caps();

                caps.iter_with_features()
                    .map(|(structure, features)| {
                        gstreamer::Caps::builder_full()
                            .structure_with_features(structure.to_owned(), features.to_owned())
                            .build()
                    })
                    .find(|caps| GStreamerVideoEncoder::is_nv12_dmabuf_input_caps(caps))
            })
            .expect("A hardware H.264 encoder should advertise NV12 DMA-BUF input caps")
    }

    fn h264_caps() -> gstreamer::Caps {
        gstreamer::Caps::builder("video/x-h264").build()
    }
}
