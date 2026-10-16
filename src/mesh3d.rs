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

#[derive(Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Option<Vec<[f32; 3]>>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
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
    last_render_size: (usize, usize),
}

impl Model3D {
    pub fn load_path(path: &Path) -> anyhow::Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let mesh = match ext.as_str() {
            "obj" => load_obj(path)?,
            "stl" => load_stl(path)?,
            "gltf" | "glb" | "vrm" => load_gltf(path)?,
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
    let mut out = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        colors: None,
        indices: Vec::new(),
    };
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

fn load_gltf(path: &Path) -> anyhow::Result<Mesh> {
    let (document, buffers, _images) = gltf::import(path)?;
    let mut out = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        colors: None,
        indices: Vec::new(),
    };

    fn read_node(
        node: &gltf::Node,
        parent: Mat4,
        buffers: &[gltf::buffer::Data],
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
                let colors: Option<Vec<[f32; 3]>> =
                    reader.read_colors(0).map(|c| c.into_rgb_f32().collect());
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                if !matches!(primitive.mode(), gltf::mesh::Mode::Triangles) {
                    continue;
                }
                let mut piece = Mesh {
                    positions,
                    normals,
                    colors,
                    indices,
                };
                piece.ensure_normals();
                out.append_mesh(piece, world);
            }
        }
        for child in node.children() {
            read_node(&child, world, buffers, out)?;
        }
        Ok(())
    }

    for scene in document.scenes() {
        for node in scene.nodes() {
            read_node(&node, Mat4::IDENTITY, &buffers, &mut out)?;
        }
    }
    if out.positions.is_empty() {
        for node in document.nodes() {
            read_node(&node, Mat4::IDENTITY, &buffers, &mut out)?;
        }
    }
    if out.positions.is_empty() {
        anyhow::bail!("glTF 中没有三角网格");
    }
    out.ensure_normals();
    Ok(out)
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
    let mut out = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        colors: None,
        indices: Vec::new(),
    };
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
    let mut out = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        colors: None,
        indices: Vec::new(),
    };
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
    let mut out = Mesh {
        positions: Vec::new(),
        normals: Vec::new(),
        colors: None,
        indices: Vec::new(),
    };
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

fn load_fbx_binary(data: &[u8]) -> anyhow::Result<Mesh> {
    let find_bytes = |hay: &[u8], needle: &[u8]| -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    };
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    if let Some(pos) = find_bytes(data, b"Vertices")
        && let Some(arr) = extract_fbx_bin_array_f64(data, pos)
    {
        positions = arr;
    }
    if let Some(pos) = find_bytes(data, b"PolygonVertexIndex")
        && let Some(arr) = extract_fbx_bin_array_i32(data, pos)
    {
        let mut poly = Vec::new();
        for v in arr {
            if v < 0 {
                poly.push((-v - 1) as u32);
                fan_triangles(&mut indices, &poly);
                poly.clear();
            } else {
                poly.push(v as u32);
            }
        }
    }
    if positions.len() < 9 || indices.is_empty() {
        anyhow::bail!("二进制 FBX 暂无法可靠解析，请导出为 OBJ/GLB/STL");
    }
    let verts: Vec<[f32; 3]> = positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let n = verts.len() as u32;
    indices.retain(|i| *i < n);
    Ok(Mesh::from_positions_indices(verts, indices, None))
}

fn extract_fbx_bin_array_f64(data: &[u8], name_at: usize) -> Option<Vec<f32>> {
    let start = name_at + 8;
    for off in 0..80usize {
        let p = start + off;
        if p + 12 > data.len() {
            break;
        }
        let len = u32::from_le_bytes(data[p..p + 4].try_into().ok()?) as usize;
        if !(9..=50_000_000).contains(&len) {
            continue;
        }
        let enc = u32::from_le_bytes(data[p + 4..p + 8].try_into().ok()?);
        let comp = u32::from_le_bytes(data[p + 8..p + 12].try_into().ok()?);
        if enc != 0 || comp != 0 {
            continue;
        }
        let byte = len * 8;
        if p + 12 + byte > data.len() {
            continue;
        }
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let b = &data[p + 12 + i * 8..p + 12 + i * 8 + 8];
            out.push(f64::from_le_bytes(b.try_into().ok()?) as f32);
        }
        return Some(out);
    }
    None
}

fn extract_fbx_bin_array_i32(data: &[u8], name_at: usize) -> Option<Vec<i32>> {
    let start = name_at + 18;
    for off in 0..80usize {
        let p = start + off;
        if p + 12 > data.len() {
            break;
        }
        let len = u32::from_le_bytes(data[p..p + 4].try_into().ok()?) as usize;
        if !(3..=80_000_000).contains(&len) {
            continue;
        }
        let enc = u32::from_le_bytes(data[p + 4..p + 8].try_into().ok()?);
        let comp = u32::from_le_bytes(data[p + 8..p + 12].try_into().ok()?);
        if enc != 0 || comp != 0 {
            continue;
        }
        let byte = len * 4;
        if p + 12 + byte > data.len() {
            continue;
        }
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let b = &data[p + 12 + i * 4..p + 12 + i * 4 + 4];
            out.push(i32::from_le_bytes(b.try_into().ok()?));
        }
        return Some(out);
    }
    None
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

        let (cr0, cg0, cb0) = match &mesh.colors {
            Some(cols) if i0 < cols.len() && i1 < cols.len() && i2 < cols.len() => (
                (cols[i0][0] + cols[i1][0] + cols[i2][0]) / 3.0,
                (cols[i0][1] + cols[i1][1] + cols[i2][1]) / 3.0,
                (cols[i0][2] + cols[i1][2] + cols[i2][2]) / 3.0,
            ),
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
        let packed = Color32::from_rgb(
            (cr0 * shade * 255.0).min(255.0) as u8,
            (cg0 * shade * 255.0).min(255.0) as u8,
            (cb0 * shade * 255.0).min(255.0) as u8,
        );

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
                depth[di] = z;
                pixels[di] = packed;
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

    #[test]
    fn load_obj_cube() {
        assert!(Model3D::load_path(&sample_path("cube.obj")).is_ok());
    }

    #[test]
    fn load_off_cube() {
        assert!(Model3D::load_path(&sample_path("cube.off")).is_ok());
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
}
