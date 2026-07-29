use windows::Win32::{
    Graphics::{
        Direct3D11::D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
        Dxgi::{
            Common::{
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_TYPE,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            },
            DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAG_BT709,
            DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAG_NOMINAL_RANGE, DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAGS,
        },
    },
    Media::MediaFoundation::{
        IMFMediaType, MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
        MF_MT_YUV_MATRIX, MFNominalRange_16_235, MFVideoPrimaries_BT709, MFVideoTransFunc_709,
        MFVideoTransferMatrix_BT709,
    },
};

use crate::kernel::video_color::{STREAM_VIDEO_COLORIMETRY, VideoColorimetry};

pub(crate) fn captured_rgb_color_space() -> DXGI_COLOR_SPACE_TYPE {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
    }
}

pub(crate) fn encoded_ycbcr_color_space() -> DXGI_COLOR_SPACE_TYPE {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    }
}

pub(crate) fn legacy_captured_rgb_color_space() -> D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
    match STREAM_VIDEO_COLORIMETRY {
        // D3D11_VIDEO_PROCESSOR_COLOR_SPACE: RGB_Range=0 means full range.
        VideoColorimetry::Bt709Limited => D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
    }
}

pub(crate) fn legacy_encoded_ycbcr_color_space() -> D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
    match STREAM_VIDEO_COLORIMETRY {
        // YCbCr_Matrix=1 selects BT.709; Nominal_Range=1 selects 16-235.
        VideoColorimetry::Bt709Limited => D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: (1 << 2) | (1 << 4),
        },
    }
}

pub(crate) fn decode_swap_chain_color_space() -> DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAGS {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => {
            DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAG_BT709
                | DXGI_MULTIPLANE_OVERLAY_YCbCr_FLAG_NOMINAL_RANGE
        }
    }
}

pub(crate) fn set_media_type_colorimetry(media_type: &IMFMediaType) -> windows::core::Result<()> {
    match STREAM_VIDEO_COLORIMETRY {
        VideoColorimetry::Bt709Limited => unsafe {
            media_type.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
            media_type.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
            media_type.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
            media_type.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{legacy_captured_rgb_color_space, legacy_encoded_ycbcr_color_space};

    #[test]
    fn legacy_video_processor_mapping_is_full_rgb_to_limited_bt709() {
        assert_eq!(legacy_captured_rgb_color_space()._bitfield, 0);
        assert_eq!(
            legacy_encoded_ycbcr_color_space()._bitfield,
            (1 << 2) | (1 << 4)
        );
    }
}
