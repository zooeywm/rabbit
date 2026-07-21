use std::{
    ffi::{CStr, c_void},
    marker::PhantomData,
    os::fd::AsRawFd as _,
    ptr,
    rc::Rc,
};

use drm::buffer::DrmModifier;
use eros::Context as _;
use glow::HasContext as _;
use khronos_egl as egl;

use crate::{
    infra::platform::{
        client_video_probe::ClientVideoProbeReporter,
        egl_dma_buf::{
            DMA_BUF_PLANE_FD_EXT, DMA_BUF_PLANE_MODIFIER_HI_EXT, DMA_BUF_PLANE_MODIFIER_LO_EXT,
            DMA_BUF_PLANE_OFFSET_EXT, DMA_BUF_PLANE_PITCH_EXT, ITU_REC709_EXT, LINUX_DMA_BUF_EXT,
            LINUX_DRM_FOURCC_EXT, SAMPLE_RANGE_HINT_EXT, YUV_COLOR_SPACE_HINT_EXT,
            YUV_NARROW_RANGE_EXT,
        },
        video_decoder::GStreamerDecodedFrame,
    },
    kernel::video_renderer::{VideoRenderer, VideoViewport},
};

const TEXTURE_EXTERNAL: u32 = 0x8D65;
const TEXTURE_BINDING_EXTERNAL: u32 = 0x8D67;

type ImageTargetTexture = unsafe extern "system" fn(u32, *const c_void);

const VERTEX_SHADER: &str = r#"#version 300 es
const vec2 positions[4] = vec2[4](
    vec2(-1.0, -1.0),
    vec2( 1.0, -1.0),
    vec2(-1.0,  1.0),
    vec2( 1.0,  1.0)
);
const vec2 texture_coordinates[4] = vec2[4](
    vec2(0.0, 1.0),
    vec2(1.0, 1.0),
    vec2(0.0, 0.0),
    vec2(1.0, 0.0)
);

out vec2 sampled_coordinate;

