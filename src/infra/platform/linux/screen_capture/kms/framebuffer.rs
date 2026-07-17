use drm::{
    buffer::{DrmModifier, Handle},
    control::Device as _,
};

use crate::{
    infra::platform::screen_capture::kms::{
        output::KmsOutput,
        types::{
            DmaBufFrame, DmaBufObject, DmaBufPlane, KmsActivePlane,
            KmsFramebufferPlane, KmsFramebufferSnapshot, KmsPlaneCaptureError,
            KmsPlaneIssue, KmsPlaneSnapshot,
        },
    },
    kernel::geometry::PixelSize,
};

impl KmsOutput {
    pub(crate) fn snapshot_framebuffers(&self) -> eros::Result<KmsFramebufferSnapshot> {
        let KmsPlaneSnapshot {
            output_size,
            planes: active_planes,
            mut issues,
        } = self.snapshot_planes()?;
        let mut planes = Vec::with_capacity(active_planes.len());

        for active_plane in active_planes {
            match self.export_framebuffer(&active_plane) {
                Ok(buffer) => match self.validate_plane_snapshot(&active_plane) {
                    Ok(()) => planes.push(KmsFramebufferPlane {
                        id: active_plane.id,
                        plane_type: active_plane.plane_type,
                        buffer,
                        placement: active_plane.placement,
                        blend: active_plane.blend,
                        color: active_plane.color,
                        cursor_hotspot: active_plane.cursor_hotspot,
                    }),
                    Err(error) => issues.push(KmsPlaneIssue {
                        plane_id: active_plane.id,
                        plane_type: Some(active_plane.plane_type),
                        error,
                    }),
                },
                Err(errors) => issues.extend(errors.into_iter().map(|error| KmsPlaneIssue {
                    plane_id: active_plane.id,
                    plane_type: Some(active_plane.plane_type),
                    error,
                })),
            }
        }

        Ok(KmsFramebufferSnapshot {
            output_size,
            planes,
            issues,
        })
    }

    fn export_framebuffer(
        &self,
        active_plane: &KmsActivePlane,
    ) -> Result<DmaBufFrame, Vec<KmsPlaneCaptureError>> {
        let framebuffer = self
            .device
            .get_planar_framebuffer(active_plane.framebuffer)
            .map_err(|error| vec![KmsPlaneCaptureError::QueryFramebuffer(error)])?;
        let handles = framebuffer.buffers();
        let pitches = framebuffer.pitches();
        let offsets = framebuffer.offsets();
        let modifier = framebuffer.modifier().unwrap_or(DrmModifier::Invalid);
        let mut unique_handles = Vec::new();
        let mut planes = Vec::new();
        let mut errors = Vec::new();

        for plane_index in 0..handles.len() {
            let Some(handle) = handles[plane_index] else {
                if plane_index == 0 || pitches[plane_index] != 0 {
                    errors.push(KmsPlaneCaptureError::MissingBufferHandle { plane_index });
                }
                break;
            };
            let object_index = match unique_handles.iter().position(|candidate| *candidate == handle)
            {
                Some(index) => index,
                None => {
                    unique_handles.push(handle);
                    unique_handles.len() - 1
                }
            };

            planes.push(DmaBufPlane {
                object_index,
                offset: offsets[plane_index],
                stride: pitches[plane_index],
                modifier,
            });
        }

        let objects = self.export_objects(&unique_handles, &mut errors);

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(DmaBufFrame {
            size: PixelSize {
                width: framebuffer.size().0,
                height: framebuffer.size().1,
            },
            format: framebuffer.pixel_format(),
            objects,
            planes,
            readiness_fence: None,
        })
    }

    fn export_objects(
        &self,
        handles: &[Handle],
        errors: &mut Vec<KmsPlaneCaptureError>,
    ) -> Vec<DmaBufObject> {
        let mut objects = Vec::with_capacity(handles.len());

        for (object_index, handle) in handles.iter().copied().enumerate() {
            match self
                .device
                .buffer_to_prime_fd(handle, drm::CLOEXEC | drm::RDWR)
            {
                Ok(fd) => objects.push(DmaBufObject { fd }),
                Err(source) => errors.push(KmsPlaneCaptureError::ExportBuffer {
                    object_index,
                    source,
                }),
            }

            if let Err(source) = self.device.close_buffer(handle) {
                errors.push(KmsPlaneCaptureError::CloseBuffer {
                    object_index,
                    source,
                });
            }
        }

        objects
    }

    fn validate_plane_snapshot(
        &self,
        active_plane: &KmsActivePlane,
    ) -> Result<(), KmsPlaneCaptureError> {
        let current = self
            .device
            .get_plane(active_plane.id)
            .map_err(KmsPlaneCaptureError::QueryPlane)?;

        if current.crtc() != Some(self.crtc)
            || current.framebuffer() != Some(active_plane.framebuffer)
        {
            return Err(KmsPlaneCaptureError::SnapshotChanged {
                expected_crtc: self.crtc,
                actual_crtc: current.crtc(),
                expected_framebuffer: active_plane.framebuffer,
                actual_framebuffer: current.framebuffer(),
            });
        }

        Ok(())
    }
}
