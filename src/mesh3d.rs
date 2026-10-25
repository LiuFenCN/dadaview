//! 3D 模型加载与软件光栅化渲染
//!
//! 应用内解析，不依赖 Blender/FreeCAD 等第三方工具。

use eframe::egui::Color32;
use glam::{Mat4, Vec3};
use std::path::Path;

pub const MODEL_EXTS: &[&str] = &[
    "obj", "stl", "gltf", "glb", "ply", "off", "3ds", "dae", "vtk", "wrl", "vrml", "amf", "3mf",
    "gcode", "gco", "g", "vrm", "fbx", "pmx", "pmd", "usd", "usda", "usdc", "usdz",
];

pub const UNSUPPORTED_3D_EXTS: &[&str] = &[
    "blend", "step", "stp", "iges", "igs", "3dm", "c4d", "max", "skp", "sat", "x_t", "x_b", "prt",
    "sldprt", "sldasm", "ipt", "catpart", "3dxml", "abc",
];

pub fn is_model_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MODEL_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn is_unsupported_3d_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| UNSUPPORTED_3D_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

const MAX_RENDER_TRIS_HQ: usize = 2_000_000;
const MAX_RENDER_TRIS_FAST: usize = 2_000_000;
const SKIP_VERTEX_NORMALS_ABOVE: usize = 400_000;

/// 8 位 sRGB 纹理。像素按解码顺序存放（第 0 行是图像顶部），
/// 这样软件光栅化与 wgpu 用同一份数据、同一个 UV 约定
#[derive(Clone, Default)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<Vec<u8>>,
}

impl Texture {
    #[inline]
    fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        let o = (y * self.width as usize + x) * 4;
        match self.rgba.get(o..o + 4) {
            Some(p) => [
                p[0] as f32 / 255.0,
                p[1] as f32 / 255.0,
                p[2] as f32 / 255.0,
                p[3] as f32 / 255.0,
            ],
            None => [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// 该顶点不使用贴图（走顶点色/纯色）
pub const NO_TEXTURE: u32 = u32::MAX;

#[derive(Clone, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Option<Vec<[f32; 3]>>,
    pub indices: Vec<u32>,
    /// 逐顶点 UV，长度与 positions 一致
    pub uvs: Option<Vec<[f32; 2]>>,
    /// 逐顶点使用的纹理下标（指向 textures），与 uvs 同步存在
    pub tex_ids: Option<Vec<u32>>,
    pub textures: Vec<std::sync::Arc<Texture>>,
}

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
pub(crate) fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// 双线性 + clamp 采样，返回线性 RGB 与 alpha
fn sample_texture(tex: &Texture, u: f32, v: f32) -> ([f32; 3], f32) {
    let tw = tex.width.max(1) as i32;
    let th = tex.height.max(1) as i32;
    let fx = u * tw as f32 - 0.5;
    let fy = v * th as f32 - 0.5;
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let grab =
        |xi: i32, yi: i32| tex.texel(xi.clamp(0, tw - 1) as usize, yi.clamp(0, th - 1) as usize);
    let a = grab(x0, y0);
    let b = grab(x0 + 1, y0);
    let c = grab(x0, y0 + 1);
    let d = grab(x0 + 1, y0 + 1);
    let top = |i: usize| a[i] + (b[i] - a[i]) * tx;
    let bot = |i: usize| c[i] + (d[i] - c[i]) * tx;
    let mix = |i: usize| top(i) + (bot(i) - top(i)) * ty;
    (
        [
            srgb_to_linear(mix(0).clamp(0.0, 1.0)),
            srgb_to_linear(mix(1).clamp(0.0, 1.0)),
            srgb_to_linear(mix(2).clamp(0.0, 1.0)),
        ],
        mix(3).clamp(0.0, 1.0),
    )
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// 贴图通道是否可用（三者长度与顶点数一致才算）
    pub(crate) fn texture_channels(&self) -> Option<(&Vec<[f32; 2]>, &Vec<u32>)> {
        let uvs = self.uvs.as_ref()?;
        let ids = self.tex_ids.as_ref()?;
        if self.textures.is_empty()
            || uvs.len() != self.positions.len()
            || ids.len() != self.positions.len()
        {
            return None;
        }
        Some((uvs, ids))
    }

    /// 该三角形用哪张贴图 + 三个顶点的 UV。软件光栅化与 GPU 都用它，画面才一致
    pub(crate) fn tri_textured(
        &self,
        i0: usize,
        i1: usize,
        i2: usize,
    ) -> Option<(usize, [[f32; 2]; 3])> {
        let (uvs, ids) = self.texture_channels()?;
        let t = *ids.get(i0)?;
        if t == NO_TEXTURE || ids.get(i1).copied() != Some(t) || ids.get(i2).copied() != Some(t) {
            return None;
        }
        if t as usize >= self.textures.len() {
            return None;
        }
        let uv = [*uvs.get(i0)?, *uvs.get(i1)?, *uvs.get(i2)?];
        Some((t as usize, uv))
    }

    /// 该顶点是否绑定了一张有效贴图（GPU 用它在缺顶点色时选白，与软件光栅化一致）
    pub(crate) fn vertex_textured(&self, i: usize) -> bool {
        let Some((_, ids)) = self.texture_channels() else {
            return false;
        };
        match ids.get(i).copied() {
            Some(t) if t != NO_TEXTURE => (t as usize) < self.textures.len(),
            _ => false,
        }
    }

    /// 该三角形用的贴图下标
    pub(crate) fn tri_texture(&self, i0: usize, i1: usize, i2: usize) -> Option<usize> {
        self.tri_textured(i0, i1, i2).map(|(t, _)| t)
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            let v = Vec3::from(*p);
            mn = mn.min(v);
            mx = mx.max(v);
        }
        if !mn.is_finite() || !mx.is_finite() {
            return (Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        if (mx - mn).max_element() < 1e-6 {
            mn = Vec3::splat(-0.5);
            mx = Vec3::splat(0.5);
        }
        (mn, mx)
    }

    fn ensure_normals(&mut self) {
        if self.normals.len() == self.positions.len() {
            return;
        }
        if self.positions.len() > SKIP_VERTEX_NORMALS_ABOVE {
            self.normals.clear();
            return;
        }
        let mut normals = vec![[0.0f32; 3]; self.positions.len()];
        for tri in self.indices.as_chunks::<3>().0 {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 >= self.positions.len()
                || i1 >= self.positions.len()
                || i2 >= self.positions.len()
            {
                continue;
            }
            let p0 = Vec3::from(self.positions[i0]);
            let p1 = Vec3::from(self.positions[i1]);
            let p2 = Vec3::from(self.positions[i2]);
            let n = (p1 - p0).cross(p2 - p0);
            for &i in &[i0, i1, i2] {
                let cur = Vec3::from(normals[i]) + n;
                normals[i] = [cur.x, cur.y, cur.z];
            }
        }
        for n in &mut normals {
            let v = Vec3::from(*n);
            let len = v.length();
            *n = if len > 1e-12 {
                let u = v / len;
                [u.x, u.y, u.z]
            } else {
                [0.0, 1.0, 0.0]
            };
        }
        self.normals = normals;
    }

    fn append_mesh(&mut self, other: Mesh, xform: Mat4) {
        let base = self.positions.len() as u32;
        let old_len = base as usize;
        let normal_mat = xform.inverse().transpose();
        for p in &other.positions {
            let v = xform.transform_point3(Vec3::from(*p));
            self.positions.push([v.x, v.y, v.z]);
        }
        if other.normals.len() == other.positions.len() {
            for n in &other.normals {
                let v = normal_mat
                    .transform_vector3(Vec3::from(*n))
                    .normalize_or_zero();
                self.normals.push([v.x, v.y, v.z]);
            }
        } else {
            for _ in &other.positions {
                self.normals.push([0.0, 0.0, 0.0]);
            }
        }
        match (&mut self.colors, &other.colors) {
            (Some(acc), Some(cols)) => acc.extend_from_slice(cols),
            (Some(acc), None) => {
                acc.extend(std::iter::repeat_n(
                    [0.78, 0.80, 0.84],
                    other.positions.len(),
                ));
            }
            (None, Some(cols)) => {
                let mut acc =
                    vec![[0.78, 0.80, 0.84]; self.positions.len() - other.positions.len()];
                acc.extend_from_slice(cols);
                self.colors = Some(acc);
            }
            (None, None) => {}
        }
        for i in &other.indices {
            self.indices.push(base + i);
        }
        // 贴图：先并纹理表（Arc 指针相同即同一张，避免同一图重复上传），再接 UV 与逐顶点纹理号
        let mut remap: Vec<u32> = Vec::new();
        for t in &other.textures {
            let reuse = self
                .textures
                .iter()
                .position(|cur| std::sync::Arc::ptr_eq(cur, t));
            remap.push(match reuse {
                Some(j) => j as u32,
                None => {
                    self.textures.push(t.clone());
                    (self.textures.len() - 1) as u32
                }
            });
        }
        let other_tex = !other.textures.is_empty();
        if other_tex || !self.textures.is_empty() {
            let mut uvs = self.uvs.take().unwrap_or_else(|| vec![[0.0; 2]; old_len]);
            uvs.resize(self.positions.len(), [0.0; 2]);
            if let Some(src) = &other.uvs {
                for (k, uv) in src.iter().enumerate().take(other.positions.len()) {
                    uvs[old_len + k] = *uv;
                }
            }
            self.uvs = Some(uvs);

            let mut ids = self
                .tex_ids
                .take()
                .unwrap_or_else(|| vec![NO_TEXTURE; old_len]);
            ids.resize(self.positions.len(), NO_TEXTURE);
            if let Some(src) = &other.tex_ids {
                for (k, id) in src.iter().enumerate().take(other.positions.len()) {
                    let mapped = if other_tex {
                        remap.get(*id as usize).copied().unwrap_or(NO_TEXTURE)
                    } else {
                        NO_TEXTURE
                    };
                    ids[old_len + k] = mapped;
                }
            }
            self.tex_ids = Some(ids);
        }
    }

    fn from_positions_indices(
        positions: Vec<[f32; 3]>,
        indices: Vec<u32>,
        colors: Option<Vec<[f32; 3]>>,
    ) -> Mesh {
        let mut m = Mesh {
            positions,
            normals: Vec::new(),
            colors,
            indices,
            ..Default::default()
        };
        m.ensure_normals();
        m
    }
}

#[derive(Clone)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
    pub fov_y: f32,
    pub default_yaw: f32,
    pub default_pitch: f32,
    pub default_distance: f32,
}

impl OrbitCamera {
    fn from_bounds(mn: Vec3, mx: Vec3) -> Self {
        let center = (mn + mx) * 0.5;
        let radius = ((mx - mn) * 0.5).length().max(0.1);
        let distance = radius * 2.6;
        Self {
            yaw: 0.65,
            pitch: 0.45,
            distance,
            target: center,
            fov_y: 45.0f32.to_radians(),
            default_yaw: 0.65,
            default_pitch: 0.45,
            default_distance: distance,
        }
    }

    pub fn reset(&mut self) {
        self.yaw = self.default_yaw;
        self.pitch = self.default_pitch;
        self.distance = self.default_distance;
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.012;
        self.pitch = (self.pitch + dy * 0.012).clamp(-1.55, 1.55);
    }

    pub fn pan(&mut self, dx: f32, dy: f32, aspect: f32) {
        let (right, up) = self.basis();
        let scale = self.distance * 0.0018 / aspect.max(0.1);
        self.target += right * (-dx * scale) + up * (dy * scale);
    }

    pub fn zoom_by(&mut self, scroll_y: f32) {
        let factor = if scroll_y > 0.0 { 0.88 } else { 1.14 };
        self.distance = (self.distance * factor)
            .clamp(self.default_distance * 0.05, self.default_distance * 8.0);
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye_position(), self.target, Vec3::Y)
    }

    pub fn eye_position(&self) -> Vec3 {
        let cp = self.pitch.cos();
        let sp = self.pitch.sin();
        let cy = self.yaw.cos();
        let sy = self.yaw.sin();
        self.target
            + Vec3::new(
                self.distance * cp * sy,
                self.distance * sp,
                self.distance * cp * cy,
            )
    }

    fn basis(&self) -> (Vec3, Vec3) {
        let eye = self.eye_position();
        let forward = (self.target - eye).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        (right, up)
    }

    pub fn proj_matrix(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(
            self.fov_y,
            aspect.max(0.05),
            self.distance * 0.01,
            self.distance * 20.0,
        )
    }
}

pub struct Model3D {
    pub mesh: std::sync::Arc<Mesh>,
    pub camera: OrbitCamera,
    pub format_label: String,
    pub needs_redraw: bool,
    /// glTF 外部贴图缺失数量（>0 时以无贴图模式显示）
    pub missing_textures: usize,
    last_render_size: (usize, usize),
}

impl Model3D {
    pub fn load_path(path: &Path) -> anyhow::Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let mut missing_textures = 0usize;
        let mesh = match ext.as_str() {
            "obj" => load_obj(path)?,
            "stl" => load_stl(path)?,
            "gltf" | "glb" | "vrm" => {
                let (m, n) = load_gltf(path)?;
                missing_textures = n;
                m
            }
            "ply" => load_ply(path)?,
            "off" => load_off(path)?,
            "3ds" => load_3ds(path)?,
            "dae" => load_dae(path)?,
            "vtk" => load_vtk(path)?,
            "wrl" | "vrml" => load_wrl(path)?,
            "amf" => load_amf(path)?,
            "3mf" => load_3mf(path)?,
            "gcode" | "gco" | "g" => load_gcode(path)?,
            "fbx" => load_fbx(path)?,
            "pmx" | "pmd" => load_pmx(path)?,
            "usd" | "usda" | "usdc" | "usdz" => load_usd(path)?,
            other => anyhow::bail!("不支持的 3D 格式: {}", other),
        };
        if mesh.positions.is_empty() || mesh.indices.is_empty() {
            anyhow::bail!("模型为空或无三角形数据");
        }
        let mut mesh = mesh;
        mesh.ensure_normals();
        let (mn, mx) = mesh.bounds();
        Ok(Self {
            mesh: std::sync::Arc::new(mesh),
            camera: OrbitCamera::from_bounds(mn, mx),
            format_label: ext.to_uppercase(),
            needs_redraw: true,
            missing_textures,
            last_render_size: (0, 0),
        })
    }

    pub fn vertex_count(&self) -> usize {
        self.mesh.vertex_count()
    }

    pub fn triangle_count(&self) -> usize {
        self.mesh.triangle_count()
    }

    #[cfg(test)]
    pub fn render(&mut self, width: usize, height: usize) -> Vec<Color32> {
        let w = width.max(2);
        let h = height.max(2);
        self.last_render_size = (w, h);
        self.needs_redraw = false;
        render_mesh(&self.mesh, &self.camera, w, h)
    }

    pub fn render_snapshot(&self) -> (std::sync::Arc<Mesh>, OrbitCamera) {
        (self.mesh.clone(), self.camera.clone())
    }

    pub fn last_size(&self) -> (usize, usize) {
        self.last_render_size
    }

    pub fn mark_dirty(&mut self) {
        self.needs_redraw = true;
    }

    pub fn note_rendered(&mut self, w: usize, h: usize) {
        self.last_render_size = (w, h);
        self.needs_redraw = false;
    }
}

fn fan_triangles(indices: &mut Vec<u32>, poly: &[u32]) {
    if poly.len() < 3 {
        return;
    }
    for i in 1..poly.len().saturating_sub(1) {
        indices.push(poly[0]);
        indices.push(poly[i]);
        indices.push(poly[i + 1]);
    }
}

// ─── OBJ / STL / GLTF / PLY / OFF ───────────────

fn load_obj(path: &Path) -> anyhow::Result<Mesh> {
    let (models, materials) = tobj::load_obj(
        path,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
    )?;
    let materials = materials.unwrap_or_default();
    let mut out = Mesh::default();
    for model in models {
        let mesh = model.mesh;
        if mesh.positions.is_empty() || mesh.indices.is_empty() {
            continue;
        }
        let positions: Vec<[f32; 3]> = mesh
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        let normals: Vec<[f32; 3]> = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| [c[0], c[1], c[2]])
                .collect()
        } else {
            Vec::new()
        };
        let colors = mesh
            .material_id
            .and_then(|id| materials.get(id))
            .map(|mat| {
                let d = mat.diffuse.unwrap_or([0.78, 0.80, 0.84]);
                vec![d; positions.len()]
            });
        let mut piece = Mesh {
            positions,
            normals,
            colors,
            indices: mesh.indices,
            ..Default::default()
        };
        piece.ensure_normals();
        out.append_mesh(piece, Mat4::IDENTITY);
    }
    if out.positions.is_empty() {
        anyhow::bail!("OBJ 中没有有效网格");
    }
    out.ensure_normals();
    Ok(out)
}

