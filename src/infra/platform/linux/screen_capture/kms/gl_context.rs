use std::{ffi::c_void, marker::PhantomData, ptr, rc::Rc};

use eros::Context as _;
use glow::HasContext as _;
use khronos_egl as egl;

use crate::{
    infra::platform::screen_capture::kms::types::{KmsPixelBlendMode, KmsPlaneBlend},
    kernel::geometry::PixelSize,
};

const TEXTURE_EXTERNAL: u32 = 0x8D65;

type ImageTargetTexture = unsafe extern "system" fn(u32, *const c_void);

const COMPOSITION_VERTEX_SHADER: &str = r#"#version 300 es
const vec2 positions[4] = vec2[4](
    vec2(-1.0, -1.0),
    vec2( 1.0, -1.0),
    vec2(-1.0,  1.0),
    vec2( 1.0,  1.0)
);
const vec2 texture_coordinates[4] = vec2[4](
    vec2(0.0, 0.0),
    vec2(1.0, 0.0),
    vec2(0.0, 1.0),
    vec2(1.0, 1.0)
);

uniform mat3 position_transform;
uniform mat3 texture_transform;

out vec2 sampled_coordinate;

void main() {
    vec2 position = positions[gl_VertexID];
    vec2 texture_coordinate = texture_coordinates[gl_VertexID];
    vec3 transformed_position = position_transform * vec3(position, 1.0);
    gl_Position = vec4(transformed_position.xy, 0.0, 1.0);
    sampled_coordinate = (texture_transform * vec3(texture_coordinate, 1.0)).xy;
}
"#;

const COMPOSITION_FRAGMENT_SHADER: &str = r#"#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision highp float;

uniform samplerExternalOES plane_texture;
uniform float plane_alpha;
uniform int pixel_blend_mode;

in vec2 sampled_coordinate;
out vec4 composed_color;

void main() {
    vec4 sampled_color = texture(plane_texture, sampled_coordinate);

    if (pixel_blend_mode == 0) {
        composed_color = vec4(sampled_color.rgb * plane_alpha, plane_alpha);
    } else if (pixel_blend_mode == 1) {
        composed_color = sampled_color * plane_alpha;
    } else {
        float alpha = sampled_color.a * plane_alpha;
        composed_color = vec4(sampled_color.rgb * alpha, alpha);
    }
}
"#;

pub(crate) struct GlContext {
    api: glow::Context,
    image_target_texture: ImageTargetTexture,
    composition_program: GlCompositionProgram,
    thread_affinity: PhantomData<Rc<()>>,
}

struct GlCompositionProgram {
    program: glow::Program,
    position_transform: glow::UniformLocation,
    texture_transform: glow::UniformLocation,
    plane_texture: glow::UniformLocation,
    plane_alpha: glow::UniformLocation,
    pixel_blend_mode: glow::UniformLocation,
}

#[derive(Debug)]
pub(crate) struct GlExternalTexture<'context> {
    owner: &'context GlContext,
    texture: glow::Texture,
}

