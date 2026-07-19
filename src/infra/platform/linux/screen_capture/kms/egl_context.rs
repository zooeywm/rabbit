use std::{
    ffi::{CStr, c_void},
    marker::PhantomData,
    os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd},
    ptr,
    rc::Rc,
};

use drm::buffer::DrmModifier;
use eros::Context as _;
use gbm::{AsRaw as _, Device};
use khronos_egl as egl;

use crate::infra::platform::{
    dma_buf::DmaBufFrame,
    screen_capture::kms::{
        composition::KmsCompositionTransform,
        egl_ext::{
            DMA_BUF_PLANE_FD_EXT, DMA_BUF_PLANE_MODIFIER_HI_EXT, DMA_BUF_PLANE_MODIFIER_LO_EXT,
            DMA_BUF_PLANE_OFFSET_EXT, DMA_BUF_PLANE_PITCH_EXT, DupNativeFenceFdAndroid,
            ITU_REC601_EXT, ITU_REC709_EXT, ITU_REC2020_EXT, LINUX_DMA_BUF_EXT,
            LINUX_DRM_FOURCC_EXT, NO_NATIVE_FENCE_FD_ANDROID, PLATFORM_GBM_KHR,
            SAMPLE_RANGE_HINT_EXT, SYNC_NATIVE_FENCE_ANDROID, SYNC_NATIVE_FENCE_FD_ANDROID,
            YUV_COLOR_SPACE_HINT_EXT, YUV_FULL_RANGE_EXT, YUV_NARROW_RANGE_EXT,
        },
        gl_context::{GlCompositionTarget, GlContext, GlExternalTexture},
        types::{
            KmsColorEncoding, KmsColorRange, KmsFramebufferPlane, KmsPlaneBlend,
            KmsPlaneCaptureError, KmsPlaneColor,
        },
    },
};

pub(crate) struct EglContext {
    instance: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    gl: GlContext,
    dup_native_fence_fd: DupNativeFenceFdAndroid,
    supports_modifiers: bool,
    thread_affinity: PhantomData<Rc<()>>,
}

#[derive(Debug)]
struct EglImage<'context> {
    owner: &'context EglContext,
    image: egl::Image,
    size: crate::kernel::geometry::PixelSize,
}

#[derive(Debug)]
pub(crate) struct EglPlaneImage<'context>(EglImage<'context>);

#[derive(Debug)]
pub(crate) struct EglCompositionImage<'context>(EglImage<'context>);

#[derive(Debug)]
pub(crate) struct EglDmaBufImage<'context>(EglImage<'context>);

impl std::fmt::Debug for EglContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EglContext")
            .field("display", &self.display)
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

impl EglContext {
    pub(crate) fn new<T: std::os::fd::AsFd>(device: &Device<T>) -> eros::Result<Self> {
        let instance = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .with_context(|| "Failed to load EGL 1.5")?;
        let client_extensions = instance
            .query_string(None, egl::EXTENSIONS)
            .with_context(|| "Failed to query EGL client extensions")?;

        if !has_extension(client_extensions, "EGL_KHR_platform_gbm")
            && !has_extension(client_extensions, "EGL_MESA_platform_gbm")
        {
            eros::bail!("EGL does not support the GBM platform");
        }

        let native_display = device.as_raw_mut().cast::<c_void>();
        let display = unsafe {
            instance.get_platform_display(PLATFORM_GBM_KHR, native_display, &[egl::ATTRIB_NONE])
        }
        .with_context(|| "Failed to create an EGL display from the GBM device")?;
        let version = instance
            .initialize(display)
            .with_context(|| "Failed to initialize the GBM EGL display")?;

        let (context, gl, dup_native_fence_fd, supports_modifiers) =
            match initialize_context(&instance, display, version) {
                Ok(initialized) => initialized,
                Err(error) => {
                    let _ = instance.terminate(display);
                    return Err(error);
                }
            };

        Ok(Self {
            instance,
            display,
            context,
            gl,
            dup_native_fence_fd,
            supports_modifiers,
            thread_affinity: PhantomData,
        })
    }

