use std::{
    ffi::{CStr, c_void},
    marker::PhantomData,
    os::fd::AsRawFd as _,
    ptr,
    rc::Rc,
};

use drm::buffer::DrmModifier;
use eros::Context as _;
use gbm::{AsRaw as _, Device};
use khronos_egl as egl;

const PLATFORM_GBM: egl::Enum = 0x31D7;
const LINUX_DMA_BUF: egl::Enum = 0x3270;
const LINUX_DRM_FOURCC: egl::Attrib = 0x3271;
const PLANE_FD: [egl::Attrib; 4] = [0x3272, 0x3275, 0x3278, 0x3440];
const PLANE_OFFSET: [egl::Attrib; 4] = [0x3273, 0x3276, 0x3279, 0x3441];
const PLANE_PITCH: [egl::Attrib; 4] = [0x3274, 0x3277, 0x327A, 0x3442];
const PLANE_MODIFIER_LOW: [egl::Attrib; 4] = [0x3443, 0x3445, 0x3447, 0x3449];
const PLANE_MODIFIER_HIGH: [egl::Attrib; 4] = [0x3444, 0x3446, 0x3448, 0x344A];

use crate::infra::platform::screen_capture::kms::{
    gl_context::{GlCompositionTarget, GlContext, GlExternalTexture},
    types::DmaBufFrame,
};

pub(crate) struct EglContext {
    instance: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    gl: GlContext,
    supports_modifiers: bool,
    thread_affinity: PhantomData<Rc<()>>,
}

#[derive(Debug)]
pub(crate) struct EglImage<'context> {
    owner: &'context EglContext,
    image: egl::Image,
    size: crate::kernel::geometry::PixelSize,
}

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
            instance.get_platform_display(PLATFORM_GBM, native_display, &[egl::ATTRIB_NONE])
        }
        .with_context(|| "Failed to create an EGL display from the GBM device")?;
        let version = instance
            .initialize(display)
            .with_context(|| "Failed to initialize the GBM EGL display")?;

        let (context, gl, supports_modifiers) =
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
            supports_modifiers,
            thread_affinity: PhantomData,
        })
    }

    pub(crate) fn import_dma_buf<'context>(
        &'context self,
        frame: &DmaBufFrame,
    ) -> eros::Result<EglImage<'context>> {
        if frame.planes.is_empty() {
            eros::bail!("Cannot import a DMA-BUF frame without planes");
        }
        if frame.planes.len() > PLANE_FD.len() {
            eros::bail!(
                "Cannot import a DMA-BUF frame with {} planes; EGL supports at most {}",
                frame.planes.len(),
                PLANE_FD.len()
            );
        }

        let mut attributes = vec![
            egl::WIDTH as egl::Attrib,
            frame.size.width as egl::Attrib,
            egl::HEIGHT as egl::Attrib,
            frame.size.height as egl::Attrib,
            LINUX_DRM_FOURCC,
            frame.format as u32 as egl::Attrib,
        ];

        for (plane_index, plane) in frame.planes.iter().enumerate() {
            let object = frame.objects.get(plane.object_index).with_context(|| {
                format!(
                    "DMA-BUF plane {plane_index} references missing object {}",
                    plane.object_index
                )
            })?;

            attributes.extend_from_slice(&[
                PLANE_FD[plane_index],
                object.fd.as_raw_fd() as egl::Attrib,
                PLANE_OFFSET[plane_index],
                plane.offset as egl::Attrib,
                PLANE_PITCH[plane_index],
                plane.stride as egl::Attrib,
            ]);

            if plane.modifier != DrmModifier::Invalid {
                if !self.supports_modifiers {
                    eros::bail!(
                        "EGL cannot import DMA-BUF plane {plane_index} modifier {:?}",
                        plane.modifier
                    );
                }

                let modifier: u64 = plane.modifier.into();
                attributes.extend_from_slice(&[
                    PLANE_MODIFIER_LOW[plane_index],
                    (modifier as u32) as egl::Attrib,
                    PLANE_MODIFIER_HIGH[plane_index],
                    ((modifier >> 32) as u32) as egl::Attrib,
                ]);
            }
        }

        attributes.push(egl::ATTRIB_NONE);
        let no_context = unsafe { egl::Context::from_ptr(ptr::null_mut()) };
        let no_buffer = unsafe { egl::ClientBuffer::from_ptr(ptr::null_mut()) };
        let image = self
            .instance
            .create_image(
                self.display,
                no_context,
                LINUX_DMA_BUF,
                no_buffer,
                &attributes,
            )
            .with_context(|| {
                format!(
                    "Failed to import {}x{} {:?} DMA-BUF as an EGLImage",
                    frame.size.width, frame.size.height, frame.format
                )
            })?;

        Ok(EglImage {
            owner: self,
            image,
            size: frame.size,
        })
    }

    pub(crate) fn create_external_texture<'context>(
        &'context self,
        image: &EglImage<'_>,
    ) -> eros::Result<GlExternalTexture<'context>> {
        if !ptr::eq(self, image.owner) {
            eros::bail!("Cannot bind an EGLImage created by another EGL context");
        }

        self.gl.create_external_texture(image.image)
    }

    pub(crate) fn create_composition_target<'context>(
        &'context self,
        image: &EglImage<'_>,
    ) -> eros::Result<GlCompositionTarget<'context>> {
        if !ptr::eq(self, image.owner) {
            eros::bail!("Cannot render to an EGLImage created by another EGL context");
        }

        self.gl.create_composition_target(image.image, image.size)
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        let _ = self
            .instance
            .make_current(self.display, None, None, None);
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
) -> eros::Result<(egl::Context, GlContext, bool)> {
    let extensions = instance
        .query_string(Some(display), egl::EXTENSIONS)
        .with_context(|| "Failed to query EGL display extensions")?;

    if version < (1, 5) && !has_extension(extensions, "EGL_KHR_surfaceless_context") {
        eros::bail!("EGL display does not support surfaceless contexts");
    }
    if !has_extension(extensions, "EGL_EXT_image_dma_buf_import") {
        eros::bail!("EGL display does not support DMA-BUF image import");
    }

    let supports_modifiers =
        has_extension(extensions, "EGL_EXT_image_dma_buf_import_modifiers");

    instance
        .bind_api(egl::OPENGL_ES_API)
        .with_context(|| "Failed to bind the OpenGL ES API")?;
    let config = instance
        .choose_first_config(
            display,
            &[
                egl::RENDERABLE_TYPE,
                egl::OPENGL_ES3_BIT,
                egl::NONE,
            ],
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

    Ok((context, gl, supports_modifiers))
}

fn has_extension(extensions: &CStr, expected: &str) -> bool {
    extensions
        .to_bytes()
        .split(|byte| byte.is_ascii_whitespace())
        .any(|extension| extension == expected.as_bytes())
}
