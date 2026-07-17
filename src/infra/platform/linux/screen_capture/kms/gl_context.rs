use std::{ffi::c_void, marker::PhantomData, ptr, rc::Rc};

use eros::Context as _;
use glow::HasContext as _;
use khronos_egl as egl;

const TEXTURE_EXTERNAL: u32 = 0x8D65;

type ImageTargetTexture = unsafe extern "system" fn(u32, *const c_void);

pub(crate) struct GlContext {
    api: glow::Context,
    image_target_texture: ImageTargetTexture,
    thread_affinity: PhantomData<Rc<()>>,
}

#[derive(Debug)]
pub(crate) struct GlExternalTexture<'context> {
    owner: &'context GlContext,
    texture: glow::Texture,
}

impl std::fmt::Debug for GlContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GlContext")
            .field("version", self.api.version())
            .finish_non_exhaustive()
    }
}

impl GlContext {
    pub(crate) fn new(instance: &egl::DynamicInstance<egl::EGL1_5>) -> eros::Result<Self> {
        let api = unsafe {
            glow::Context::from_loader_function(|name| {
                instance
                    .get_proc_address(name)
                    .map(|function| function as *const () as *const c_void)
                    .unwrap_or(ptr::null())
            })
        };

        if !api
            .supported_extensions()
            .contains("GL_OES_EGL_image_external")
        {
            eros::bail!("OpenGL ES does not support external EGLImage textures");
        }

        let image_target_texture = instance
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .with_context(|| "OpenGL ES did not provide glEGLImageTargetTexture2DOES")?;
        let image_target_texture = unsafe {
            std::mem::transmute::<extern "system" fn(), ImageTargetTexture>(image_target_texture)
        };

        Ok(Self {
            api,
            image_target_texture,
            thread_affinity: PhantomData,
        })
    }

    pub(crate) fn create_external_texture(
        &self,
        image: egl::Image,
    ) -> eros::Result<GlExternalTexture<'_>> {
        let texture = match unsafe { self.api.create_texture() } {
            Ok(texture) => texture,
            Err(error) => eros::bail!("Failed to create an OpenGL ES texture: {error}"),
        };

        unsafe {
            self.api.bind_texture(TEXTURE_EXTERNAL, Some(texture));
            self.api.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.api.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.api.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.api.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            (self.image_target_texture)(TEXTURE_EXTERNAL, image.as_ptr());
            self.api.bind_texture(TEXTURE_EXTERNAL, None);
        }

        let error = unsafe { self.api.get_error() };
        if error != glow::NO_ERROR {
            unsafe { self.api.delete_texture(texture) };
            eros::bail!("Failed to bind EGLImage to an external texture: GL error 0x{error:04X}");
        }

        Ok(GlExternalTexture {
            owner: self,
            texture,
        })
    }
}

impl Drop for GlExternalTexture<'_> {
    fn drop(&mut self) {
        unsafe { self.owner.api.delete_texture(self.texture) };
    }
}
