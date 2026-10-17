//! 离屏 wgpu 渲染：真深度缓冲，密集模型表面更干净

use eframe::egui::Color32;
use wgpu::util::DeviceExt;

use crate::mesh3d::{self, Mesh, OrbitCamera};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    pos: [f32; 3],
    _p0: f32,
    normal: [f32; 3],
    _p1: f32,
    color: [f32; 3],
    _p2: f32,
    uv: [f32; 2],
    _p3: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    light_dir: [f32; 4],
    eye: [f32; 4],
}

/// 逐三角形的一段索引，tex == NO_TEXTURE 表示用无贴图管线
#[derive(Clone, Copy)]
struct DrawGroup {
    start: u32,
    count: u32,
    tex: u32,
}

const SHADER: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    light_dir: vec4<f32>,
    eye: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var base_color: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.mvp * vec4<f32>(in.pos, 1.0);
    o.normal = in.normal;
    o.color = in.color;
    o.uv = in.uv;
    return o;
}

fn lighting(nrm: vec3<f32>) -> f32 {
    var n = normalize(nrm);
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
    return clamp(ambient + diffuse + spec, 0.0, 1.6);
}

// 与软件光栅化同一套 sRGB 规则，静止帧和拖拽预览才不会两个亮度
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    let c = in.color * lighting(in.normal);
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_textured(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(base_color, base_sampler, in.uv);
    if (t.a < 0.5) {
        discard;
    }
    // 贴图是 sRGB 纹理，采样出来已是线性值；顶点色（底色因子）要先转线性
    let c = t.rgb * to_linear(in.color) * lighting(in.normal);
    return vec4<f32>(c, 1.0);
}
"#;

const VERTEX_ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x3,
    3 => Float32x2
];

fn make_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment_entry: &str,
    color_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment_entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &VERTEX_ATTRS,
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: fragment_entry,
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
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
    })
}

pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline_flat: wgpu::RenderPipeline,
    pipeline_tex: wgpu::RenderPipeline,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buf: Option<wgpu::Buffer>,
    index_buf: Option<wgpu::Buffer>,
    index_count: u32,
    groups: Vec<DrawGroup>,
    tex_binds: Vec<Option<wgpu::BindGroup>>,
    keep_tex: Vec<(wgpu::Texture, wgpu::TextureView)>,
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
                label: Some("dadaview-3d"),
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

        let layout_flat = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pl-flat"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        // 贴图 + 采样器（group 1），每张纹理一个 bind group
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texbl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout_tex = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pl-tex"),
            bind_group_layouts: &[&bind_layout, &tex_layout],
            push_constant_ranges: &[],
        });

        let pipeline_flat = make_pipeline(
            &device,
            &shader,
            &layout_flat,
            "fs_flat",
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let pipeline_tex = make_pipeline(
            &device,
            &shader,
            &layout_tex,
            "fs_textured",
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );

        Some(Self {
            device,
            queue,
            pipeline_flat,
            pipeline_tex,
            tex_layout,
            sampler,
            uniform_buf,
            bind_group,
            vertex_buf: None,
            index_buf: None,
            index_count: 0,
            groups: Vec::new(),
            tex_binds: Vec::new(),
            keep_tex: Vec::new(),
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

    /// 建一张 sRGB 纹理并配好 bind group；失败返回 None，该图退回顶点色。
    /// texture/view 存进 keep_tex 继续持有：queue.write_texture 延后到 submit 才执行，
    /// 句柄提前掉引用会被回收
    fn make_texture(&mut self, tex: &mesh3d::Texture) -> Option<wgpu::BindGroup> {
        let w = tex.width;
        let h = tex.height;
        if w == 0
            || h == 0
            || w > mesh3d::MAX_TEXTURE_DIM
            || h > mesh3d::MAX_TEXTURE_DIM
            || tex.rgba.len() < (w as usize) * (h as usize) * 4
        {
            return None;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("model-texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // 纹理上传不受 COPY_BYTES_PER_ROW_ALIGNMENT 约束，直接按行宽写
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &tex.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        self.keep_tex.push((texture, view));
        let (_, view) = self.keep_tex.last()?;
        Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texbg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }))
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
                // 贴图顶点的"顶点色"是 baseColorFactor（白 = 不染色），与软件路径一致
                .unwrap_or(if mesh.vertex_textured(i) {
                    [1.0; 3]
                } else {
                    default_col
                });
            let uv = mesh
                .uvs
                .as_ref()
                .and_then(|u| u.get(i))
                .copied()
                .unwrap_or([0.0; 2]);
            verts.push(GpuVertex {
                pos: *p,
                _p0: 0.0,
                normal: n,
                _p1: 0.0,
                color: c,
                _p2: 0.0,
                uv,
                _p3: [0.0; 2],
            });
        }
        let indices: Vec<u32> = mesh.indices.clone();
        if verts.is_empty() || indices.is_empty() {
            self.vertex_buf = None;
            self.index_buf = None;
            self.index_count = 0;
            self.groups.clear();
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
        self.upload_textures(mesh);
        let groups = split_groups(mesh, &self.tex_binds);
        self.groups = groups;
        self.mesh_key = key;
    }

    fn upload_textures(&mut self, mesh: &Mesh) {
        self.keep_tex.clear();
        self.tex_binds.clear();
        for t in &mesh.textures {
            let bg = self.make_texture(t);
            self.tex_binds.push(bg);
        }
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
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.as_ref()?.slice(..));
            let ib = self.index_buf.as_ref()?;
            // 按贴图分段画：同一段用同一管线，深度缓冲共用所以互相遮挡仍正确
            for g in self.groups.clone() {
                let byte_lo = g.start as u64 * 4;
                let byte_hi = (g.start + g.count) as u64 * 4;
                match self.tex_binds.get(g.tex as usize).and_then(|b| b.as_ref()) {
                    Some(bg) => {
                        pass.set_pipeline(&self.pipeline_tex);
                        pass.set_bind_group(1, bg, &[]);
                    }
                    None => {
                        pass.set_pipeline(&self.pipeline_flat);
                    }
                }
                pass.set_index_buffer(ib.slice(byte_lo..byte_hi), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.count, 0, 0..1);
            }
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

/// 把索引按"用哪张贴图"切成连续段；贴图上传失败的段退回无贴图管线
fn split_groups(mesh: &Mesh, tex_binds: &[Option<wgpu::BindGroup>]) -> Vec<DrawGroup> {
    let usable = |t: Option<usize>| -> u32 {
        match t {
            Some(i) if tex_binds.get(i).and_then(|b| b.as_ref()).is_some() => i as u32,
            _ => mesh3d::NO_TEXTURE,
        }
    };
    let tri_tex = |k: usize| {
        let a = k * 3;
        usable(mesh.tri_texture(
            mesh.indices[a] as usize,
            mesh.indices[a + 1] as usize,
            mesh.indices[a + 2] as usize,
        ))
    };
    let tris = mesh.indices.len() / 3;
    let mut out: Vec<DrawGroup> = Vec::new();
    let mut k = 0usize;
    while k < tris {
        let cur = tri_tex(k);
        let mut end = k + 1;
        while end < tris && tri_tex(end) == cur {
            end += 1;
        }
        out.push(DrawGroup {
            start: (k * 3) as u32,
            count: ((end - k) * 3) as u32,
            tex: cur,
        });
        k = end;
    }
    out
}

/// 供调用方做 mesh 指纹，避免无意义重传。取首/中/末顶点与首/末索引+各数组长度，
/// 降低同文件换模型（顶点数不变）时的碰撞概率
pub fn mesh_upload_key(mesh: &Mesh) -> usize {
    let mut h: usize = 0x9e37_79b9;
    h ^= mesh.positions.len().wrapping_mul(0x85eb_ca6b);
    h ^= mesh.indices.len().wrapping_mul(0xc2b2_ae35);
    h ^= mesh.normals.len().wrapping_mul(0x5f35_6495);
    h ^= mesh
        .colors
        .as_ref()
        .map_or(0, |c| c.len())
        .wrapping_mul(0x2722_0a95);
    h ^= mesh
        .uvs
        .as_ref()
        .map_or(0, |u| u.len())
        .wrapping_mul(0x6c62_2729);
    h ^= mesh
        .tex_ids
        .as_ref()
        .map_or(0, |t| t.len())
        .wrapping_mul(0x9e37_79b1);
    let mid = mesh.positions.len() / 2;
    for p in [
        mesh.positions.first(),
        mesh.positions.get(mid),
        mesh.positions.last(),
    ]
    .into_iter()
    .flatten()
    {
        h = h.rotate_left(13);
        h ^= (p[0].to_bits() as usize).wrapping_mul(31);
        h ^= (p[1].to_bits() as usize).wrapping_mul(37);
        h ^= (p[2].to_bits() as usize).wrapping_mul(41);
    }
    for i in [mesh.indices.first(), mesh.indices.last()]
        .into_iter()
        .flatten()
    {
        h = h.rotate_left(7);
        h ^= (*i as usize).wrapping_mul(43);
    }
    // UV / 贴图号同样抽样（首/中/尾）：改 UV 或重映射后顶点缓冲内容变了，必须重传
    if let Some(u) = mesh.uvs.as_ref() {
        for uv in [u.first(), u.get(mid), u.last()].into_iter().flatten() {
            h = h.rotate_left(17);
            h ^= (uv[0].to_bits() as usize).wrapping_mul(53);
            h ^= (uv[1].to_bits() as usize).wrapping_mul(59);
        }
    }
    if let Some(t) = mesh.tex_ids.as_ref() {
        for id in [t.first(), t.get(mid), t.last()].into_iter().flatten() {
            h = h.rotate_left(5);
            h ^= (*id as usize).wrapping_mul(61);
        }
    }
    // 贴图也进指纹：同一网格换贴图（比如重新导出）必须重传
    h ^= mesh.textures.len().wrapping_mul(0x85eb_ca6b);
    for t in &mesh.textures {
        h = h.rotate_left(11);
        h ^= (t.width as usize).wrapping_mul(29);
        h ^= (t.height as usize).wrapping_mul(31);
        h ^= t.rgba.len();
        for &b in t.rgba.iter().take(4) {
            h = (h ^ b as usize).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_mesh(count: usize, x: f32) -> Mesh {
        Mesh {
            positions: vec![[x, 0.5, -0.5]; count],
            normals: vec![[0.0, 1.0, 0.0]; count],
            colors: None,
            indices: (0..count as u32).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn upload_key_is_stable() {
        let m = tri_mesh(6, 0.0);

        assert_eq!(mesh_upload_key(&m), mesh_upload_key(&m));
    }

    #[test]
    fn upload_key_distinguishes_meshes() {
        let a = tri_mesh(3, 0.0);

        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&tri_mesh(4, 0.0)));
        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&tri_mesh(3, 1.0)));
    }

    #[test]
    fn upload_key_distinguishes_tail_edits() {
        // 同顶点数、只改末尾顶点：旧指纹（只看首元素）会碰撞导致 GPU 用旧几何
        let a = tri_mesh(6, 0.0);

        let mut b = a.clone();

        b.positions.last_mut().unwrap()[0] = 9.0;

        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&b));
    }

    fn tex(v: u8) -> std::sync::Arc<mesh3d::Texture> {
        std::sync::Arc::new(mesh3d::Texture {
            width: 2,
            height: 2,
            rgba: std::sync::Arc::new(vec![v; 16]),
        })
    }

    fn textured_mesh() -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            colors: None,
            indices: vec![0, 1, 2],
            uvs: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            tex_ids: Some(vec![0, 0, 0]),
            textures: vec![tex(10)],
        }
    }

    #[test]
    fn upload_key_distinguishes_textures_and_uvs() {
        let a = textured_mesh();

        assert_eq!(mesh_upload_key(&a), mesh_upload_key(&textured_mesh()));
        // 只换贴图内容：必须重传，否则 GPU 一直用旧图
        let mut b = a.clone();
        b.textures = vec![tex(20)];
        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&b));
        // 只加一张图 / 只改 UV 同样要失效
        let mut c = a.clone();
        c.textures.push(tex(10));
        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&c));
        let mut d = a.clone();
        d.uvs.as_mut().unwrap()[1] = [0.5, 0.5];
        assert_ne!(mesh_upload_key(&a), mesh_upload_key(&d));
        let plain = tri_mesh(3, 0.0);
        assert_ne!(mesh_upload_key(&plain), mesh_upload_key(&a));
    }
}