void main() {
    gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
    sampled_coordinate = texture_coordinates[gl_VertexID];
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision highp float;

uniform samplerExternalOES video_texture;
in vec2 sampled_coordinate;
out vec4 output_color;

void main() {
    output_color = vec4(texture(video_texture, sampled_coordinate).rgb, 1.0);
}
"#;

pub(crate) struct OpenGlVideoRenderer {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    gl: glow::Context,
    image_target_texture: ImageTargetTexture,
    program: glow::Program,
    texture_uniform: glow::UniformLocation,
    vertex_array: glow::VertexArray,
    viewport: Option<VideoViewport>,
    pending_frame: Option<GStreamerDecodedFrame>,
    current_frame: Option<ImportedFrame>,
    probe_reporter: ClientVideoProbeReporter,
    thread_affinity: PhantomData<Rc<()>>,
}

struct ImportedFrame {
    frame: GStreamerDecodedFrame,
    image: egl::Image,
    texture: glow::Texture,
}

struct GlState {
    program: Option<glow::Program>,
    vertex_array: Option<glow::VertexArray>,
    active_texture: i32,
    external_texture: Option<glow::Texture>,
    viewport: [i32; 4],
    blend: bool,
    cull_face: bool,
    depth_test: bool,
    scissor_test: bool,
}

impl OpenGlVideoRenderer {
    pub(crate) fn new(get_proc_address: &dyn Fn(&CStr) -> *const c_void) -> eros::Result<Self> {
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .with_context(|| "Failed to load EGL 1.5 for video rendering")?;
        let display = egl
            .get_current_display()
            .with_context(|| "Slint OpenGL context has no current EGL display")?;
        let extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .with_context(|| "Failed to query Slint EGL display extensions")?;
        if !has_extension(extensions, "EGL_EXT_image_dma_buf_import") {
            eros::bail!("Slint EGL display does not support DMA-BUF import");
        }

        let image_target_texture = get_proc_address(c"glEGLImageTargetTexture2DOES");
        if image_target_texture.is_null() {
            eros::bail!("Slint OpenGL context did not provide glEGLImageTargetTexture2DOES");
        }
        let image_target_texture = unsafe {
            std::mem::transmute::<*const c_void, ImageTargetTexture>(image_target_texture)
        };
        let gl = unsafe { glow::Context::from_loader_function_cstr(get_proc_address) };
        for extension in [
            "GL_OES_EGL_image",
            "GL_OES_EGL_image_external",
            "GL_OES_EGL_image_external_essl3",
        ] {
            if !gl.supported_extensions().contains(extension) {
                eros::bail!("Slint OpenGL context does not support {}", extension);
            }
        }
        let program = create_program(&gl)?;
        let texture_uniform = match unsafe { gl.get_uniform_location(program, "video_texture") } {
            Some(uniform) => uniform,
            None => {
                unsafe { gl.delete_program(program) };
                eros::bail!("Video renderer program does not expose video_texture");
            }
        };
        let vertex_array = match unsafe { gl.create_vertex_array() } {
            Ok(vertex_array) => vertex_array,
            Err(error) => {
                unsafe { gl.delete_program(program) };
                eros::bail!("Failed to create video renderer vertex array: {}", error);
            }
        };

        Ok(Self {
            egl,
            display,
            gl,
            image_target_texture,
            program,
            texture_uniform,
            vertex_array,
            viewport: None,
            pending_frame: None,
            current_frame: None,
            probe_reporter: ClientVideoProbeReporter::default(),
            thread_affinity: PhantomData,
        })
    }

    pub(crate) fn teardown(&mut self) -> eros::Result<()> {
        self.clear()?;
        unsafe {
            self.gl.delete_vertex_array(self.vertex_array);
            self.gl.delete_program(self.program);
        }
        Ok(())
    }

    fn import_pending_frame(&mut self) -> eros::Result<()> {
        let Some(mut frame) = self.pending_frame.take() else {
            return Ok(());
        };
        self.release_current()?;
        if let Some(probe) = &mut frame.probe {
            probe.mark_dma_buf_import_started();
        }
        let image = self.import_dma_buf(&frame)?;
        let texture = match self.create_external_texture(image) {
            Ok(texture) => texture,
            Err(error) => {
                return match self.egl.destroy_image(self.display, image) {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => eros::bail!(
                        "Failed to create video texture: {}; additionally failed to destroy its EGLImage: {}",
                        error,
                        cleanup_error
                    ),
                };
            }
        };
        if let Some(probe) = &mut frame.probe {
            probe.mark_dma_buf_import_completed();
        }
        self.current_frame = Some(ImportedFrame {
            frame,
            image,
            texture,
        });
        Ok(())
    }

    fn import_dma_buf(&self, frame: &GStreamerDecodedFrame) -> eros::Result<egl::Image> {
        let buffer = &frame.buffer;
        if buffer.planes.is_empty() || buffer.planes.len() > DMA_BUF_PLANE_FD_EXT.len() {
            eros::bail!(
                "Video DMA-BUF contains unsupported plane count {}",
                buffer.planes.len()
            );
        }
        let mut attributes = vec![
            egl::WIDTH as egl::Attrib,
            buffer.size.width as egl::Attrib,
            egl::HEIGHT as egl::Attrib,
            buffer.size.height as egl::Attrib,
            LINUX_DRM_FOURCC_EXT,
            buffer.format as u32 as egl::Attrib,
            YUV_COLOR_SPACE_HINT_EXT,
            ITU_REC709_EXT,
            SAMPLE_RANGE_HINT_EXT,
            YUV_NARROW_RANGE_EXT,
        ];
        let extensions = self
            .egl
            .query_string(Some(self.display), egl::EXTENSIONS)
            .with_context(|| "Failed to query EGL extensions before video DMA-BUF import")?;

        for (plane_index, plane) in buffer.planes.iter().enumerate() {
            let object = buffer.objects.get(plane.object_index).with_context(|| {
                format!(
                    "Video DMA-BUF plane {} references missing object {}",
                    plane_index, plane.object_index
                )
            })?;
            attributes.extend_from_slice(&[
                DMA_BUF_PLANE_FD_EXT[plane_index],
                object.fd.as_raw_fd() as egl::Attrib,
                DMA_BUF_PLANE_OFFSET_EXT[plane_index],
                plane.offset as egl::Attrib,
                DMA_BUF_PLANE_PITCH_EXT[plane_index],
                plane.stride as egl::Attrib,
            ]);
            if plane.modifier != DrmModifier::Invalid {
                if !has_extension(extensions, "EGL_EXT_image_dma_buf_import_modifiers") {
                    eros::bail!(
                        "Video DMA-BUF uses modifier {:?}, but Slint EGL display does not support modifier import",
                        plane.modifier
                    );
                }
                let modifier: u64 = plane.modifier.into();
                attributes.extend_from_slice(&[
                    DMA_BUF_PLANE_MODIFIER_LO_EXT[plane_index],
                    modifier as u32 as egl::Attrib,
                    DMA_BUF_PLANE_MODIFIER_HI_EXT[plane_index],
                    (modifier >> 32) as u32 as egl::Attrib,
                ]);
            }
        }
        attributes.push(egl::ATTRIB_NONE);
        let no_context = unsafe { egl::Context::from_ptr(ptr::null_mut()) };
        let no_buffer = unsafe { egl::ClientBuffer::from_ptr(ptr::null_mut()) };

        Ok(self
            .egl
            .create_image(
                self.display,
                no_context,
                LINUX_DMA_BUF_EXT,
                no_buffer,
                &attributes,
            )
            .with_context(|| {
                format!(
                    "Failed to import decoded {:?} DMA-BUF into Slint EGL context",
                    buffer.format
                )
            })?)
    }

    fn create_external_texture(&self, image: egl::Image) -> eros::Result<glow::Texture> {
        let active_texture = unsafe { self.gl.get_parameter_i32(glow::ACTIVE_TEXTURE) };
        unsafe { self.gl.active_texture(glow::TEXTURE0) };
        let previous_texture = unsafe { self.gl.get_parameter_texture(TEXTURE_BINDING_EXTERNAL) };
        let texture = match unsafe { self.gl.create_texture() } {
            Ok(texture) => texture,
            Err(error) => {
                unsafe { self.gl.active_texture(active_texture as u32) };
                eros::bail!("Failed to create video texture: {}", error)
            }
        };
        unsafe {
            self.gl.bind_texture(TEXTURE_EXTERNAL, Some(texture));
            self.gl.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                TEXTURE_EXTERNAL,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            (self.image_target_texture)(TEXTURE_EXTERNAL, image.as_ptr());
            self.gl.bind_texture(TEXTURE_EXTERNAL, previous_texture);
            self.gl.active_texture(active_texture as u32);
        }
        let error = unsafe { self.gl.get_error() };
        if error != glow::NO_ERROR {
            unsafe { self.gl.delete_texture(texture) };
            eros::bail!(
                "Failed to bind video EGLImage to external texture: GL error 0x{:04X}",
                error
            );
        }
        Ok(texture)
    }

    fn release_current(&mut self) -> eros::Result<()> {
        let Some(current) = self.current_frame.take() else {
            return Ok(());
        };
        unsafe { self.gl.delete_texture(current.texture) };
        self.egl
            .destroy_image(self.display, current.image)
            .with_context(|| "Failed to destroy video EGLImage")?;
        drop(current.frame);
        Ok(())
    }

    fn draw_current(&mut self) -> eros::Result<()> {
        let (Some(viewport), Some(current)) = (self.viewport, &self.current_frame) else {
            return Ok(());
        };
        if viewport.width == 0 || viewport.height == 0 {
            return Ok(());
        }
        let state = GlState::capture(&self.gl);
        let fitted = fit_viewport(viewport, current.frame.buffer.size)?;
        let window_height = state.viewport[3];
        let x = i32::try_from(fitted.x).with_context(|| "Video viewport x exceeds i32")?;
        let top = fitted
            .y
            .checked_add(fitted.height)
            .with_context(|| "Video viewport vertical extent overflows u32")?;
        let y = window_height
            .checked_sub(i32::try_from(top).with_context(|| "Video viewport y exceeds i32")?)
            .with_context(|| "Video viewport lies outside the window framebuffer")?;
        let width = i32::try_from(fitted.width).with_context(|| "Video width exceeds i32")?;
        let height = i32::try_from(fitted.height).with_context(|| "Video height exceeds i32")?;
        let texture = current.texture;

        if let Some(probe) = self
            .current_frame
            .as_mut()
            .and_then(|current| current.frame.probe.as_mut())
        {
            probe.mark_render_started();
        }

        unsafe {
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::CULL_FACE);
            self.gl.disable(glow::DEPTH_TEST);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.viewport(x, y, width, height);
            self.gl.use_program(Some(self.program));
            self.gl.bind_vertex_array(Some(self.vertex_array));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(TEXTURE_EXTERNAL, Some(texture));
            self.gl.uniform_1_i32(Some(&self.texture_uniform), 0);
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        let error = unsafe { self.gl.get_error() };
        state.restore(&self.gl);
        if error != glow::NO_ERROR {
            eros::bail!("Failed to draw video DMA-BUF: GL error 0x{:04X}", error);
        }

        let Some(current) = self.current_frame.as_mut() else {
            eros::bail!("Current video frame disappeared while rendering");
        };
        let screen_id = current.frame.screen_id;
        if let Some(mut probe) = current.frame.probe.take() {
            probe.mark_render_completed();
            self.probe_reporter.record_frame(screen_id, probe);
        }
        Ok(())
    }
}

impl VideoRenderer for OpenGlVideoRenderer {
    type Frame = GStreamerDecodedFrame;

    fn set_viewport(&mut self, viewport: VideoViewport) {
        self.viewport = Some(viewport);
    }

    fn present(&mut self, mut frame: Self::Frame) {
        if let Some(probe) = &mut frame.probe {
            probe.mark_gui_received();
        }
        self.pending_frame = Some(frame);
    }

    fn render(&mut self) -> eros::Result<()> {
        self.import_pending_frame()?;
        self.draw_current()
    }

    fn clear(&mut self) -> eros::Result<()> {
        self.pending_frame = None;
        let release = self.release_current();
        self.probe_reporter.finish();
        release
    }
}

impl GlState {
    fn capture(gl: &glow::Context) -> Self {
        let active_texture = unsafe { gl.get_parameter_i32(glow::ACTIVE_TEXTURE) };
        unsafe { gl.active_texture(glow::TEXTURE0) };
        let external_texture = unsafe { gl.get_parameter_texture(TEXTURE_BINDING_EXTERNAL) };
        let mut viewport = [0; 4];
        unsafe { gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport) };
        Self {
            program: unsafe { gl.get_parameter_program(glow::CURRENT_PROGRAM) },
            vertex_array: unsafe { gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING) },
            active_texture,
            external_texture,
            viewport,
            blend: unsafe { gl.is_enabled(glow::BLEND) },
            cull_face: unsafe { gl.is_enabled(glow::CULL_FACE) },
            depth_test: unsafe { gl.is_enabled(glow::DEPTH_TEST) },
            scissor_test: unsafe { gl.is_enabled(glow::SCISSOR_TEST) },
        }
    }

    fn restore(self, gl: &glow::Context) {
        unsafe {
            gl.bind_texture(TEXTURE_EXTERNAL, self.external_texture);
            gl.active_texture(self.active_texture as u32);
            gl.use_program(self.program);
            gl.bind_vertex_array(self.vertex_array);
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            set_enabled(gl, glow::BLEND, self.blend);
            set_enabled(gl, glow::CULL_FACE, self.cull_face);
            set_enabled(gl, glow::DEPTH_TEST, self.depth_test);
            set_enabled(gl, glow::SCISSOR_TEST, self.scissor_test);
        }
    }
}