#[derive(Debug)]
pub(crate) struct GlCompositionTarget<'context> {
    owner: &'context GlContext,
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    size: PixelSize,
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

        if !api.supported_extensions().contains("GL_OES_EGL_image") {
            eros::bail!("OpenGL ES does not support EGLImage textures");
        }
        if !api
            .supported_extensions()
            .contains("GL_OES_EGL_image_external")
        {
            eros::bail!("OpenGL ES does not support external EGLImage textures");
        }
        if !api
            .supported_extensions()
            .contains("GL_OES_EGL_image_external_essl3")
        {
            eros::bail!("OpenGL ES 3 does not support external EGLImage samplers");
        }

        let image_target_texture = instance
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .with_context(|| "OpenGL ES did not provide glEGLImageTargetTexture2DOES")?;
        let image_target_texture = unsafe {
            std::mem::transmute::<extern "system" fn(), ImageTargetTexture>(image_target_texture)
        };
        let composition_program = create_composition_program(&api)?;

        Ok(Self {
            api,
            image_target_texture,
            composition_program,
            thread_affinity: PhantomData,
        })
    }

    pub(crate) fn destroy(&mut self) {
        unsafe {
            self.api.delete_program(self.composition_program.program);
        }
    }

    pub(crate) fn create_external_texture(
        &self,
        image: egl::Image,
    ) -> eros::Result<GlExternalTexture<'_>> {
        let texture = match unsafe { self.api.create_texture() } {
            Ok(texture) => texture,
            Err(error) => eros::bail!("Failed to create an OpenGL ES texture: {}", error),
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
            eros::bail!(
                "Failed to bind EGLImage to an external texture: GL error 0x{:04X}",
                error
            );
        }

        Ok(GlExternalTexture {
            owner: self,
            texture,
        })
    }

    pub(crate) fn create_composition_target(
        &self,
        image: egl::Image,
        size: PixelSize,
    ) -> eros::Result<GlCompositionTarget<'_>> {
        let texture = match unsafe { self.api.create_texture() } {
            Ok(texture) => texture,
            Err(error) => {
                eros::bail!("Failed to create an OpenGL ES composition texture: {}", error)
            }
        };

        unsafe {
            self.api.bind_texture(glow::TEXTURE_2D, Some(texture));
            (self.image_target_texture)(glow::TEXTURE_2D, image.as_ptr());
            self.api.bind_texture(glow::TEXTURE_2D, None);
        }

        let error = unsafe { self.api.get_error() };
        if error != glow::NO_ERROR {
            unsafe { self.api.delete_texture(texture) };
            eros::bail!(
                "Failed to bind EGLImage to a composition texture: GL error 0x{:04X}",
                error
            );
        }

        let framebuffer = match unsafe { self.api.create_framebuffer() } {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                unsafe { self.api.delete_texture(texture) };
                eros::bail!("Failed to create an OpenGL ES framebuffer: {}", error);
            }
        };

        unsafe {
            self.api
                .bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            self.api.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
        }
        let status = unsafe { self.api.check_framebuffer_status(glow::FRAMEBUFFER) };
        unsafe { self.api.bind_framebuffer(glow::FRAMEBUFFER, None) };

        if status != glow::FRAMEBUFFER_COMPLETE {
            unsafe {
                self.api.delete_framebuffer(framebuffer);
                self.api.delete_texture(texture);
            }
            eros::bail!(
                "EGLImage composition framebuffer is incomplete: GL status 0x{:04X}",
                status
            );
        }

        Ok(GlCompositionTarget {
            owner: self,
            texture,
            framebuffer,
            size,
        })
    }

    pub(crate) fn clear_composition_target(
        &self,
        target: &GlCompositionTarget<'_>,
    ) -> eros::Result<()> {
        if !ptr::eq(self, target.owner) {
            eros::bail!("Cannot clear a composition target created by another OpenGL context");
        }

        let (width, height) = composition_target_size(target)?;
        unsafe {
            self.api
                .bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            self.api.viewport(0, 0, width, height);
            self.api.clear_color(0.0, 0.0, 0.0, 1.0);
            self.api.clear(glow::COLOR_BUFFER_BIT);
            self.api.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        let error = unsafe { self.api.get_error() };
        if error != glow::NO_ERROR {
            eros::bail!(
                "Failed to clear the composition target: GL error 0x{:04X}",
                error
            );
        }

        Ok(())
    }

    pub(crate) fn compose_plane(
        &self,
        target: &GlCompositionTarget<'_>,
        texture: &GlExternalTexture<'_>,
        position_transform: &[f32; 9],
        texture_transform: &[f32; 9],
        blend: KmsPlaneBlend,
    ) -> eros::Result<()> {
        if !ptr::eq(self, target.owner) {
            eros::bail!("Cannot compose into a target created by another OpenGL context");
        }
        if !ptr::eq(self, texture.owner) {
            eros::bail!("Cannot compose a texture created by another OpenGL context");
        }

        let (width, height) = composition_target_size(target)?;
        let program = &self.composition_program;
        unsafe {
            self.api
                .bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            self.api.viewport(0, 0, width, height);
            self.api.use_program(Some(program.program));
            self.api.active_texture(glow::TEXTURE0);
            self.api.bind_texture(TEXTURE_EXTERNAL, Some(texture.texture));
            self.api
                .uniform_1_i32(Some(&program.plane_texture), 0);
            self.api.uniform_matrix_3_f32_slice(
                Some(&program.position_transform),
                false,
                position_transform,
            );
            self.api.uniform_matrix_3_f32_slice(
                Some(&program.texture_transform),
                false,
                texture_transform,
            );
            self.api.uniform_1_f32(
                Some(&program.plane_alpha),
                f32::from(blend.alpha) / f32::from(u16::MAX),
            );
            self.api.uniform_1_i32(
                Some(&program.pixel_blend_mode),
                pixel_blend_mode(blend.pixel_mode),
            );
            self.api.enable(glow::BLEND);
            self.api
                .blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            self.api.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            self.api.disable(glow::BLEND);
            self.api.bind_texture(TEXTURE_EXTERNAL, None);
            self.api.use_program(None);
            self.api.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        let error = unsafe { self.api.get_error() };
        if error != glow::NO_ERROR {
            eros::bail!("Failed to compose a KMS plane: GL error 0x{:04X}", error);
        }

        Ok(())
    }
}

impl Drop for GlExternalTexture<'_> {
    fn drop(&mut self) {
        unsafe { self.owner.api.delete_texture(self.texture) };
    }
}