    pub(crate) fn import_plane<'context>(
        &'context self,
        plane: &KmsFramebufferPlane,
    ) -> Result<EglPlaneImage<'context>, KmsPlaneCaptureError> {
        Ok(EglPlaneImage(
            self.import_dma_buf(&plane.buffer, Some(plane.color))?,
        ))
    }

    pub(crate) fn import_composition_target<'context>(
        &'context self,
        frame: &DmaBufFrame,
    ) -> eros::Result<EglCompositionImage<'context>> {
        Ok(EglCompositionImage(
            self.import_dma_buf(frame, None)
                .with_context(|| "Failed to import the KMS composition target")?,
        ))
    }

    pub(crate) fn import_dma_buf_frame<'context>(
        &'context self,
        frame: &DmaBufFrame,
    ) -> eros::Result<EglDmaBufImage<'context>> {
        Ok(EglDmaBufImage(
            self.import_dma_buf(frame, None)
                .with_context(|| "Failed to import DMA-BUF frame into EGL")?,
        ))
    }

    pub(crate) fn wait_on_native_fence(&self, fence: OwnedFd) -> eros::Result<()> {
        let raw_fd = fence.as_raw_fd();
        let sync = unsafe {
            self.instance.create_sync(
                self.display,
                SYNC_NATIVE_FENCE_ANDROID,
                &[
                    SYNC_NATIVE_FENCE_FD_ANDROID,
                    raw_fd as egl::Attrib,
                    egl::ATTRIB_NONE,
                ],
            )
        }
        .with_context(|| "Failed to import DMA-BUF readiness fence into EGL")?;

        std::mem::forget(fence);

        if let Err(error) = self.instance.wait_sync(self.display, sync, 0) {
            let _ = unsafe { self.instance.destroy_sync(self.display, sync) };
            return Ok(
                Err(error).with_context(|| "Failed to enqueue the DMA-BUF readiness fence wait")?
            );
        }

        unsafe { self.instance.destroy_sync(self.display, sync) }
            .with_context(|| "Failed to destroy the imported DMA-BUF readiness fence")?;

        Ok(())
    }

    fn import_dma_buf<'context>(
        &'context self,
        frame: &DmaBufFrame,
        color: Option<KmsPlaneColor>,
    ) -> Result<EglImage<'context>, KmsPlaneCaptureError> {
        if frame.planes.is_empty() {
            return Err(KmsPlaneCaptureError::MissingDmaBufPlanes);
        }
        if frame.planes.len() > DMA_BUF_PLANE_FD_EXT.len() {
            return Err(KmsPlaneCaptureError::TooManyDmaBufPlanes {
                count: frame.planes.len(),
                maximum: DMA_BUF_PLANE_FD_EXT.len(),
            });
        }

        let mut attributes = vec![
            egl::WIDTH as egl::Attrib,
            frame.size.width as egl::Attrib,
            egl::HEIGHT as egl::Attrib,
            frame.size.height as egl::Attrib,
            LINUX_DRM_FOURCC_EXT,
            frame.format as u32 as egl::Attrib,
        ];

        if let Some(color) = color {
            attributes.extend_from_slice(&[
                YUV_COLOR_SPACE_HINT_EXT,
                color_space_hint(color.encoding),
                SAMPLE_RANGE_HINT_EXT,
                sample_range_hint(color.range),
            ]);
        }

        for (plane_index, plane) in frame.planes.iter().enumerate() {
            let object = frame.objects.get(plane.object_index).ok_or(
                KmsPlaneCaptureError::MissingDmaBufObject {
                    plane_index,
                    object_index: plane.object_index,
                },
            )?;

            attributes.extend_from_slice(&[
                DMA_BUF_PLANE_FD_EXT[plane_index],
                object.fd.as_raw_fd() as egl::Attrib,
                DMA_BUF_PLANE_OFFSET_EXT[plane_index],
                plane.offset as egl::Attrib,
                DMA_BUF_PLANE_PITCH_EXT[plane_index],
                plane.stride as egl::Attrib,
            ]);

            if plane.modifier != DrmModifier::Invalid {
                if !self.supports_modifiers {
                    return Err(KmsPlaneCaptureError::UnsupportedFormat {
                        format: frame.format,
                        modifier: plane.modifier,
                    });
                }

                let modifier: u64 = plane.modifier.into();
                attributes.extend_from_slice(&[
                    DMA_BUF_PLANE_MODIFIER_LO_EXT[plane_index],
                    (modifier as u32) as egl::Attrib,
                    DMA_BUF_PLANE_MODIFIER_HI_EXT[plane_index],
                    ((modifier >> 32) as u32) as egl::Attrib,
                ]);
            }
        }

        attributes.push(egl::ATTRIB_NONE);
        let no_context = unsafe { egl::Context::from_ptr(ptr::null_mut()) };
        let no_buffer = unsafe { egl::ClientBuffer::from_ptr(ptr::null_mut()) };
        let modifier = frame.planes[0].modifier;
        let image = self
            .instance
            .create_image(
                self.display,
                no_context,
                LINUX_DMA_BUF_EXT,
                no_buffer,
                &attributes,
            )
            .map_err(|source| KmsPlaneCaptureError::ImportDmaBuf {
                format: frame.format,
                modifier,
                source,
            })?;

        Ok(EglImage {
            owner: self,
            image,
            size: frame.size,
        })
    }

    pub(crate) fn create_external_texture<'context>(
        &'context self,
        image: &EglPlaneImage<'_>,
    ) -> eros::Result<GlExternalTexture<'context>> {
        if !ptr::eq(self, image.0.owner) {
            eros::bail!("Cannot bind an EGLImage created by another EGL context");
        }

        self.gl.create_external_texture(image.0.image)
    }

    pub(crate) fn create_composition_target<'context>(
        &'context self,
        image: &EglCompositionImage<'_>,
    ) -> eros::Result<GlCompositionTarget<'context>> {
        if !ptr::eq(self, image.0.owner) {
            eros::bail!("Cannot use an EGLImage from another EGL context as a composition target");
        }

        self.gl
            .create_composition_target(image.0.image, image.0.size)
    }

    pub(crate) fn clear_composition_target(
        &self,
        target: &GlCompositionTarget<'_>,
    ) -> eros::Result<()> {
        self.gl.clear_composition_target(target)
    }

    pub(crate) fn compose_plane(
        &self,
        target: &GlCompositionTarget<'_>,
        texture: &GlExternalTexture<'_>,
        transform: &KmsCompositionTransform,
        blend: KmsPlaneBlend,
    ) -> eros::Result<()> {
        self.gl.compose_plane(target, texture, transform, blend)
    }

    pub(crate) fn finish_composition(&self) -> eros::Result<OwnedFd> {
        let sync = unsafe {
            self.instance
                .create_sync(self.display, SYNC_NATIVE_FENCE_ANDROID, &[egl::ATTRIB_NONE])
                .with_context(|| "Failed to create an EGL native fence for KMS composition")?
        };

        if let Err(error) = self.gl.flush_composition() {
            let _ = unsafe { self.instance.destroy_sync(self.display, sync) };
            return Err(error);
        }

        let raw_fd = unsafe { (self.dup_native_fence_fd)(self.display.as_ptr(), sync.as_ptr()) };
        if raw_fd == NO_NATIVE_FENCE_FD_ANDROID {
            let source = self.instance.get_error();
            let _ = unsafe { self.instance.destroy_sync(self.display, sync) };
            return match source {
                Some(source) => Ok(Err(source)
                    .with_context(|| "Failed to export the KMS composition readiness fence")?),
                None => eros::bail!("Failed to export the KMS composition readiness fence"),
            };
        }

        let fence = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        unsafe { self.instance.destroy_sync(self.display, sync) }
            .with_context(|| "Failed to destroy the exported EGL native fence")?;

        Ok(fence)
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        if self
            .instance
            .make_current(self.display, None, None, Some(self.context))
            .is_ok()
        {
            self.gl.destroy();
        }
        let _ = self.instance.make_current(self.display, None, None, None);
        let _ = self.instance.destroy_context(self.display, self.context);
        let _ = self.instance.terminate(self.display);
        let _ = self.instance.release_thread();
    }
}