fn fit_viewport(
    viewport: VideoViewport,
    frame_size: crate::kernel::geometry::PixelSize,
) -> eros::Result<VideoViewport> {
    if frame_size.width == 0 || frame_size.height == 0 {
        eros::bail!("Cannot render a zero-sized video frame");
    }
    let width_limited = u64::from(viewport.width) * u64::from(frame_size.height)
        <= u64::from(viewport.height) * u64::from(frame_size.width);
    let (width, height) = if width_limited {
        (
            viewport.width,
            u32::try_from(
                u64::from(viewport.width) * u64::from(frame_size.height)
                    / u64::from(frame_size.width),
            )
            .with_context(|| "Fitted video height exceeds u32")?,
        )
    } else {
        (
            u32::try_from(
                u64::from(viewport.height) * u64::from(frame_size.width)
                    / u64::from(frame_size.height),
            )
            .with_context(|| "Fitted video width exceeds u32")?,
            viewport.height,
        )
    };
    Ok(VideoViewport {
        x: viewport.x + (viewport.width - width) / 2,
        y: viewport.y + (viewport.height - height) / 2,
        width,
        height,
    })
}

unsafe fn set_enabled(gl: &glow::Context, capability: u32, enabled: bool) {
    if enabled {
        unsafe { gl.enable(capability) };
    } else {
        unsafe { gl.disable(capability) };
    }
}

