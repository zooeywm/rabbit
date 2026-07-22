use std::collections::HashSet;

use drm::{
    buffer::{DrmModifier, Handle},
    control::{Device as _, PlaneType},
};

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
        screen_capture::kms::{
            output::KmsOutput,
            types::{
                KmsActivePlane, KmsFramebufferCacheKey, KmsFramebufferPlane,
                KmsFramebufferSnapshot, KmsPlaneCaptureError, KmsPlaneIssue, KmsPlaneSnapshot,
            },
        },
    },
    kernel::geometry::PixelSize,
};

const MAX_COHERENT_SNAPSHOT_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KmsFramebufferDescriptor {
    size: (u32, u32),
    format: drm::buffer::DrmFourcc,
    buffers: [Option<Handle>; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: DrmModifier,
}

#[derive(Debug)]
pub(super) struct KmsFramebufferCacheEntry {
    descriptor: KmsFramebufferDescriptor,
    handles: Vec<Handle>,
    buffer: DmaBufFrame,
    key: KmsFramebufferCacheKey,
}

impl KmsOutput {
    pub(crate) fn snapshot_framebuffers(&mut self) -> eros::Result<Option<KmsFramebufferSnapshot>> {
        for _ in 0..MAX_COHERENT_SNAPSHOT_ATTEMPTS {
            if let Some(snapshot) = self.try_snapshot_framebuffers()? {
                return Ok(Some(snapshot));
            }
        }

        tracing::trace!(
            attempts = MAX_COHERENT_SNAPSHOT_ATTEMPTS,
            "Skipped a KMS capture cycle because plane framebuffers kept changing"
        );
        Ok(None)
    }

    fn try_snapshot_framebuffers(&mut self) -> eros::Result<Option<KmsFramebufferSnapshot>> {
        let KmsPlaneSnapshot {
            output_size,
            frame_rate,
            planes: active_planes,
            mut issues,
        } = self.snapshot_planes()?;
        let mut planes = Vec::with_capacity(active_planes.len());
        let active_framebuffers = active_planes
            .iter()
            .map(|plane| plane.framebuffer)
            .collect::<HashSet<_>>();

        for active_plane in active_planes {
            match self.export_framebuffer(&active_plane) {
                Ok((buffer, cache_key)) => match self.validate_plane_snapshot(&active_plane) {
                    Ok(()) => planes.push(KmsFramebufferPlane {
                        id: active_plane.id,
                        plane_type: active_plane.plane_type,
                        buffer,
                        placement: active_plane.placement,
                        blend: active_plane.blend,
                        color: active_plane.color,
                        cursor_hotspot: active_plane.cursor_hotspot,
                        cache_key,
                    }),
                    Err(KmsPlaneCaptureError::SnapshotChanged { .. }) => return Ok(None),
                    Err(error) => issues.push(KmsPlaneIssue {
                        plane_id: active_plane.id,
                        plane_type: Some(active_plane.plane_type),
                        error,
                    }),
                },
                Err(errors) => {
                    if primary_framebuffer_handle_is_unavailable(active_plane.plane_type, &errors) {
                        eros::bail!(
                            "KMS GETFB2 did not expose framebuffer handles for primary plane {:?}; screen capture requires DRM master or CAP_SYS_ADMIN",
                            active_plane.id
                        );
                    }
                    issues.extend(errors.into_iter().map(|error| KmsPlaneIssue {
                        plane_id: active_plane.id,
                        plane_type: Some(active_plane.plane_type),
                        error,
                    }));
                }
            }
        }
        self.retain_active_framebuffers(&active_framebuffers);

        Ok(Some(KmsFramebufferSnapshot {
            output_size,
            frame_rate,
            planes,
            issues,
        }))
    }

    fn export_framebuffer(
        &mut self,
        active_plane: &KmsActivePlane,
    ) -> Result<(DmaBufFrame, KmsFramebufferCacheKey), Vec<KmsPlaneCaptureError>> {
        let framebuffer = self
            .device
            .get_planar_framebuffer(active_plane.framebuffer)
            .map_err(|error| vec![KmsPlaneCaptureError::QueryFramebuffer(error)])?;
        let descriptor = KmsFramebufferDescriptor {
            size: framebuffer.size(),
            format: framebuffer.pixel_format(),
            buffers: framebuffer.buffers(),
            pitches: framebuffer.pitches(),
            offsets: framebuffer.offsets(),
            modifier: framebuffer.modifier().unwrap_or(DrmModifier::Invalid),
        };
        if let Some(entry) = self.framebuffer_cache.get(&active_plane.framebuffer)
            && entry.descriptor == descriptor
        {
            return entry
                .buffer
                .try_clone()
                .map(|buffer| (buffer, entry.key))
                .map_err(|error| {
                    vec![KmsPlaneCaptureError::CloneCachedBuffer {
                        reason: format!("{error:?}"),
                    }]
                });
        }

        let handles = descriptor.buffers;
        let pitches = descriptor.pitches;
        let offsets = descriptor.offsets;
        let modifier = descriptor.modifier;
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
            let object_index = match unique_handles
                .iter()
                .position(|candidate| *candidate == handle)
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
            self.close_unreferenced_handles(&unique_handles, &mut errors);
            return Err(errors);
        }

        let cached = DmaBufFrame {
            size: PixelSize {
                width: descriptor.size.0,
                height: descriptor.size.1,
            },
            format: descriptor.format,
            objects,
            planes,
            readiness_fence: None,
            lease: None,
            va_backing: None,
        };
        let buffer = match cached.try_clone() {
            Ok(buffer) => buffer,
            Err(error) => {
                errors.push(KmsPlaneCaptureError::CloneCachedBuffer {
                    reason: format!("{error:?}"),
                });
                self.close_unreferenced_handles(&unique_handles, &mut errors);
                return Err(errors);
            }
        };
        let key = KmsFramebufferCacheKey(self.next_framebuffer_generation);
        self.next_framebuffer_generation = self.next_framebuffer_generation.wrapping_add(1);
        self.retain_handles(&unique_handles);
        let replaced = self.framebuffer_cache.insert(
            active_plane.framebuffer,
            KmsFramebufferCacheEntry {
                descriptor,
                handles: unique_handles,
                buffer: cached,
                key,
            },
        );
        if let Some(replaced) = replaced {
            self.release_handles(&replaced.handles, active_plane.framebuffer);
        }

        Ok((buffer, key))
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
                Ok(fd) => match DmaBufObject::try_from(fd) {
                    Ok(object) => objects.push(object),
                    Err(source) => errors.push(KmsPlaneCaptureError::QueryBufferSize {
                        object_index,
                        source,
                    }),
                },
                Err(source) => errors.push(KmsPlaneCaptureError::ExportBuffer {
                    object_index,
                    source,
                }),
            }
        }

        objects
    }

    fn retain_handles(&mut self, handles: &[Handle]) {
        for handle in handles {
            *self.framebuffer_handle_refs.entry(*handle).or_default() += 1;
        }
    }

    fn close_unreferenced_handles(
        &self,
        handles: &[Handle],
        errors: &mut Vec<KmsPlaneCaptureError>,
    ) {
        for (object_index, handle) in handles.iter().copied().enumerate() {
            if self.framebuffer_handle_refs.contains_key(&handle) {
                continue;
            }
            if let Err(source) = self.device.close_buffer(handle) {
                errors.push(KmsPlaneCaptureError::CloseBuffer {
                    object_index,
                    source,
                });
            }
        }
    }

    fn release_handles(
        &mut self,
        handles: &[Handle],
        framebuffer: drm::control::framebuffer::Handle,
    ) {
        for handle in handles {
            let Some(references) = self.framebuffer_handle_refs.get_mut(handle) else {
                continue;
            };
            *references -= 1;
            if *references != 0 {
                continue;
            }
            self.framebuffer_handle_refs.remove(handle);
            if let Err(error) = self.device.close_buffer(*handle) {
                tracing::warn!(
                    target: "rabbit::screen_capture::kms",
                    ?framebuffer,
                    ?handle,
                    ?error,
                    "Failed to close an evicted KMS framebuffer handle"
                );
            }
        }
    }

    fn retain_active_framebuffers(&mut self, active: &HashSet<drm::control::framebuffer::Handle>) {
        let stale = self
            .framebuffer_cache
            .keys()
            .filter(|framebuffer| !active.contains(framebuffer))
            .copied()
            .collect::<Vec<_>>();
        for framebuffer in stale {
            if let Some(entry) = self.framebuffer_cache.remove(&framebuffer) {
                self.release_handles(&entry.handles, framebuffer);
            }
        }
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

fn primary_framebuffer_handle_is_unavailable(
    plane_type: PlaneType,
    errors: &[KmsPlaneCaptureError],
) -> bool {
    plane_type == PlaneType::Primary
        && errors
            .iter()
            .any(|error| matches!(error, KmsPlaneCaptureError::MissingBufferHandle { .. }))
}

#[cfg(test)]
mod tests {
    use drm::control::PlaneType;

    use crate::infra::platform::screen_capture::kms::{
        framebuffer::primary_framebuffer_handle_is_unavailable, types::KmsPlaneCaptureError,
    };

    #[test]
    fn missing_primary_framebuffer_handle_is_fatal() {
        let missing_handle = [KmsPlaneCaptureError::MissingBufferHandle { plane_index: 0 }];

        assert!(primary_framebuffer_handle_is_unavailable(
            PlaneType::Primary,
            &missing_handle
        ));
        assert!(!primary_framebuffer_handle_is_unavailable(
            PlaneType::Overlay,
            &missing_handle
        ));
        assert!(!primary_framebuffer_handle_is_unavailable(
            PlaneType::Cursor,
            &missing_handle
        ));
    }

    #[test]
    fn other_primary_export_errors_remain_plane_issues() {
        let other_error = [KmsPlaneCaptureError::CloneCachedBuffer {
            reason: "test failure".to_owned(),
        }];

        assert!(!primary_framebuffer_handle_is_unavailable(
            PlaneType::Primary,
            &other_error
        ));
    }
}
