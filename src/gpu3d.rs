//! 离屏 wgpu 渲染：真深度缓冲，密集模型表面更干净

use eframe::egui::Color32;
use wgpu::util::DeviceExt;

use crate::mesh3d::{Mesh, OrbitCamera};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    pos: [f32; 3],
    _p0: f32,
    normal: [f32; 3],
    _p1: f32,
    color: [f32; 3],
    _p2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    light_dir: [f32; 4],
    eye: [f32; 4],
}

const SHADER: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    light_dir: vec4<f32>,
    eye: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.mvp * vec4<f32>(in.pos, 1.0);
    o.normal = in.normal;
    o.color = in.color;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    let l = normalize(u.light_dir.xyz);
    if (dot(n, l) < 0.0) {
        n = -n;
    }
    let ndl = max(dot(n, l), 0.0);
    let ambient = 0.38;
    let diffuse = 0.72 * ndl;
    // 简单高光
    let v = normalize(u.eye.xyz);
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 28.0) * 0.18;
    let shade = clamp(ambient + diffuse + spec, 0.0, 1.6);
    let c = in.color * shade;
    return vec4<f32>(c, 1.0);
}
"#;

pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buf: Option<wgpu::Buffer>,
    index_buf: Option<wgpu::Buffer>,
    index_count: u32,
    color_tex: Option<wgpu::Texture>,
    color_view: Option<wgpu::TextureView>,
    resolve_tex: Option<wgpu::Texture>,
    resolve_view: Option<wgpu::TextureView>,
    depth_tex: Option<wgpu::Texture>,
    depth_view: Option<wgpu::TextureView>,
    read_buf: Option<wgpu::Buffer>,
    cur_size: (u32, u32),
    mesh_key: usize,
}

impl GpuRenderer {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("photoview-3d"),
                required_features: wgpu::Features::empty(),
                required_limits:
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            },
            None,
        ))
        .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model3d"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ubl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x3
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: 4,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        Some(Self {
            device,
            queue,
            pipeline,
            uniform_buf,
            bind_group,
            vertex_buf: None,
            index_buf: None,
            index_count: 0,
            color_tex: None,
            color_view: None,
            resolve_tex: None,
            resolve_view: None,
            depth_tex: None,
            depth_view: None,
            read_buf: None,
            cur_size: (0, 0),
            mesh_key: 0,
        })
    }

    fn ensure_targets(&mut self, w: u32, h: u32) {
        if self.cur_size == (w, h) && self.color_tex.is_some() {
            return;
        }
        let extent = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        // MSAA 颜色 + resolve 单采样（回读用）
        let msaa = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color-msaa"),
            size: extent,
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let resolve = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color-resolve"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.color_view = Some(msaa.create_view(&Default::default()));
        self.resolve_view = Some(resolve.create_view(&Default::default()));
        self.depth_view = Some(depth.create_view(&Default::default()));
        let bytes = (w * h * 4) as u64;
        self.read_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        self.color_tex = Some(msaa);
        self.resolve_tex = Some(resolve);
        self.depth_tex = Some(depth);
        self.cur_size = (w, h);
    }

    pub fn upload_mesh(&mut self, mesh: &Mesh, key: usize) {
        if self.mesh_key == key && self.vertex_buf.is_some() {
            return;
        }
        let default_col = [0.88f32, 0.90, 0.93];
        let mut verts = Vec::with_capacity(mesh.positions.len());
        for (i, p) in mesh.positions.iter().enumerate() {
            let n = mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            let c = mesh
                .colors
                .as_ref()
                .and_then(|c| c.get(i).copied())
                .unwrap_or(default_col);
            verts.push(GpuVertex {
                pos: *p,
                _p0: 0.0,
                normal: n,
                _p1: 0.0,
                color: c,
                _p2: 0.0,
            });
        }
        let indices: Vec<u32> = mesh.indices.clone();
        if verts.is_empty() || indices.is_empty() {
            self.vertex_buf = None;
            self.index_buf = None;
            self.index_count = 0;
            return;
        }
        self.vertex_buf = Some(
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vb"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
        );
        self.index_buf = Some(
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("ib"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
        );
        self.index_count = indices.len() as u32;
        self.mesh_key = key;
    }

    /// 渲染一帧并回读像素。失败时返回 None，调用方可回退软件渲染。
    pub fn render(&mut self, cam: &OrbitCamera, width: u32, height: u32) -> Option<Vec<Color32>> {
        if self.index_count == 0 || self.vertex_buf.is_none() {
            return None;
        }
        let w = width.clamp(16, 2048);
        let h = height.clamp(16, 2048);
        self.ensure_targets(w, h);

        let view = cam.view_matrix();
        let proj = cam.proj_matrix(w as f32 / h as f32);
        // GPU NDC y 向上；与软件渲染一致
        let mvp = proj * view;
        let eye = cam.eye_position();
        let uniforms = Uniforms {
            mvp: mvp.to_cols_array_2d(),
            light_dir: [0.45, 0.85, 0.35, 0.0],
            eye: [eye.x, eye.y, eye.z, 1.0],
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.color_view.as_ref()?,
                    resolve_target: self.resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.118,
                            g: 0.125,
                            b: 0.149,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth_view.as_ref()?,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.as_ref()?.slice(..));
            pass.set_index_buffer(
                self.index_buf.as_ref()?.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }

        let bytes_per_row = aligned_bytes_per_row(w);
        let read = self.read_buf.as_ref()?;
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: self.resolve_tex.as_ref()?,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: read,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        self.device.poll(wgpu::Maintain::Wait);
        if !rx.recv().ok().unwrap_or(false) {
            return None;
        }

        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h) as usize);
        let tight = (w * 4) as usize;
        for y in 0..h as usize {
            let row = &data[y * bytes_per_row as usize..y * bytes_per_row as usize + tight];
            for px in row.as_chunks::<4>().0 {
                // srgb 目标已是最终 0-255 显示值
                pixels.push(Color32::from_rgba_unmultiplied(px[0], px[1], px[2], 255));
            }
        }
        drop(data);
        read.unmap();
        Some(pixels)
    }
}

fn aligned_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(align) * align
}

/// 供调用方做 mesh 指纹，避免无意义重传
pub fn mesh_upload_key(mesh: &Mesh) -> usize {
    let mut h: usize = 0x9e37_79b9;
    h ^= mesh.positions.len().wrapping_mul(0x85eb_ca6b);
    h ^= mesh.indices.len().wrapping_mul(0xc2b2_ae35);
    if let Some(p) = mesh.positions.first() {
        h ^= (p[0].to_bits() as usize).wrapping_mul(31);
        h ^= (p[1].to_bits() as usize).wrapping_mul(37);
        h ^= (p[2].to_bits() as usize).wrapping_mul(41);
    }
    if let Some(i) = mesh.indices.first() {
        h ^= (*i as usize).wrapping_mul(43);
    }
    h
}