fn create_program(gl: &glow::Context) -> eros::Result<glow::Program> {
    let vertex = compile_shader(gl, glow::VERTEX_SHADER, VERTEX_SHADER, "video vertex")?;
    let fragment =
        match compile_shader(gl, glow::FRAGMENT_SHADER, FRAGMENT_SHADER, "video fragment") {
            Ok(fragment) => fragment,
            Err(error) => {
                unsafe { gl.delete_shader(vertex) };
                return Err(error);
            }
        };
    let program = match unsafe { gl.create_program() } {
        Ok(program) => program,
        Err(error) => {
            unsafe {
                gl.delete_shader(vertex);
                gl.delete_shader(fragment);
            }
            eros::bail!("Failed to create video renderer program: {}", error);
        }
    };
    unsafe {
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
    }
    if !unsafe { gl.get_program_link_status(program) } {
        let log = unsafe { gl.get_program_info_log(program) };
        unsafe { gl.delete_program(program) };
        eros::bail!("Failed to link video renderer program: {}", log);
    }
    Ok(program)
}

fn compile_shader(
    gl: &glow::Context,
    shader_type: u32,
    source: &str,
    description: &str,
) -> eros::Result<glow::Shader> {
    let shader = match unsafe { gl.create_shader(shader_type) } {
        Ok(shader) => shader,
        Err(error) => eros::bail!("Failed to create {} shader: {}", description, error),
    };
    unsafe {
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
    }
    if !unsafe { gl.get_shader_compile_status(shader) } {
        let log = unsafe { gl.get_shader_info_log(shader) };
        unsafe { gl.delete_shader(shader) };
        eros::bail!("Failed to compile {} shader: {}", description, log);
    }
    Ok(shader)
}

fn has_extension(extensions: &CStr, expected: &str) -> bool {
    extensions
        .to_bytes()
        .split(|byte| *byte == b' ')
        .any(|name| name == expected.as_bytes())
}

// Focused test: cargo test infra::platform::video_renderer::tests:: --lib
#[cfg(test)]
mod tests {
    use crate::{
        infra::platform::video_renderer::fit_viewport,
        kernel::{geometry::PixelSize, video_renderer::VideoViewport},
    };

    #[test]
    fn fits_video_inside_viewport_without_changing_aspect_ratio() {
        let fitted = fit_viewport(
            VideoViewport {
                x: 10,
                y: 20,
                width: 1920,
                height: 1200,
            },
            PixelSize {
                width: 1920,
                height: 1080,
            },
        )
        .expect("Video viewport should fit");

        assert_eq!(
            fitted,
            VideoViewport {
                x: 10,
                y: 80,
                width: 1920,
                height: 1080,
            }
        );
    }
}