impl Drop for EglImage<'_> {
    fn drop(&mut self) {
        let _ = self
            .owner
            .instance
            .destroy_image(self.owner.display, self.image);
    }
}

fn initialize_context(
    instance: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    version: (egl::Int, egl::Int),
) -> eros::Result<(egl::Context, GlContext, DupNativeFenceFdAndroid, bool)> {
    let extensions = instance
        .query_string(Some(display), egl::EXTENSIONS)
        .with_context(|| "Failed to query EGL display extensions")?;

    if version < (1, 5) && !has_extension(extensions, "EGL_KHR_surfaceless_context") {
        eros::bail!("EGL display does not support surfaceless contexts");
    }
    if !has_extension(extensions, "EGL_EXT_image_dma_buf_import") {
        eros::bail!("EGL display does not support DMA-BUF image import");
    }
    if !has_extension(extensions, "EGL_ANDROID_native_fence_sync") {
        eros::bail!("EGL display does not support native fence synchronization");
    }

    let dup_native_fence_fd = instance
        .get_proc_address("eglDupNativeFenceFDANDROID")
        .with_context(|| "EGL did not provide eglDupNativeFenceFDANDROID")?;
    let dup_native_fence_fd = unsafe {
        std::mem::transmute::<extern "system" fn(), DupNativeFenceFdAndroid>(dup_native_fence_fd)
    };

    let supports_modifiers = has_extension(extensions, "EGL_EXT_image_dma_buf_import_modifiers");

    instance
        .bind_api(egl::OPENGL_ES_API)
        .with_context(|| "Failed to bind the OpenGL ES API")?;
    let config = instance
        .choose_first_config(
            display,
            &[egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT, egl::NONE],
        )
        .with_context(|| "Failed to choose an OpenGL ES 3 EGL config")?
        .with_context(|| "EGL did not provide an OpenGL ES 3 config")?;
    let context = instance
        .create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
        )
        .with_context(|| "Failed to create an OpenGL ES 3 context")?;

    if let Err(error) = instance.make_current(display, None, None, Some(context)) {
        let _ = instance.destroy_context(display, context);
        return Ok(Err(error).with_context(|| "Failed to make the EGL context current")?);
    }

    let gl = match GlContext::new(instance) {
        Ok(gl) => gl,
        Err(error) => {
            let _ = instance.make_current(display, None, None, None);
            let _ = instance.destroy_context(display, context);
            return Err(error);
        }
    };

    Ok((context, gl, dup_native_fence_fd, supports_modifiers))
}

fn has_extension(extensions: &CStr, expected: &str) -> bool {
    extensions
        .to_bytes()
        .split(|byte| byte.is_ascii_whitespace())
        .any(|extension| extension == expected.as_bytes())
}

fn color_space_hint(encoding: KmsColorEncoding) -> egl::Attrib {
    match encoding {
        KmsColorEncoding::Bt601 => ITU_REC601_EXT,
        KmsColorEncoding::Bt709 => ITU_REC709_EXT,
        KmsColorEncoding::Bt2020 => ITU_REC2020_EXT,
    }
}

fn sample_range_hint(range: KmsColorRange) -> egl::Attrib {
    match range {
        KmsColorRange::Limited => YUV_NARROW_RANGE_EXT,
        KmsColorRange::Full => YUV_FULL_RANGE_EXT,
    }
}
