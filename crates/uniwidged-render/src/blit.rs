//! Getting the intermediate texture onto a swapchain image.
//!
//! This looks like it should be `wgpu::util::TextureBlitter`, and it was, until
//! a headless render was compared against a screenshot of the same scene and
//! the two disagreed by up to 110 per channel.
//!
//! The cause is a mismatch nothing in either API declares. **Vello writes
//! straight (separated) alpha** — `fine.wgsl` divides the accumulated colour by
//! its alpha before `textureStore`. **Wayland, DirectComposition and
//! `CAMetalLayer` all want premultiplied**, and `CompositeAlphaMode::Opaque`
//! aside, `PreMultiplied` is usually the only mode a surface offers. A plain
//! copy hands straight-alpha pixels to a compositor that multiplies them as if
//! they were premultiplied, and every translucent pixel comes out too bright —
//! subtly enough that a glass panel still looks like glass, which is why this
//! survived the M0 spike.
//!
//! So the blit does the conversion. It is one multiply in a fragment shader,
//! and it is the difference between the headless golden images being evidence
//! and being decoration.

use std::collections::HashMap;

use vello::wgpu;

/// What the destination surface expects to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AlphaConversion {
    /// Multiply the colour by its alpha. What `CompositeAlphaMode::PreMultiplied`
    /// means, and what almost every real surface asks for.
    Premultiply,
    /// Copy through. Correct for `PostMultiplied`, and the least wrong thing
    /// for `Opaque`, where the alpha channel is ignored anyway.
    Straight,
}

impl AlphaConversion {
    pub(crate) fn for_mode(mode: wgpu::CompositeAlphaMode) -> Self {
        match mode {
            wgpu::CompositeAlphaMode::PreMultiplied => Self::Premultiply,
            // `Auto` and `Inherit` leave the platform to decide, and every
            // platform this runs on decides premultiplied.
            wgpu::CompositeAlphaMode::Auto | wgpu::CompositeAlphaMode::Inherit => Self::Premultiply,
            wgpu::CompositeAlphaMode::PostMultiplied | wgpu::CompositeAlphaMode::Opaque => {
                Self::Straight
            }
        }
    }

    fn entry_point(self) -> &'static str {
        match self {
            Self::Premultiply => "fs_premultiply",
            Self::Straight => "fs_straight",
        }
    }
}

const SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One oversized triangle rather than two triangles: no vertex buffer, no index
// buffer, and no seam down the diagonal.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> VsOut {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: VsOut;
    out.uv = vec2<f32>(x, y);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_straight(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}

@fragment
fn fs_premultiply(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(source, source_sampler, in.uv);
    return vec4<f32>(c.rgb * c.a, c.a);
}
"#;

/// The pipelines and the sampler, shared by every surface.
///
/// Keyed by destination format *and* conversion: two monitors can disagree
/// about their swapchain format, and a readback surface and a swapchain surface
/// can disagree about alpha.
pub(crate) struct Blitter {
    shader: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: HashMap<(wgpu::TextureFormat, AlphaConversion), wgpu::RenderPipeline>,
}

impl Blitter {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("uniwidged-blit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniwidged-blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uniwidged-blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Nearest, because source and destination are the same size and every
        // sample lands on a texel centre. Filtering here would soften every
        // widget by a fraction of a pixel for no reason at all.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uniwidged-blit"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            shader,
            layout,
            pipeline_layout,
            sampler,
            pipelines: HashMap::new(),
        }
    }

    /// Returns a handle, not a borrow: wgpu resources are reference-counted, so
    /// cloning one is a refcount bump and it frees the caller from holding
    /// `self` borrowed across the render pass.
    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        conversion: AlphaConversion,
    ) -> wgpu::RenderPipeline {
        self.pipelines
            .entry((format, conversion))
            .or_insert_with(|| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("uniwidged-blit"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vs"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some(conversion.entry_point()),
                        // No blending: the shader's output *is* the frame. The
                        // compositing happens on the other side of the
                        // swapchain, which is the whole point.
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .clone()
    }

    /// Records the blit onto `encoder`.
    pub(crate) fn blit(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        conversion: AlphaConversion,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniwidged-blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let pipeline = self.pipeline(device, format, conversion);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("uniwidged-blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every pixel is written by the triangle, so there is
                    // nothing to clear and nothing to preserve.
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