fn load_stl(path: &Path) -> anyhow::Result<Mesh> {
    let data = std::fs::read(path)?;
    let (positions, indices) = if is_ascii_stl(&data) {
        parse_ascii_stl(&data)?
    } else {
        parse_binary_stl(&data)?
    };
    if positions.is_empty() {
        anyhow::bail!("STL 中没有有效三角形");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

/// 二进制 STL 长度恰好为 84 + 50×三角形数；否则按 "solid" 前缀判 ASCII
fn is_ascii_stl(data: &[u8]) -> bool {
    if data.len() >= 84 {
        let count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;

        if let Some(total) = count.checked_mul(50).and_then(|x| x.checked_add(84))
            && total == data.len()
        {
            return false;
        }
    }

    let head: String = data.iter().take(256).map(|&b| b as char).collect();

    head.trim_start().to_ascii_lowercase().starts_with("solid")
}

fn read_f32_at(data: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn parse_binary_stl(data: &[u8]) -> anyhow::Result<(Vec<[f32; 3]>, Vec<u32>)> {
    if data.len() < 84 {
        anyhow::bail!("STL 文件过小");
    }

    let count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;

    let need = 84usize
        .checked_add(
            count
                .checked_mul(50)
                .ok_or_else(|| anyhow::anyhow!("STL 三角形数过大"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("STL 长度溢出"))?;

    if data.len() < need {
        anyhow::bail!("STL 二进制数据不完整");
    }

    let mut positions = Vec::with_capacity(count * 3);

    let mut indices = Vec::with_capacity(count * 3);

    let mut off = 84 + 12; // 跳过法线

    for _ in 0..count {
        for _ in 0..3 {
            positions.push([
                read_f32_at(data, off),
                read_f32_at(data, off + 4),
                read_f32_at(data, off + 8),
            ]);

            indices.push((positions.len() - 1) as u32);

            off += 12;
        }

        off += 12 + 2; // 跳过下一条法线 + 属性字节
    }

    Ok((positions, indices))
}

fn parse_ascii_stl(data: &[u8]) -> anyhow::Result<(Vec<[f32; 3]>, Vec<u32>)> {
    let text = std::str::from_utf8(data).map_err(|_| anyhow::anyhow!("STL 编码错误"))?;

    let mut positions = Vec::new();

    let mut indices = Vec::new();

    for line in text.lines() {
        let mut it = line.split_whitespace();

        if it.next() != Some("vertex") {
            continue;
        }

        let mut p = [0.0f32; 3];

        for slot in p.iter_mut() {
            let v = it
                .next()
                .ok_or_else(|| anyhow::anyhow!("STL vertex 行不完整"))?;

            *slot = v.parse().map_err(|_| anyhow::anyhow!("STL 数字解析失败"))?;
        }

        positions.push(p);

        indices.push((positions.len() - 1) as u32);
    }

    Ok((positions, indices))
}

/// 贴图尺寸上限：超过就只画几何，避免一张 8K 图把内存吃掉
pub(crate) const MAX_TEXTURE_DIM: u32 = 8192;
const MAX_TEXTURE_PIXELS: usize = 4096 * 4096;

/// gltf::import 已把内嵌图片解码成 8 位像素；这里统一成 RGBA8。
/// 少见格式（单通道、高位深）直接放弃贴图而不是猜颜色
fn decode_texture(data: &gltf::image::Data) -> Option<Texture> {
    use gltf::image::Format;
    let (w, h) = (data.width, data.height);
    if w == 0
        || h == 0
        || w > MAX_TEXTURE_DIM
        || h > MAX_TEXTURE_DIM
        || (w as usize) * (h as usize) > MAX_TEXTURE_PIXELS
    {
        return None;
    }
    let n = (w as usize) * (h as usize);
    let mut rgba = vec![0u8; n * 4];
    match data.format {
        Format::R8G8B8A8 => {
            if data.pixels.len() < n * 4 {
                return None;
            }
            rgba.copy_from_slice(&data.pixels[..n * 4]);
        }
        Format::R8G8B8 => {
            if data.pixels.len() < n * 3 {
                return None;
            }
            for i in 0..n {
                let s = &data.pixels[i * 3..i * 3 + 3];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[s[0], s[1], s[2], 255]);
            }
        }
        _ => return None,
    }
    Some(Texture {
        width: w,
        height: h,
        rgba: std::sync::Arc::new(rgba),
    })
}

/// 线性纹素 × 底色 × 光照 → 显示色（与 wgpu 里的算法一致）
#[inline]
fn texel_color(lin: [f32; 3], mul: [f32; 3], shade: f32) -> Color32 {
    Color32::from_rgb(
        (linear_to_srgb(lin[0] * mul[0] * shade) * 255.0).clamp(0.0, 255.0) as u8,
        (linear_to_srgb(lin[1] * mul[1] * shade) * 255.0).clamp(0.0, 255.0) as u8,
        (linear_to_srgb(lin[2] * mul[2] * shade) * 255.0).clamp(0.0, 255.0) as u8,
    )
}

/// gltf::import 的宽容版：外部 .bin / 贴图文件缺失时不整体失败。
/// 常见场景：Blender 导出的 .gltf 把 textures/ 目录删了或拷贝时漏掉，
/// 而几何数据（.bin）完好——查看器应当能打开模型本体。
/// 返回 (文档, buffers, images, 缺失贴图数)。
/// - buffer 缺失/过短 → 按声明 length 填零（访问器读到 0，不会 panic）
/// - image 缺失/解码失败 → 1×1 中性灰占位（材质退回 baseColorFactor 纯色）
fn import_gltf_lenient(
    path: &Path,
) -> anyhow::Result<(
    gltf::Document,
    Vec<gltf::buffer::Data>,
    Vec<gltf::image::Data>,
    usize,
)> {
    use std::fs;
    use std::io::BufReader;

    let base = path.parent().unwrap_or_else(|| Path::new("./"));
    let file = fs::File::open(path).map_err(|e| anyhow::anyhow!("无法打开 glTF 文件: {e}"))?;
    let reader = BufReader::new(file);
    let gltf = gltf::Gltf::from_reader(reader)?;
    let document = gltf.document;
    let mut blob = gltf.blob;

    // ── buffers：读失败就按 buffer.length() 补零，保证后续切片不越界 ──
    let mut buffers = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        let mut data = match gltf::buffer::Data::from_source_and_blob(
            buffer.source(),
            Some(base),
            &mut blob,
        ) {
            Ok(d) => d,
            Err(_) => gltf::buffer::Data(Vec::new()),
        };
        // gltf-rs 的 import 会校验长度并拒绝短 buffer；这里放行并补齐，
        // 让引用到缺失区段的访问器读到 0 而不是让整个加载失败
        if data.0.len() < buffer.length() {
            data.0.resize(buffer.length(), 0);
        }
        buffers.push(data);
    }

    // ── images：缺失的用 1×1 中性灰占位，材质仍按 baseColorFactor 上色 ──
    let mut missing = 0usize;
    let mut images = Vec::with_capacity(document.images().len());
    for image in document.images() {
        match gltf::image::Data::from_source(image.source(), Some(base), &buffers) {
            Ok(d) => images.push(d),
            Err(_) => {
                missing += 1;
                images.push(gltf::image::Data {
                    pixels: vec![204, 204, 204, 255],
                    format: gltf::image::Format::R8G8B8A8,
                    width: 1,
                    height: 1,
                });
            }
        }
    }
    Ok((document, buffers, images, missing))
}

fn load_gltf(path: &Path) -> anyhow::Result<(Mesh, usize)> {
    let (document, buffers, images, missing_textures) = import_gltf_lenient(path)?;
    // 同一张图被多个材质引用时只解码一次（Arc 共享，append_mesh 靠指针去重）
    let mut tex_cache: Vec<Option<std::sync::Arc<Texture>>> = vec![None; document.images().len()];
    let mut out = Mesh::default();

    fn read_node(
        node: &gltf::Node,
        parent: Mat4,
        buffers: &[gltf::buffer::Data],
        images: &[gltf::image::Data],
        tex_cache: &mut [Option<std::sync::Arc<Texture>>],
        out: &mut Mesh,
    ) -> anyhow::Result<()> {
        let local = Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
                let positions: Vec<[f32; 3]> = match reader.read_positions() {
                    Some(iter) => iter.collect(),
                    None => continue,
                };
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .map(|i| i.collect())
                    .unwrap_or_default();
                let material = primitive.material();
                let pbr = material.pbr_metallic_roughness();
                // baseColorFactor 是线性值，转成显示色存进顶点色，两条光栅化路径都用它
                let factor = pbr.base_color_factor();
                let factor_col = if factor[0] < 0.999
                    || factor[1] < 0.999
                    || factor[2] < 0.999
                    || material.alpha_mode() != gltf::material::AlphaMode::Opaque
                {
                    Some([
                        linear_to_srgb(factor[0]),
                        linear_to_srgb(factor[1]),
                        linear_to_srgb(factor[2]),
                    ])
                } else {
                    None
                };
                let mut colors: Option<Vec<[f32; 3]>> =
                    reader.read_colors(0).map(|c| c.into_rgb_f32().collect());
                if colors.is_none()
                    && let Some(c) = factor_col
                {
                    colors = Some(vec![c; positions.len()]);
                }
                // 只认 baseColorTexture： MetallicRoughness/Normal/Occlusion/Emissive 暂不接
                let mut tex: Option<std::sync::Arc<Texture>> = None;
                let mut uvs: Option<Vec<[f32; 2]>> = None;
                if let Some(info) = pbr.base_color_texture() {
                    let img = info.texture().source().index();
                    if let Some(slot) = tex_cache.get_mut(img) {
                        if slot.is_none() {
                            *slot = images
                                .get(img)
                                .and_then(decode_texture)
                                .map(std::sync::Arc::new);
                        }
                        tex = slot.clone();
                    }
                    if let Some(t) = &tex {
                        let set = info.tex_coord();
                        let read = reader
                            .read_tex_coords(set)
                            .map(|c| c.into_f32().collect::<Vec<[f32; 2]>>());
                        // 有贴图却没 UV 集时，用全 1 色当贴图会糊成一团，直接退回顶点色
                        match read {
                            Some(u) if u.len() == positions.len() && t.width > 0 => uvs = Some(u),
                            _ => tex = None,
                        }
                    }
                }
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                if !matches!(primitive.mode(), gltf::mesh::Mode::Triangles) {
                    continue;
                }
                let has_tex = tex.is_some();
                let textures: Vec<std::sync::Arc<Texture>> = tex.into_iter().collect();
                let tex_ids = has_tex.then(|| vec![0u32; positions.len()]);
                let mut piece = Mesh {
                    positions,
                    normals,
                    colors,
                    indices,
                    uvs,
                    tex_ids,
                    textures,
                };
                piece.ensure_normals();
                out.append_mesh(piece, world);
            }
        }
        for child in node.children() {
            read_node(&child, world, buffers, images, tex_cache, out)?;
        }
        Ok(())
    }

    for scene in document.scenes() {
        for node in scene.nodes() {
            read_node(
                &node,
                Mat4::IDENTITY,
                &buffers,
                &images,
                &mut tex_cache,
                &mut out,
            )?;
        }
    }
    if out.positions.is_empty() {
        for node in document.nodes() {
            read_node(
                &node,
                Mat4::IDENTITY,
                &buffers,
                &images,
                &mut tex_cache,
                &mut out,
            )?;
        }
    }
    if out.positions.is_empty() {
        anyhow::bail!("glTF 中没有三角网格");
    }
    out.ensure_normals();
    Ok((out, missing_textures))
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    let needle = b"end_header";
    let mut i = 0;
    while i + needle.len() <= data.len() {
        if &data[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < data.len() && (data[j] == b'\r' || data[j] == b'\n') {
                j += 1;
            }
            return Some(j);
        }
        i += 1;
    }
    None
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl Cursor<'_> {
    fn read_scalar(&mut self, ty: &str) -> anyhow::Result<f32> {
        let need = match ty {
            "char" | "int8" | "uchar" | "uint8" => 1,
            "short" | "int16" | "ushort" | "uint16" => 2,
            "int" | "int32" | "uint" | "uint32" | "float" | "float32" => 4,
            "double" | "float64" => 8,
            _ => anyhow::bail!("不支持的 PLY 类型 {}", ty),
        };
        if self.pos + need > self.data.len() {
            anyhow::bail!("PLY 二进制数据不足");
        }
        let bytes = &self.data[self.pos..self.pos + need];
        self.pos += need;
        let v = match ty {
            "char" | "int8" => bytes[0] as i8 as f32,
            "uchar" | "uint8" => bytes[0] as f32,
            "short" | "int16" => i16::from_ne_bytes(bytes.try_into()?) as f32,
            "ushort" | "uint16" => u16::from_ne_bytes(bytes.try_into()?) as f32,
            "int" | "int32" | "uint" | "uint32" => {
                if self.big_endian {
                    u32::from_be_bytes(bytes.try_into()?) as f32
                } else {
                    u32::from_le_bytes(bytes.try_into()?) as f32
                }
            }
            "float" | "float32" => {
                if self.big_endian {
                    f32::from_be_bytes(bytes.try_into()?)
                } else {
                    f32::from_le_bytes(bytes.try_into()?)
                }
            }
            "double" | "float64" => {
                if self.big_endian {
                    f64::from_be_bytes(bytes.try_into()?) as f32
                } else {
                    f64::from_le_bytes(bytes.try_into()?) as f32
                }
            }
            _ => unreachable!(),
        };
        Ok(v)
    }
}

fn load_ply(path: &Path) -> anyhow::Result<Mesh> {
    let data = std::fs::read(path)?;
    let header_end = find_header_end(&data).ok_or_else(|| anyhow::anyhow!("PLY 头部无效"))?;
    let header = std::str::from_utf8(&data[..header_end])
        .map_err(|_| anyhow::anyhow!("PLY 头部不是 UTF-8"))?;
    let mut format = String::from("ascii");
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut vertex_props: Vec<(String, String)> = Vec::new();
    let mut face_list: Option<(String, String)> = None;
    let mut section = String::new();

    for line in header.lines().map(|l| l.trim()) {
        let mut parts = line.split_whitespace();
        let kw = parts.next().unwrap_or("").to_ascii_lowercase();
        match kw.as_str() {
            "format" => format = parts.next().unwrap_or("ascii").to_ascii_lowercase(),
            "element" => {
                let name = parts.next().unwrap_or("").to_ascii_lowercase();
                let n: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                section = name;
                if section == "vertex" {
                    vertex_count = n;
                } else if section == "face" {
                    face_count = n;
                }
            }
            "property" => {
                if section == "vertex" {
                    let ty = parts.next().unwrap_or("").to_ascii_lowercase();
                    let name = parts.next().unwrap_or("").to_ascii_lowercase();
                    if ty != "list" {
                        vertex_props.push((name, ty));
                    }
                } else if section == "face" {
                    let ty = parts.next().unwrap_or("").to_ascii_lowercase();
                    if ty == "list" {
                        let count_ty = parts.next().unwrap_or("uchar").to_ascii_lowercase();
                        let idx_ty = parts.next().unwrap_or("int").to_ascii_lowercase();
                        face_list = Some((count_ty, idx_ty));
                    }
                }
            }
            "end_header" => break,
            _ => {}
        }
    }

    let prop_index = |name: &str| vertex_props.iter().position(|(n, _)| n == name);
    let ix = prop_index("x").ok_or_else(|| anyhow::anyhow!("PLY 缺少 x"))?;
    let iy = prop_index("y").ok_or_else(|| anyhow::anyhow!("PLY 缺少 y"))?;
    let iz = prop_index("z").ok_or_else(|| anyhow::anyhow!("PLY 缺少 z"))?;
    let ir = prop_index("red").or_else(|| prop_index("r"));
    let ig = prop_index("green").or_else(|| prop_index("g"));
    let ib = prop_index("blue").or_else(|| prop_index("b"));
    let scale_color = |v: f32| {
        if v > 1.0 {
            (v / 255.0).clamp(0.0, 1.0)
        } else {
            v.clamp(0.0, 1.0)
        }
    };

    let body = data.get(header_end..).unwrap_or(&[]);
    let mut positions = Vec::with_capacity(vertex_count);
    let mut colors = if ir.is_some() && ig.is_some() && ib.is_some() {
        Some(Vec::with_capacity(vertex_count))
    } else {
        None
    };
    let mut indices = Vec::with_capacity(face_count.saturating_mul(3));

    if format.starts_with("ascii") {
        let text = String::from_utf8_lossy(body);
        let mut iter = text.lines().filter(|l| !l.trim().is_empty());
        for _ in 0..vertex_count {
            let line = iter
                .next()
                .ok_or_else(|| anyhow::anyhow!("PLY 顶点数据不足"))?;
            let vals: Vec<&str> = line.split_whitespace().collect();
            let get = |i: usize| -> f32 { vals.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
            positions.push([get(ix), get(iy), get(iz)]);
            if let (Some(cr), Some(cg), Some(cb)) = (ir, ig, ib) {
                colors.as_mut().unwrap().push([
                    scale_color(get(cr)),
                    scale_color(get(cg)),
                    scale_color(get(cb)),
                ]);
            }
        }
        for _ in 0..face_count {
            let line = iter
                .next()
                .ok_or_else(|| anyhow::anyhow!("PLY 面数据不足"))?;
            let parts: Vec<&str> = line.split_whitespace().collect();
            let n: usize = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            if parts.len() < 1 + n {
                continue;
            }
            let idx: Vec<u32> = parts[1..=n]
                .iter()
                .filter_map(|s| s.parse::<u32>().ok())
                .collect();
            fan_triangles(&mut indices, &idx);
        }
    } else {
        let big_endian = format.contains("big");
        let mut cursor = Cursor {
            data: body,
            pos: 0,
            big_endian,
        };
        for _ in 0..vertex_count {
            let mut vals = Vec::with_capacity(vertex_props.len());
            for (_, ty) in &vertex_props {
                vals.push(cursor.read_scalar(ty)?);
            }
            positions.push([vals[ix], vals[iy], vals[iz]]);
            if let (Some(cr), Some(cg), Some(cb)) = (ir, ig, ib) {
                colors.as_mut().unwrap().push([
                    scale_color(vals[cr]),
                    scale_color(vals[cg]),
                    scale_color(vals[cb]),
                ]);
            }
        }
        let (count_ty, idx_ty) = face_list.unwrap_or_else(|| ("uchar".into(), "int".into()));
        for _ in 0..face_count {
            let n = cursor.read_scalar(&count_ty)? as usize;
            let mut idx = Vec::with_capacity(n);
            for _ in 0..n {
                idx.push(cursor.read_scalar(&idx_ty)? as u32);
            }
            fan_triangles(&mut indices, &idx);
        }
    }

    Ok(Mesh::from_positions_indices(positions, indices, colors))
}

fn load_off(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty());
    let header = lines.next().unwrap_or("");
    if !header.eq_ignore_ascii_case("off") && !header.eq_ignore_ascii_case("coff") {
        anyhow::bail!("不是 OFF 文件");
    }
    let pending: Vec<usize> = header
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let (nv, nf) = if pending.len() >= 2 {
        (pending[0], pending[1])
    } else {
        let line = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("OFF 缺少计数行"))?;
        let parts: Vec<usize> = line
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 2 {
            anyhow::bail!("OFF 计数行无效");
        }
        (parts[0], parts[1])
    };

    let mut positions = Vec::with_capacity(nv);
    for _ in 0..nv {
        let line = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("OFF 顶点不足"))?;
        let vals: Vec<f32> = line
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if vals.len() < 3 {
            anyhow::bail!("OFF 顶点坐标无效");
        }
        positions.push([vals[0], vals[1], vals[2]]);
    }
    let mut indices = Vec::with_capacity(nf * 3);
    for _ in 0..nf {
        let line = lines.next().ok_or_else(|| anyhow::anyhow!("OFF 面不足"))?;
        let parts: Vec<u32> = line
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 2 {
            continue;
        }
        let n = parts[0] as usize;
        if parts.len() < 1 + n {
            continue;
        }
        fan_triangles(&mut indices, &parts[1..=n]);
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

// ─── 3DS ───────────────────────────────────────

fn load_3ds(path: &Path) -> anyhow::Result<Mesh> {
    let data = std::fs::read(path)?;
    let mut out = Mesh::default();
    walk_3ds(&data, &mut out)?;
    if out.positions.is_empty() || out.indices.is_empty() {
        anyhow::bail!("3DS 中没有有效网格");
    }
    out.ensure_normals();
    Ok(out)
}

fn walk_3ds(data: &[u8], out: &mut Mesh) -> anyhow::Result<()> {
    let mut pos = 0usize;
    while pos + 6 <= data.len() {
        let id = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let len = u32::from_le_bytes([data[pos + 2], data[pos + 3], data[pos + 4], data[pos + 5]])
            as usize;
        if len < 6 || pos + len > data.len() {
            break;
        }
        let body = &data[pos + 6..pos + len];
        match id {
            0x4D4D | 0x3D3D | 0x4000 => {
                let child = if id == 0x4000 {
                    let nul = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                    body.get((nul + 1).min(body.len())..).unwrap_or(&[])
                } else {
                    body
                };
                walk_3ds(child, out)?;
            }
            0x4100 => {
                let mut verts: Vec<[f32; 3]> = Vec::new();
                let mut faces: Vec<u32> = Vec::new();
                let mut p = 0usize;
                while p + 6 <= body.len() {
                    let sid = u16::from_le_bytes([body[p], body[p + 1]]);
                    let slen =
                        u32::from_le_bytes([body[p + 2], body[p + 3], body[p + 4], body[p + 5]])
                            as usize;
                    if slen < 6 || p + slen > body.len() {
                        break;
                    }
                    let sb = &body[p + 6..p + slen];
                    if sid == 0x4110 && sb.len() >= 2 {
                        let n = u16::from_le_bytes([sb[0], sb[1]]) as usize;
                        let mut q = 2usize;
                        for _ in 0..n {
                            if q + 12 > sb.len() {
                                break;
                            }
                            let x = f32::from_le_bytes(sb[q..q + 4].try_into()?);
                            let y = f32::from_le_bytes(sb[q + 4..q + 8].try_into()?);
                            let z = f32::from_le_bytes(sb[q + 8..q + 12].try_into()?);
                            verts.push([x, y, z]);
                            q += 12;
                        }
                    } else if sid == 0x4120 && sb.len() >= 2 {
                        let n = u16::from_le_bytes([sb[0], sb[1]]) as usize;
                        let mut q = 2usize;
                        for _ in 0..n {
                            if q + 8 > sb.len() {
                                break;
                            }
                            let a = u16::from_le_bytes([sb[q], sb[q + 1]]) as u32;
                            let b = u16::from_le_bytes([sb[q + 2], sb[q + 3]]) as u32;
                            let c = u16::from_le_bytes([sb[q + 4], sb[q + 5]]) as u32;
                            faces.extend_from_slice(&[a, b, c]);
                            q += 8;
                        }
                    }
                    p += slen;
                }
                if !verts.is_empty() && !faces.is_empty() {
                    let base = out.positions.len() as u32;
                    let nvert = verts.len() as u32;
                    out.positions.extend(verts);
                    for i in faces {
                        if i < nvert {
                            out.indices.push(base + i);
                        }
                    }
                }
            }
            _ => {}
        }
        pos += len;
    }
    Ok(())
}

// ─── XML 工具 ───────────────────────────────────

fn find_tag(hay: &str, tag: &str, from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    let open = format!("<{}", tag);
    let i = hay[from..].find(&open)?;
    let abs = from + i + open.len();
    let rest = &hay[abs.min(hay.len())..];
    if rest.starts_with('>')
        || rest.starts_with(' ')
        || rest.starts_with('\t')
        || rest.starts_with('\n')
        || rest.starts_with('\r')
        || rest.starts_with('/')
    {
        Some(hay[abs..].find('>').map(|g| abs + g + 1).unwrap_or(abs))
    } else {
        find_tag(hay, tag, abs)
    }
}

fn find_close_tag(hay: &str, tag: &str, from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    let close = format!("</{}", tag);
    hay[from..].find(&close).map(|i| from + i)
}

fn find_open_tag_span(hay: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    if from >= hay.len() {
        return None;
    }
    let open = format!("<{}", tag);
    let mut from = from;
    while from < hay.len() {
        let i = hay[from..].find(&open)?;
        let abs = from + i + open.len();
        let rest = &hay[abs.min(hay.len())..];
        let ok = rest.starts_with('>')
            || rest.starts_with(' ')
            || rest.starts_with('\t')
            || rest.starts_with('\n')
            || rest.starts_with('\r')
            || rest.starts_with('/');
        if ok {
            let end = hay[abs..].find('>').map(|g| abs + g + 1)?;
            return Some((from + i, end));
        }
        from = abs;
    }
    None
}

fn attr_value(s: &str, name: &str) -> Option<String> {
    let pat = format!("{}=\"", name);
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    let end = rest.find('"')?;
    Some(rest[..end].trim().to_string())
}

fn attr_value_ci(s: &str, name: &str) -> Option<String> {
    let name_l = name.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < s.len() {
        if i + name_l.len() <= s.len() && s[i..i + name_l.len()].eq_ignore_ascii_case(&name_l) {
            let after = i + name_l.len();
            let ok_prev = i == 0 || {
                let c = bytes[i - 1];
                !(c.is_ascii_alphanumeric() || c == b'_')
            };
            let mut j = after;
            while j < s.len()
                && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r')
            {
                j += 1;
            }
            if ok_prev && j < s.len() && bytes[j] == b'=' {
                j += 1;
                while j < s.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < s.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let q = bytes[j];
                    let start = j + 1;
                    if let Some(rel) = s[start..].find(q as char) {
                        return Some(s[start..start + rel].trim().to_string());
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn extract_float_array(block: &str) -> Option<Vec<f32>> {
    let lower = block.to_ascii_lowercase();
    let start = lower.find("<float_array")?;
    let content_start = lower[start..].find('>').map(|i| start + i + 1)?;
    let content_end = lower[content_start..]
        .find("</float_array>")
        .map(|i| content_start + i)?;
    Some(
        block[content_start..content_end]
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect(),
    )
}

// ─── DAE ───────────────────────────────────────

fn load_dae(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let lower = text.to_ascii_lowercase();
    let mut out = Mesh::default();
    let mut search_from = 0usize;
    let mut found = false;
    while let Some(mesh_start) = find_tag(&lower, "mesh", search_from) {
        let mesh_end = find_close_tag(&lower, "mesh", mesh_start).unwrap_or(lower.len());
        let block = &text[mesh_start.min(text.len())..mesh_end.min(text.len())];
        let block_l = &lower[mesh_start.min(lower.len())..mesh_end.min(lower.len())];
        if let Some((positions, indices)) = parse_dae_mesh_block(block, block_l) {
            let base = out.positions.len() as u32;
            let n = positions.len();
            out.positions.extend(positions);
            for _ in 0..n {
                out.normals.push([0.0, 0.0, 0.0]);
            }
            for i in indices {
                out.indices.push(base + i);
            }
            found = true;
        }
        search_from = mesh_end.max(search_from + 1);
    }
    if !found || out.positions.is_empty() {
        anyhow::bail!("DAE 中没有有效 mesh");
    }
    out.ensure_normals();
    Ok(out)
}

fn parse_dae_mesh_block(block: &str, block_l: &str) -> Option<(Vec<[f32; 3]>, Vec<u32>)> {
    let mut sources: Vec<(String, Vec<f32>)> = Vec::new();
    let mut src_search = 0usize;
    while let Some(s) = find_tag(block_l, "source", src_search) {
        let e = find_close_tag(block_l, "source", s).unwrap_or(block_l.len());
        let sb = &block[s.min(block.len())..e.min(block.len())];
        let sbl = &block_l[s.min(block_l.len())..e.min(block_l.len())];
        let id = attr_value(sbl, "id").unwrap_or_default();
        if let Some(fa) = extract_float_array(sb) {
            sources.push((id, fa));
        }
        src_search = e.max(s + 1);
    }

    let mut pos_source_id = String::new();
    if let Some(vs) = find_tag(block_l, "vertices", 0) {
        let ve = find_close_tag(block_l, "vertices", vs).unwrap_or(block_l.len());
        let vb = &block_l[vs..ve];
        if let Some(idx) = vb.find("position") {
            let rest = &vb[idx..];
            if let Some(src) = rest.find("source") {
                let after = &rest[src..];
                if let Some(hash) = after.find('#') {
                    let end = after[hash..]
                        .find(|c: char| c == '"' || c == '\'' || c.is_whitespace())
                        .map(|i| hash + i)
                        .unwrap_or(after.len());
                    pos_source_id = after[hash + 1..end].to_string();
                }
            }
        }
    }

    let raw = sources
        .iter()
        .find(|(id, _)| *id == pos_source_id || id.to_ascii_lowercase().contains("position"))
        .map(|(_, f)| f.clone())
        .or_else(|| {
            sources
                .iter()
                .find(|(_, f)| f.len() % 3 == 0 && f.len() >= 9)
                .map(|(_, f)| f.clone())
        })?;

    let mut indices = Vec::new();
    for tag in ["triangles", "polylist", "polygons"] {
        let mut tsearch = 0usize;
        while let Some(t) = find_tag(block_l, tag, tsearch) {
            let te = find_close_tag(block_l, tag, t).unwrap_or(block_l.len());
            let tb = &block[t.min(block.len())..te.min(block.len())];
            let tbl = &block_l[t.min(block_l.len())..te.min(block_l.len())];
            let mut vertex_offset = 0usize;
            let mut max_offset = 0usize;
            let mut input_search = 0usize;
            while let Some(inp) = tbl[input_search..].find("<input") {
                let inp_abs = input_search + inp;
                let inp_end = tbl[inp_abs..]
                    .find('>')
                    .map(|i| inp_abs + i + 1)
                    .unwrap_or(inp_abs + 20);
                let piece = &tbl[inp_abs..inp_end.min(tbl.len())];
                let sem = attr_value(piece, "semantic").unwrap_or_default();
                let off: usize = attr_value(piece, "offset")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                max_offset = max_offset.max(off);
                if sem.eq_ignore_ascii_case("vertex") || sem.eq_ignore_ascii_case("vertices") {
                    vertex_offset = off;
                }
                input_search = inp_end;
            }
            let stride = max_offset + 1;
            let p_idx = tb
                .find("<p>")
                .or_else(|| tb.find("<p "))
                .and_then(|i| tb[i..].find('>').map(|g| i + g + 1));
            let p_end = tb.find("</p>");
            if let (Some(ps), Some(pe)) = (p_idx, p_end) {
                let nums: Vec<usize> = tb[ps..pe]
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if tag == "polylist" {
                    let vc = tb
                        .find("<vcount")
                        .and_then(|i| tb[i..].find('>').map(|g| i + g + 1))
                        .and_then(|start| {
                            tb[start..].find("</vcount>").map(|e| &tb[start..start + e])
                        });
                    let counts: Vec<usize> = vc
                        .map(|t| {
                            t.split_whitespace()
                                .filter_map(|s| s.parse().ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut cursor = 0usize;
                    for &c in &counts {
                        let mut poly = Vec::with_capacity(c);
                        for _ in 0..c {
                            let vi = cursor * stride + vertex_offset;
                            if vi < nums.len() {
                                poly.push(nums[vi] as u32);
                            }
                            cursor += 1;
                        }
                        fan_triangles(&mut indices, &poly);
                    }
                } else {
                    let tri_count = nums.len() / (stride * 3);
                    for t in 0..tri_count {
                        for k in 0..3 {
                            let vi = (t * 3 + k) * stride + vertex_offset;
                            if vi < nums.len() {
                                indices.push(nums[vi] as u32);
                            }
                        }
                    }
                }
            }
            tsearch = te.max(t + 1);
        }
    }

    let positions: Vec<[f32; 3]> = raw
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let n = positions.len();
    indices.retain(|i| (*i as usize) < n);
    if positions.is_empty() || indices.is_empty() {
        return None;
    }
    Some((positions, indices))
}

// ─── VTK ───────────────────────────────────────

fn load_vtk(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines().map(|l| l.trim());
    let mut section = String::new();
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for l in lines.by_ref() {
        if l.is_empty() {
            continue;
        }
        let u = l.to_ascii_uppercase();
        if u.starts_with("POINTS") {
            section = "points".into();
            continue;
        }
        if u.starts_with("POLYGONS") || u.starts_with("TRIANGLESTRIPS") {
            section = "polys".into();
            continue;
        }
        if u.starts_with("VERTICES")
            || u.starts_with("LINES")
            || u.starts_with("FIELD")
            || u.starts_with("POINT_DATA")
            || u.starts_with("CELL_DATA")
            || u.starts_with("DATASET")
            || u.starts_with("CELL_TYPES")
        {
            section = "other".into();
            continue;
        }
        match section.as_str() {
            "points" => {
                let vals: Vec<f32> = l
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                for c in vals.as_chunks::<3>().0 {
                    positions.push([c[0], c[1], c[2]]);
                }
            }
            "polys" => {
                let nums: Vec<u32> = l
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if nums.is_empty() {
                    continue;
                }
                let n = nums[0] as usize;
                if nums.len() > n {
                    fan_triangles(&mut indices, &nums[1..=n]);
                }
            }
            _ => {}
        }
    }
    if positions.is_empty() || indices.is_empty() {
        anyhow::bail!("VTK 中没有有效多边形");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

// ─── WRL ───────────────────────────────────────

fn load_wrl(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let lower = text.to_ascii_lowercase();
    let positions = {
        let pc = lower
            .find("point")
            .and_then(|i| lower[i..].find('[').map(|j| i + j + 1));
        let pe = pc.and_then(|s| lower[s..].find(']').map(|j| s + j));
        match (pc, pe) {
            (Some(s), Some(e)) => {
                let nums: Vec<f32> = text[s..e]
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter_map(|t| t.trim().parse().ok())
                    .collect();
                nums.as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| [c[0], c[1], c[2]])
                    .collect()
            }
            _ => Vec::new(),
        }
    };
    let mut indices = Vec::new();
    let ic = lower
        .find("coordindex")
        .and_then(|i| lower[i..].find('[').map(|j| i + j + 1));
    let ie = ic.and_then(|s| lower[s..].find(']').map(|j| s + j));
    if let (Some(s), Some(e)) = (ic, ie) {
        let nums: Vec<i32> = text[s..e]
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter_map(|t| t.trim().parse().ok())
            .collect();
        let mut poly: Vec<u32> = Vec::new();
        for v in nums {
            if v < 0 {
                fan_triangles(&mut indices, &poly);
                poly.clear();
            } else {
                poly.push(v as u32);
            }
        }
        if !poly.is_empty() {
            fan_triangles(&mut indices, &poly);
        }
    }
    if positions.is_empty() || indices.is_empty() {
        anyhow::bail!("WRL 中没有有效网格");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

// ─── AMF ───────────────────────────────────────

fn load_amf(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    let lower = text.to_ascii_lowercase();
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut msearch = 0usize;
    while let Some(ms) = find_tag(&lower, "mesh", msearch) {
        let me = find_close_tag(&lower, "mesh", ms).unwrap_or(lower.len());
        let mb = &text[ms.min(text.len())..me.min(text.len())];
        let mbl = &lower[ms.min(lower.len())..me.min(lower.len())];
        let base = positions.len() as u32;
        let mut xsearch = 0usize;
        while let Some(vtag) = find_tag(mbl, "vertex", xsearch) {
            let vte = find_close_tag(mbl, "vertex", vtag).unwrap_or(mbl.len());
            let chunk = &mb[vtag.min(mb.len())..vte.min(mb.len())];
            let cl = &mbl[vtag.min(mbl.len())..vte.min(mbl.len())];
            let get = |name: &str| -> Option<f32> {
                let t = find_tag(cl, name, 0)?;
                let e = find_close_tag(cl, name, t).unwrap_or(cl.len());
                chunk[t.min(chunk.len())..e.min(chunk.len())]
                    .trim()
                    .parse()
                    .ok()
            };
            if let (Some(x), Some(y), Some(z)) = (get("x"), get("y"), get("z")) {
                positions.push([x, y, z]);
            }
            xsearch = vte.max(vtag + 1);
        }
        let mut tsearch = 0usize;
        while let Some(ttag) = find_tag(mbl, "triangle", tsearch) {
            let tte = find_close_tag(mbl, "triangle", ttag).unwrap_or(mbl.len());
            let chunk = &mb[ttag.min(mb.len())..tte.min(mb.len())];
            let cl = &mbl[ttag.min(mbl.len())..tte.min(mbl.len())];
            let get_u = |name: &str| -> Option<u32> {
                let t = find_tag(cl, name, 0)?;
                let e = find_close_tag(cl, name, t).unwrap_or(cl.len());
                chunk[t.min(chunk.len())..e.min(chunk.len())]
                    .trim()
                    .parse()
                    .ok()
            };
            if let (Some(a), Some(b), Some(c)) = (get_u("v1"), get_u("v2"), get_u("v3")) {
                indices.extend_from_slice(&[base + a, base + b, base + c]);
            }
            tsearch = tte.max(ttag + 1);
        }
        msearch = me.max(ms + 1);
    }
    if positions.is_empty() || indices.is_empty() {
        anyhow::bail!("AMF 中没有有效网格");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

// ─── 3MF ───────────────────────────────────────

fn load_3mf(path: &Path) -> anyhow::Result<Mesh> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| anyhow::anyhow!("3MF 不是有效 ZIP: {}", e))?;
    let mut names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(z) = archive.by_index(i) {
            let n = z.name().to_string();
            if n.to_ascii_lowercase().ends_with(".model") {
                names.push(n);
            }
        }
    }
    names.sort_by_key(|n| {
        let l = n.to_ascii_lowercase();
        if l == "3d/3dmodel.model" {
            0u8
        } else if l.ends_with("3dmodel.model") {
            1
        } else {
            2
        }
    });
    if names.is_empty() {
        anyhow::bail!("3MF 中未找到 .model 文件");
    }
    let mut out = Mesh::default();
    let mut any = false;
    for name in &names {
        let mut s = String::new();
        {
            let mut z = archive
                .by_name(name)
                .map_err(|e| anyhow::anyhow!("读取 {} 失败: {}", name, e))?;
            std::io::Read::read_to_string(&mut z, &mut s)?;
        }
        let s = s.strip_prefix('\u{feff}').unwrap_or(&s);
        if instantiate_3mf(s, &mut out) {
            any = true;
        }
    }
    if !any || out.positions.is_empty() || out.indices.is_empty() {
        anyhow::bail!("3MF 中没有有效网格");
    }
    let n = out.positions.len();
    out.indices.retain(|i| (*i as usize) < n);
    if out.indices.is_empty() {
        anyhow::bail!("3MF 索引越界");
    }
    out.ensure_normals();
    Ok(out)
}

struct ThreemfObj {
    mesh: Option<Mesh>,
    components: Vec<(String, Mat4)>,
}

fn instantiate_3mf(xml: &str, out: &mut Mesh) -> bool {
    use std::collections::HashMap;
    let lower = xml.to_ascii_lowercase();
    let mut objects: HashMap<String, ThreemfObj> = HashMap::new();

    let mut osearch = 0usize;
    while let Some((os, oe_tag)) = find_open_tag_span(&lower, "object", osearch) {
        let oe = find_close_tag(&lower, "object", oe_tag).unwrap_or(lower.len());
        let open_tag = &xml[os.min(xml.len())..oe_tag.min(xml.len()).max(os)];
        let body = &xml[oe_tag.min(xml.len())..oe.min(xml.len())];
        let body_l = &lower[oe_tag.min(lower.len())..oe.min(lower.len())];
        let id = attr_value_ci(open_tag, "id").unwrap_or_else(|| format!("anon{}", objects.len()));
        let mesh = parse_3mf_mesh_block(body, body_l);
        let mut components = Vec::new();
        let mut csearch = 0usize;
        while let Some((cs, ce)) = find_open_tag_span(body_l, "component", csearch) {
            let tag = &body[cs.min(body.len())..ce.min(body.len()).max(cs)];
            if let Some(oid) = attr_value_ci(tag, "objectid") {
                let tr = attr_value_ci(tag, "transform").unwrap_or_default();
                components.push((oid, parse_3mf_transform(&tr)));
            }
            csearch = ce.max(csearch + 1);
        }
        objects.entry(id).or_insert(ThreemfObj { mesh, components });
        osearch = oe_tag.max(os + 1);
    }

    let mut build: Vec<(String, Mat4)> = Vec::new();
    let mut isearch = 0usize;
    while let Some((is0, ie0)) = find_open_tag_span(&lower, "item", isearch) {
        let tag = &xml[is0.min(xml.len())..ie0.min(xml.len()).max(is0)];
        if let Some(oid) = attr_value_ci(tag, "objectid") {
            let tr = attr_value_ci(tag, "transform").unwrap_or_default();
            build.push((oid, parse_3mf_transform(&tr)));
        }
        isearch = ie0.max(isearch + 1);
    }

    let before = out.indices.len();
    if !build.is_empty() {
        for (id, xf) in &build {
            inst_3mf(&objects, id, *xf, out, 0);
        }
    } else {
        for id in objects.keys() {
            inst_3mf(&objects, id, Mat4::IDENTITY, out, 0);
        }
    }
    out.indices.len() > before
}

fn inst_3mf(
    objects: &std::collections::HashMap<String, ThreemfObj>,
    id: &str,
    xform: Mat4,
    out: &mut Mesh,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Some(obj) = objects.get(id) else { return };
    if let Some(mesh) = &obj.mesh {
        out.append_mesh(mesh.clone(), xform);
    }
    for (cid, local) in &obj.components {
        inst_3mf(objects, cid, xform * *local, out, depth + 1);
    }
}

fn parse_3mf_transform(s: &str) -> Mat4 {
    let n: Vec<f32> = s
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if n.len() < 12 {
        return Mat4::IDENTITY;
    }
    Mat4::from_cols_array(&[
        n[0], n[3], n[6], 0.0, //
        n[1], n[4], n[7], 0.0, //
        n[2], n[5], n[8], 0.0, //
        n[9], n[10], n[11], 1.0,
    ])
}

fn parse_3mf_mesh_block(mb: &str, mbl: &str) -> Option<Mesh> {
    let mut mesh_xml: Option<(usize, usize)> = None;
    if let Some(ms) = find_tag(mbl, "mesh", 0) {
        let me = find_close_tag(mbl, "mesh", ms).unwrap_or(mbl.len());
        mesh_xml = Some((ms, me));
    }
    let (body, body_l) = if let Some((a, b)) = mesh_xml {
        (
            &mb[a.min(mb.len())..b.min(mb.len())],
            &mbl[a.min(mbl.len())..b.min(mbl.len())],
        )
    } else {
        (mb, mbl)
    };
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut vsearch = 0usize;
    while let Some((vs, ve)) = find_open_tag_span(body_l, "vertex", vsearch) {
        let tag_text = &body[vs.min(body.len())..ve.min(body.len()).max(vs)];
        let x = attr_value_ci(tag_text, "x").and_then(|s| s.parse().ok());
        let y = attr_value_ci(tag_text, "y").and_then(|s| s.parse().ok());
        let z = attr_value_ci(tag_text, "z").and_then(|s| s.parse().ok());
        if let (Some(x), Some(y), Some(z)) = (x, y, z) {
            positions.push([x, y, z]);
        }
        vsearch = ve.max(vsearch + 1);
    }
    let mut tsearch = 0usize;
    while let Some((ts, te)) = find_open_tag_span(body_l, "triangle", tsearch) {
        let tag_text = &body[ts.min(body.len())..te.min(body.len()).max(ts)];
        let a = attr_value_ci(tag_text, "v1").and_then(|s| s.parse::<u32>().ok());
        let b = attr_value_ci(tag_text, "v2").and_then(|s| s.parse::<u32>().ok());
        let c = attr_value_ci(tag_text, "v3").and_then(|s| s.parse::<u32>().ok());
        if let (Some(a), Some(b), Some(c)) = (a, b, c) {
            indices.extend_from_slice(&[a, b, c]);
        }
        tsearch = te.max(tsearch + 1);
    }
    if positions.is_empty() || indices.is_empty() {
        return None;
    }
    let n = positions.len();
    indices.retain(|i| (*i as usize) < n);
    if indices.is_empty() {
        return None;
    }
    Some(Mesh {
        positions,
        normals: Vec::new(),
        colors: None,
        indices,
        ..Default::default()
    })
}

// ─── GCODE ─────────────────────────────────────

fn load_gcode(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path)?;
    #[derive(Clone, Copy)]
    struct Pos {
        x: f32,
        y: f32,
        z: f32,
        e: f32,
    }
    let mut cur = Pos {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        e: 0.0,
    };
    let mut absolute = true;
    let mut absolute_e = true;
    let mut extruding;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut half_w = 0.2f32;
    let mut layer_h = 0.2f32;

    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("").to_ascii_uppercase();
        match cmd.as_str() {
            "G90" => {
                absolute = true;
                absolute_e = true;
            }
            "G91" => {
                absolute = false;
                absolute_e = false;
            }
            "M82" => absolute_e = true,
            "M83" => absolute_e = false,
            "G92" => {
                for p in parts {
                    if let Some(v) = p.strip_prefix('E').or_else(|| p.strip_prefix('e'))
                        && let Ok(v) = v.parse::<f32>()
                    {
                        cur.e = v;
                    }
                }
            }
            "G0" | "G00" | "G1" | "G01" => {
                let mut nx = cur.x;
                let mut ny = cur.y;
                let mut nz = cur.z;
                let mut ne = cur.e;
                let mut has_e = false;
                let mut has_xyz = false;
                for p in parts {
                    let (k, v) = p.split_at(1);
                    let k = k.to_ascii_uppercase();
                    let val: f32 = match v.parse() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    match k.as_str() {
                        "X" => {
                            nx = if absolute { val } else { cur.x + val };
                            has_xyz = true;
                        }
                        "Y" => {
                            ny = if absolute { val } else { cur.y + val };
                            has_xyz = true;
                        }
                        "Z" => {
                            nz = if absolute { val } else { cur.z + val };
                            has_xyz = true;
                        }
                        "E" => {
                            ne = if absolute_e { val } else { cur.e + val };
                            has_e = true;
                        }
                        _ => {}
                    }
                }
                if has_e {
                    let de = ne - cur.e;
                    extruding = de > 1e-6 && has_xyz;
                    let dx = nx - cur.x;
                    let dy = ny - cur.y;
                    let dz = nz - cur.z;
                    let len = (dx * dx + dy * dy + dz * dz).sqrt();
                    if extruding && len > 1e-4 {
                        let w = (de / len * 1.5).clamp(0.15, 0.8);
                        half_w = half_w * 0.9 + w * 0.5 * 0.1;
                        layer_h = dz.abs().clamp(0.12, 0.4);
                    }
                } else {
                    extruding = false;
                }
                if extruding && has_xyz {
                    let (dx, dy, dz) = (nx - cur.x, ny - cur.y, nz - cur.z);
                    let len = (dx * dx + dy * dy + dz * dz).sqrt();
                    if len > 1e-4 {
                        push_extrude_box(
                            &mut positions,
                            &mut indices,
                            [cur.x, cur.y, cur.z],
                            [nx, ny, nz],
                            half_w,
                            layer_h.max(0.12),
                        );
                    }
                }
                cur = Pos {
                    x: nx,
                    y: ny,
                    z: nz,
                    e: ne,
                };
            }
            _ => {}
        }
    }
    if positions.is_empty() {
        anyhow::bail!("GCODE 中未解析到挤出路径");
    }
    if indices.len() > 2_400_000 {
        indices.truncate(2_400_000);
        let max_i = indices.iter().copied().max().unwrap_or(0) as usize + 1;
        positions.truncate(max_i);
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

fn push_extrude_box(
    positions: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    a: [f32; 3],
    b: [f32; 3],
    half_w: f32,
    height: f32,
) {
    let va = Vec3::from(a);
    let vb = Vec3::from(b);
    let dir = vb - va;
    let len = dir.length();
    if len < 1e-6 {
        return;
    }
    let fwd = dir / len;
    let mut side = fwd.cross(Vec3::Z);
    if side.length_squared() < 1e-8 {
        side = fwd.cross(Vec3::Y);
    }
    let side = side.normalize_or_zero() * half_w;
    let up = Vec3::Z * (height * 0.5);
    let base = positions.len() as u32;
    let corners = [
        va - side - up,
        va + side - up,
        va + side + up,
        va - side + up,
        vb - side - up,
        vb + side - up,
        vb + side + up,
        vb - side + up,
    ];
    for c in corners {
        positions.push([c.x, c.y, c.z]);
    }
    let faces: [[u32; 3]; 12] = [
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
        [0, 3, 2],
        [0, 2, 1],
        [4, 5, 6],
        [4, 6, 7],
    ];
    for f in faces {
        indices.extend_from_slice(&[base + f[0], base + f[1], base + f[2]]);
    }
}

// ─── FBX / PMX / USD ───────────────────────────

fn load_fbx(path: &Path) -> anyhow::Result<Mesh> {
    let data = std::fs::read(path)?;
    if data.starts_with(b"Kaydara FBX Binary") {
        return load_fbx_binary(&data);
    }
    let text = String::from_utf8_lossy(&data);
    load_fbx_ascii(&text)
}

fn load_fbx_ascii(text: &str) -> anyhow::Result<Mesh> {
    let mut flat: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut mode = 0u8;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Vertices:") {
            mode = 1;
            if let Some((_, rest)) = line.split_once(':') {
                collect_fbx_floats(rest, &mut flat);
            }
            continue;
        }
        if line.starts_with("PolygonVertexIndex:") {
            mode = 2;
            if let Some((_, rest)) = line.split_once(':') {
                collect_fbx_poly(rest, &mut indices);
            }
            continue;
        }
        if line.starts_with('*') {
            match mode {
                1 => collect_fbx_floats(line, &mut flat),
                2 => collect_fbx_poly(line, &mut indices),
                _ => {}
            }
        }
    }
    if flat.len() < 9 || indices.is_empty() {
        anyhow::bail!("FBX 中未解析到网格，请导出为 OBJ/GLB/STL");
    }
    let verts: Vec<[f32; 3]> = flat
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let n = verts.len() as u32;
    indices.retain(|i| *i < n);
    if indices.is_empty() {
        anyhow::bail!("FBX 索引无效");
    }
    Ok(Mesh::from_positions_indices(verts, indices, None))
}

fn collect_fbx_floats(s: &str, out: &mut Vec<f32>) {
    let s = s.trim().trim_start_matches('*').trim();
    for t in s.split(|c: char| c == ',' || c.is_whitespace()) {
        if let Ok(v) = t.parse::<f32>() {
            out.push(v);
        }
    }
}

fn collect_fbx_poly(s: &str, out: &mut Vec<u32>) {
    let s = s.trim().trim_start_matches('*').trim();
    let mut poly: Vec<u32> = Vec::new();
    for t in s.split(|c: char| c == ',' || c.is_whitespace()) {
        if let Ok(v) = t.parse::<i32>() {
            if v < 0 {
                poly.push((-v - 1) as u32);
                fan_triangles(out, &poly);
                poly.clear();
            } else {
                poly.push(v as u32);
            }
        }
    }
}

// ─── FBX 二进制节点树解析 ───
// 参考 Autodesk FBX SDK 文档 BinaryFileFormat.pdf：
// 节点 = 结束偏移 + 属性计数 + 属性列表长度 + 名字 + 属性 + 子节点，
// 子节点块以一条结束偏移为 0 的空记录收尾。
// 实测导出器有两套写法：Blender 把 Vertices / PolygonVertexIndex 直接作为
// Geometry 的子节点（数组大且 zlib 压缩），老式 SDK 把它们放在 Properties70
// 的 P 记录里。负数收尾的 PolygonVertexIndex 表示一个多边形结束。

enum FbxProp {
    Str(String),
    F64s(Vec<f64>),
    I32s(Vec<i32>),
    I64s(Vec<i64>),
    /// 标量整型：Model/Geometry 的对象 ID、Connections 的连接关系都用它
    I64Scalar(i64),
    Other,
}

struct FbxCur<'a> {
    d: &'a [u8],
    p: usize,
    ver: u32,
}

impl<'a> FbxCur<'a> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.d.get(self.p)?;
        self.p += 1;
        Some(v)
    }

    fn u16(&mut self) -> Option<u16> {
        let b: [u8; 2] = self.d.get(self.p..self.p + 2)?.try_into().ok()?;
        self.p += 2;
        Some(u16::from_le_bytes(b))
    }

    fn u32(&mut self) -> Option<u32> {
        let b: [u8; 4] = self.d.get(self.p..self.p + 4)?.try_into().ok()?;
        self.p += 4;
        Some(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Option<u64> {
        let b: [u8; 8] = self.d.get(self.p..self.p + 8)?.try_into().ok()?;
        self.p += 8;
        Some(u64::from_le_bytes(b))
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }

    /// 版本 ≥7500 用 64 位结束偏移，否则 32 位
    fn off(&mut self) -> Option<usize> {
        if self.ver >= 7500 {
            Some(self.u64()? as usize)
        } else {
            Some(self.u32()? as usize)
        }
    }

    fn off_size(&self) -> usize {
        if self.ver >= 7500 { 8 } else { 4 }
    }

    /// 节点头里的 NumProperties / PropertyListLen：FBX ≥7500 是 64 位，之前是 32 位。
    /// 按 32 位读会把前一个字段的高 4 字节（恒 0）当成后一个字段，
    /// 于是 PropertyListLen 变成 0、属性全被跳过（7.5+ 文件因此「找不到网格」）。
    fn count_field(&mut self) -> Option<u64> {
        if self.ver >= 7500 {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    fn read_property(&mut self) -> Option<FbxProp> {
        let t = self.u8()?;
        match t {
            b'C' => {
                self.u8()?;
                Some(FbxProp::Other)
            }
            b'Y' => {
                self.u16()?;
                Some(FbxProp::Other)
            }
            b'B' => {
                self.bytes(1)?;
                Some(FbxProp::Other)
            }
            b'F' => {
                self.bytes(4)?;
                Some(FbxProp::Other)
            }
            b'D' => {
                self.bytes(8)?;
                Some(FbxProp::Other)
            }
            b'L' => {
                let v = self.u64()? as i64;
                Some(FbxProp::I64Scalar(v))
            }
            b'I' => {
                let _ = self.u32()?;
                Some(FbxProp::Other)
            }
            b'S' | b'R' => {
                let n = self.u32()? as usize;
                let s = self.bytes(n)?;
                Some(FbxProp::Str(String::from_utf8_lossy(s).into_owned()))
            }
            b'f' | b'd' | b'i' | b'l' | b'y' | b'c' | b'b' => self.read_array(t),
            _ => None,
        }
    }

    /// 数组属性：头是 元素个数 + 编码 + 存储字节数，负载按存储长度前进。
    /// 实测 Blender/Kaydara 导出即为此顺序（encoding=1 时第三个字段是压缩后长度）
    fn read_array(&mut self, t: u8) -> Option<FbxProp> {
        let count = self.u32()? as usize;
        let encoding = self.u32()?;
        let stored_size = self.u32()? as usize;
        let esz = match t {
            b'f' | b'i' => 4usize,
            b'd' | b'l' => 8,
            b'y' => 2,
            b'c' | b'b' => 1,
            _ => return None,
        };
        let raw_len = count.checked_mul(esz)?;
        if raw_len > 2_000_000_000 {
            return None;
        }
        // 未压缩时 ArrayDataSize 常为 0，按元素数推算；压缩时它就是压缩后长度
        let stored = if encoding == 0 { raw_len } else { stored_size };
        let slice = self.bytes(stored)?;
        let decoded: std::borrow::Cow<'a, [u8]> = match encoding {
            0 => slice.into(),
            1 => {
                let mut stream = flate2::read::ZlibDecoder::new(slice);
                let mut buf = Vec::with_capacity(raw_len.min(64 * 1024 * 1024));
                std::io::Read::read_to_end(&mut stream, &mut buf).ok()?;
                buf.into()
            }
            _ => return None, // LZMA：极少见，放弃
        };
        let src = decoded.as_ref();
        if src.len() < raw_len {
            return None;
        }
        match t {
            b'f' => Some(FbxProp::F64s(
                src.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b) as f64)
                    .collect(),
            )),
            b'd' => Some(FbxProp::F64s(
                src.as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| f64::from_le_bytes(*b))
                    .collect(),
            )),
            b'i' => Some(FbxProp::I32s(
                src.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| i32::from_le_bytes(*b))
                    .collect(),
            )),
            b'l' => Some(FbxProp::I64s(
                src.as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| i64::from_le_bytes(*b))
                    .collect(),
            )),
            _ => Some(FbxProp::Other),
        }
    }
}

/// 解析一个节点：返回（名字、属性、子节点起始位置）
fn fbx_read_node(c: &mut FbxCur, end: usize) -> Option<(String, Vec<FbxProp>, usize)> {
    let node_end = c.off()?;
    let _num_props = c.count_field()?;
    let prop_list_len = c.count_field()? as usize;
    let name_len = c.u8()? as usize;
    if node_end <= c.p || node_end > end {
        return None;
    }
    let name = String::from_utf8_lossy(c.bytes(name_len)?).into_owned();
    let mut props = Vec::new();
    if prop_list_len > 0 {
        // 编码块（版本 ≥7500）：整个属性列表 zlib 压缩，先解压再解析
        let stop = (c.p + prop_list_len).min(end);
        let raw: &[u8] = &c.d[c.p.min(c.d.len())..stop];
        let mut decompressed: Option<Vec<u8>> = None;
        if c.ver >= 7500 && raw.first() == Some(&1) && raw.get(1) == Some(&0x78) {
            let mut stream = flate2::read::ZlibDecoder::new(&raw[1..]);
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut buf).ok()?;
            decompressed = Some(buf);
        }
        c.p = stop;
        let list: &[u8] = decompressed.as_deref().unwrap_or(raw);
        let mut inner = FbxCur {
            d: list,
            p: 0,
            ver: c.ver,
        };
        // 属性列表以 0x00 结束；解析失败即停，剩余字节交给子节点层
        while inner.d.get(inner.p) != Some(&0) {
            match inner.read_property() {
                Some(p) => props.push(p),
                None => break,
            }
        }
    }
    Some((name, props, c.p))
}

impl FbxProp {
    fn f64s(&self) -> Option<&[f64]> {
        match self {
            FbxProp::F64s(v) => Some(v),
            _ => None,
        }
    }

    fn i32s(&self) -> Option<Vec<i32>> {
        match self {
            FbxProp::I32s(v) => Some(v.clone()),
            FbxProp::I64s(v) => Some(v.iter().map(|&x| x as i32).collect()),
            _ => None,
        }
    }
}

/// 一个 Geometry 子树里累积的网格数据：顶点与索引往往分散在不同节点，
/// 要收集齐了才结算成一块网格。poly_idx 保留原始的 PolygonVertexIndex
/// （负数收尾表示多边形结束），三角化留给结算时做。
#[derive(Default)]
struct FbxGeom {
    id: i64,
    verts: Option<Vec<[f32; 3]>>,
    poly_idx: Option<Vec<i32>>,
    normals: Option<FbxNormals>,
}

/// FBX 法线层的三种映射方式（MappingInformationType）
enum FbxNormals {
    /// ByPolygonVertex：每个多边形角一条法线（Blender/Maya 默认）
    PerCorner(Vec<[f32; 3]>),
    /// ByVertice：每个顶点一条
    PerVertex(Vec<[f32; 3]>),
    /// ByPolygon：每个多边形一条（整体 Flat）
    PerPoly(Vec<[f32; 3]>),
}

/// 结算完成的网格数据：等 Connections 解析出 Model 变换后统一处理
struct FbxGeomData {
    id: i64,
    verts: Vec<[f32; 3]>,
    poly_idx: Vec<i32>,
    normals: Option<FbxNormals>,
}

/// FBX 对象节点（Model）：局部 TRS + 父节点 ID（-1 = 无父）
struct FbxModel {
    id: i64,
    local: Mat4,
    parent: i64,
}

fn fbx_verts_from(props: &[FbxProp]) -> Option<Vec<[f32; 3]>> {
    let v = props.iter().find_map(FbxProp::f64s)?;
    if v.len() >= 9 && v.len() % 3 == 0 {
        Some(
            v.as_chunks::<3>()
                .0
                .iter()
                .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
                .collect(),
        )
    } else {
        None
    }
}

/// 取原始 PolygonVertexIndex 数组（含负数收尾）
fn fbx_poly_idx_from(props: &[FbxProp]) -> Option<Vec<i32>> {
    if let Some(v) = props.iter().find_map(FbxProp::i32s) {
        if v.len() >= 3 {
            return Some(v);
        }
    }
    // 'l'（int64）数组也会落到 I64s
    props
        .iter()
        .find_map(|p| match p {
            FbxProp::I64s(v) if v.len() >= 3 => Some(v.iter().map(|&x| x as i32).collect()),
            _ => None,
        })
}

/// 从属性列表里找命名标量整型（如 RotationOrder）
fn fbx_p_named_int(props: &[FbxProp], key: &str) -> Option<i32> {
    for (i, p) in props.iter().enumerate() {
        let FbxProp::Str(s) = p else { continue };
        if s != key {
            continue;
        }
        for q in &props[i + 1..] {
            match q {
                FbxProp::I32s(v) if !v.is_empty() => return Some(v[0]),
                FbxProp::I64s(v) if !v.is_empty() => return Some(v[0] as i32),
                FbxProp::I64Scalar(v) => return Some(*v as i32),
                _ => {}
            }
        }
    }
    None
}

/// 从属性列表里找命名 Vec3（如 Lcl Translation）
fn fbx_p_named_vec3(props: &[FbxProp], key: &str) -> Option<[f64; 3]> {
    for (i, p) in props.iter().enumerate() {
        let FbxProp::Str(name) = p else { continue };
        if name != key {
            continue;
        }
        if let Some(v) = props[i + 1..].iter().find_map(FbxProp::f64s) {
            if v.len() >= 3 {
                return Some([v[0], v[1], v[2]]);
            }
        }
    }
    None
}

/// 网格数据有两种写法，都要支持：
/// 1) Geometry 的直接子节点名为 Vertices / PolygonVertexIndex，属性就是数组本身；
/// 2) Properties70 里的 P 记录，形如 [名字, "KDoubleArray", 元素类型, 默认值, …, 数组]，
///    数组不一定紧跟名字，命中后要在其后向前找第一个同类数组。
fn fbx_props_take(node: &str, props: &[FbxProp], cur: &mut FbxGeom) {
    match node {
        "Vertices" | "ControlPoints" if cur.verts.is_none() => cur.verts = fbx_verts_from(props),
        "PolygonVertexIndex" | "Indexes" if cur.poly_idx.is_none() => {
            cur.poly_idx = fbx_poly_idx_from(props)
        }
        _ => {}
    }
    for (i, p) in props.iter().enumerate() {
        let FbxProp::Str(name) = p else { continue };
        let rest = &props[i + 1..];
        match name.as_str() {
            "Vertices" | "ControlPoints" if cur.verts.is_none() => cur.verts = fbx_verts_from(rest),
            "PolygonVertexIndex" | "Indexes" if cur.poly_idx.is_none() => {
                cur.poly_idx = fbx_poly_idx_from(rest)
            }
            _ => {}
        }
    }
}

/// 解析 LayerElementNormal 子树，返回法线数据
fn fbx_normals_from(children: &[(String, Vec<FbxProp>)]) -> Option<FbxNormals> {
    let mut mapping = String::new();
    let mut reference = String::new();
    let mut normals: Option<Vec<[f32; 3]>> = None;
    let mut normals_index: Option<Vec<i32>> = None;
    for (name, props) in children {
        match name.as_str() {
            "MappingInformationType" => {
                if let Some(FbxProp::Str(s)) = props.first() {
                    mapping = s.clone();
                }
            }
            "ReferenceInformationType" => {
                if let Some(FbxProp::Str(s)) = props.first() {
                    reference = s.clone();
                }
            }
            "Normals" => {
                if let Some(v) = props.iter().find_map(FbxProp::f64s) {
                    if v.len() >= 3 && v.len() % 3 == 0 {
                        normals = Some(
                            v.as_chunks::<3>()
                                .0
                                .iter()
                                .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
                                .collect(),
                        );
                    }
                }
            }
            "NormalsIndex" => {
                if let Some(v) = props.iter().find_map(FbxProp::i32s) {
                    normals_index = Some(v);
                }
            }
            _ => {}
        }
    }
    let normals = normals?;
    // IndexToDirect：先用索引表展开成 Direct
    let normals = if reference == "IndexToDirect" {
        let idx = normals_index?;
        idx.iter()
            .map(|&i| normals.get(i as usize).copied().unwrap_or([0.0, 0.0, 1.0]))
            .collect()
    } else {
        normals
    };
    match mapping.as_str() {
        "ByPolygonVertex" => Some(FbxNormals::PerCorner(normals)),
        "ByVertice" | "ByVertex" => Some(FbxNormals::PerVertex(normals)),
        "ByPolygon" => Some(FbxNormals::PerPoly(normals)),
        _ => None,
    }
}

/// FBX 欧拉角（度）→ 四元数。order 为 FBX EulerOrder 枚举，默认 0=XYZ。
fn fbx_euler_to_quat(deg: [f64; 3], order: i32) -> glam::Quat {
    let x = (deg[0] as f32).to_radians();
    let y = (deg[1] as f32).to_radians();
    let z = (deg[2] as f32).to_radians();
    let qx = glam::Quat::from_rotation_x(x);
    let qy = glam::Quat::from_rotation_y(y);
    let qz = glam::Quat::from_rotation_z(z);
    match order {
        1 => qx * qz * qy, // XZY
        2 => qy * qz * qx, // YZX
        3 => qy * qx * qz, // YXZ
        4 => qz * qx * qy, // ZXY
        5 => qz * qy * qx, // ZYX
        _ => qx * qy * qz, // XYZ（FBX 默认）
    }
}

/// 从 Model 的 P 记录属性里取 Lcl TRS，组成本地矩阵。
/// p_props 是 Model 子树里全部 P 记录的属性列表。
fn fbx_model_local(p_props: &[Vec<FbxProp>]) -> Mat4 {
    let mut t = [0.0f64; 3];
    let mut r = [0.0f64; 3];
    let mut s = [1.0f64; 3];
    let mut order = 0i32;
    for props in p_props {
        if let Some(v) = fbx_p_named_vec3(props, "Lcl Translation") {
            t = v;
        }
        if let Some(v) = fbx_p_named_vec3(props, "Lcl Rotation") {
            r = v;
        }
        if let Some(v) = fbx_p_named_vec3(props, "Lcl Scaling") {
            s = v;
        }
        if let Some(v) = fbx_p_named_int(props, "RotationOrder") {
            order = v;
        }
    }
    let rot = fbx_euler_to_quat(r, order);
    Mat4::from_scale_rotation_translation(
        Vec3::new(s[0] as f32, s[1] as f32, s[2] as f32),
        rot,
        Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32),
    )
}

/// 沿 parent 链累乘出世界矩阵（防环）
fn fbx_model_world(models: &[FbxModel], id: i64) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    let mut cur = id;
    let mut guard = 0;
    while cur >= 0 && guard < 256 {
        guard += 1;
        let Some(mo) = models.iter().find(|m| m.id == cur) else {
            break;
        };
        m = mo.local * m;
        cur = mo.parent;
    }
    m
}

/// 校验 p 处是否像一个节点记录：结束偏移在文件内、属性计数与列表长度自洽、
/// 名字是可打印 ASCII（根节点名字为空）
fn fbx_looks_like_node(c: &FbxCur, p: usize) -> bool {
    let mut t = FbxCur {
        d: c.d,
        p,
        ver: c.ver,
    };
    let sz = t.off_size();
    // 节点头固定部分：偏移 + NumProperties + PropertyListLen + NameLen
    let hdr = if t.ver >= 7500 { 8 + 8 + 8 + 1 } else { 4 + 4 + 4 + 1 };
    let (Some(e), Some(np), Some(pl), Some(nl)) =
        (t.off(), t.count_field(), t.count_field(), t.u8())
    else {
        return false;
    };
    let body = p + hdr;
    let nl = nl as usize;
    // 记录内容 = 名字 + 属性列表 + 至少一条子节点结束空记录
    if e < body + nl + pl as usize + sz + hdr || e > c.d.len() || np > 1000 {
        return false;
    }
    // 无名记录只能是根节点：根节点不带属性
    if nl == 0 && (np != 0 || pl != 0) {
        return false;
    }
    let name = &c.d[body..body + nl];
    name.iter().all(|b| b.is_ascii_graphic() || *b == b' ')
}

/// 扫描期的累积状态
struct FbxScan {
    geoms: Vec<FbxGeomData>,
    cur: FbxGeom,
    models: Vec<FbxModel>,
}

impl FbxScan {
    /// Geometry 数据齐了才结算
    fn flush(&mut self) {
        let (Some(verts), Some(poly_idx)) = (self.cur.verts.take(), self.cur.poly_idx.take()) else {
            self.cur.normals = None;
            return;
        };
        if verts.len() < 3 || poly_idx.len() < 3 {
            self.cur.normals = None;
            return;
        }
        // 真正的 PolygonVertexIndex 每个多边形都以负数收尾；
        // 无任何收尾符的数组是 Edges/索引表之类的死数据，不是网格
        if !poly_idx.iter().any(|&i| i < 0) {
            self.cur.normals = None;
            return;
        }
        let n = verts.len() as u32;
        let poly_idx: Vec<i32> = poly_idx
            .into_iter()
            .filter(|&i| i.unsigned_abs() <= n)
            .collect();
        if poly_idx.len() < 3 {
            self.cur.normals = None;
            return;
        }
        let id = std::mem::take(&mut self.cur.id);
        let normals = self.cur.normals.take();
        self.geoms.push(FbxGeomData {
            id,
            verts,
            poly_idx,
            normals,
        });
    }
}

/// 读一个节点段的全部直接子节点（名字+属性+子节点范围），
/// 用于 Model / LayerElementNormal / Connections
fn fbx_read_children(
    d: &[u8],
    ver: u32,
    mut p: usize,
    end: usize,
    out: &mut Vec<(String, Vec<FbxProp>, usize, usize)>,
) {
    while p + if ver >= 7500 { 8 } else { 4 } <= end {
        let mut c = FbxCur { d, p, ver };
        let sz = c.off_size();
        let Some(node_end) = c.off() else { return };
        c.p -= sz;
        if node_end == 0 || node_end <= c.p || node_end > end {
            return;
        }
        match fbx_read_node(&mut c, node_end) {
            Some((name, props, children_at)) => {
                out.push((name, props, children_at, node_end));
                p = node_end;
            }
            None => return,
        }
    }
}

/// 递归收集子树里全部 P 记录的属性（P 只出现在 Properties70 下）
fn fbx_collect_p_props(
    d: &[u8],
    ver: u32,
    start: usize,
    end: usize,
    out: &mut Vec<Vec<FbxProp>>,
) {
    let mut kids: Vec<(String, Vec<FbxProp>, usize, usize)> = Vec::new();
    fbx_read_children(d, ver, start, end, &mut kids);
    for (name, props, cs, ce) in kids {
        if name == "P" {
            out.push(props);
        }
        fbx_collect_p_props(d, ver, cs, ce, out);
    }
}

/// 无 FBX 法线的几何体：自己算平滑法线（与原 ensure_normals 行为一致）
fn fbx_smooth_normals(verts: &[[f32; 3]], poly_idx: &[i32]) -> Vec<[f32; 3]> {
    let mut acc = vec![[0.0f32; 3]; verts.len()];
    let mut poly: Vec<u32> = Vec::new();
    for &v in poly_idx {
        if v < 0 {
            poly.push((-v - 1) as u32);
            if poly.len() >= 3 {
                let p0 = Vec3::from(verts[poly[0] as usize]);
                let p1 = Vec3::from(verts[poly[1] as usize]);
                let p2 = Vec3::from(verts[poly[2] as usize]);
                let n = (p1 - p0).cross(p2 - p0);
                for &i in &poly {
                    let c = Vec3::from(acc[i as usize]) + n;
                    acc[i as usize] = [c.x, c.y, c.z];
                }
            }
            poly.clear();
        } else {
            poly.push(v as u32);
        }
    }
    acc.iter_mut()
        .map(|n| {
            let v = Vec3::from(*n);
            let len = v.length();
            if len > 1e-12 {
                let u = v / len;
                [u.x, u.y, u.z]
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

/// 结算一块几何：应用 Model 世界矩阵，展开成逐角顶点（带 FBX 法线时逐角取值，
/// 否则用平滑法线），追加进总网格。逐角展开保证 normals 与 positions 等长。
fn fbx_emit_geom(
    g: FbxGeomData,
    world: Mat4,
    out_pos: &mut Vec<[f32; 3]>,
    out_idx: &mut Vec<u32>,
    out_nrm: &mut Vec<[f32; 3]>,
) {
    if g.verts.is_empty() || g.poly_idx.is_empty() {
        return;
    }
    let nmat = world.inverse().transpose();
    let push_normal = |n: [f32; 3], out_nrm: &mut Vec<[f32; 3]>| {
        let wn = nmat.transform_vector3(Vec3::from(n));
        let wn = if wn.length_squared() > 1e-20 {
            wn.normalize()
        } else {
            Vec3::Y
        };
        out_nrm.push([wn.x, wn.y, wn.z]);
    };

    match &g.normals {
        // 无法线：索引式追加 + 平滑法线（与历史行为一致，省内存）
        None => {
            let smooth = fbx_smooth_normals(&g.verts, &g.poly_idx);
            let base = out_pos.len() as u32;
            for (i, &p) in g.verts.iter().enumerate() {
                let wp = world.transform_point3(Vec3::from(p));
                out_pos.push([wp.x, wp.y, wp.z]);
                push_normal(smooth[i], out_nrm);
            }
            let mut poly: Vec<u32> = Vec::new();
            for &raw in &g.poly_idx {
                if raw < 0 {
                    poly.push((-raw - 1) as u32 + base);
                    fan_triangles(out_idx, &poly);
                    poly.clear();
                } else {
                    poly.push(raw as u32 + base);
                }
            }
        }
        // 有 FBX 法线：逐角展开（PerCorner 法线需要按角存放顶点）
        Some(_) => {
            let mut poly: Vec<u32> = Vec::new();
            let mut corner = 0usize;
            let mut poly_no = 0usize;
            for &raw in &g.poly_idx {
                let vi = (if raw < 0 { -raw - 1 } else { raw }) as usize;
                let Some(&p) = g.verts.get(vi) else {
                    corner += 1;
                    continue;
                };
                let wp = world.transform_point3(Vec3::from(p));
                out_pos.push([wp.x, wp.y, wp.z]);
                let n = match &g.normals {
                    Some(FbxNormals::PerCorner(v)) => {
                        v.get(corner).copied().unwrap_or([0.0, 1.0, 0.0])
                    }
                    Some(FbxNormals::PerVertex(v)) => {
                        v.get(vi).copied().unwrap_or([0.0, 1.0, 0.0])
                    }
                    Some(FbxNormals::PerPoly(v)) => {
                        v.get(poly_no).copied().unwrap_or([0.0, 1.0, 0.0])
                    }
                    None => [0.0, 1.0, 0.0],
                };
                corner += 1;
                push_normal(n, out_nrm);
                poly.push(out_pos.len() as u32 - 1);
                if raw < 0 {
                    fan_triangles(out_idx, &poly);
                    poly.clear();
                    poly_no += 1;
                }
            }
            // 注意：不给未收尾的残段兜底三角化——FBX 多边形必须负数收尾，
            // 没收尾的残段是坏数据，fan 出垃圾三角形不如丢弃
        }
    }
}

fn load_fbx_binary(data: &[u8]) -> anyhow::Result<Mesh> {
    if data.len() < 32 {
        anyhow::bail!("FBX 文件过小");
    }
    let ver = u32::from_le_bytes(data[23..27].try_into().unwrap());
    let probe = FbxCur {
        d: data,
        p: 27,
        ver,
    };
    // 23 字节魔数 + 4 字节版本之后，不同导出器写法不一：有的直接跟第一个节点，
    // 有的先写保留字段和文档结束偏移再跟无名根节点。逐格试探定位第一条记录。
    let Some(start) = (0..8)
        .map(|i| 27 + i * 4)
        .find(|p| *p + probe.off_size() + 9 < data.len() && fbx_looks_like_node(&probe, *p))
    else {
        anyhow::bail!("二进制 FBX 头无法识别");
    };
    let mut c = FbxCur {
        d: data,
        p: start,
        ver,
    };

    let mut scan = FbxScan {
        geoms: Vec::new(),
        cur: FbxGeom::default(),
        models: Vec::new(),
    };

    fn walk(c: &mut FbxCur, end: usize, depth: u32, inside_geometry: bool, scan: &mut FbxScan) {
        if depth > 24 {
            return;
        }
        while c.p + c.off_size() <= end {
            // 空记录 = 结束偏移为 0，它总在子节点块末尾；不能只看首字节，
            // ≥7500 的 8 字节偏移低 4 字节恒为 0
            let sz = c.off_size();
            let Some(node_end) = c.off() else { return };
            c.p -= sz; // 回退，让 fbx_read_node 从记录起点重新解析
            if node_end == 0 {
                return;
            }
            if node_end <= c.p || node_end > end {
                // 头不合法：前进一格避免死循环，让外层尽早放弃
                c.p += sz;
                continue;
            }
            match fbx_read_node(c, node_end) {
                Some((name, props, children_at)) => {
                    // Model 节点：记录 ID + 局部 TRS（Blender 把 Model 平铺在
                    // Objects 下，父子关系由 Connections 的 OO 记录给出）
                    if name == "Model"
                        && let Some(id) = props.iter().find_map(|p| match p {
                            FbxProp::I64Scalar(v) => Some(*v),
                            _ => None,
                        })
                    {
                        let mut p_props: Vec<Vec<FbxProp>> = Vec::new();
                        fbx_collect_p_props(c.d, c.ver, children_at, node_end, &mut p_props);
                        let local = fbx_model_local(&p_props);
                        scan.models.push(FbxModel {
                            id,
                            local,
                            parent: -1,
                        });
                    }
                    let is_geom_node = name == "Geometry";
                    if is_geom_node {
                        // 嵌套 Geometry（少见）：先把上一层的数据结算掉
                        scan.flush();
                        scan.cur = FbxGeom::default();
                        scan.cur.id = props
                            .iter()
                            .find_map(|p| match p {
                                FbxProp::I64Scalar(v) => Some(*v),
                                _ => None,
                            })
                            .unwrap_or(-1);
                    }
                    if is_geom_node || inside_geometry {
                        fbx_props_take(&name, &props, &mut scan.cur);
                        // 法线层：Geometry 下的 LayerElementNormal 子树
                        if name == "LayerElementNormal" && scan.cur.normals.is_none() {
                            let mut kids: Vec<(String, Vec<FbxProp>, usize, usize)> = Vec::new();
                            fbx_read_children(c.d, c.ver, children_at, node_end, &mut kids);
                            let flat: Vec<(String, Vec<FbxProp>)> = kids
                                .into_iter()
                                .map(|(n, p, _, _)| (n, p))
                                .collect();
                            scan.cur.normals = fbx_normals_from(&flat);
                        }
                    }
                    let mut ch = FbxCur {
                        d: c.d,
                        p: children_at,
                        ver: c.ver,
                    };
                    walk(
                        &mut ch,
                        node_end,
                        depth + 1,
                        is_geom_node || inside_geometry,
                        scan,
                    );
                    if is_geom_node {
                        scan.flush();
                    }
                    c.p = node_end;
                }
                // 解析失败：信任结束偏移跳到下一兄弟，避免死循环
                None => c.p = node_end,
            }
        }
    }

    walk(&mut c, data.len(), 0, false, &mut scan);
    scan.flush();

    // ── Connections：解析 OO 记录，拿 Geometry→Model 与 Model→Model 的父子关系 ──
    let mut geom_to_model: Vec<(i64, i64)> = Vec::new();
    {
        let mut cc = FbxCur {
            d: data,
            p: start,
            ver,
        };
        // 递归找顶层 Connections 节点
        fn find_conn(c: &mut FbxCur, end: usize, depth: u32) -> Option<(usize, usize)> {
            if depth > 8 {
                return None;
            }
            while c.p + c.off_size() <= end {
                let sz = c.off_size();
                let Some(node_end) = c.off() else { return None };
                c.p -= sz;
                if node_end == 0 {
                    return None;
                }
                if node_end <= c.p || node_end > end {
                    c.p += sz;
                    continue;
                }
                match fbx_read_node(c, node_end) {
                    Some((name, props, children_at)) => {
                        if name == "Connections" {
                            return Some((children_at, node_end));
                        }
                        let _ = props;
                        let mut ch = FbxCur {
                            d: c.d,
                            p: children_at,
                            ver: c.ver,
                        };
                        if let Some(r) = find_conn(&mut ch, node_end, depth + 1) {
                            return Some(r);
                        }
                        c.p = node_end;
                    }
                    None => c.p = node_end,
                }
            }
            None
        }
        if let Some((cs, ce)) = find_conn(&mut cc, data.len(), 0) {
            let mut kids: Vec<(String, Vec<FbxProp>, usize, usize)> = Vec::new();
            fbx_read_children(data, ver, cs, ce, &mut kids);
            for (name, props, _, _) in &kids {
                if name != "C" {
                    continue;
                }
                // C: "OO", childId, parentId
                if !matches!(props.first(), Some(FbxProp::Str(k)) if k == "OO") {
                    continue;
                }
                let ids: Vec<i64> = props
                    .iter()
                    .filter_map(|p| match p {
                        FbxProp::I64Scalar(v) => Some(*v),
                        _ => None,
                    })
                    .collect();
                if ids.len() < 2 {
                    continue;
                }
                let (child, parent) = (ids[0], ids[1]);
                if scan.geoms.iter().any(|g| g.id == child) {
                    geom_to_model.push((child, parent));
                } else if scan.models.iter().any(|m| m.id == child)
                    && scan.models.iter().any(|m| m.id == parent)
                    && let Some(m) = scan.models.iter_mut().find(|m| m.id == child)
                {
                    m.parent = parent;
                }
            }
        }
    }

    // ── 结算：应用世界矩阵，展开成带法线的网格 ──
    let mut out_pos: Vec<[f32; 3]> = Vec::new();
    let mut out_idx: Vec<u32> = Vec::new();
    let mut out_nrm: Vec<[f32; 3]> = Vec::new();
    for g in std::mem::take(&mut scan.geoms) {
        let world = geom_to_model
            .iter()
            .find(|(cg, _)| *cg == g.id)
            .map(|(_, m)| fbx_model_world(&scan.models, *m))
            .unwrap_or(Mat4::IDENTITY);
        fbx_emit_geom(g, world, &mut out_pos, &mut out_idx, &mut out_nrm);
    }

    if out_pos.len() < 3 || out_idx.len() < 3 {
        anyhow::bail!("二进制 FBX 未找到多边形网格（可能只含曲线/点），请导出为 OBJ/GLB/STL");
    }
    let mut mesh = Mesh {
        positions: out_pos,
        normals: out_nrm,
        colors: None,
        indices: out_idx,
        ..Default::default()
    };
    mesh.ensure_normals();
    Ok(mesh)
}

fn load_pmx(path: &Path) -> anyhow::Result<Mesh> {
    let data = std::fs::read(path)?;
    if data.starts_with(b"PMX ") {
        return parse_pmx(&data);
    }
    if data.starts_with(b"Pmd") {
        return parse_pmd(&data);
    }
    anyhow::bail!("不是 PMX/PMD 文件");
}

fn parse_pmd(data: &[u8]) -> anyhow::Result<Mesh> {
    if data.len() < 285 {
        anyhow::bail!("PMD 过短");
    }
    let vcount = u16::from_le_bytes([data[283], data[284]]) as usize;
    let mut positions = Vec::with_capacity(vcount);
    let mut off = 285usize;
    for _ in 0..vcount {
        if off + 12 > data.len() {
            break;
        }
        let x = f32::from_le_bytes(data[off..off + 4].try_into()?);
        let y = f32::from_le_bytes(data[off + 4..off + 8].try_into()?);
        let z = f32::from_le_bytes(data[off + 8..off + 12].try_into()?);
        positions.push([x, y, z]);
        off += 38;
    }
    if off + 4 > data.len() || positions.is_empty() {
        anyhow::bail!("PMD 顶点无效");
    }
    let fcount = u32::from_le_bytes(data[off..off + 4].try_into()?) as usize;
    let mut indices = Vec::with_capacity(fcount);
    off += 4;
    let n = positions.len() as u32;
    for _ in 0..fcount {
        if off + 2 > data.len() {
            break;
        }
        let i = u16::from_le_bytes([data[off], data[off + 1]]) as u32;
        indices.push(i);
        off += 2;
    }
    indices.retain(|i| *i < n);
    if indices.is_empty() {
        anyhow::bail!("PMD 面无效");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

fn parse_pmx(data: &[u8]) -> anyhow::Result<Mesh> {
    if data.len() < 8 {
        anyhow::bail!("PMX 过短");
    }
    let mut off = 8usize;
    let gcount = *data.get(off).ok_or_else(|| anyhow::anyhow!("PMX 头无效"))? as usize;
    off += 1;
    if off + gcount > data.len() {
        anyhow::bail!("PMX 头无效");
    }
    let globals = &data[off..off + gcount];
    off += gcount;
    if globals.len() < 5 {
        anyhow::bail!("PMX globals 过短");
    }
    let add_uv = globals[1] as usize;
    let idx_size = |b: u8| -> usize {
        match b {
            1 => 1,
            2 => 2,
            4 => 4,
            _ => 2,
        }
    };
    let vidx_sz = idx_size(if globals.len() > 2 { globals[2] } else { 4 });
    let bone_sz = idx_size(if globals.len() > 5 { globals[5] } else { 2 }).max(1);

    for _ in 0..4 {
        off = skip_pmx_text(data, off)?;
    }
    if off + 4 > data.len() {
        anyhow::bail!("PMX 结构错误");
    }
    let vcount = u32::from_le_bytes(data[off..off + 4].try_into()?) as usize;
    off += 4;
    let mut positions = Vec::with_capacity(vcount.min(2_000_000));
    for _ in 0..vcount {
        if off + 36 > data.len() {
            break;
        }
        let x = f32::from_le_bytes(data[off..off + 4].try_into()?);
        let y = f32::from_le_bytes(data[off + 4..off + 8].try_into()?);
        let z = f32::from_le_bytes(data[off + 8..off + 12].try_into()?);
        positions.push([x, y, z]);
        off += 12;
        off += 20; // normal + uv
        off += add_uv * 12;
        let wt = *data.get(off).unwrap_or(&0);
        off += 1;
        match wt {
            0 => off += bone_sz + 4,
            1 => off += bone_sz * 2 + 4,
            2 => off += bone_sz * 4 + 16,
            3 => off += bone_sz * 2 + 8 + 24,
            4 => off += bone_sz * 4 + 16 + 24,
            _ => off += bone_sz * 2 + 4,
        }
        off += 4; // edge
    }
    if off + 4 > data.len() {
        anyhow::bail!("PMX 面数据缺失");
    }
    let fcount = u32::from_le_bytes(data[off..off + 4].try_into()?) as usize;
    off += 4;
    let mut indices = Vec::with_capacity(fcount.min(6_000_000));
    let read_idx = |off: &mut usize| -> Option<u32> {
        match vidx_sz {
            1 => {
                let v = *data.get(*off)?;
                *off += 1;
                Some(v as u32)
            }
            2 => {
                if *off + 2 > data.len() {
                    return None;
                }
                let v = u16::from_le_bytes([data[*off], data[*off + 1]]) as u32;
                *off += 2;
                Some(v)
            }
            4 => {
                if *off + 4 > data.len() {
                    return None;
                }
                let v = u32::from_le_bytes(data[*off..*off + 4].try_into().ok()?);
                *off += 4;
                Some(v)
            }
            _ => None,
        }
    };
    for _ in 0..fcount {
        let a = match read_idx(&mut off) {
            Some(v) => v,
            None => break,
        };
        let b = match read_idx(&mut off) {
            Some(v) => v,
            None => break,
        };
        let c = match read_idx(&mut off) {
            Some(v) => v,
            None => break,
        };
        indices.extend_from_slice(&[a, b, c]);
    }
    if positions.is_empty() || indices.is_empty() {
        anyhow::bail!("PMX 中没有有效网格");
    }
    let n = positions.len() as u32;
    indices.retain(|i| *i < n);
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

fn skip_pmx_text(data: &[u8], mut off: usize) -> anyhow::Result<usize> {
    if off + 4 > data.len() {
        anyhow::bail!("PMX 文本长度无效");
    }
    let len = u32::from_le_bytes(data[off..off + 4].try_into()?) as usize;
    off += 4 + len;
    if off > data.len() {
        anyhow::bail!("PMX 文本越界");
    }
    Ok(off)
}

fn load_usd(path: &Path) -> anyhow::Result<Mesh> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext == "usdz" {
        return load_usdz(path);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|_| anyhow::anyhow!("USD 二进制 (.usdc) 请转为 .usda/.usdz 或 glTF"))?;
    parse_usda(&text)
}

fn load_usdz(path: &Path) -> anyhow::Result<Mesh> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| anyhow::anyhow!("USDZ 不是有效 ZIP: {}", e))?;
    let mut names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(z) = archive.by_index(i) {
            let n = z.name().to_ascii_lowercase();
            if n.ends_with(".usda") || n.ends_with(".usd") {
                names.push(z.name().to_string());
            }
        }
    }
    names.sort();
    for name in names {
        let mut s = String::new();
        if let Ok(mut z) = archive.by_name(&name)
            && std::io::Read::read_to_string(&mut z, &mut s).is_ok()
            && let Ok(m) = parse_usda(&s)
        {
            return Ok(m);
        }
    }
    anyhow::bail!("USDZ 中未找到可解析的 usda 网格")
}

fn parse_usda(text: &str) -> anyhow::Result<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut face_counts: Vec<usize> = Vec::new();
    let mut face_indices: Vec<u32> = Vec::new();
    let lower = text.to_ascii_lowercase();

    let mut search = 0usize;
    while let Some(i) = lower[search..].find("point3f[] points") {
        let abs = search + i;
        if let Some(start) = lower[abs..].find('[').map(|j| abs + j + 1)
            && let Some(end) = find_matching_bracket(&lower, start - 1)
        {
            let body = &text[start..end];
            for tup in body.split(',') {
                let nums: Vec<f32> = tup
                    .trim()
                    .trim_matches(|c| c == '(' || c == ')')
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if nums.len() >= 3 {
                    positions.push([nums[0], nums[1], nums[2]]);
                }
            }
        }
        search = abs + 8;
    }

    if let Some(i) = lower.find("facevertexcounts")
        && let Some(start) = lower[i..].find('[').map(|j| i + j + 1)
        && let Some(end) = find_matching_bracket(&lower, start - 1)
    {
        face_counts = text[start..end]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|s| s.trim().parse().ok())
            .collect();
    }
    if let Some(i) = lower.find("facevertexindices")
        && let Some(start) = lower[i..].find('[').map(|j| i + j + 1)
        && let Some(end) = find_matching_bracket(&lower, start - 1)
    {
        face_indices = text[start..end]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|s| s.trim().parse().ok())
            .collect();
    }

    if !face_counts.is_empty() && !face_indices.is_empty() {
        let mut cur = 0usize;
        for &c in &face_counts {
            if cur + c <= face_indices.len() && c >= 3 {
                let poly = &face_indices[cur..cur + c];
                fan_triangles(&mut indices, poly);
            }
            cur += c;
        }
    } else if !face_indices.is_empty() {
        for tri in face_indices.as_chunks::<3>().0 {
            indices.extend_from_slice(tri);
        }
    }

    if positions.is_empty() || indices.is_empty() {
        anyhow::bail!("USD 中未找到 mesh（请用 usda 或转 glTF）");
    }
    let n = positions.len() as u32;
    indices.retain(|i| *i < n);
    if indices.is_empty() {
        anyhow::bail!("USD 索引无效");
    }
    Ok(Mesh::from_positions_indices(positions, indices, None))
}

fn find_matching_bracket(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'[' {
        return None;
    }
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open_idx) {
        if b == b'[' {
            depth += 1;
        } else if b == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

// ─── 软件光栅化 ───────────────────────────────────

fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (c.0 - a.0) * (b.1 - a.1) - (c.1 - a.1) * (b.0 - a.0)
}

#[cfg(test)]
pub fn render_mesh(mesh: &Mesh, cam: &OrbitCamera, width: usize, height: usize) -> Vec<Color32> {
    render_mesh_ex(mesh, cam, width, height, false)
}

pub fn render_mesh_ex(
    mesh: &Mesh,
    cam: &OrbitCamera,
    width: usize,
    height: usize,
    fast: bool,
) -> Vec<Color32> {
    let w = width.max(2);
    let h = height.max(2);
    let mut pixels = vec![Color32::from_rgb(30, 32, 38); w * h];
    let mut depth = vec![f32::INFINITY; w * h];
    pixels
        .iter_mut()
        .for_each(|p| *p = Color32::from_rgb(32, 34, 40));

    if mesh.positions.is_empty() || mesh.indices.is_empty() {
        return pixels;
    }

    let aspect = w as f32 / h as f32;
    let vp = cam.proj_matrix(aspect) * cam.view_matrix();
    let nverts = mesh.positions.len();
    let mut clip_pos = vec![glam::Vec4::ZERO; nverts];
    for (i, p) in mesh.positions.iter().enumerate() {
        let wp = Vec3::from(*p);
        clip_pos[i] = vp * wp.extend(1.0);
    }

    let eye = cam.eye_position();
    let lx = 0.45f32;
    let ly = 0.85f32;
    let lz = 0.35f32;
    let ll = (lx * lx + ly * ly + lz * lz).sqrt();
    let (lx, ly, lz) = (lx / ll, ly / ll, lz / ll);

    let max_tris = if fast {
        MAX_RENDER_TRIS_FAST
    } else {
        MAX_RENDER_TRIS_HQ
    };
    let tri_total = mesh.indices.len() / 3;
    let stride = if tri_total > max_tris {
        tri_total.div_ceil(max_tris).max(2)
    } else {
        1
    };
    let default_col = [0.88f32, 0.90, 0.93];
    let ambient = 0.42f32;

    for (tri_i, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        if stride > 1 && (tri_i % stride) != 0 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= nverts || i1 >= nverts || i2 >= nverts {
            continue;
        }
        let c0 = clip_pos[i0];
        let c1 = clip_pos[i1];
        let c2 = clip_pos[i2];
        if c0.w.abs() < 1e-5 || c1.w.abs() < 1e-5 || c2.w.abs() < 1e-5 {
            continue;
        }
        let inv0 = 1.0 / c0.w;
        let inv1 = 1.0 / c1.w;
        let inv2 = 1.0 / c2.w;
        let s0 = (
            ((c0.x * inv0) * 0.5 + 0.5) * w as f32,
            (0.5 - (c0.y * inv0) * 0.5) * h as f32,
        );
        let s1 = (
            ((c1.x * inv1) * 0.5 + 0.5) * w as f32,
            (0.5 - (c1.y * inv1) * 0.5) * h as f32,
        );
        let s2 = (
            ((c2.x * inv2) * 0.5 + 0.5) * w as f32,
            (0.5 - (c2.y * inv2) * 0.5) * h as f32,
        );

        let (w0x, w0y, w0z) = (
            mesh.positions[i0][0],
            mesh.positions[i0][1],
            mesh.positions[i0][2],
        );
        let (w1x, w1y, w1z) = (
            mesh.positions[i1][0],
            mesh.positions[i1][1],
            mesh.positions[i1][2],
        );
        let (w2x, w2y, w2z) = (
            mesh.positions[i2][0],
            mesh.positions[i2][1],
            mesh.positions[i2][2],
        );
        let fnx = (w1y - w0y) * (w2z - w0z) - (w1z - w0z) * (w2y - w0y);
        let fny = (w1z - w0z) * (w2x - w0x) - (w1x - w0x) * (w2z - w0z);
        let fnz = (w1x - w0x) * (w2y - w0y) - (w1y - w0y) * (w2x - w0x);
        let fl = (fnx * fnx + fny * fny + fnz * fnz).sqrt();
        if fl < 1e-16 {
            continue;
        }
        let (fnx, fny, fnz) = (fnx / fl, fny / fl, fnz / fl);
        let area2 = edge(s0, s1, s2);

        let tex_tri = mesh.tri_textured(i0, i1, i2);
        let tex_img = tex_tri.as_ref().map(|(t, _)| &*mesh.textures[*t]);

        // 贴图三角形的底色：有顶点色（含 baseColorFactor）用它，否则纯白别压暗贴图
        let (cr0, cg0, cb0) = match &mesh.colors {
            Some(cols) if i0 < cols.len() && i1 < cols.len() && i2 < cols.len() => (
                (cols[i0][0] + cols[i1][0] + cols[i2][0]) / 3.0,
                (cols[i0][1] + cols[i1][1] + cols[i2][1]) / 3.0,
                (cols[i0][2] + cols[i1][2] + cols[i2][2]) / 3.0,
            ),
            _ if tex_img.is_some() => (1.0, 1.0, 1.0),
            _ => (default_col[0], default_col[1], default_col[2]),
        };

        let cxw = (w0x + w1x + w2x) / 3.0;
        let cyw = (w0y + w1y + w2y) / 3.0;
        let czw = (w0z + w1z + w2z) / 3.0;
        let mut nx = fnx;
        let mut ny = fny;
        let mut nz = fnz;
        let vx = eye.x - cxw;
        let vy = eye.y - cyw;
        let vz = eye.z - czw;
        let vl = (vx * vx + vy * vy + vz * vz).sqrt().max(1e-6);
        if (nx * vx + ny * vy + nz * vz) < 0.0 {
            nx = -nx;
            ny = -ny;
            nz = -nz;
        }
        let ndl = (nx * lx + ny * ly + nz * lz).abs();
        let mut shade = ambient + 0.70 * ndl;
        if !fast {
            let hx = lx + vx / vl;
            let hy = ly + vy / vl;
            let hz = lz + vz / vl;
            let hl = (hx * hx + hy * hy + hz * hz).sqrt().max(1e-6);
            shade += ((nx * hx + ny * hy + nz * hz) / hl).abs().powf(16.0) * 0.12;
        }
        shade = shade.clamp(0.0, 1.5);
        // 底色乘子也要在线性空间相乘，才不会把 baseColorFactor 叠暗
        let mul_lin = if tex_img.is_some() {
            [
                srgb_to_linear(cr0),
                srgb_to_linear(cg0),
                srgb_to_linear(cb0),
            ]
        } else {
            [cr0, cg0, cb0]
        };
        // 退化三角形只落一个中心像素：按重心（等权）采样
        let packed = match (&tex_tri, tex_img) {
            (Some((_, uv)), Some(tex)) => {
                let iw = (inv0 + inv1 + inv2).max(1e-12);
                let u = (uv[0][0] * inv0 + uv[1][0] * inv1 + uv[2][0] * inv2) / iw;
                let v = (uv[0][1] * inv0 + uv[1][1] * inv1 + uv[2][1] * inv2) / iw;
                let (lin, a) = sample_texture(tex, u, v);
                if a < 0.5 {
                    continue;
                }
                texel_color(lin, mul_lin, shade)
            }
            _ => Color32::from_rgb(
                (cr0 * shade * 255.0).min(255.0) as u8,
                (cg0 * shade * 255.0).min(255.0) as u8,
                (cb0 * shade * 255.0).min(255.0) as u8,
            ),
        };

        let cx = (s0.0 + s1.0 + s2.0) / 3.0;
        let cy = (s0.1 + s1.1 + s2.1) / 3.0;
        let zc = (inv0 + inv1 + inv2) / 3.0;
        let z_c = if zc.abs() > 1e-12 { 1.0 / zc } else { f32::MAX };

        if area2.abs() < 0.5 {
            let ix = cx.round() as i32;
            let iy = cy.round() as i32;
            if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                let di = iy as usize * w + ix as usize;
                if z_c < depth[di] {
                    depth[di] = z_c;
                    pixels[di] = packed;
                }
            }
            continue;
        }

        let inv_area = 1.0 / area2;
        let min_x = s0.0.min(s1.0).min(s2.0).floor().max(0.0) as i32;
        let max_x = s0.0.max(s1.0).max(s2.0).ceil().min((w - 1) as f32) as i32;
        let min_y = s0.1.min(s1.1).min(s2.1).floor().max(0.0) as i32;
        let max_y = s0.1.max(s1.1).max(s2.1).ceil().min((h - 1) as f32) as i32;
        if max_x < min_x || max_y < min_y {
            let ix = cx.round() as i32;
            let iy = cy.round() as i32;
            if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                let di = iy as usize * w + ix as usize;
                if z_c < depth[di] {
                    depth[di] = z_c;
                    pixels[di] = packed;
                }
            }
            continue;
        }

        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            let row = y as usize * w;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let e0 = edge(s1, s2, (px, py)) * inv_area;
                let e1 = edge(s2, s0, (px, py)) * inv_area;
                let e2 = edge(s0, s1, (px, py)) * inv_area;
                if e0 < -0.02 || e1 < -0.02 || e2 < -0.02 {
                    continue;
                }
                let inv_w = e0 * inv0 + e1 * inv1 + e2 * inv2;
                if inv_w <= 1e-12 {
                    continue;
                }
                let z = 1.0 / inv_w;
                let di = row + x as usize;
                if z >= depth[di] {
                    continue;
                }
                // 贴图：透视正确的重心权重 uv = Σ(bary·uv·invW) / Σ(bary·invW)
                let color = match (&tex_tri, tex_img) {
                    (Some((_, uv)), Some(tex)) => {
                        let u =
                            (uv[0][0] * inv0 * e0 + uv[1][0] * inv1 * e1 + uv[2][0] * inv2 * e2)
                                * z;
                        let v =
                            (uv[0][1] * inv0 * e0 + uv[1][1] * inv1 * e1 + uv[2][1] * inv2 * e2)
                                * z;
                        let (lin, a) = sample_texture(tex, u, v);
                        if a < 0.5 {
                            continue;
                        }
                        texel_color(lin, mul_lin, shade)
                    }
                    _ => packed,
                };
                depth[di] = z;
                pixels[di] = color;
            }
        }
    }

    pixels
}

