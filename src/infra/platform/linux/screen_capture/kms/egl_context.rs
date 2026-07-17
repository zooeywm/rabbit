use std::{ffi::c_void, marker::PhantomData, rc::Rc};

use eros::Context as _;
use gbm::{AsRaw as _, Device};
use khronos_egl as egl;

const PLATFORM_GBM: egl::Enum = 0x31D7;

pub(crate) struct EglContext {
    instance: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    thread_affinity: PhantomData<Rc<()>>,
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

        let context = match create_current_context(&instance, display, version) {
            Ok(context) => context,
            Err(error) => {
                let _ = instance.terminate(display);
                return Err(error);
            }
        };

        Ok(Self {
            instance,
            display,
            context,
            thread_affinity: PhantomData,
        })
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

fn create_current_context(
    instance: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    version: (egl::Int, egl::Int),
) -> eros::Result<egl::Context> {
    if version < (1, 5) {
        let extensions = instance
            .query_string(Some(display), egl::EXTENSIONS)
            .with_context(|| "Failed to query EGL display extensions")?;

        if !has_extension(extensions, "EGL_KHR_surfaceless_context") {
            eros::bail!("EGL display does not support surfaceless contexts");
        }
    }

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

    Ok(context)
}

fn has_extension(extensions: &std::ffi::CStr, expected: &str) -> bool {
    extensions
        .to_bytes()
        .split(|byte| byte.is_ascii_whitespace())
        .any(|extension| extension == expected.as_bytes())
}