impl Drop for GlCompositionTarget<'_> {
    fn drop(&mut self) {
        unsafe {
            self.owner.api.delete_framebuffer(self.framebuffer);
            self.owner.api.delete_texture(self.texture);
        }
    }
}

fn create_composition_program(api: &glow::Context) -> eros::Result<GlCompositionProgram> {
    let vertex = compile_shader(api, glow::VERTEX_SHADER, COMPOSITION_VERTEX_SHADER)?;
    let fragment = match compile_shader(api, glow::FRAGMENT_SHADER, COMPOSITION_FRAGMENT_SHADER) {
        Ok(fragment) => fragment,
        Err(error) => {
            unsafe { api.delete_shader(vertex) };
            return Err(error);
        }
    };
    let program = match unsafe { api.create_program() } {
        Ok(program) => program,
        Err(error) => {
            unsafe {
                api.delete_shader(vertex);
                api.delete_shader(fragment);
            }
            eros::bail!("Failed to create the KMS composition program: {}", error);
        }
    };

    unsafe {
        api.attach_shader(program, vertex);
        api.attach_shader(program, fragment);
        api.link_program(program);
        api.detach_shader(program, vertex);
        api.detach_shader(program, fragment);
        api.delete_shader(vertex);
        api.delete_shader(fragment);
    }

    if !unsafe { api.get_program_link_status(program) } {
        let log = unsafe { api.get_program_info_log(program) };
        unsafe { api.delete_program(program) };
        eros::bail!("Failed to link the KMS composition program: {}", log);
    }

    match composition_program(api, program) {
        Ok(program) => Ok(program),
        Err(error) => {
            unsafe { api.delete_program(program) };
            Err(error)
        }
    }
}

fn composition_program(
    api: &glow::Context,
    program: glow::Program,
) -> eros::Result<GlCompositionProgram> {
    Ok(GlCompositionProgram {
        program,
        position_transform: uniform(api, program, "position_transform")?,
        texture_transform: uniform(api, program, "texture_transform")?,
        plane_texture: uniform(api, program, "plane_texture")?,
        plane_alpha: uniform(api, program, "plane_alpha")?,
        pixel_blend_mode: uniform(api, program, "pixel_blend_mode")?,
    })
}

fn compile_shader(
    api: &glow::Context,
    shader_type: u32,
    source: &str,
) -> eros::Result<glow::Shader> {
    let shader = match unsafe { api.create_shader(shader_type) } {
        Ok(shader) => shader,
        Err(error) => eros::bail!("Failed to create a KMS composition shader: {}", error),
    };
    unsafe {
        api.shader_source(shader, source);
        api.compile_shader(shader);
    }

    if !unsafe { api.get_shader_compile_status(shader) } {
        let log = unsafe { api.get_shader_info_log(shader) };
        unsafe { api.delete_shader(shader) };
        eros::bail!("Failed to compile a KMS composition shader: {}", log);
    }

    Ok(shader)
}

fn uniform(
    api: &glow::Context,
    program: glow::Program,
    name: &'static str,
) -> eros::Result<glow::UniformLocation> {
    Ok(unsafe { api.get_uniform_location(program, name) }
        .with_context(|| format!("KMS composition program does not expose uniform {name}"))?)
}

fn composition_target_size(target: &GlCompositionTarget<'_>) -> eros::Result<(i32, i32)> {
    let width = i32::try_from(target.size.width)
        .with_context(|| "KMS composition target width exceeds OpenGL limits")?;
    let height = i32::try_from(target.size.height)
        .with_context(|| "KMS composition target height exceeds OpenGL limits")?;
    Ok((width, height))
}

fn pixel_blend_mode(mode: KmsPixelBlendMode) -> i32 {
    match mode {
        KmsPixelBlendMode::None => 0,
        KmsPixelBlendMode::PreMultiplied => 1,
        KmsPixelBlendMode::Coverage => 2,
    }
}