pub fn load_model(path: &Path) -> anyhow::Result<Model3D> {
    Model3D::load_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(name)
    }

    /// 贴图文件缺失的 glTF 仍应加载出几何，并报告缺失数量。
    /// 对应真实场景：Blender 导出的 .gltf 把 textures/ 目录弄丢了。
    #[test]
    fn gltf_missing_texture_still_loads() {
        // 单个三角形的 glTF：buffer 用 data URI（保证几何可用），
        // image 指向一个不存在的文件（模拟 textures/ 丢失）
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let mut bin: Vec<u8> = Vec::new();
        for p in &positions {
            for c in p {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for _ in 0..3 {
            bin.extend_from_slice(&0.0f32.to_le_bytes());
            bin.extend_from_slice(&0.0f32.to_le_bytes());
            bin.extend_from_slice(&1.0f32.to_le_bytes());
        }
        for _ in 0..3 {
            bin.extend_from_slice(&0.5f32.to_le_bytes());
            bin.extend_from_slice(&0.5f32.to_le_bytes());
        }
        let gltf_json = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "uri": "data:application/octet-stream;base64,{}", "byteLength": {} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0,  "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": 36, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": 72, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "max": [1,1,0], "min": [0,0,0] }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "images": [{{ "uri": "textures/does_not_exist.png" }}],
  "samplers": [],
  "textures": [{{ "source": 0 }}],
  "materials": [{{ "pbrMetallicRoughness": {{ "baseColorTexture": {{ "index": 0 }} }} }}],
  "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 }}, "indices": null, "material": 0, "mode": 4 }}] }}],
  "nodes": [{{ "mesh": 0 }}],
  "scenes": [{{ "nodes": [0] }}],
  "scene": 0
}}"#,
            base64_encode(&bin),
            bin.len()
        );
        let dir = std::env::temp_dir().join(format!("dadaview_gltf_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("missing_tex.gltf");
        std::fs::write(&path, gltf_json.as_bytes()).unwrap();

        let (mesh, missing) = load_gltf(&path).expect("贴图缺失不应阻止加载");
        assert_eq!(mesh.positions.len(), 3, "三角形顶点应完整加载");
        assert_eq!(missing, 1, "应报告 1 张缺失贴图");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn base64_encode(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            out.push(T[(b[0] >> 2) as usize] as char);
            out.push(T[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                T[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                T[(b[2] & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    /// 合成 FBX：三角形 + Model 的 Lcl 变换（平移(10,0,0)·绕Z 90°·缩放2倍）。
    /// 顶点应被变换到 (10,0,0) (10,2,0) (8,0,0)，法线保持 (0,0,1)。
    /// 覆盖：P 记录 TRS 解析、Connections OO、世界矩阵、法线矩阵、逐角展开。
    #[test]
    fn fbx_model_transform_and_normals() {
        let m = Model3D::load_path(&sample_path("transform_test.fbx")).expect("合成 FBX 应能加载");
        let mesh = &m.mesh;
        assert_eq!(mesh.positions.len(), 3, "三角形应展开成 3 个角顶点");
        assert_eq!(mesh.normals.len(), 3, "FBX 法线应被采用");
        let expect = [
            [10.0f32, 0.0, 0.0],
            [10.0, 2.0, 0.0],
            [8.0, 0.0, 0.0],
        ];
        for (got, want) in mesh.positions.iter().zip(expect.iter()) {
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() < 1e-4,
                    "顶点变换错误: got {:?}, want {:?}",
                    got,
                    want
                );
            }
        }
        for n in &mesh.normals {
            assert!((n[0]).abs() < 1e-4 && (n[1]).abs() < 1e-4 && (n[2] - 1.0).abs() < 1e-4,
                "法线应保持 (0,0,1)，得到 {:?}", n);
        }
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    /// 两级 Model 层级：父(100,0,0) + 子(5,0,0)，几何挂子节点。
    /// 世界矩阵应累加，顶点 (0,0,0) 应落到 (105,0,0)。
    #[test]
    fn fbx_nested_model_transforms() {
        let m = Model3D::load_path(&sample_path("transform_nested_test.fbx")).expect("嵌套 FBX 应能加载");
        let mesh = &m.mesh;
        assert_eq!(mesh.positions.len(), 3);
        // 三角形三点 (0,0,0) (1,0,0) (0,1,0) 平移 (105,0,0) 后：
        let expect = [[105.0f32, 0.0, 0.0], [106.0, 0.0, 0.0], [105.0, 1.0, 0.0]];
        for (got, want) in mesh.positions.iter().zip(expect.iter()) {
            for k in 0..3 {
                assert!((got[k] - want[k]).abs() < 1e-4,
                    "嵌套变换应累加，得到 {:?}，期望 {:?}", got, want);
            }
        }
    }

    #[test]
    fn load_obj_cube() {
        assert!(Model3D::load_path(&sample_path("cube.obj")).is_ok());
    }

    #[test]
    fn load_off_cube() {
        assert!(Model3D::load_path(&sample_path("cube.off")).is_ok());
    }

    fn p_str(s: &str) -> Vec<u8> {
        let mut v = vec![b'S'];
        v.extend_from_slice(&(s.len() as u32).to_le_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// 数组属性头：元素个数 + 编码 + 存储字节数
    fn p_array(tag: u8, count: usize, enc: u32, stored: u32) -> Vec<u8> {
        let mut v = vec![tag];
        v.extend_from_slice(&(count as u32).to_le_bytes());
        v.extend_from_slice(&enc.to_le_bytes());
        v.extend_from_slice(&stored.to_le_bytes());
        v
    }

    fn p_d_raw(vals: &[f64]) -> Vec<u8> {
        let mut v = p_array(b'd', vals.len(), 0, 0);
        for x in vals {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    fn p_i_raw(vals: &[i32]) -> Vec<u8> {
        let mut v = p_array(b'i', vals.len(), 0, 0);
        for x in vals {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut e, bytes).unwrap();
        e.finish().unwrap()
    }

    /// Blender 导出风格：大数组用 zlib 压缩
    fn p_d_zipped(vals: &[f64]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(vals.len() * 8);
        for x in vals {
            raw.extend_from_slice(&x.to_le_bytes());
        }
        let body = zlib(&raw);
        let mut v = p_array(b'd', vals.len(), 1, body.len() as u32);
        v.extend_from_slice(&body);
        v
    }

    fn p_i_zipped(vals: &[i32]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(vals.len() * 4);
        for x in vals {
            raw.extend_from_slice(&x.to_le_bytes());
        }
        let body = zlib(&raw);
        let mut v = p_array(b'i', vals.len(), 1, body.len() as u32);
        v.extend_from_slice(&body);
        v
    }

    struct Spec {
        name: &'static str,
        props: Vec<Vec<u8>>,
        children: Vec<Spec>,
    }

    /// 写一条节点记录：base 是它在整个文件里的起始偏移
    fn node(s: &Spec, base: usize) -> Vec<u8> {
        node_v(s, base, 7400)
    }

    /// 同上，但按版本决定节点头宽：FBX ≥7500 的偏移/计数/属性长度都是 64 位
    fn node_v(s: &Spec, base: usize, ver: u32) -> Vec<u8> {
        let wide = ver >= 7500;
        let mut blob = Vec::new();
        for p in &s.props {
            blob.extend_from_slice(p);
        }
        if !blob.is_empty() {
            blob.push(0); // 属性列表结束符计入 PropertyListLen
        }
        let fields = if wide { 8 * 3 + 1 } else { 4 * 3 + 1 };
        let head = fields + s.name.len();
        let tail = if wide { 25 } else { 13 }; // 子节点结束空记录
        let mut kids = Vec::new();
        for c in &s.children {
            let at = base + head + blob.len() + kids.len();
            kids.extend_from_slice(&node_v(c, at, ver));
        }
        let node_end = base + head + blob.len() + kids.len() + tail;
        let mut out = Vec::new();
        if wide {
            out.extend_from_slice(&(node_end as u64).to_le_bytes());
            out.extend_from_slice(&(s.props.len() as u64).to_le_bytes());
            out.extend_from_slice(&(blob.len() as u64).to_le_bytes());
        } else {
            out.extend_from_slice(&(node_end as u32).to_le_bytes());
            out.extend_from_slice(&(s.props.len() as u32).to_le_bytes());
            out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        }
        out.push(s.name.len() as u8);
        out.extend_from_slice(s.name.as_bytes());
        out.extend_from_slice(&blob);
        out.extend_from_slice(&kids);
        out.extend_from_slice(&vec![0u8; tail]);
        out
    }

    /// 版本 7400 文档外壳：魔数 + 版本 + 保留 + 文档结束偏移 + 无名根节点
    fn fbx_doc(top: Vec<Spec>) -> Vec<u8> {
        fbx_doc_v(7400, top)
    }

    fn fbx_doc_v(ver: u32, top: Vec<Spec>) -> Vec<u8> {
        let wide = ver >= 7500;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Kaydara FBX Binary  \x00\x1A\x00");
        buf.extend_from_slice(&ver.to_le_bytes());
        if wide {
            // FBX SDK ≥7500：版本号后直接跟第一条节点记录，没有保留/文档偏移字段
            let root_base = buf.len();
            let root_head = 8 * 3 + 1;
            let root_tail = 25;
            let mut kids = Vec::new();
            for s in &top {
                let at = root_base + root_head + kids.len();
                kids.extend_from_slice(&node_v(s, at, ver));
            }
            buf.extend_from_slice(&((root_base + root_head + kids.len() + root_tail) as u64).to_le_bytes());
            buf.extend_from_slice(&0u64.to_le_bytes()); // num_properties
            buf.extend_from_slice(&0u64.to_le_bytes()); // property_list_len
            buf.push(0); // name_length
            buf.extend_from_slice(&kids);
            buf.extend_from_slice(&[0u8; 25]);
            return buf;
        }
        buf.extend_from_slice(&0u32.to_le_bytes()); // 保留
        let doc_end_at = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // 文档结束偏移，最后回填
        let root_base = buf.len();
        let mut kids = Vec::new();
        for s in &top {
            let at = root_base + 13 + kids.len();
            kids.extend_from_slice(&node(s, at));
        }
        buf.extend_from_slice(&((root_base + 13 + kids.len() + 13) as u32).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_properties
        buf.extend_from_slice(&0u32.to_le_bytes()); // property_list_len
        buf.push(0); // name_length
        buf.extend_from_slice(&kids);
        buf.extend_from_slice(&[0u8; 13]);
        let doc = buf.len();
        buf[doc_end_at..doc_end_at + 4].copy_from_slice(&(doc as u32).to_le_bytes());
        buf
    }

    fn fbx_objects(geom_children: Vec<Spec>) -> Spec {
        Spec {
            name: "Objects",
            props: vec![],
            children: vec![Spec {
                name: "Geometry",
                props: vec![p_str("Geometry::Mesh"), p_str("Mesh")],
                children: geom_children,
            }],
        }
    }

    /// 老式（ComfyUI/SDK）：网格数据放在 Properties70 的 P 记录里，未压缩
    fn build_fbx_p70(verts: &[f64], poly_idx: &[i32]) -> Vec<u8> {
        let p = |name: &str, arr: Vec<u8>| Spec {
            name: "P",
            props: vec![
                p_str(name),
                p_str("KDoubleArray"),
                p_str("VectorArray"),
                p_str(""),
                arr,
            ],
            children: vec![],
        };
        fbx_doc(vec![fbx_objects(vec![Spec {
            name: "Properties70",
            props: vec![],
            children: vec![
                p("Vertices", p_d_raw(verts)),
                p("PolygonVertexIndex", p_i_raw(poly_idx)),
            ],
        }])])
    }

    /// Blender：网格数据放在名为 Vertices / PolygonVertexIndex 的子节点里，且 zlib 压缩
    fn build_fbx_blender(verts: &[f64], poly_idx: &[i32]) -> Vec<u8> {
        build_fbx_blender_v(verts, poly_idx, 7400)
    }

    /// 同上，可指定 FBX 版本（≥7500 时节点头为 64 位）
    fn build_fbx_blender_v(verts: &[f64], poly_idx: &[i32], ver: u32) -> Vec<u8> {
        fbx_doc_v(
            ver,
            vec![fbx_objects(vec![
                Spec {
                    name: "Vertices",
                    props: vec![p_d_zipped(verts)],
                    children: vec![],
                },
                Spec {
                    name: "PolygonVertexIndex",
                    props: vec![p_i_zipped(poly_idx)],
                    children: vec![],
                },
            ])],
        )
    }

    /// 8 顶点立方体：6 个四边形面，负数收尾
    fn cube_data() -> (Vec<f64>, Vec<i32>) {
        let verts: Vec<f64> = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
        .into_iter()
        .flatten()
        .collect();
        let faces = [
            [0u32, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut poly: Vec<i32> = Vec::new();
        for f in faces {
            poly.extend_from_slice(&[f[0] as i32, f[1] as i32, f[2] as i32]);
            poly.push(-(f[3] as i32) - 1);
        }
        (verts, poly)
    }

    fn assert_cube(mesh: &Mesh) {
        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn load_fbx_binary_properties70() {
        let (verts, poly) = cube_data();
        let mesh = load_fbx_binary(&build_fbx_p70(&verts, &poly)).expect("p70 fbx");
        assert_cube(&mesh);
    }

    #[test]
    fn load_fbx_binary_blender_children_zlib() {
        let (verts, poly) = cube_data();
        let mesh = load_fbx_binary(&build_fbx_blender(&verts, &poly)).expect("blender fbx");
        assert_cube(&mesh);
    }

    /// FBX 7500+ 的节点头是 64 位偏移/计数/属性长度。此前按 32 位读，
    /// 会把 NumProperties 的高 4 字节（恒 0）当成 PropertyListLen，
    /// 导致属性全部跳过、所有 ≥7.5 文件都报「未找到多边形网格」。
    #[test]
    fn load_fbx_binary_7500_u64_node_header() {
        let (verts, poly) = cube_data();
        for ver in [7500u32, 7700] {
            let mesh = load_fbx_binary(&build_fbx_blender_v(&verts, &poly, ver))
                .unwrap_or_else(|e| panic!("fbx {ver} 解析失败: {e}"));
            assert_cube(&mesh);
        }
    }
    /// 用真实导出文件验证解析器（文件不入库）：
    /// `DADAVIEW_FBX=模型.fbx cargo test -- --ignored load_real_fbx_sample`
    #[test]
    #[ignore = "需要本机一个真实二进制 FBX 文件路径"]
    fn load_real_fbx_sample() {
        let path = std::env::var("DADAVIEW_FBX").expect("设置 DADAVIEW_FBX 指向二进制 fbx");
        let data = std::fs::read(&path).unwrap();
        let name = Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mesh = load_fbx_binary(&data).unwrap_or_else(|e| panic!("{name} 解析失败: {e}"));
        println!(
            "{name}: {} 顶点 / {} 三角形",
            mesh.vertex_count(),
            mesh.triangle_count()
        );
        assert!(mesh.vertex_count() >= 3, "{name} 顶点数异常");
        assert!(mesh.triangle_count() >= 1, "{name} 三角形数异常");
    }

    #[test]
    fn load_fbx_binary_rejects_garbage() {
        let mut data = b"Kaydara FBX Binary  \x00\x1A\x00".to_vec();
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.resize(64, 0xAB);
        assert!(load_fbx_binary(&data).is_err());
    }

    #[test]
    fn load_ply_cube() {
        assert!(Model3D::load_path(&sample_path("cube.ply")).is_ok());
    }

    #[test]
    fn load_stl_cube() {
        assert!(Model3D::load_path(&sample_path("cube.stl")).is_ok());
    }

    #[test]
    fn stl_binary_and_ascii_parse() {
        let mut bytes = vec![0u8; 84];

        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());

        bytes.extend_from_slice(&[0u8; 12]); // 法线

        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        bytes.extend_from_slice(&0u16.to_le_bytes()); // 属性字节

        let (pos, idx) = parse_binary_stl(&bytes).expect("binary stl");

        assert_eq!(pos.len(), 3);

        assert_eq!(idx, vec![0, 1, 2]);

        assert!(!is_ascii_stl(&bytes));

        let ascii = b"solid s\nfacet normal 0 0 1\nouter loop\nvertex 1 2 3\nvertex 4 5 6\nvertex 7 8 9\nendloop\nendfacet\nendsolid s";

        assert!(is_ascii_stl(ascii));

        let (pos, _) = parse_ascii_stl(ascii).expect("ascii stl");

        assert_eq!(pos[0], [1.0, 2.0, 3.0]);
    }

    #[test]
    fn load_3mf_cube() {
        let m = Model3D::load_path(&sample_path("cube.3mf")).expect("3mf");
        assert_eq!(m.triangle_count(), 12);
    }

    #[test]
    fn load_gcode_square() {
        assert!(Model3D::load_path(&sample_path("square.gcode")).is_ok());
    }

    #[test]
    fn render_cube_is_solid_silhouette() {
        let mut model = Model3D::load_path(&sample_path("cube.obj")).expect("obj");
        let pixels = model.render(120, 120);
        let bg = pixels[0];
        let body = pixels
            .iter()
            .filter(|p| **p != bg && (p.r() > 50 || p.g() > 50 || p.b() > 50))
            .count();
        assert!(
            body > 120 * 120 / 8,
            "silhouette too sparse: {}/{}",
            body,
            120 * 120
        );
    }

    #[test]
    fn blend_and_step_are_unsupported() {
        assert!(is_unsupported_3d_path(std::path::Path::new("a.blend")));
        assert!(is_unsupported_3d_path(std::path::Path::new("a.step")));
        assert!(!is_model_path(std::path::Path::new("a.blend")));
    }

    /// 2x2 贴图：v=0 是图像第一行（glTF 左上原点）→ 红 | 绿 / 蓝 | 白
    fn quad_tex() -> std::sync::Arc<Texture> {
        std::sync::Arc::new(Texture {
            width: 2,
            height: 2,
            rgba: std::sync::Arc::new(vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 255, //
            ]),
        })
    }

    fn solid_tex(v: u8) -> std::sync::Arc<Texture> {
        std::sync::Arc::new(Texture {
            width: 1,
            height: 1,
            rgba: std::sync::Arc::new(vec![v, v, v, 255]),
        })
    }

    #[test]
    fn sample_texture_is_bilinear_clamped_and_linear() {
        let tex = quad_tex();
        let (rgb, a) = sample_texture(&tex, 0.0, 0.0);
        assert!(
            rgb[0] > 0.99 && rgb[1] < 0.01 && rgb[2] < 0.01 && a > 0.99,
            "{rgb:?} {a}"
        );
        // 出界夹边，不环绕
        let (rgb, _) = sample_texture(&tex, -3.0, -3.0);
        assert!(rgb[0] > 0.99 && rgb[1] < 0.01, "{rgb:?}");
        let (rgb, _) = sample_texture(&tex, 4.0, 0.0);
        assert!(rgb[1] > 0.99 && rgb[0] < 0.01, "{rgb:?}");
        // 水平中点：红绿各半（sRGB 空间插值后转线性，两边相等且明显 <1）
        let (rgb, _) = sample_texture(&tex, 0.5, 0.0);
        assert!(
            (rgb[0] - rgb[1]).abs() < 1e-3 && rgb[0] > 0.15 && rgb[0] < 0.35,
            "{rgb:?}"
        );
        // 第二行是蓝/白
        let (rgb, _) = sample_texture(&tex, 0.0, 1.0);
        assert!(rgb[2] > 0.99 && rgb[0] < 0.01, "{rgb:?}");
    }

    fn flat_tri(uvs: Vec<[f32; 2]>, ids: Vec<u32>, textures: Vec<std::sync::Arc<Texture>>) -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            colors: None,
            indices: vec![0, 1, 2],
            uvs: Some(uvs),
            tex_ids: Some(ids),
            textures,
        }
    }

    #[test]
    fn tri_textured_requires_one_valid_texture_per_triangle() {
        let t = quad_tex();
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let ok = flat_tri(uv.to_vec(), vec![0, 0, 0], vec![t.clone()]);
        assert!(ok.tri_textured(0, 1, 2).is_some());
        assert!(ok.tri_texture(0, 1, 2) == Some(0));
        assert!(ok.vertex_textured(1));
        // 三个顶点跨两张贴图：拆不开就不采贴图，退回平面着色
        assert!(
            flat_tri(uv.to_vec(), vec![0, 0, 1], vec![t.clone(), solid_tex(9)])
                .tri_textured(0, 1, 2)
                .is_none()
        );
        assert!(
            flat_tri(uv.to_vec(), vec![0, 0, NO_TEXTURE], vec![t.clone()])
                .tri_textured(0, 1, 2)
                .is_none()
        );
        // 贴图号越界 / 表为空 / UV 长度不符 → 都不能采样
        assert!(
            flat_tri(uv.to_vec(), vec![0, 0, 7], vec![t.clone()])
                .tri_textured(0, 1, 2)
                .is_none()
        );
        assert!(
            flat_tri(uv.to_vec(), vec![0, 0, 0], vec![])
                .tri_textured(0, 1, 2)
                .is_none()
        );
        let short = flat_tri(vec![[0.0, 0.0]; 2], vec![0, 0, 0], vec![t]);
        assert!(short.tri_textured(0, 1, 2).is_none());
        assert!(!short.vertex_textured(0));
    }

    #[test]
    fn append_mesh_dedups_shared_textures_and_remaps_ids() {
        let shared = quad_tex();
        let second = solid_tex(9);
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

        let mut out = Mesh::default();
        out.append_mesh(
            flat_tri(uv.to_vec(), vec![0, 0, 0], vec![shared.clone()]),
            Mat4::IDENTITY,
        );
        out.append_mesh(
            flat_tri(
                uv.to_vec(),
                vec![0, 1, 1],
                vec![shared.clone(), second.clone()],
            ),
            Mat4::IDENTITY,
        );

        // 同一张 Arc 只入库一次，分片自己的 1 号图重映射成全局 1 号
        assert_eq!(out.textures.len(), 2);
        assert!(std::sync::Arc::ptr_eq(&out.textures[0], &shared));
        assert!(std::sync::Arc::ptr_eq(&out.textures[1], &second));
        assert_eq!(out.tex_ids.as_deref(), Some(&[0u32, 0, 0, 0, 1, 1][..]));
        let uvs = out.uvs.clone().unwrap();
        assert_eq!(uvs.len(), 6);
        assert_eq!(uvs[4], uv[1]);

        // 无贴图的分片并入有贴图的网格时补 NO_TEXTURE，不会被误判成贴图顶点
        let mut plain = Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            colors: None,
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        plain.append_mesh(
            flat_tri(uv.to_vec(), vec![0, 0, 0], vec![shared.clone()]),
            Mat4::IDENTITY,
        );
        assert_eq!(
            plain.tex_ids.as_deref(),
            Some(&[NO_TEXTURE, NO_TEXTURE, NO_TEXTURE, 0, 0, 0][..])
        );
        assert!(!plain.vertex_textured(0));
        assert!(plain.vertex_textured(4));
    }

    /// 手工拼一个带 baseColorTexture 的 GLB：立方体 + 2x2 PNG，不依赖仓库外部文件
    fn build_textured_glb() -> Vec<u8> {
        let img = image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
        .expect("2x2 rgba");
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        let png = png.into_inner();

        let pos: [[f32; 3]; 8] = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
        ];
        let uvs: [[f32; 2]; 8] = pos.map(|p| [p[0] + 0.5, p[1] + 0.5]);
        const IDX: [u16; 36] = [
            0, 1, 2, 2, 1, 3, // -z
            4, 6, 5, 5, 6, 7, // +z
            0, 4, 1, 1, 4, 5, // -y
            2, 6, 3, 3, 6, 7, // +y
            0, 2, 4, 4, 2, 6, // -x
            1, 3, 5, 5, 3, 7, // +x
        ];

        let mut bin: Vec<u8> = Vec::new();
        for p in &pos {
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let pos_view = (0usize, bin.len());
        for u in &uvs {
            for v in u {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let uv_view = (pos_view.1, bin.len() - pos_view.1);
        for i in &IDX {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let idx_view = (uv_view.0 + uv_view.1, bin.len() - uv_view.0 - uv_view.1);
        let img_view = (bin.len(), png.len());
        bin.extend_from_slice(&png);

        let json = serde_json::json!({
            "asset": { "version": "2.0", "generator": "dadaview-test" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{ "mesh": 0 }],
            "meshes": [{
                "primitives": [{
                    "attributes": { "POSITION": 0, "TEXCOORD_0": 1 },
                    "indices": 2,
                    "material": 0,
                }]
            }],
            "materials": [{
                "pbrMetallicRoughness": {
                    "baseColorTexture": { "index": 0, "texCoord": 0 },
                    "metallicFactor": 0.0,
                    "roughnessFactor": 1.0,
                }
            }],
            "textures": [{ "sampler": 0, "source": 0 }],
            "samplers": [{ "magFilter": 9729, "minFilter": 9729 }],
            "images": [{ "bufferView": 3, "mimeType": "image/png" }],
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": pos_view.0, "byteLength": pos_view.1, "target": 34962 },
                { "buffer": 0, "byteOffset": uv_view.0, "byteLength": uv_view.1, "target": 34962 },
                { "buffer": 0, "byteOffset": idx_view.0, "byteLength": idx_view.1, "target": 34963 },
                { "buffer": 0, "byteOffset": img_view.0, "byteLength": img_view.1 },
            ],
            "accessors": [
                {
                    "bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3",
                    "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5]
                },
                { "bufferView": 1, "componentType": 5126, "count": 8, "type": "VEC2" },
                { "bufferView": 2, "componentType": 5123, "count": 36, "type": "SCALAR" },
            ]
        });
        let mut json_bytes = serde_json::to_vec(&json).expect("json");
        let mut bin_bytes = bin;
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        while !bin_bytes.len().is_multiple_of(4) {
            bin_bytes.push(0);
        }
        let total = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&bin_bytes);
        out
    }

    #[test]
    fn load_glb_texture_reaches_pixels() {
        let mut path = std::env::temp_dir();
        path.push(format!("dadaview_textured_{}.glb", std::process::id()));
        std::fs::write(&path, build_textured_glb()).expect("write glb");
        let loaded = Model3D::load_path(&path);
        let _ = std::fs::remove_file(&path);
        let model = loaded.expect("textured glb");
        let (mesh, cam) = model.render_snapshot();

        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.triangle_count(), 12);
        assert_eq!(mesh.textures.len(), 1, "贴图没进 Mesh");
        assert_eq!((mesh.textures[0].width, mesh.textures[0].height), (2, 2));
        let (uvs, ids) = mesh.texture_channels().expect("UV / 贴图号通道");
        assert_eq!(uvs.len(), 8);
        assert_eq!(ids.len(), 8);
        assert!(ids.iter().all(|&i| i == 0));
        assert_eq!(uvs[0], [0.0, 0.0]);

        let pixels = render_mesh_ex(&mesh, &cam, 64, 64, false);
        let dominant = |ch: usize| {
            pixels
                .iter()
                .filter(|p| {
                    let v = [p.r() as i32, p.g() as i32, p.b() as i32];
                    let (hi, m1, m2) = match ch {
                        0 => (v[0], v[1], v[2]),
                        1 => (v[1], v[0], v[2]),
                        _ => (v[2], v[0], v[1]),
                    };
                    hi > 60 && hi > m1 + 50 && hi > m2 + 50
                })
                .count()
        };
        let (red, green, blue) = (dominant(0), dominant(1), dominant(2));
        assert!(
            red > 4 && green > 4 && blue > 4,
            "贴图未生效：red={red} green={green} blue={blue}"
        );
    }

    /// 用真实导出文件验证贴图（文件不入库）：
    /// `DADAVIEW_GLB=模型.glb cargo test -- --ignored load_real_glb_sample`
    #[test]
    #[ignore = "需要本机一个真实带贴图 GLB 路径"]
    fn load_real_glb_sample() {
        let path = std::env::var("DADAVIEW_GLB").expect("设置 DADAVIEW_GLB 指向 glb");
        let model =
            Model3D::load_path(Path::new(&path)).unwrap_or_else(|e| panic!("{path} 解析失败: {e}"));
        let (mesh, cam) = model.render_snapshot();
        let pixels = render_mesh_ex(&mesh, &cam, 128, 128, false);
        let bg = pixels[0];
        let (body, hues) = {
            let mut body = 0usize;
            let mut set = std::collections::HashSet::new();
            for p in &pixels {
                if *p != bg {
                    body += 1;
                    set.insert((p.r() / 24, p.g() / 24, p.b() / 24));
                }
            }
            (body, set.len())
        };
        println!(
            "{path}: {} 顶点 / {} 三角形 / {} 张贴图 / UV {} / 前景 {body} 像素 / {hues} 种色阶",
            mesh.vertex_count(),
            mesh.triangle_count(),
            mesh.textures.len(),
            mesh.uvs.as_ref().map_or(0, |u| u.len()),
        );
        assert!(mesh.triangle_count() >= 1);
        assert!(body > 128 * 128 / 20, "模型几乎没画出来：{body}");
        if !mesh.textures.is_empty() {
            assert!(hues > 8, "贴图内容没进画面：色阶 {hues}");
        }
    }
}
