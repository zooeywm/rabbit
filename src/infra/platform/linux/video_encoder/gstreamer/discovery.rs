//! VAAPI VPP input discovery for DMA-BUF formats and modifiers.

use drm::buffer::{DrmFourcc, DrmModifier};
use eros::Context as _;

use crate::infra::platform::dma_buf::DmaBufProfile;

#[cfg(test)]
pub(crate) fn va_vpp_input_modifier(format: DrmFourcc) -> eros::Result<DrmModifier> {
    Ok(va_vpp_input_modifiers(format)?
        .into_iter()
        .next()
        .with_context(|| "VAAPI VPP modifier discovery returned an empty result")?)
}

pub(crate) fn va_vpp_input_profiles(format: DrmFourcc) -> eros::Result<Vec<DmaBufProfile>> {
    Ok(va_vpp_input_modifiers(format)?
        .into_iter()
        .map(|modifier| DmaBufProfile { format, modifier })
        .collect())
}

pub(crate) fn va_vpp_input_modifiers(format: DrmFourcc) -> eros::Result<Vec<DrmModifier>> {
    gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
    let factory = gstreamer::ElementFactory::find("vapostproc")
        .with_context(|| "GStreamer VAAPI video postprocessor is unavailable")?;
    let mut modifiers = Vec::new();

    for template in factory
        .static_pad_templates()
        .into_iter()
        .filter(|template| template.direction() == gstreamer::PadDirection::Sink)
    {
        for (structure, features) in template.caps().iter_with_features() {
            if !features.contains("memory:DMABuf") {
                continue;
            }
            let Ok(value) = structure.value("drm-format") else {
                continue;
            };
            let mut candidates = Vec::new();
            if let Ok(candidate) = value.get::<&str>() {
                candidates.push(candidate.to_owned());
            } else if let Ok(candidate_list) = value.get::<gstreamer::List>() {
                candidates.extend(
                    candidate_list
                        .as_slice()
                        .iter()
                        .filter_map(|candidate| candidate.get::<&str>().ok())
                        .map(str::to_owned),
                );
            }

            for candidate in candidates {
                let Ok((fourcc, modifier)) = gstreamer_video::dma_drm_fourcc_from_str(&candidate)
                else {
                    continue;
                };
                if fourcc == format as u32 {
                    let modifier = DrmModifier::from(modifier);
                    if !modifiers.contains(&modifier) {
                        modifiers.push(modifier);
                    }
                }
            }
        }
    }

    if modifiers.is_empty() {
        eros::bail!(
            "GStreamer VAAPI video postprocessor exposes no {:?} DMA-BUF modifier",
            format
        );
    }

    Ok(modifiers)
}
