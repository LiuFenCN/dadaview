// 隐藏 Windows 控制台窗口

#![windows_subsystem = "windows"]

mod gpu3d;
mod mesh3d;

use eframe::{NativeOptions, egui};

use egui::{Color32, RichText, TextureHandle};

use image::DynamicImage;

use image::imageops::FilterType;

use image::AnimationDecoder;

use rfd::FileDialog;

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use std::path::PathBuf;

use std::sync::{Arc, Mutex};

use std::thread;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dwm::{
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};

#[cfg(target_os = "windows")]
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
};
use windows::core::PCWSTR;

const ARROW_ZONE_W: f32 = 80.0;

const ARROW_ZONE_H: f32 = 100.0;

const MAX_TEXTURE_DIM: u32 = 2048;

const SUPPORTED_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif", "ico", "obj", "stl", "gltf", "glb",
    "ply", "off", "3ds", "dae", "vtk", "wrl", "vrml", "amf", "3mf", "gcode", "gco", "g", "vrm",
    "fbx", "pmx", "pmd", "usd", "usda", "usdc", "usdz",
];

/// 计算 2cm 对应的像素值（适配 DPI 缩放）
fn mini_size_pp(ppi: f32) -> f32 {
    (2.0 * 96.0 / 2.54 * ppi).round()
}

/// 人类可读文件大小
fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;

    const MB: u64 = 1024 * KB;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// 获取 exe 所在目录
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf()
}

fn settings_path() -> PathBuf {
    exe_dir().join("settings.json")
}

// ─── 快捷键配置 ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindings {
    pub prev: String,

    pub next: String,

    pub zoom_in: String,

    pub zoom_out: String,

    pub minimize: String, // 双击，特殊处理

    pub close: String, // 右键，特殊处理

    pub comic_mode: String,

    pub show_info: String, // 中键，特殊处理

    pub settings: String,

    // ── 3D 模型 ──
    pub model_orbit: String, // LeftDrag / MiddleDrag / ShiftLeftDrag

    pub model_pan: String, // MiddleDrag / ShiftLeftDrag / LeftDrag

    pub model_reset: String, // 键盘键
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            prev: "ArrowLeft".into(),

            next: "ArrowRight".into(),

            zoom_in: "ScrollUp".into(),

            zoom_out: "ScrollDown".into(),

            minimize: "DoubleClick".into(),

            close: "RightClick".into(),

            comic_mode: "C".into(),

            show_info: "MiddleClick".into(),

            settings: "F5".into(),

            model_orbit: "LeftDrag".into(),

            model_pan: "MiddleDrag".into(),

            model_reset: "R".into(),
        }
    }
}

impl KeyBindings {
    fn labels(&self) -> Vec<(&'static str, String)> {
        vec![
            ("上一页", self.prev.clone()),
            ("下一页", self.next.clone()),
            ("放大", self.zoom_in.clone()),
            ("缩小", self.zoom_out.clone()),
            ("最小化", self.minimize.clone()),
            ("关闭", self.close.clone()),
            ("漫画模式", self.comic_mode.clone()),
            ("图片信息", self.show_info.clone()),
            ("设置", self.settings.clone()),
            ("3D 复位", self.model_reset.clone()),
        ]
    }

    fn get_mut(&mut self, label: &str) -> Option<&mut String> {
        match label {
            "上一页" => Some(&mut self.prev),

            "下一页" => Some(&mut self.next),

            "放大" => Some(&mut self.zoom_in),

            "缩小" => Some(&mut self.zoom_out),

            "最小化" => Some(&mut self.minimize),

            "关闭" => Some(&mut self.close),

            "漫画模式" => Some(&mut self.comic_mode),

            "图片信息" => Some(&mut self.show_info),

            "设置" => Some(&mut self.settings),

            "3D 复位" => Some(&mut self.model_reset),

            _ => None,
        }
    }
}

fn drag_binding_label(code: &str) -> String {
    match code {
        "LeftDrag" => "左键拖动".into(),

        "MiddleDrag" => "中键拖动".into(),

        "ShiftLeftDrag" => "Shift+左键拖动".into(),

        other => other.to_string(),
    }
}

/// 将 egui Key 转为可读字符串
fn key_to_string(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowLeft => "ArrowLeft".into(),

        egui::Key::ArrowRight => "ArrowRight".into(),

        egui::Key::ArrowUp => "ArrowUp".into(),

        egui::Key::ArrowDown => "ArrowDown".into(),

        k => format!("{:?}", k),
    }
}

/// 将字符串解析回 egui::Key（部分支持）
fn string_to_key(s: &str) -> Option<egui::Key> {
    match s {
        "ArrowLeft" => Some(egui::Key::ArrowLeft),

        "ArrowRight" => Some(egui::Key::ArrowRight),

        "ArrowUp" => Some(egui::Key::ArrowUp),

        "ArrowDown" => Some(egui::Key::ArrowDown),

        "Escape" => Some(egui::Key::Escape),

        "Space" => Some(egui::Key::Space),

        "Enter" => Some(egui::Key::Enter),

        "C" => Some(egui::Key::C),

        "A" => Some(egui::Key::A),

        "B" => Some(egui::Key::B),

        "D" => Some(egui::Key::D),

        "Q" => Some(egui::Key::Q),

        "W" => Some(egui::Key::W),

        "E" => Some(egui::Key::E),

        "R" => Some(egui::Key::R),

        "F" => Some(egui::Key::F),

        "G" => Some(egui::Key::G),

        "H" => Some(egui::Key::H),

        "I" => Some(egui::Key::I),

        "J" => Some(egui::Key::J),

        "K" => Some(egui::Key::K),

        "L" => Some(egui::Key::L),

        "M" => Some(egui::Key::M),

        "N" => Some(egui::Key::N),

        "O" => Some(egui::Key::O),

        "P" => Some(egui::Key::P),

        "S" => Some(egui::Key::S),

        "T" => Some(egui::Key::T),

        "U" => Some(egui::Key::U),

        "V" => Some(egui::Key::V),

        "X" => Some(egui::Key::X),

        "Y" => Some(egui::Key::Y),

        "Z" => Some(egui::Key::Z),

        s if s.starts_with('F') => {
            // 显式枚举：egui::Key 判别式不连续，不能用数值推算
            match s {
                "F1" => Some(egui::Key::F1),
                "F2" => Some(egui::Key::F2),
                "F3" => Some(egui::Key::F3),
                "F4" => Some(egui::Key::F4),
                "F5" => Some(egui::Key::F5),
                "F6" => Some(egui::Key::F6),
                "F7" => Some(egui::Key::F7),
                "F8" => Some(egui::Key::F8),
                "F9" => Some(egui::Key::F9),
                "F10" => Some(egui::Key::F10),
                "F11" => Some(egui::Key::F11),
                "F12" => Some(egui::Key::F12),
                _ => None,
            }
        }

        _ => None,
    }
}

// ─── 设置 ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub keybindings: KeyBindings,

    pub associated_exts: Vec<String>,
}

impl AppSettings {
    fn load() -> Self {
        let path = settings_path();

        if path.exists()
            && let Ok(data) = std::fs::read_to_string(&path)
            && let Ok(s) = serde_json::from_str(&data)
        {
            return s;
        }

        Self::default()
    }

    fn save(&self) {
        let path = settings_path();

        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, data);
        }
    }
}

// ─── 图片数据 ───────────────────────────────────────

struct PendingImage {
    path: PathBuf,

    pixels: Vec<Color32>,

    tex_w: usize,

    tex_h: usize,

    orig_w: usize,

    orig_h: usize,
}

#[derive(Clone)]

struct PreloadedImage {
    path: PathBuf,

    tex_w: usize,

    tex_h: usize,

    pixels: Vec<Color32>,

    orig_w: usize,

    orig_h: usize,
}

/// 动画帧
struct AnimFrame {
    pixels: Vec<Color32>,

    tex_w: usize,

    tex_h: usize,

    delay_ms: u32, // 帧延迟（毫秒）
}

type ModelRenderResult = (usize, usize, Vec<Color32>, bool);

// ─── 主应用 ───────────────────────────────────────

struct PhotoViewApp {
    current_texture: Option<TextureHandle>,

    current_pixels: Vec<Color32>, // 当前纹理像素副本，用于箭头颜色自适应

    current_tex_size: [usize; 2], // [tex_w, tex_h]

    // 漫画模式第二张纹理
    comic_second_texture: Option<TextureHandle>,

    comic_second_w: usize,

    comic_second_h: usize,

    current_path: Option<PathBuf>,

    image_files: Vec<PathBuf>,

    last_scanned_dir: Option<PathBuf>,

    current_index: usize,

    zoom: f32,

    previous_zoom: f32,

    previous_window_size: Option<egui::Vec2>,

    is_minimized: bool,

    drag_started: bool,

    img_w: usize,

    img_h: usize,

    pending: Arc<Mutex<Option<PendingImage>>>,

    // 漫画模式第二张待加载
    pending_second: Arc<Mutex<Option<PendingImage>>>,

    loaded_next: Arc<Mutex<Option<PreloadedImage>>>,

    loaded_prev: Arc<Mutex<Option<PreloadedImage>>>,

    is_loading: bool,

    show_info: bool,

    round_corners_applied: bool,

    // 新功能
    comic_mode: bool,

    show_settings: bool,

    initial_path: Option<PathBuf>,

    settings: AppSettings,

    rebinding: Option<String>, // 正在重新绑定的快捷键标签

    file_assoc_check: BTreeMap<String, bool>, // 文件关联复选框状态

    deferred_resize_time: Option<f64>, // 翻页后延迟 resize 的时间戳

    // GIF 动画支持
    anim_frames: Vec<AnimFrame>, // 所有动画帧

    anim_textures: Vec<Option<TextureHandle>>, // 每帧纹理缓存，避免来回重传

    anim_current: usize, // 当前帧索引

    anim_next_time: f64, // 下一帧切换时间

    is_animated: bool, // 当前图片是否是动画

    anim_pending: Arc<Mutex<Option<Vec<AnimFrame>>>>, // 异步加载的 GIF 帧结果

    anim_paused: bool, // 逐帧模式暂停

    anim_timeline: bool, // 是否显示逐帧时间轴

    always_on_top: bool, // 置顶开关
    should_close_next_frame: bool,

    // 3D 模型查看
    model: Option<mesh3d::Model3D>,
    model_pending: Arc<Mutex<Option<Result<mesh3d::Model3D, String>>>>,
    viewing_3d: bool,
    model_error: Option<String>,
    middle_down: bool,
    middle_moved: bool,
    // 异步渲染
    model_render_out: Arc<Mutex<Option<ModelRenderResult>>>,
    model_render_busy: bool,
    model_render_wanted: Option<(usize, usize)>,
    model_interacting: bool,
    model_hq_after: Option<f64>,
    model_last_fast: bool,
    gpu3d: Arc<Mutex<Option<gpu3d::GpuRenderer>>>,
    gpu_init_started: bool,
    // 图片加载失败信号
    pending_error: Arc<Mutex<Option<String>>>,
    last_drop_path: Option<PathBuf>,
}

impl Default for PhotoViewApp {
    fn default() -> Self {
        let settings = AppSettings::load();

        let mut file_assoc_check = BTreeMap::new();

        for ext in SUPPORTED_EXTS {
            file_assoc_check.insert(
                ext.to_string(),
                settings.associated_exts.contains(&ext.to_string()),
            );
        }

        Self {
            current_texture: None,

            current_pixels: Vec::new(),

            current_tex_size: [0, 0],

            comic_second_texture: None,

            comic_second_w: 0,

            comic_second_h: 0,

            current_path: None,

            image_files: Vec::new(),
            last_scanned_dir: None,
            current_index: 0,

            zoom: 1.0,
            previous_zoom: 1.0,
            previous_window_size: None,

            is_minimized: false,
            drag_started: false,

            img_w: 0,
            img_h: 0,

            pending: Arc::new(Mutex::new(None)),

            pending_second: Arc::new(Mutex::new(None)),

            loaded_next: Arc::new(Mutex::new(None)),

            loaded_prev: Arc::new(Mutex::new(None)),

            is_loading: false,

            show_info: false,

            round_corners_applied: false,

            comic_mode: false,

            show_settings: false,

            initial_path: None,

            settings,

            rebinding: None,

            file_assoc_check,

            deferred_resize_time: None,

            anim_frames: Vec::new(),
            anim_textures: Vec::new(),

            anim_current: 0,

            anim_next_time: 0.0,

            is_animated: false,

            anim_pending: Arc::new(Mutex::new(None)),

            anim_paused: false,

            anim_timeline: false,

            always_on_top: false,
            should_close_next_frame: false,

            model: None,
            model_pending: Arc::new(Mutex::new(None)),
            viewing_3d: false,
            model_error: None,
            middle_down: false,
            middle_moved: false,
            model_render_out: Arc::new(Mutex::new(None)),
            model_render_busy: false,
            model_render_wanted: None,
            model_interacting: false,
            model_hq_after: None,
            model_last_fast: false,
            gpu3d: Arc::new(Mutex::new(None)),
            gpu_init_started: false,
            pending_error: Arc::new(Mutex::new(None)),
            last_drop_path: None,
        }
    }
}

fn resize_for_texture(img: DynamicImage) -> DynamicImage {
    let max_dim = img.width().max(img.height());

    if max_dim > MAX_TEXTURE_DIM {
        let scale = MAX_TEXTURE_DIM as f32 / max_dim as f32;

        let nw = (img.width() as f32 * scale).round() as u32;

        let nh = (img.height() as f32 * scale).round() as u32;

        img.resize_exact(nw, nh, FilterType::Triangle)
    } else {
        img
    }
}

fn rgba_to_color32(rgba: &[u8]) -> Vec<Color32> {
    let mut pixels = Vec::with_capacity(rgba.len() / 4);

    for chunk in rgba.as_chunks::<4>().0 {
        pixels.push(Color32::from_rgba_unmultiplied(
            chunk[0], chunk[1], chunk[2], chunk[3],
        ));
    }

    pixels
}

/// 把整帧抽成小图，仅用于箭头/UI 自适应配色
fn downsample_for_tint(src: &[Color32], w: usize, h: usize, side: usize) -> Vec<Color32> {
    if w == 0 || h == 0 || src.len() < w * h {
        return src.to_vec();
    }

    let side = side.max(4);

    let mut out = Vec::with_capacity(side * side);

    for y in 0..side {
        let sy = y * h / side;

        for x in 0..side {
            let sx = x * w / side;

            out.push(src[sy * w + sx]);
        }
    }

    out
}

/// 用内容嗅探打开图片，避免扩展名不匹配导致解码失败（如QQ截图保存为.jpg但实际是PNG）
fn open_image_robust(path: &std::path::Path) -> Result<image::DynamicImage, image::ImageError> {
    // 先尝试按扩展名打开（快）

    match image::open(path) {
        Ok(img) => Ok(img),

        Err(_) => {
            // 扩展名匹配失败，用 with_guessed_format 按内容嗅探

            image::ImageReader::open(path)?
                .with_guessed_format()?
                .decode()
        }
    }
}

/// 用内容嗅探获取图片尺寸，避免扩展名不匹配
fn image_dimensions_robust(path: &std::path::Path) -> Result<(u32, u32), image::ImageError> {
    // 先尝试按扩展名

    match image::image_dimensions(path) {
        Ok(dim) => Ok(dim),

        Err(_) => {
            // 回退：内容嗅探只读头部取尺寸，避免整图解码卡 UI 线程
            let reader = image::ImageReader::open(path)?.with_guessed_format()?;
            reader.into_dimensions()
        }
    }
}

fn load_image_async(
    path: PathBuf,
    dest: Arc<Mutex<Option<PendingImage>>>,
    err: Arc<Mutex<Option<String>>>,
) {
    thread::spawn(move || match open_image_robust(&path) {
        Ok(img) => {
            let orig_w = img.width() as usize;

            let orig_h = img.height() as usize;

            let tex = resize_for_texture(img);

            let tw = tex.width() as usize;

            let th = tex.height() as usize;

            let rgba = tex.to_rgba8();

            let pixels = rgba_to_color32(&rgba);

            *dest.lock().unwrap() = Some(PendingImage {
                path: path.clone(),
                pixels,
                tex_w: tw,
                tex_h: th,
                orig_w,
                orig_h,
            });
        }

        Err(e) => {
            *err.lock().unwrap() = Some(format!("无法打开图片: {}", e));
        }
    });
}

/// 异步加载 GIF 所有帧
fn load_gif_frames(path: &PathBuf, dest: Arc<Mutex<Option<Vec<AnimFrame>>>>) {
    use std::fs::File;

    use std::io::BufReader;

    if let Ok(file) = File::open(path) {
        let reader = BufReader::new(file);

        if let Ok(decoder) = image::codecs::gif::GifDecoder::new(reader) {
            let mut frames = Vec::new();

            for frame in decoder.into_frames() {
                if let Ok(frame) = frame {
                    let delay_ms = {
                        let (n, d) = frame.delay().numer_denom_ms();

                        if d == 0 { 100u32 } else { n / d }
                    };

                    let delay_ms = if delay_ms == 0 { 100 } else { delay_ms };

                    let mut buffer = frame.into_buffer();

                    // 如果帧太大，需要缩小

                    let max_dim = buffer.width().max(buffer.height());

                    if max_dim > MAX_TEXTURE_DIM {
                        let scale = MAX_TEXTURE_DIM as f32 / max_dim as f32;

                        let nw = (buffer.width() as f32 * scale).round() as u32;

                        let nh = (buffer.height() as f32 * scale).round() as u32;

                        let img = DynamicImage::ImageRgba8(buffer);

                        let resized = img.resize_exact(nw, nh, FilterType::Triangle);

                        buffer = resized.into_rgba8();
                    }

                    let tw = buffer.width() as usize;

                    let th = buffer.height() as usize;

                    let pixels: Vec<Color32> = buffer
                        .pixels()
                        .map(|p| Color32::from_rgba_unmultiplied(p.0[0], p.0[1], p.0[2], p.0[3]))
                        .collect();

                    frames.push(AnimFrame {
                        pixels,
                        tex_w: tw,
                        tex_h: th,
                        delay_ms,
                    });
                }
            }

            if frames.len() > 1 {
                *dest.lock().unwrap() = Some(frames);
            }
        }
    }
}

fn preload_async(path: PathBuf, dest: Arc<Mutex<Option<PreloadedImage>>>) {
    thread::spawn(move || {
        if let Ok(img) = open_image_robust(&path) {
            let orig_w = img.width() as usize;

            let orig_h = img.height() as usize;

            let tex = resize_for_texture(img);

            let tw = tex.width() as usize;

            let th = tex.height() as usize;

            let rgba = tex.to_rgba8();

            let pixels = rgba_to_color32(&rgba);

            *dest.lock().unwrap() = Some(PreloadedImage {
                path: path.clone(),
                tex_w: tw,
                tex_h: th,
                pixels,
                orig_w,
                orig_h,
            });
        }
    });
}

/// 根据纹理像素在指定区域（屏幕坐标）的平均亮度，返回箭头颜色（黑或白）
/// screen_rect 是当前窗口区域，arrow_pos 是箭头中心点
fn adaptive_arrow_color(
    pixels: &[Color32],
    tex_size: [usize; 2],
    img_w: usize,
    img_h: usize,
    zoom: f32,
    screen_rect: egui::Rect,
    arrow_center: egui::Pos2,
) -> Color32 {
    let [tw, th] = tex_size;

    if tw == 0 || th == 0 || pixels.is_empty() {
        return Color32::WHITE;
    }

    // 将箭头位置转换为纹理像素坐标

    // 图片在窗口中居中显示，先算图片在窗口中的偏移

    let display_w = img_w as f32 * zoom;

    let display_h = img_h as f32 * zoom;

    let offset_x = (screen_rect.width() - display_w) / 2.0;

    let offset_y = (screen_rect.height() - display_h) / 2.0;

    // 箭头中心相对窗口左上角 → 相对图片左上角 → 归一化到纹理坐标

    let rel_x = arrow_center.x - screen_rect.left() - offset_x;

    let rel_y = arrow_center.y - screen_rect.top() - offset_y;

    // 缩放到纹理像素坐标

    let px = (rel_x / display_w * tw as f32) as isize;

    let py = (rel_y / display_h * th as f32) as isize;

    // 采样一个区域（中心±8像素范围），计算平均亮度

    let r = 8;

    let mut total_luma = 0u32;

    let mut count = 0u32;

    for dy in -r..=r {
        for dx in -r..=r {
            let sx = px + dx;

            let sy = py + dy;

            if sx >= 0 && sx < tw as isize && sy >= 0 && sy < th as isize {
                let c = pixels[sy as usize * tw + sx as usize];

                let luma = c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114;

                total_luma += luma / 1000;

                count += 1;
            }
        }
    }

    if count == 0 {
        return Color32::WHITE;
    }

    let avg_luma = total_luma / count;

    // 亮度>128用黑箭头，否则用白箭头

    if avg_luma > 140 {
        Color32::from_rgba_unmultiplied(0, 0, 0, 180) // 半透明黑
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 200) // 半透明白
    }
}

fn in_arrow_zone(pos: egui::Pos2, screen: egui::Rect) -> bool {
    let cy = screen.center().y;

    let left_zone = egui::Rect::from_min_max(
        egui::pos2(screen.left(), cy - ARROW_ZONE_H / 2.0),
        egui::pos2(screen.left() + ARROW_ZONE_W, cy + ARROW_ZONE_H / 2.0),
    );

    let right_zone = egui::Rect::from_min_max(
        egui::pos2(screen.right() - ARROW_ZONE_W, cy - ARROW_ZONE_H / 2.0),
        egui::pos2(screen.right(), cy + ARROW_ZONE_H / 2.0),
    );

    left_zone.contains(pos) || right_zone.contains(pos)
}

#[cfg(target_os = "windows")]
fn apply_win11_effects() {
    unsafe {
        let title: Vec<u16> = "PhotoView\0".encode_utf16().collect();

        if let Ok(hwnd) = FindWindowW(None, PCWSTR(title.as_ptr())) {
            // Win11 圆角

            let corner = DWMWCP_ROUND;

            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );

            // Win11 22H2+ 毛玻璃/Acrylic 效果

            // DWMWA_SYSTEMBACKDROP_TYPE: 0=None, 1=Auto, 2=Mica, 3=Acrylic, 4=Tabbed

            let backdrop: i32 = 3; // Acrylic

            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const _ as *const _,
                std::mem::size_of_val(&backdrop) as u32,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]

fn apply_win11_effects() {}

/// 注册文件关联到 Windows 注册表（仅 HKCU）
fn register_file_association(exts: &[String], set_default: bool) -> Result<(), String> {
    use winreg::enums::*;

    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let exe_path = std::env::current_exe().map_err(|e| format!("无法获取 exe 路径: {}", e))?;

    let exe_str = exe_path.to_str().ok_or("exe 路径包含非法字符")?;

    // 注册 ProgID

    let prog_id = "PhotoView.Image";

    let (prog_key, _) = hkcu
        .create_subkey(format!("SOFTWARE\\Classes\\{}", prog_id))
        .map_err(|e| format!("创建 ProgID 失败: {}", e))?;

    prog_key
        .set_value("", &"PhotoView 图片查看器")
        .map_err(|e| e.to_string())?;

    prog_key
        .set_value("FriendlyTypeName", &"PhotoView 图片查看器")
        .map_err(|e| e.to_string())?;

    let (shell_key, _) = prog_key
        .create_subkey("shell\\open\\command")
        .map_err(|e| format!("创建 command 失败: {}", e))?;

    shell_key
        .set_value("", &format!("\"{}\" \"%1\"", exe_str))
        .map_err(|e| e.to_string())?;

    let (icon_key, _) = prog_key
        .create_subkey("DefaultIcon")
        .map_err(|e| format!("创建 DefaultIcon 失败: {}", e))?;

    icon_key
        .set_value("", &format!("\"{}\",0", exe_str))
        .map_err(|e| e.to_string())?;

    // 关联各扩展名

    for ext in exts {
        let ext_key_name = format!("SOFTWARE\\Classes\\.{}", ext);

        let (ext_key, _) = hkcu
            .create_subkey(&ext_key_name)
            .map_err(|e| format!("创建 .{} 键失败: {}", ext, e))?;

        // OpenWithProgids — 出现在"打开方式"列表

        let (open_with, _) = ext_key
            .create_subkey("OpenWithProgids")
            .map_err(|e| format!("创建 OpenWithProgids 失败: {}", e))?;

        open_with
            .set_value(prog_id, &"")
            .map_err(|e| e.to_string())?;

        if set_default {
            // 设置默认值 — 在 HKCU\SOFTWARE\Classes 下设置

            ext_key.set_value("", &prog_id).map_err(|e| e.to_string())?;

            // 同时写入 FileExts 以增强兼容性

            let file_exts_path = format!(
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{}",
                ext
            );

            if let Ok((fe_key, _)) = hkcu.create_subkey(&file_exts_path) {
                // 写入 ProgID 到 OpenWithList

                if let Ok((owl_key, _)) = fe_key.create_subkey("OpenWithList") {
                    let _ = owl_key.set_value("a", &"PhotoView.exe");

                    if let Ok(mrulist) = fe_key.open_subkey("OpenWithList") {
                        let _ = mrulist.set_value("MRUList", &"a");
                    }
                }

                // 写入 ProgID 到 OpenWithProgids

                if let Ok((owp_key, _)) = fe_key.create_subkey("OpenWithProgids") {
                    let _ = owp_key.set_value(prog_id, &0u32);
                }
            }

            // 修复旧版 ProgID（如 Photoview.png）的 command 路径和 DefaultIcon

            // 旧版安装可能注册了 Photoview.ext 格式的 ProgID，其 exe 路径可能已失效

            let old_prog_id = format!("Photoview.{}", ext);

            if let Ok(_old_key) = hkcu.open_subkey(format!("SOFTWARE\\Classes\\{}", old_prog_id)) {
                // 旧 ProgID 存在，更新 command 和 DefaultIcon 指向当前 exe

                if let Ok((fix_key, _)) = hkcu.create_subkey(format!(
                    "SOFTWARE\\Classes\\{}\\shell\\open\\command",
                    old_prog_id
                )) {
                    let _ = fix_key.set_value("", &format!("\"{}\" \"%1\"", exe_str));
                }

                if let Ok((icon_key, _)) =
                    hkcu.create_subkey(format!("SOFTWARE\\Classes\\{}\\DefaultIcon", old_prog_id))
                {
                    let _ = icon_key.set_value("", &format!("\"{}\",0", exe_str));
                }

                // 更新友好名称

                if let Ok(name_key) =
                    hkcu.open_subkey(format!("SOFTWARE\\Classes\\{}", old_prog_id))
                {
                    let _ = name_key.set_value("", &"PhotoView 图片查看器");

                    let _ = name_key.set_value("FriendlyTypeName", &"PhotoView 图片查看器");
                }
            }
        }
    }

    // 通知 shell 刷新

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::Shell::SHChangeNotify;

        use windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED;

        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            windows::Win32::UI::Shell::SHCNF_IDLIST,
            None,
            None,
        );
    }

    Ok(())
}

impl PhotoViewApp {
    fn scan_dir(&mut self, dir: &std::path::Path) {
        if !dir.is_dir() {
            return;
        }
        // 同一目录跳过：网络盘/超大目录不重复扫描
        if self.last_scanned_dir.as_deref() == Some(dir) {
            return;
        }

        self.image_files.clear();

        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if let Some(ext) = e.path().extension().and_then(|e| e.to_str()) {
                    let lower = ext.to_lowercase();

                    if SUPPORTED_EXTS.contains(&lower.as_str()) {
                        self.image_files.push(e.path());
                    }
                }
            }
        }

        self.image_files.sort();
        self.last_scanned_dir = Some(dir.to_path_buf());
    }

    fn open_image(&mut self, path: PathBuf, ctx: &egui::Context) {
        if let Some(d) = path.parent() {
            self.scan_dir(d);
        }

        // 打开新图片时总是退出漫画模式

        if self.comic_mode {
            self.comic_mode = false;

            self.comic_second_texture = None;

            self.comic_second_w = 0;

            self.comic_second_h = 0;
        }

        let idx = self.image_files.iter().position(|p| *p == path);

        if let Some(i) = idx {
            self.current_index = i;
        }

        self.current_path = Some(path.clone());

        self.last_drop_path = Some(path.clone());

        self.is_minimized = false;

        self.is_loading = true;

        self.deferred_resize_time = None; // 首次打开不延迟

        // 清除动画状态

        self.anim_frames.clear();

        self.anim_current = 0;

        self.is_animated = false;

        self.anim_paused = false;

        self.anim_timeline = false;

        *self.anim_pending.lock().unwrap() = None;

        // 3D 模型走独立加载路径

        if mesh3d::is_model_path(&path) {
            self.open_model(path, ctx);

            return;
        }

        // 明确不支持的 3D/CAD 格式：立刻报错，避免卡在加载中

        if mesh3d::is_unsupported_3d_path(&path) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("?")
                .to_uppercase();

            self.viewing_3d = false;

            self.model = None;

            self.current_texture = None;

            self.is_loading = false;

            self.model_error = Some(format!(
                ".{} 暂不支持预览，请转换为 STL/3MF/OBJ/GLB 等",
                ext
            ));

            self.last_drop_path = Some(path);

            return;
        }

        self.viewing_3d = false;

        self.model = None;

        self.model_error = None;

        *self.model_pending.lock().unwrap() = None;

        *self.pending_error.lock().unwrap() = None;

        self.model_render_busy = false;

        self.model_render_wanted = None;

        *self.model_render_out.lock().unwrap() = None;

        if let Ok((w, h)) = image_dimensions_robust(&path) {
            self.img_w = w as usize;

            self.img_h = h as usize;

            if self.comic_mode {
                self.fit_zoom_comic(ctx);

                self.resize_for_comic_mode(ctx);
            } else {
                let monitor = ctx.input(|i| {
                    i.viewport()
                        .monitor_size
                        .unwrap_or(egui::vec2(1920.0, 1080.0))
                });

                self.zoom = self.fit_zoom_for(self.img_w, self.img_h, monitor);

                self.resize_for_single(ctx);
            }
        }

        *self.pending.lock().unwrap() = None;

        load_image_async(path, self.pending.clone(), self.pending_error.clone());

        self.load_comic_second(ctx);

        self.preload_neighbors();
    }

    /// 打开 3D 模型文件
    fn open_model(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.viewing_3d = true;

        self.comic_mode = false;

        self.comic_second_texture = None;

        self.comic_second_w = 0;

        self.comic_second_h = 0;

        self.model = None;

        self.model_error = None;

        self.current_texture = None;

        self.current_pixels.clear();

        self.current_tex_size = [0, 0];

        self.img_w = 0;

        self.img_h = 0;

        self.zoom = 1.0;

        self.is_animated = false;

        self.anim_frames.clear();

        self.is_loading = true;

        self.is_minimized = false;

        self.deferred_resize_time = None;

        self.model_render_busy = false;

        self.model_render_wanted = None;

        *self.model_render_out.lock().unwrap() = None;

        self.last_drop_path = Some(path.clone());

        // 3D 模式使用固定合理窗口尺寸

        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        let w = (monitor.x * 0.65).clamp(800.0, 1280.0);

        let h = (monitor.y * 0.70).clamp(600.0, 900.0);

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));

        *self.model_pending.lock().unwrap() = None;

        let dest = self.model_pending.clone();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mesh3d::load_model(&path).map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("模型加载崩溃（文件可能损坏或格式不完整）".into()));

            *dest.lock().unwrap() = Some(result);
        });
    }

    fn load_image_keep_size(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.current_path = Some(path.clone());

        self.is_minimized = false;

        self.is_loading = true;

        // 清除动画状态

        self.anim_frames.clear();

        self.anim_current = 0;

        self.is_animated = false;

        self.anim_paused = false;

        self.anim_timeline = false;

        *self.anim_pending.lock().unwrap() = None;

        if let Ok((w, h)) = image_dimensions_robust(&path) {
            self.img_w = w as usize;

            self.img_h = h as usize;

            if self.comic_mode {
                self.fit_zoom_comic(ctx);

                // 不立即 resize，等延迟触发
            } else {
                let monitor = ctx.input(|i| {
                    i.viewport()
                        .monitor_size
                        .unwrap_or(egui::vec2(1920.0, 1080.0))
                });

                self.zoom = self.fit_zoom_for(self.img_w, self.img_h, monitor);
            }

            // 延迟 resize：2秒后

            let now = ctx.input(|i| i.time);

            self.deferred_resize_time = Some(now + 1.0);
        }

        *self.pending.lock().unwrap() = None;

        load_image_async(path, self.pending.clone(), self.pending_error.clone());

        self.load_comic_second(ctx);

        self.preload_neighbors();
    }

    fn apply_pending(&mut self, ctx: &egui::Context) {
        // 图片加载失败：结束加载态并提示

        if let Some(err) = self.pending_error.lock().unwrap().take()
            && !self.viewing_3d
        {
            self.is_loading = false;

            self.model_error = Some(err);
        }

        let maybe_pending = self.pending.lock().unwrap().take();
        let pending = match maybe_pending {
            Some(p) if Some(p.path.as_path()) == self.current_path.as_deref() => Some(p),
            Some(_) => None, // 路径已变：丢弃旧线程结果，防止显示错图
            None => None,
        };

        if let Some(pending) = pending {
            let color_image = egui::ColorImage {
                size: [pending.tex_w, pending.tex_h],
                pixels: pending.pixels.clone(),
            };

            let texture = ctx.load_texture("image", color_image, egui::TextureOptions::default());

            self.current_texture = Some(texture);

            self.current_pixels = pending.pixels.clone();

            self.current_tex_size = [pending.tex_w, pending.tex_h];

            self.img_w = pending.orig_w;

            self.img_h = pending.orig_h;

            self.is_loading = false;

            // 检测是否是动画文件，如果是则异步加载所有帧

            self.anim_frames.clear();

            self.anim_current = 0;

            self.is_animated = false;

            if let Some(ref path) = self.current_path {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();

                if ext == "gif" {
                    self.is_animated = true;

                    // 保存第一帧

                    self.anim_frames.push(AnimFrame {
                        pixels: pending.pixels,

                        tex_w: pending.tex_w,

                        tex_h: pending.tex_h,

                        delay_ms: 100, // 默认
                    });

                    let now = ctx.input(|i| i.time);

                    self.anim_next_time = now + 0.1;

                    // 异步加载剩余帧

                    let path_clone = path.clone();

                    let anim_dest = self.anim_pending.clone();

                    thread::spawn(move || {
                        load_gif_frames(&path_clone, anim_dest);
                    });
                }
            }

            self.preload_neighbors();
        }

        // 漫画模式第二张

        let maybe_second = self.pending_second.lock().unwrap().take();

        if let Some(pending) = maybe_second {
            let color_image = egui::ColorImage {
                size: [pending.tex_w, pending.tex_h],
                pixels: pending.pixels,
            };

            let texture =
                ctx.load_texture("comic_second", color_image, egui::TextureOptions::default());

            self.comic_second_texture = Some(texture);

            self.comic_second_w = pending.orig_w;

            self.comic_second_h = pending.orig_h;

            // 第二张加载完成，重新计算 zoom

            if self.comic_mode {
                self.fit_zoom_comic(ctx);

                // 如果没有延迟 resize 在等待，立即 resize；否则等延迟触发

                if self.deferred_resize_time.is_none() {
                    self.resize_for_comic_mode(ctx);
                }
            }
        }
    }

    /// 应用异步加载完成的 3D 模型，并启动/收取后台渲染
    fn apply_model_pending(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);

        // 交互结束后尽快补一帧高清

        if let Some(t) = self.model_hq_after
            && now >= t
        {
            self.model_hq_after = None;

            self.model_interacting = false;

            self.model_last_fast = true; // 强制触发 HQ

            if let Some(m) = &mut self.model {
                m.mark_dirty();
            }

            self.model_render_wanted =
                Some(self.model_render_size(ctx.screen_rect().width(), ctx.screen_rect().height()));
        }

        let maybe = self.model_pending.lock().unwrap().take();

        if let Some(result) = maybe {
            match result {
                Ok(model) => {
                    self.model_error = None;

                    self.model = Some(model);

                    // 先快速出一帧，再补高清

                    self.model_interacting = true;

                    self.model_hq_after = Some(ctx.input(|i| i.time) + 0.05);
                }

                Err(e) => {
                    self.model = None;

                    self.model_error = Some(e);

                    self.viewing_3d = false;

                    self.is_loading = false;

                    self.model_render_busy = false;

                    self.model_render_wanted = None;

                    self.model_interacting = false;

                    self.model_hq_after = None;
                }
            }
        }

        // 收取后台渲染结果

        let maybe_px = self.model_render_out.lock().unwrap().take();

        if let Some((rw, rh, pixels, was_fast)) = maybe_px {
            self.model_render_busy = false;

            self.model_last_fast = was_fast;

            let sample = downsample_for_tint(&pixels, rw, rh, 40);

            let color_image = egui::ColorImage {
                size: [rw, rh],
                pixels,
            };

            let texture = ctx.load_texture("image", color_image, egui::TextureOptions::LINEAR);

            self.current_texture = Some(texture);

            self.current_pixels = sample;

            self.current_tex_size = [40, 40];

            self.img_w = rw;

            self.img_h = rh;

            self.zoom = 1.0;

            self.is_loading = false;

            if let Some(m) = &mut self.model {
                m.note_rendered(rw, rh);
            }
        }

        if !self.viewing_3d {
            return;
        }

        let size = ctx.screen_rect().size();

        let target = self.model_render_size(size.x, size.y);

        let mut need = self
            .model
            .as_ref()
            .map(|m| m.needs_redraw || m.last_size() != target)
            .unwrap_or(false);

        // 静止时若上一帧是快速预览，必须再出一帧高清

        if !self.model_interacting && self.model_last_fast {
            need = true;
        }

        if self.model_render_wanted.is_some() {
            need = true;
        }

        if need && self.model.is_some() {
            if self.model_render_busy {
                self.model_render_wanted = Some(target);

                if let Some(m) = &mut self.model {
                    m.mark_dirty();
                }
            } else {
                let (rw, rh) = self.model_render_wanted.take().unwrap_or(target);

                self.start_model_render(ctx, rw, rh);
            }
        }

        if self.model_render_busy {
            ctx.request_repaint();
        } else if self.model_hq_after.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn start_model_render(&mut self, ctx: &egui::Context, rw: usize, rh: usize) {
        let Some(model) = &self.model else { return };

        // 后台预热 GPU（初始化可能很慢/卡住，绝不能挡住出图）

        if !self.gpu_init_started {
            self.gpu_init_started = true;

            let gpu = self.gpu3d.clone();

            thread::spawn(move || {
                if let Some(g) = gpu3d::GpuRenderer::new() {
                    *gpu.lock().unwrap() = Some(g);
                }
            });
        }

        let (mesh, cam) = model.render_snapshot();

        let dest = self.model_render_out.clone();

        let fast = self.model_interacting;

        let gpu = self.gpu3d.clone();

        self.model_render_busy = true;

        if let Some(m) = &mut self.model {
            m.needs_redraw = false;
        }

        thread::spawn(move || {
            let key = gpu3d::mesh_upload_key(&mesh);

            let mut pixels = None;

            // 静止帧尝试 GPU，限时 1s，超时/失败立刻软件渲染，避免卡在“渲染中”

            if !fast {
                let gpu_c = gpu.clone();

                let mesh_c = mesh.clone();

                let cam_c = cam.clone();

                let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<Color32>>>();

                thread::spawn(move || {
                    let px = gpu_c.try_lock().ok().and_then(|mut slot| {
                        let g = slot.as_mut()?;

                        g.upload_mesh(&mesh_c, key);

                        g.render(&cam_c, rw as u32, rh as u32)
                    });

                    let _ = tx.send(px);
                });

                if let Ok(px) = rx.recv_timeout(std::time::Duration::from_millis(1000)) {
                    pixels = px;
                }
            }

            let pixels =
                pixels.unwrap_or_else(|| mesh3d::render_mesh_ex(&mesh, &cam, rw, rh, fast));

            *dest.lock().unwrap() = Some((rw, rh, pixels, fast));
        });

        ctx.request_repaint();
    }

    fn model_render_size(&self, screen_w: f32, screen_h: f32) -> (usize, usize) {
        let sw = screen_w.max(2.0);

        let sh = screen_h.max(2.0);

        // 仅拖动时降分辨率；静止时尽量接近窗口尺寸

        let max_side = if self.model_interacting {
            400.0f32
        } else {
            1200.0f32
        };

        let scale = (max_side / sw.max(sh)).min(1.0);

        let lo = if self.model_interacting { 240 } else { 400 };

        let hi = if self.model_interacting { 400 } else { 1200 };

        let rw = ((sw * scale).round() as usize).clamp(lo, hi);

        let rh = ((sh * scale).round() as usize).clamp(lo, hi);

        (rw, rh)
    }

    fn note_model_interaction(&mut self, ctx: &egui::Context) {
        self.model_interacting = true;

        let now = ctx.input(|i| i.time);

        self.model_hq_after = Some(now + 0.12);

        ctx.request_repaint();
    }

    fn reset_model_view(&mut self) {
        if let Some(model) = &mut self.model {
            model.camera.reset();

            model.mark_dirty();
        }
    }

    /// 消费槽位仅当路径匹配；不匹配则原样保留，防止旧线程/另一方向的数据写上当前图
    fn take_slot_if_match<T>(
        slot: &Arc<Mutex<Option<T>>>,
        expect: &std::path::Path,
        get_path: impl Fn(&T) -> &PathBuf,
    ) -> Option<T> {
        let mut guard = slot.lock().unwrap();
        let matched = match &*guard {
            Some(v) => get_path(v) == expect,
            None => false,
        };
        if matched {
            let mut take = None;
            std::mem::swap(&mut *guard, &mut take);
            take
        } else {
            None
        }
    }

    fn load_comic_second(&mut self, _ctx: &egui::Context) {
        self.comic_second_texture = None;

        self.comic_second_w = 0;

        self.comic_second_h = 0;

        *self.pending_second.lock().unwrap() = None;

        if !self.comic_mode || self.image_files.len() < 2 {
            return;
        }

        let l = self.image_files.len();

        let next_idx = (self.current_index + 1) % l;

        let next_path = self.image_files[next_idx].clone();

        // 先看预加载是否命中（路径必须匹配，否则弃用重读）
        let maybe_pre = Self::take_slot_if_match(&self.loaded_next, &next_path, |p| &p.path);
        if let Some(pre) = maybe_pre {
            *self.pending_second.lock().unwrap() = Some(PendingImage {
                path: next_path,
                pixels: pre.pixels,

                tex_w: pre.tex_w,

                tex_h: pre.tex_h,

                orig_w: pre.orig_w,

                orig_h: pre.orig_h,
            });
        } else {
            load_image_async(
                next_path,
                self.pending_second.clone(),
                Arc::new(Mutex::new(None)),
            );
        }
    }

    fn preload_neighbors(&mut self) {
        if self.image_files.is_empty() {
            return;
        }

        let l = self.image_files.len();

        let step = if self.comic_mode { 2 } else { 1 };

        let next_idx = (self.current_index + step) % l;

        let prev_idx = (self.current_index + l - step) % l;

        // 3D 模型不走图片预加载

        if !mesh3d::is_model_path(&self.image_files[next_idx]) {
            preload_async(self.image_files[next_idx].clone(), self.loaded_next.clone());
        }

        if !mesh3d::is_model_path(&self.image_files[prev_idx]) {
            preload_async(self.image_files[prev_idx].clone(), self.loaded_prev.clone());
        }
    }

    /// 根据图片尺寸和 monitor 上限计算 fit zoom（不依赖当前窗口大小）
    fn fit_zoom_for(&self, img_w: usize, img_h: usize, monitor: egui::Vec2) -> f32 {
        if img_w > 0 && img_h > 0 {
            let max_w = monitor.x * 0.85;

            let max_h = monitor.y * 0.9;

            ((max_w / img_w as f32).min(max_h / img_h as f32)).min(1.0)
        } else {
            1.0
        }
    }

    fn fit_zoom(&mut self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        self.zoom = self.fit_zoom_for(self.img_w, self.img_h, monitor);
    }

    /// 设置单图模式的窗口大小（根据 zoom 和图片尺寸）
    fn resize_for_single(&self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        let max_w = monitor.x * 0.95;

        let max_h = monitor.y * 0.95;

        let w = (self.img_w as f32 * self.zoom).clamp(200.0, max_w);

        let h = (self.img_h as f32 * self.zoom).clamp(200.0, max_h);

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
    }

    /// 漫画模式 zoom：两张图并排适配窗口
    /// 必须与绘制逻辑一致：两张图按各自宽高比缩放到统一 display_h
    fn fit_zoom_comic(&mut self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        let max_w = monitor.x * 0.85;

        let max_h = monitor.y * 0.9;

        if self.img_w > 0 && self.img_h > 0 {
            let has_second = self.image_files.len() > 1;

            let sec_w = if self.comic_second_w > 0 {
                self.comic_second_w
            } else {
                self.img_w
            };

            let sec_h = if self.comic_second_h > 0 {
                self.comic_second_h
            } else {
                self.img_h
            };

            let h1 = self.img_h as f32;

            let h2 = sec_h as f32;

            let max_h_img = h1.max(h2);

            if has_second {
                // display_h = max_h_img * z

                // dw1 = img_w * display_h / h1 = img_w * max_h_img * z / h1

                // dw2 = sec_w * display_h / h2 = sec_w * max_h_img * z / h2

                // total_w = max_h_img * z * (img_w/h1 + sec_w/h2)

                // 约束 total_w <= max_w => z <= max_w / (max_h_img * (img_w/h1 + sec_w/h2))

                // 约束 display_h <= max_h => z <= max_h / max_h_img

                let width_factor =
                    max_h_img * (self.img_w as f32 / h1.max(1.0) + sec_w as f32 / h2.max(1.0));

                self.zoom = (max_w / width_factor).min(max_h / max_h_img).min(1.0);
            } else {
                self.zoom = (max_w / self.img_w as f32).min(max_h / h1).min(1.0);
            }
        }
    }

    fn zoom_image(&mut self, delta: f32, ctx: &egui::Context) {
        let new_zoom = if delta < 0.0 {
            (self.zoom / 1.15).max(0.02)
        } else {
            (self.zoom * 1.15).min(20.0)
        };

        self.zoom = new_zoom;

        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        let max_w = monitor.x * 0.95;

        let max_h = monitor.y * 0.95;

        if self.comic_mode {
            let has_second = self.image_files.len() > 1;

            let sec_w = if self.comic_second_w > 0 {
                self.comic_second_w
            } else {
                self.img_w
            };

            let sec_h = if self.comic_second_h > 0 {
                self.comic_second_h
            } else {
                self.img_h
            };

            let total_w = if has_second {
                self.img_w as f32 + sec_w as f32
            } else {
                self.img_w as f32
            };

            let max_h_img = self.img_h.max(sec_h) as f32;

            let w = (total_w * self.zoom).clamp(80.0, max_w);

            let h = (max_h_img * self.zoom).clamp(80.0, max_h);

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        } else {
            let w = (self.img_w as f32 * self.zoom).clamp(80.0, max_w);

            let h = (self.img_h as f32 * self.zoom).clamp(80.0, max_h);

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }
    }

    fn next_img(&mut self, ctx: &egui::Context) {
        if self.image_files.is_empty() {
            return;
        }

        let step = if self.comic_mode { 2 } else { 1 };

        let l = self.image_files.len();

        let next_idx = (self.current_index + step) % l;

        let path = self.image_files[next_idx].clone();

        if mesh3d::is_model_path(&path) {
            self.open_image(path, ctx);

            return;
        }

        // 从 3D 切回图片时清理模型状态

        self.viewing_3d = false;

        self.model = None;

        self.model_error = None;

        *self.model_pending.lock().unwrap() = None;

        // 清除动画状态，否则动画循环会覆盖新图片纹理

        self.anim_frames.clear();

        self.anim_current = 0;

        self.is_animated = false;

        self.anim_paused = false;

        self.anim_timeline = false;

        *self.anim_pending.lock().unwrap() = None;

        let maybe_pre = Self::take_slot_if_match(&self.loaded_next, &path, |p| &p.path);

        if let Some(pre) = maybe_pre {
            self.current_index = next_idx;

            self.current_path = Some(path.clone());

            self.is_minimized = false;

            let color_image = egui::ColorImage {
                size: [pre.tex_w, pre.tex_h],
                pixels: pre.pixels.clone(),
            };

            let texture = ctx.load_texture("image", color_image, egui::TextureOptions::default());

            self.current_texture = Some(texture);

            self.current_pixels = pre.pixels.clone();

            self.current_tex_size = [pre.tex_w, pre.tex_h];

            self.img_w = pre.orig_w;

            self.img_h = pre.orig_h;

            if self.comic_mode {
                self.fit_zoom_comic(ctx);
                self.load_comic_second(ctx);
            } else {
                let monitor = ctx.input(|i| {
                    i.viewport()
                        .monitor_size
                        .unwrap_or(egui::vec2(1920.0, 1080.0))
                });

                self.zoom = self.fit_zoom_for(self.img_w, self.img_h, monitor);
            }

            // 延迟 resize

            let now = ctx.input(|i| i.time);

            self.deferred_resize_time = Some(now + 1.0);

            // GIF 检测：预加载的数据没有 is_animated 信息，需要检查扩展名

            let maybe_is_gif = self
                .current_path
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase()
                == "gif";

            if maybe_is_gif {
                self.is_animated = true;

                self.anim_frames.clear();

                self.anim_current = 0;

                self.anim_frames.push(AnimFrame {
                    pixels: pre.pixels.clone(),

                    tex_w: pre.tex_w,

                    tex_h: pre.tex_h,

                    delay_ms: 100,
                });

                self.anim_next_time = now + 0.1;

                let _ = self.anim_pending.lock().unwrap().take();

                let anim_dest = self.anim_pending.clone();

                thread::spawn(move || {
                    load_gif_frames(&path, anim_dest);
                });
            }

            self.preload_neighbors();
        } else {
            self.current_index = next_idx;

            self.load_image_keep_size(path, ctx);
        }
    }

    fn prev_img(&mut self, ctx: &egui::Context) {
        if self.image_files.is_empty() {
            return;
        }

        let step = if self.comic_mode { 2 } else { 1 };

        let l = self.image_files.len();

        let prev_idx = (self.current_index + l - step) % l;

        let path = self.image_files[prev_idx].clone();

        if mesh3d::is_model_path(&path) {
            self.open_image(path, ctx);

            return;
        }

        // 从 3D 切回图片时清理模型状态

        self.viewing_3d = false;

        self.model = None;

        self.model_error = None;

        *self.model_pending.lock().unwrap() = None;

        // 清除动画状态，否则动画循环会覆盖新图片纹理

        self.anim_frames.clear();

        self.anim_current = 0;

        self.is_animated = false;

        self.anim_paused = false;

        self.anim_timeline = false;

        *self.anim_pending.lock().unwrap() = None;

        let maybe_pre = Self::take_slot_if_match(&self.loaded_prev, &path, |p| &p.path);

        if let Some(pre) = maybe_pre {
            self.current_index = prev_idx;

            self.current_path = Some(path.clone());

            self.is_minimized = false;

            let color_image = egui::ColorImage {
                size: [pre.tex_w, pre.tex_h],
                pixels: pre.pixels.clone(),
            };

            let texture = ctx.load_texture("image", color_image, egui::TextureOptions::default());

            self.current_texture = Some(texture);

            self.current_pixels = pre.pixels.clone();

            self.current_tex_size = [pre.tex_w, pre.tex_h];

            self.img_w = pre.orig_w;

            self.img_h = pre.orig_h;

            if self.comic_mode {
                self.fit_zoom_comic(ctx);
                self.load_comic_second(ctx);
            } else {
                let monitor = ctx.input(|i| {
                    i.viewport()
                        .monitor_size
                        .unwrap_or(egui::vec2(1920.0, 1080.0))
                });

                self.zoom = self.fit_zoom_for(self.img_w, self.img_h, monitor);
            }

            // 延迟 resize

            let now = ctx.input(|i| i.time);

            self.deferred_resize_time = Some(now + 1.0);

            // GIF 检测：预加载数据缺少帧信息，检查扩展名

            let maybe_is_gif = self
                .current_path
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase()
                == "gif";

            if maybe_is_gif {
                self.is_animated = true;

                self.anim_frames.clear();

                self.anim_current = 0;

                self.anim_frames.push(AnimFrame {
                    pixels: pre.pixels.clone(),

                    tex_w: pre.tex_w,

                    tex_h: pre.tex_h,

                    delay_ms: 100,
                });

                self.anim_next_time = now + 0.1;

                let _ = self.anim_pending.lock().unwrap().take();

                let anim_dest = self.anim_pending.clone();

                thread::spawn(move || {
                    load_gif_frames(&path, anim_dest);
                });
            }

            self.preload_neighbors();
        } else {
            self.current_index = prev_idx;

            self.load_image_keep_size(path, ctx);
        }
    }

    /// 逐帧步进（dir: -1=上一帧, 1=下一帧）
    fn step_anim_frame(&mut self, dir: i32, ctx: &egui::Context) {
        if self.anim_frames.is_empty() {
            return;
        }

        let total = self.anim_frames.len();

        let new_idx = if dir > 0 {
            (self.anim_current + 1) % total
        } else {
            (self.anim_current + total - 1) % total
        };

        self.anim_current = new_idx;

        self.apply_anim_frame(ctx);
    }

    fn apply_anim_frame(&mut self, ctx: &egui::Context) {
        let frame = &self.anim_frames[self.anim_current];

        let color_image = egui::ColorImage {
            size: [frame.tex_w, frame.tex_h],
            pixels: frame.pixels.clone(),
        };

        let texture = ctx.load_texture("image", color_image, egui::TextureOptions::default());

        self.current_texture = Some(texture);

        self.current_pixels = frame.pixels.clone();

        self.current_tex_size = [frame.tex_w, frame.tex_h];
    }

    fn resize_for_comic_mode(&self, ctx: &egui::Context) {
        let h1 = self.img_h as f32;

        let h2 = if self.comic_second_h > 0 {
            self.comic_second_h as f32
        } else {
            h1
        };

        let display_h = h1.max(h2) * self.zoom;

        // 与绘制逻辑完全一致：每张图按自己的宽高比缩放到 display_h

        let dw1 = self.img_w as f32 * (display_h / h1.max(1.0));

        let dw2 = if self.comic_second_w > 0 {
            self.comic_second_w as f32 * (display_h / h2.max(1.0))
        } else {
            dw1 // 第二张尺寸未知时用第一张
        };

        let total_w = if self.image_files.len() > 1 {
            dw1 + dw2
        } else {
            dw1
        };

        let monitor = ctx.input(|i| {
            i.viewport()
                .monitor_size
                .unwrap_or(egui::vec2(1920.0, 1080.0))
        });

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            total_w.min(monitor.x * 0.95),
            display_h.min(monitor.y * 0.95),
        )));
    }

    fn toggle_comic_mode(&mut self, ctx: &egui::Context) {
        self.comic_mode = !self.comic_mode;

        if self.comic_mode {
            self.load_comic_second(ctx);

            if self.current_texture.is_some() {
                self.fit_zoom_comic(ctx);

                self.resize_for_comic_mode(ctx);
            }
        } else {
            self.comic_second_texture = None;

            self.comic_second_w = 0;

            self.comic_second_h = 0;

            if self.current_texture.is_some() {
                self.fit_zoom(ctx);

                let w = self.img_w as f32 * self.zoom;

                let h = self.img_h as f32 * self.zoom;

                let monitor = ctx.input(|i| {
                    i.viewport()
                        .monitor_size
                        .unwrap_or(egui::vec2(1920.0, 1080.0))
                });

                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    w.min(monitor.x * 0.95),
                    h.min(monitor.y * 0.95),
                )));
            }
        }

        self.preload_neighbors();
    }

    fn minimize_to_cursor(&mut self, ctx: &egui::Context) {
        self.is_minimized = true;

        self.previous_zoom = self.zoom;

        self.previous_window_size = Some(ctx.screen_rect().size());

        let ppi = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(1.5));

        let ms = mini_size_pp(ppi);

        self.zoom = ms / self.img_w.max(self.img_h) as f32;

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(ms, ms)));

        let cursor_rel = ctx.input(|i| i.pointer.latest_pos());

        let outer_rect = ctx.input(|i| i.viewport().outer_rect);

        if let (Some(cr), Some(outer)) = (cursor_rel, outer_rect) {
            let new_x = outer.left() + cr.x - ms / 2.0;

            let new_y = outer.top() + cr.y - ms / 2.0;

            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                new_x, new_y,
            )));
        }
    }

    fn restore_from_mini(&mut self, ctx: &egui::Context) {
        self.is_minimized = false;

        self.zoom = self.previous_zoom;

        if let Some(size) = self.previous_window_size {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
    }

    /// 切换窗口置顶状态
    fn toggle_always_on_top(&mut self) {
        self.always_on_top = !self.always_on_top;
        #[cfg(target_os = "windows")]
        unsafe {
            let title: Vec<u16> = "PhotoView\0".encode_utf16().collect();
            if let Ok(hwnd) = FindWindowW(None, PCWSTR(title.as_ptr())) {
                let flag = if self.always_on_top {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                };
                let _ = SetWindowPos(hwnd, flag, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            }
        }
    }

    fn paint_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let name = self
            .current_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let file_size = self
            .current_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| human_size(m.len()))
            .unwrap_or_else(|| "—".into());

        let modified = self
            .current_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();

                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|| "—".into());

        let ext = self
            .current_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| e.to_uppercase())
            .unwrap_or_else(|| "—".into());

        let zoom_pct = format!("{:.0}%", self.zoom * 100.0);

        let display_w = (self.img_w as f32 * self.zoom).round() as usize;

        let display_h = (self.img_h as f32 * self.zoom).round() as usize;

        let idx_info = format!("{} / {}", self.current_index + 1, self.image_files.len());

        let mode_str = if self.viewing_3d {
            "3D 模型"
        } else if self.comic_mode {
            "漫画"
        } else {
            "普通"
        };

        let mut lines = vec![format!("文件名：{}", name), format!("格式：{}", ext)];

        if self.viewing_3d {
            if let Some(model) = &self.model {
                lines.push(format!("3D 格式：{}", model.format_label));

                lines.push(format!("顶点数：{}", model.vertex_count()));

                lines.push(format!("三角面：{}", model.triangle_count()));

                lines.push(format!("相机距离：{:.2}", model.camera.distance));
            }

            if let Some(err) = &self.model_error {
                lines.push(format!("错误：{}", err));
            }

            lines.push(format!("文件大小：{}", file_size));

            lines.push(format!("修改日期：{}", modified));

            lines.push(format!("序号：{}", idx_info));

            lines.push(format!("模式：{}", mode_str));

            let orbit_txt = drag_binding_label(&self.settings.keybindings.model_orbit);

            let pan_txt = drag_binding_label(&self.settings.keybindings.model_pan);

            lines.push(format!(
                "旋转：{} · 平移：{} · 滚轮缩放",
                orbit_txt, pan_txt
            ));

            lines.push(format!(
                "复位：{} / F / 双击 · Alt+左键拖动窗口",
                self.settings.keybindings.model_reset
            ));
        } else {
            lines.push(format!("分辨率：{} × {}", self.img_w, self.img_h));

            lines.push(format!("显示尺寸：{} × {}", display_w, display_h));

            lines.push(format!("缩放：{}", zoom_pct));

            lines.push(format!("文件大小：{}", file_size));

            lines.push(format!("修改日期：{}", modified));

            lines.push(format!("序号：{}", idx_info));

            lines.push(format!("模式：{}", mode_str));
        }

        let lines = lines;

        let pad = 10.0;

        let line_h = 20.0;

        let font = egui::FontId::proportional(13.0);

        let bg_w = 300.0;

        let bg_h = lines.len() as f32 * line_h + pad * 2.0;

        let bg_rect = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(pad, pad),
            egui::vec2(bg_w, bg_h),
        );

        ui.painter_at(rect).rect_filled(
            bg_rect,
            6.0,
            Color32::from_rgba_premultiplied(0, 0, 0, 180),
        );

        for (i, line) in lines.iter().enumerate() {
            let pos = bg_rect.left_top() + egui::vec2(pad, pad + i as f32 * line_h);

            ui.painter_at(rect).text(
                pos,
                egui::Align2::LEFT_TOP,
                line,
                font.clone(),
                Color32::WHITE,
            );
        }
    }

    /// 绘制设置面板
    fn paint_settings(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();

        let win_w = screen.width();

        let win_h = screen.height();

        // 无遮罩，面板直接浮在图片上方

        // 弹窗尺寸

        let panel_w = 440.0_f32.min(win_w - 40.0);

        let panel_h = 540.0_f32.min(win_h - 40.0);

        let panel_rect =
            egui::Rect::from_center_size(screen.center(), egui::vec2(panel_w, panel_h));

        // 用自定义 Visuals 渲染设置面板

        let mut settings_visuals = egui::Visuals::dark();

        settings_visuals.panel_fill = Color32::from_rgb(38, 38, 46);

        settings_visuals.extreme_bg_color = Color32::from_rgb(28, 28, 34);

        settings_visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(50, 50, 60);

        settings_visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0_f32, Color32::from_rgb(160, 160, 170));

        settings_visuals.widgets.inactive.bg_fill = Color32::from_rgb(55, 55, 68);

        settings_visuals.widgets.inactive.fg_stroke =
            egui::Stroke::new(1.0_f32, Color32::from_rgb(200, 200, 210));

        settings_visuals.widgets.hovered.bg_fill = Color32::from_rgb(70, 70, 90);

        settings_visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);

        settings_visuals.widgets.active.bg_fill = Color32::from_rgb(80, 80, 110);

        settings_visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);

        settings_visuals.selection.bg_fill = Color32::from_rgb(90, 130, 220);

        settings_visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);

        let area = egui::Area::new(egui::Id::new("settings_panel"))
            .fixed_pos(panel_rect.left_top())
            .order(egui::Order::Foreground)
            .interactable(true);

        area.show(ctx, |ui| {
            ui.set_width(panel_w);

            ui.set_height(panel_h);

            // 圆角背景

            ui.painter()
                .rect_filled(panel_rect, 12.0, Color32::from_rgb(38, 38, 46));

            ui.painter().rect_stroke(
                panel_rect,
                12.0,
                egui::Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 85)),
            );

            // 临时切换到设置面板视觉风格

            ui.visuals_mut().clone_from(&settings_visuals);

            // 标题栏

            ui.horizontal(|ui| {
                ui.add_space(16.0);

                ui.label(
                    RichText::new("⚙ 设置")
                        .size(18.0)
                        .color(Color32::WHITE)
                        .strong(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);

                    let close_resp = ui.button(
                        RichText::new("✕")
                            .size(16.0)
                            .color(Color32::from_rgb(180, 180, 190)),
                    );

                    if close_resp.clicked() {
                        self.show_settings = false;

                        self.rebinding = None;
                    }
                });
            });

            ui.add_space(2.0);

            // 分隔线

            let sep_rect = ui.available_rect_before_wrap();

            ui.painter().line_segment(
                [
                    egui::pos2(sep_rect.left() + 14.0, sep_rect.top()),
                    egui::pos2(sep_rect.right() - 14.0, sep_rect.top()),
                ],
                egui::Stroke::new(1.0_f32, Color32::from_rgb(60, 60, 75)),
            );

            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .max_height(panel_h - 60.0)
                .show(ui, |ui| {
                    // ── 快捷键设置 ──

                    ui.horizontal(|ui| {
                        ui.add_space(4.0);

                        ui.label(
                            RichText::new("快捷键")
                                .size(15.0)
                                .color(Color32::from_rgb(130, 170, 255))
                                .strong(),
                        );
                    });

                    ui.add_space(8.0);

                    let labels = self.settings.keybindings.labels();

                    for (label, binding) in &labels {
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);

                            ui.label(
                                RichText::new(*label)
                                    .size(13.0)
                                    .color(Color32::from_rgb(210, 210, 220)),
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let is_rebinding = self.rebinding.as_deref() == Some(label);

                                    let btn_text = if is_rebinding {
                                        "按下新按键...".to_string()
                                    } else {
                                        binding.clone()
                                    };

                                    ui.add_space(10.0);

                                    if is_rebinding {
                                        let resp = ui.button(
                                            RichText::new(btn_text)
                                                .size(12.0)
                                                .color(Color32::from_rgb(255, 220, 100))
                                                .strong(),
                                        );

                                        let rect = resp.rect.expand(2.0);

                                        ui.painter().rect_stroke(
                                            rect,
                                            4.0,
                                            egui::Stroke::new(
                                                2.0_f32,
                                                Color32::from_rgb(255, 200, 80),
                                            ),
                                        );

                                        if resp.clicked() { /* 已经在绑定中，忽略 */ }
                                    } else {
                                        let resp = ui.button(
                                            RichText::new(btn_text)
                                                .size(12.0)
                                                .color(Color32::from_rgb(220, 220, 230)),
                                        );

                                        if resp.clicked() {
                                            self.rebinding = Some(label.to_string());
                                        }
                                    }
                                },
                            );
                        });

                        ui.add_space(1.0);
                    }

                    // 3D 鼠标绑定（下拉选择）
                    ui.add_space(8.0);

                    let drag_options = ["LeftDrag", "MiddleDrag", "ShiftLeftDrag"];

                    ui.horizontal(|ui| {
                        ui.add_space(8.0);

                        ui.label(
                            RichText::new("3D 旋转")
                                .size(13.0)
                                .color(Color32::from_rgb(210, 210, 220)),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);

                            let mut cur = self.settings.keybindings.model_orbit.clone();

                            egui::ComboBox::from_id_source("model_orbit")
                                .selected_text(drag_binding_label(&cur))
                                .width(140.0)
                                .show_ui(ui, |ui| {
                                    for opt in drag_options {
                                        ui.selectable_value(
                                            &mut cur,
                                            opt.to_string(),
                                            drag_binding_label(opt),
                                        );
                                    }
                                });

                            if cur != self.settings.keybindings.model_orbit {
                                self.settings.keybindings.model_orbit = cur;

                                self.settings.save();
                            }
                        });
                    });

                    ui.add_space(1.0);

                    ui.horizontal(|ui| {
                        ui.add_space(8.0);

                        ui.label(
                            RichText::new("3D 平移")
                                .size(13.0)
                                .color(Color32::from_rgb(210, 210, 220)),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);

                            let mut cur = self.settings.keybindings.model_pan.clone();

                            egui::ComboBox::from_id_source("model_pan")
                                .selected_text(drag_binding_label(&cur))
                                .width(140.0)
                                .show_ui(ui, |ui| {
                                    for opt in drag_options {
                                        ui.selectable_value(
                                            &mut cur,
                                            opt.to_string(),
                                            drag_binding_label(opt),
                                        );
                                    }
                                });

                            if cur != self.settings.keybindings.model_pan {
                                self.settings.keybindings.model_pan = cur;

                                self.settings.save();
                            }
                        });
                    });

                    // 处理重新绑定按键

                    if self.rebinding.is_some() {
                        ctx.input(|i| {
                            if i.key_pressed(egui::Key::Escape) {
                                self.rebinding = None;

                                return;
                            }

                            let keys_to_check = [
                                egui::Key::ArrowLeft,
                                egui::Key::ArrowRight,
                                egui::Key::ArrowUp,
                                egui::Key::ArrowDown,
                                egui::Key::Escape,
                                egui::Key::Space,
                                egui::Key::Enter,
                                egui::Key::Tab,
                                egui::Key::A,
                                egui::Key::B,
                                egui::Key::C,
                                egui::Key::D,
                                egui::Key::E,
                                egui::Key::F,
                                egui::Key::G,
                                egui::Key::H,
                                egui::Key::I,
                                egui::Key::J,
                                egui::Key::K,
                                egui::Key::L,
                                egui::Key::M,
                                egui::Key::N,
                                egui::Key::O,
                                egui::Key::P,
                                egui::Key::Q,
                                egui::Key::R,
                                egui::Key::S,
                                egui::Key::T,
                                egui::Key::U,
                                egui::Key::V,
                                egui::Key::W,
                                egui::Key::X,
                                egui::Key::Y,
                                egui::Key::Z,
                                egui::Key::F1,
                                egui::Key::F2,
                                egui::Key::F3,
                                egui::Key::F4,
                                egui::Key::F5,
                                egui::Key::F6,
                                egui::Key::F7,
                                egui::Key::F8,
                                egui::Key::F9,
                                egui::Key::F10,
                                egui::Key::F11,
                                egui::Key::F12,
                                egui::Key::Plus,
                                egui::Key::Minus,
                                egui::Key::Equals,
                                egui::Key::Backtick,
                                egui::Key::Backspace,
                                egui::Key::Delete,
                                egui::Key::Home,
                                egui::Key::End,
                                egui::Key::PageUp,
                                egui::Key::PageDown,
                                egui::Key::Insert,
                            ];

                            for key in &keys_to_check {
                                if i.key_pressed(*key) {
                                    let key_str = key_to_string(*key);

                                    if let Some(label) = &self.rebinding
                                        && let Some(slot) = self.settings.keybindings.get_mut(label)
                                    {
                                        *slot = key_str;
                                    }

                                    self.rebinding = None;

                                    self.settings.save();

                                    return;
                                }
                            }
                        });
                    }

                    ui.add_space(14.0);

                    // 分隔线

                    let sep_rect2 = ui.available_rect_before_wrap();

                    ui.painter().line_segment(
                        [
                            egui::pos2(sep_rect2.left() + 14.0, sep_rect2.top()),
                            egui::pos2(sep_rect2.right() - 14.0, sep_rect2.top()),
                        ],
                        egui::Stroke::new(1.0_f32, Color32::from_rgb(60, 60, 75)),
                    );

                    ui.add_space(10.0);

                    // ── 文件关联 ──

                    ui.horizontal(|ui| {
                        ui.add_space(4.0);

                        ui.label(
                            RichText::new("文件关联")
                                .size(15.0)
                                .color(Color32::from_rgb(130, 170, 255))
                                .strong(),
                        );
                    });

                    ui.add_space(8.0);

                    // 全选 / 全不选

                    ui.horizontal(|ui| {
                        ui.add_space(8.0);

                        if ui.button(RichText::new("全选").size(13.0)).clicked() {
                            for ext in SUPPORTED_EXTS {
                                self.file_assoc_check.insert(ext.to_string(), true);
                            }
                        }

                        if ui.button(RichText::new("全不选").size(13.0)).clicked() {
                            for ext in SUPPORTED_EXTS {
                                self.file_assoc_check.insert(ext.to_string(), false);
                            }
                        }
                    });

                    ui.add_space(6.0);

                    // 格式列表

                    let cols = 3;

                    let exts: Vec<&str> = SUPPORTED_EXTS.to_vec();

                    let rows = exts.len().div_ceil(cols);

                    for row in 0..rows {
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);

                            for col in 0..cols {
                                let idx = row * cols + col;

                                if idx >= exts.len() {
                                    break;
                                }

                                let ext = exts[idx];

                                ui.checkbox(
                                    self.file_assoc_check
                                        .entry(ext.to_string())
                                        .or_insert(false),
                                    RichText::new(format!(".{}", ext))
                                        .size(13.0)
                                        .color(Color32::from_rgb(210, 210, 220)),
                                );
                            }
                        });
                    }

                    ui.add_space(10.0);

                    // 操作按钮

                    ui.horizontal(|ui| {
                        ui.add_space(8.0);

                        let apply_resp = ui.button(
                            RichText::new("应用关联")
                                .size(13.0)
                                .color(Color32::from_rgb(200, 230, 255)),
                        );

                        if apply_resp.clicked() {
                            let selected: Vec<String> = self
                                .file_assoc_check
                                .iter()
                                .filter(|(_, v)| **v)
                                .map(|(k, _)| k.clone())
                                .collect();

                            match register_file_association(&selected, false) {
                                Ok(()) => {
                                    self.settings.associated_exts = selected;

                                    self.settings.save();
                                }

                                Err(e) => eprintln!("注册文件关联失败: {}", e),
                            }
                        }

                        let default_resp = ui.button(
                            RichText::new("设为默认查看器")
                                .size(13.0)
                                .color(Color32::from_rgb(200, 230, 255)),
                        );

                        if default_resp.clicked() {
                            let selected: Vec<String> = self
                                .file_assoc_check
                                .iter()
                                .filter(|(_, v)| **v)
                                .map(|(k, _)| k.clone())
                                .collect();

                            match register_file_association(&selected, true) {
                                Ok(()) => {
                                    self.settings.associated_exts = selected;

                                    self.settings.save();
                                }

                                Err(e) => eprintln!("设为默认查看器失败: {}", e),
                            }
                        }

                        // 打开系统默认应用设置

                        if ui
                            .button(
                                RichText::new("系统默认应用设置")
                                    .size(12.0)
                                    .color(Color32::from_rgb(180, 210, 255)),
                            )
                            .clicked()
                        {
                            let _ = std::process::Command::new("cmd")
                                .args(["/c", "start", "ms-settings:defaultapps"])
                                .spawn();
                        }
                    });

                    ui.add_space(10.0);
                });
        });

        // 点击面板外区域关闭（轻量检测）

        if ctx.input(|i| i.pointer.primary_pressed()) {
            let pos = ctx.input(|i| i.pointer.latest_pos());

            if let Some(pos) = pos
                && !panel_rect.contains(pos)
            {
                self.show_settings = false;

                self.rebinding = None;
            }
        }
    }
}

impl eframe::App for PhotoViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.round_corners_applied {
            self.round_corners_applied = true;

            apply_win11_effects();
        }

        self.apply_pending(ctx);

        self.apply_model_pending(ctx);

        //
        if self.should_close_next_frame
            && !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary))
        {
            self.settings.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // 延迟 resize：翻页停止后调整窗口

        if let Some(t) = self.deferred_resize_time {
            let now = ctx.input(|i| i.time);

            if now >= t {
                self.deferred_resize_time = None;

                if self.comic_mode {
                    self.resize_for_comic_mode(ctx);
                } else {
                    self.resize_for_single(ctx);
                }
            }
        }

        // GIF 动画帧处理

        // 检查异步加载的帧是否就绪

        {
            let maybe_frames = self.anim_pending.lock().unwrap().take();

            if let Some(frames) = maybe_frames
                && !frames.is_empty()
            {
                self.anim_frames = frames;
                self.anim_textures.clear();
                self.anim_current = 0;
            }
        }

        // 帧切换（非暂停时自动播放）

        if self.is_animated && self.anim_frames.len() > 1 && !self.anim_paused {
            let now = ctx.input(|i| i.time);
            let need_upload = self
                .anim_textures
                .get(self.anim_current)
                .map_or(true, |t| t.is_none());
            if now >= self.anim_next_time || need_upload {
                if now >= self.anim_next_time {
                    self.anim_current = (self.anim_current + 1) % self.anim_frames.len();
                    let frame = &self.anim_frames[self.anim_current];
                    self.anim_next_time = now + frame.delay_ms as f64 / 1000.0;
                }
                let fi = self.anim_current;
                let stale = self
                    .anim_textures
                    .get(fi)
                    .and_then(|o| o.as_ref())
                    .map_or(true, |tex| {
                        self.current_texture.as_ref().map_or(true, |cur| cur.id() != tex.id())
                    });
                if stale {
                    let frame = &self.anim_frames[fi];
                    let tex = ctx.load_texture(
                        "image",
                        egui::ColorImage {
                            size: [frame.tex_w, frame.tex_h],
                            pixels: frame.pixels.clone(),
                        },
                        egui::TextureOptions::default(),
                    );
                    self.current_pixels = frame.pixels.clone();
                    self.current_tex_size = [frame.tex_w, frame.tex_h];
                    self.current_texture = Some(tex);
                    if self.anim_textures.len() == self.anim_frames.len() {
                        self.anim_textures[fi] = self.current_texture.clone();
                    }
                }
            }
            // 只在下一帧到期时唤醒，不再忙刷
            let delay = ((self.anim_next_time - ctx.input(|i| i.time)) * 1000.0).max(16.0) as u64;
            ctx.request_repaint_after(std::time::Duration::from_millis(delay));
        }

        // 处理命令行传入的图片路径

        if let Some(path) = self.initial_path.take()
            && path.exists()
        {
            self.open_image(path, ctx);
        }

        let has_img = self.current_texture.is_some();

        let has_multiple = self.image_files.len() > 1;

        let screen_rect = ctx.screen_rect();

        // 解析快捷键配置

        let kb = &self.settings.keybindings;

        let key_prev = string_to_key(&kb.prev).unwrap_or(egui::Key::ArrowLeft);

        let key_next = string_to_key(&kb.next).unwrap_or(egui::Key::ArrowRight);

        let key_comic = string_to_key(&kb.comic_mode).unwrap_or(egui::Key::C);

        let key_settings = string_to_key(&kb.settings).unwrap_or(egui::Key::F5);

        let mut should_prev = false;

        let mut should_next = false;

        let mut dropped_path = None;

        let mut want_drag = false;

        let mut double_clicked = false;

        let mut should_zoom_delta: Option<f32> = None;

        let mut should_toggle_comic = false;

        let mut should_toggle_settings = false;

        let mut should_fit_zoom = false;
        let mut should_toggle_topmost = false;
        let mut model_orbit_delta: Option<egui::Vec2> = None;
        let mut model_pan_delta: Option<egui::Vec2> = None;
        let mut model_zoom_delta: Option<f32> = None;
        let mut model_reset = false;

        // 提前计算 UI 控件区域（用于保护拖拽/双击）

        let control_areas: Vec<egui::Rect> = {
            let mut areas = Vec::new();

            if has_multiple && has_img {
                areas.push(egui::Rect::from_center_size(
                    egui::pos2(screen_rect.left() + 20.0, screen_rect.center().y),
                    egui::vec2(ARROW_ZONE_W, ARROW_ZONE_H),
                ));

                areas.push(egui::Rect::from_center_size(
                    egui::pos2(screen_rect.right() - 20.0, screen_rect.center().y),
                    egui::vec2(ARROW_ZONE_W, ARROW_ZONE_H),
                ));
            }

            if self.is_animated && self.anim_frames.len() > 1 && !self.is_minimized {
                // GIF 帧步进按钮 + 帧数（底部居中区域）

                let bottom_y = screen_rect.bottom() - 24.0;

                let center_x = screen_rect.center().x;

                areas.push(egui::Rect::from_center_size(
                    egui::pos2(center_x, bottom_y),
                    egui::vec2(220.0, 36.0),
                ));
            }

            areas
        };

        let in_control =
            |pos: egui::Pos2| -> bool { control_areas.iter().any(|r| r.contains(pos)) };

        ctx.input(|i| {
            if i.pointer.secondary_pressed() {
                self.should_close_next_frame = true;
            }

            let model_reset_key =
                string_to_key(&self.settings.keybindings.model_reset).unwrap_or(egui::Key::R);

            if self.viewing_3d && has_img {
                // 3D：按配置的鼠标绑定处理旋转/平移
                let orbit = self.settings.keybindings.model_orbit.clone();
                let pan = self.settings.keybindings.model_pan.clone();
                let delta = i.pointer.delta();
                let primary_active = i.pointer.primary_down() && !i.modifiers.alt;
                let middle_active = i.pointer.middle_down();
                let shift_primary = primary_active && i.modifiers.shift;
                let plain_primary = primary_active && !i.modifiers.shift;

                // 中键按下/拖动状态：拖动超过阈值视为平移，松开且未拖动则切换信息
                if i.pointer.button_pressed(egui::PointerButton::Middle) {
                    self.middle_down = true;
                    self.middle_moved = false;
                }
                if middle_active {
                    let d = i.pointer.delta();
                    if d.x.abs() + d.y.abs() > 1.5 {
                        self.middle_moved = true;
                    }
                }

                let orbit_hit = match orbit.as_str() {
                    "LeftDrag" => plain_primary,
                    "MiddleDrag" => middle_active,
                    "ShiftLeftDrag" => shift_primary,
                    _ => plain_primary,
                };
                // 中键刚触发拖动阈值后的帧也要继续平移
                let pan_hit = match pan.as_str() {
                    "MiddleDrag" => middle_active && self.middle_moved,
                    "ShiftLeftDrag" => shift_primary,
                    "LeftDrag" => plain_primary,
                    _ => middle_active && self.middle_moved,
                };
                // 平移优先，避免同一手势同时旋转
                let orbit_hit = orbit_hit && !pan_hit;

                if delta.x.abs() > 0.0 || delta.y.abs() > 0.0 {
                    if pan_hit {
                        model_pan_delta = Some(delta);
                    } else if orbit_hit {
                        model_orbit_delta = Some(delta);
                    }
                }

                if i.pointer.primary_pressed() && i.modifiers.alt {
                    let ok = i.pointer.latest_pos().is_none_or(|p| !in_control(p));
                    if ok {
                        want_drag = true;
                    }
                }

                if i.raw_scroll_delta.y != 0.0 {
                    model_zoom_delta = Some(i.raw_scroll_delta.y);
                }

                if i.key_pressed(model_reset_key) {
                    model_reset = true;
                }
            } else if i.pointer.primary_pressed() {
                let ok = i.pointer.latest_pos().is_none_or(|p| !in_control(p));

                if ok {
                    want_drag = true;
                }
            }

            // 中键松开：3D 平移模式下未拖动则切换信息；图片模式保持点击切换
            if i.pointer.button_released(egui::PointerButton::Middle) && has_img {
                if self.viewing_3d {
                    if self.middle_down && !self.middle_moved {
                        self.show_info = !self.show_info;
                    }
                } else {
                    self.show_info = !self.show_info;
                }
                self.middle_down = false;
                self.middle_moved = false;
            }

            if i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
                && has_img
            {
                if self.viewing_3d {
                    model_reset = true;
                } else if self.is_minimized {
                    double_clicked = true;
                } else if let Some(pos) = i.pointer.latest_pos()
                    && !in_arrow_zone(pos, screen_rect)
                    && !in_control(pos)
                {
                    double_clicked = true;
                }
            }

            if !self.viewing_3d && i.raw_scroll_delta.y != 0.0 && !self.is_minimized && has_img {
                should_zoom_delta = Some(i.raw_scroll_delta.y);
            }

            if i.key_pressed(key_prev) {
                should_prev = true;
            }

            if i.key_pressed(key_next) {
                should_next = true;
            }

            if i.key_pressed(key_comic) {
                should_toggle_comic = true;
            }

            if i.key_pressed(key_settings) {
                should_toggle_settings = true;
            }

            if i.key_pressed(egui::Key::F) && has_img {
                should_fit_zoom = true;
            }
            if i.key_pressed(egui::Key::G) {
                should_toggle_topmost = true;
            }

            if let Some(path) = i.raw.dropped_files.first().and_then(|f| f.path.clone()) {
                dropped_path = Some(path);
            }
        });

        // 设置面板打开时不响应其他操作，但继续渲染图片

        if let Some(delta) = should_zoom_delta {
            self.zoom_image(delta, ctx);
        }

        if self.viewing_3d {
            if let Some(d) = model_orbit_delta {
                if let Some(m) = &mut self.model {
                    m.camera.orbit(d.x, d.y);

                    m.mark_dirty();
                }

                self.note_model_interaction(ctx);
            }

            if let Some(d) = model_pan_delta {
                if let Some(m) = &mut self.model {
                    let aspect = screen_rect.width() / screen_rect.height().max(1.0);

                    m.camera.pan(d.x, d.y, aspect);

                    m.mark_dirty();
                }

                self.note_model_interaction(ctx);
            }

            if let Some(d) = model_zoom_delta {
                if let Some(m) = &mut self.model {
                    m.camera.zoom_by(d);

                    m.mark_dirty();
                }

                self.note_model_interaction(ctx);
            }

            if model_reset {
                self.reset_model_view();

                self.note_model_interaction(ctx);
            }
        }

        if double_clicked {
            if self.is_minimized {
                self.restore_from_mini(ctx);
            } else {
                self.minimize_to_cursor(ctx);
            }
        }

        // 空格键：播放/暂停 GIF（全局生效）

        if ctx.input(|i| i.key_pressed(egui::Key::Space))
            && self.is_animated
            && self.anim_frames.len() > 1
        {
            self.anim_paused = !self.anim_paused;
        }

        // F 键：自适应缩放（3D 下复位相机）

        if should_fit_zoom && has_img {
            if self.viewing_3d {
                self.reset_model_view();
            } else {
                self.fit_zoom(ctx);

                self.resize_for_single(ctx);
            }
        }

        if should_toggle_topmost {
            self.toggle_always_on_top();
        }

        if should_prev {
            // GIF 暂停状态下方向键逐帧步进，播放状态下翻页

            if self.is_animated && self.anim_frames.len() > 1 && self.anim_paused {
                self.step_anim_frame(-1, ctx);
            } else {
                self.prev_img(ctx);
            }
        }

        if should_next {
            if self.is_animated && self.anim_frames.len() > 1 && self.anim_paused {
                self.step_anim_frame(1, ctx);
            } else {
                self.next_img(ctx);
            }
        }

        if should_toggle_comic && has_img {
            self.toggle_comic_mode(ctx);
        }

        if should_toggle_settings {
            self.show_settings = !self.show_settings;
        }

        if let Some(path) = dropped_path {
            // 同一文件重复拖入时避免反复重启加载

            let same = self.last_drop_path.as_deref() == Some(path.as_path())
                && self.current_path.as_deref() == Some(path.as_path())
                && (self.is_loading || self.current_texture.is_some());

            if !same {
                self.open_image(path, ctx);
            }
        }

        if want_drag && !self.drag_started {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);

            self.drag_started = true;
        }

        if !want_drag {
            self.drag_started = false;
        }

        if !has_img && !self.is_loading {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::from_gray(32)))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);

                        ui.heading(
                            RichText::new("🖼 PhotoView")
                                .size(28.0)
                                .color(Color32::WHITE),
                        );

                        ui.add_space(12.0);

                        ui.label(
                            RichText::new("拖入图片或 3D 模型开始查看")
                                .color(Color32::from_rgb(140, 140, 145))
                                .size(16.0),
                        );

                        ui.add_space(24.0);

                        if ui
                            .button(RichText::new("📁 打开文件...").size(14.0))
                            .clicked()
                            && let Some(path) = FileDialog::new()
                                .add_filter(
                                    "图片",
                                    &[
                                        "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif",
                                        "ico",
                                    ],
                                )
                                .add_filter(
                                    "3D 模型",
                                    &[
                                        "obj", "stl", "gltf", "glb", "ply", "off", "3ds", "dae",
                                        "vtk", "wrl", "vrml", "amf", "vrm", "fbx", "pmx", "pmd",
                                        "usd", "usda", "usdc", "usdz",
                                    ],
                                )
                                .add_filter(
                                    "3D 打印",
                                    &["stl", "3mf", "amf", "gcode", "gco", "obj"],
                                )
                                .add_filter(
                                    "所有支持",
                                    &[
                                        "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif",
                                        "ico", "obj", "stl", "gltf", "glb", "ply", "off", "3ds",
                                        "dae", "vtk", "wrl", "vrml", "amf", "3mf", "gcode", "gco",
                                        "g", "vrm", "fbx", "pmx", "pmd", "usd", "usda", "usdc",
                                        "usdz",
                                    ],
                                )
                                .pick_file()
                        {
                            self.open_image(path, ctx);
                        }

                        ui.add_space(16.0);

                        ui.label(
                            RichText::new(
                                "拖动移动 | 右键关闭 | 箭头翻页 | 中键信息 | C 漫画模式 | F5 设置",
                            )
                            .color(Color32::from_rgb(100, 100, 105))
                            .size(12.0),
                        );

                        ui.add_space(4.0);

                        let orbit_txt = drag_binding_label(&self.settings.keybindings.model_orbit);

                        let pan_txt = drag_binding_label(&self.settings.keybindings.model_pan);

                        ui.label(
                            RichText::new(format!(
                                "3D：{}旋转 · {}平移 · 滚轮缩放 · {}/F/双击复位 · Alt+拖动窗口",
                                orbit_txt, pan_txt, self.settings.keybindings.model_reset
                            ))
                            .color(Color32::from_rgb(90, 110, 130))
                            .size(12.0),
                        );

                        if let Some(err) = &self.model_error {
                            ui.add_space(12.0);

                            ui.label(
                                RichText::new(format!("模型加载失败：{}", err))
                                    .color(Color32::from_rgb(220, 120, 120))
                                    .size(12.0),
                            );
                        }
                    });
                });
        } else {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
                .show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::click());

                    if let Some(texture) = &self.current_texture {
                        if self.comic_mode && self.comic_second_texture.is_some() {
                            // ── 漫画模式：双图并排 ──

                            let tex2 = self.comic_second_texture.as_ref().unwrap();

                            let h1 = self.img_h as f32;

                            let h2 = self.comic_second_h as f32;

                            // 统一显示高度 = max(h1, h2) * zoom

                            let display_h = h1.max(h2) * self.zoom;

                            // 每张图按自己的宽高比缩放到 display_h

                            let dw1 = self.img_w as f32 * (display_h / h1.max(1.0));

                            let dw2 = self.comic_second_w as f32 * (display_h / h2.max(1.0));

                            let total_w = dw1 + dw2;

                            // 右侧=当前页(index)，左侧=下一页(index+1) — 日式漫画从右到左

                            let right_x = rect.center().x + (total_w / 2.0 - dw1);

                            let left_x = right_x - dw2;

                            let top_y = rect.center().y - display_h / 2.0;

                            // 左页（下一页）

                            ui.painter_at(rect).image(
                                tex2.id(),
                                egui::Rect::from_min_max(
                                    egui::pos2(left_x, top_y),
                                    egui::pos2(left_x + dw2, top_y + display_h),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );

                            // 右页（当前页）

                            ui.painter_at(rect).image(
                                texture.id(),
                                egui::Rect::from_min_max(
                                    egui::pos2(right_x, top_y),
                                    egui::pos2(right_x + dw1, top_y + display_h),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        } else if self.viewing_3d {
                            // ── 3D 模型：纹理铺满可用区域 ──

                            ui.painter_at(rect).image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        } else {
                            // ── 普通模式：单图 ──

                            let dw = self.img_w as f32 * self.zoom;

                            let dh = self.img_h as f32 * self.zoom;

                            let x = rect.center().x - dw / 2.0;

                            let y = rect.center().y - dh / 2.0;

                            ui.painter_at(rect).image(
                                texture.id(),
                                egui::Rect::from_min_max(
                                    egui::pos2(x, y),
                                    egui::pos2(x + dw, y + dh),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        }
                    }

                    if self.is_loading {
                        ui.painter_at(rect).rect_filled(
                            rect,
                            0.0,
                            Color32::from_rgba_premultiplied(0, 0, 0, 180),
                        );

                        let msg = if self.viewing_3d && self.model.is_some() {
                            "渲染中..."
                        } else {
                            "加载中..."
                        };

                        ui.painter_at(rect).text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            msg,
                            egui::FontId::proportional(24.0),
                            Color32::WHITE,
                        );

                        ctx.request_repaint();
                    }

                    if self.show_info && has_img {
                        self.paint_info(ui, rect);
                    }

                    // 模式标识（左下角）

                    if has_img && !self.is_minimized && !self.is_loading {
                        let mode_text = if self.always_on_top {
                            "📌 置顶"
                        } else if self.viewing_3d {
                            "3D"
                        } else if self.comic_mode {
                            "漫画"
                        } else {
                            "普通"
                        };

                        let pos = egui::pos2(rect.left() + 12.0, rect.bottom() - 20.0);

                        // 自适应颜色
                        let mode_center = egui::pos2(pos.x + 35.0, pos.y - 6.0);
                        let mode_color = adaptive_arrow_color(
                            &self.current_pixels,
                            self.current_tex_size,
                            self.img_w,
                            self.img_h,
                            self.zoom,
                            rect,
                            mode_center,
                        );

                        ui.painter_at(rect).text(
                            pos,
                            egui::Align2::LEFT_BOTTOM,
                            mode_text,
                            egui::FontId::proportional(12.0),
                            mode_color,
                        );
                    }

                    // GIF 帧数指示（底部居中）+ 逐帧步进按钮

                    if self.is_animated
                        && self.anim_frames.len() > 1
                        && !self.is_minimized
                        && !self.is_loading
                    {
                        let total = self.anim_frames.len();

                        let frame_num = self.anim_current + 1;

                        let center_x = rect.center().x;

                        let bottom_y = rect.bottom() - 24.0;

                        // 左步进按钮

                        let step_l_rect = egui::Rect::from_center_size(
                            egui::pos2(center_x - 80.0, bottom_y),
                            egui::vec2(36.0, 28.0),
                        );

                        ui.painter_at(rect).text(
                            step_l_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "◀",
                            egui::FontId::proportional(18.0),
                            Color32::from_rgba_premultiplied(100, 180, 255, 230),
                        );

                        if ui
                            .interact(
                                step_l_rect,
                                egui::Id::new("gif_step_prev"),
                                egui::Sense::click(),
                            )
                            .clicked()
                        {
                            self.anim_paused = true;

                            self.step_anim_frame(-1, ctx);
                        }

                        // 帧数文字（带半透明背景框，确保在任何背景下可读）

                        let (info, info_color) = if self.anim_paused {
                            (format!("{} / {}", frame_num, total), Color32::WHITE)
                        } else {
                            (
                                format!("{} / {}", frame_num, total),
                                Color32::from_rgba_premultiplied(200, 200, 200, 230),
                            )
                        };

                        // 背景框
                        let bg_rect = egui::Rect::from_center_size(
                            egui::pos2(center_x, bottom_y),
                            egui::vec2(110.0, 34.0),
                        );
                        ui.painter_at(rect).rect_filled(
                            bg_rect,
                            6.0,
                            Color32::from_rgba_premultiplied(0, 0, 0, 180),
                        );
                        ui.painter_at(rect).rect_stroke(
                            bg_rect,
                            1.0,
                            egui::Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_premultiplied(255, 255, 255, 60),
                            ),
                        );

                        // 文字
                        ui.painter_at(rect).text(
                            bg_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &info,
                            egui::FontId::proportional(22.0),
                            info_color,
                        );

                        // 右步进按钮

                        let step_r_rect = egui::Rect::from_center_size(
                            egui::pos2(center_x + 80.0, bottom_y),
                            egui::vec2(36.0, 28.0),
                        );

                        ui.painter_at(rect).text(
                            step_r_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(18.0),
                            Color32::from_rgba_premultiplied(100, 180, 255, 230),
                        );

                        if ui
                            .interact(
                                step_r_rect,
                                egui::Id::new("gif_step_next"),
                                egui::Sense::click(),
                            )
                            .clicked()
                        {
                            self.anim_paused = true;

                            self.step_anim_frame(1, ctx);
                        }
                    }

                    if has_multiple && !self.is_minimized && !self.is_loading {
                        let arrow_y = rect.center().y;

                        let lp = egui::pos2(rect.left() + 20.0, arrow_y);

                        let rp = egui::pos2(rect.right() - 20.0, arrow_y);

                        let left_color = adaptive_arrow_color(
                            &self.current_pixels,
                            self.current_tex_size,
                            self.img_w,
                            self.img_h,
                            self.zoom,
                            rect,
                            lp,
                        );

                        let right_color = adaptive_arrow_color(
                            &self.current_pixels,
                            self.current_tex_size,
                            self.img_w,
                            self.img_h,
                            self.zoom,
                            rect,
                            rp,
                        );

                        ui.painter_at(rect).text(
                            lp,
                            egui::Align2::CENTER_CENTER,
                            "◀",
                            egui::FontId::proportional(36.0),
                            left_color,
                        );

                        let lh = egui::Rect::from_center_size(
                            lp,
                            egui::vec2(ARROW_ZONE_W, ARROW_ZONE_H),
                        );

                        if ui
                            .interact(lh, egui::Id::new("left"), egui::Sense::click())
                            .clicked()
                        {
                            self.prev_img(ctx);
                        }

                        ui.painter_at(rect).text(
                            rp,
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(36.0),
                            right_color,
                        );

                        let rh = egui::Rect::from_center_size(
                            rp,
                            egui::vec2(ARROW_ZONE_W, ARROW_ZONE_H),
                        );

                        if ui
                            .interact(rh, egui::Id::new("right"), egui::Sense::click())
                            .clicked()
                        {
                            self.next_img(ctx);
                        }
                    }
                });
        }

        // 设置面板浮在图片上方

        if self.show_settings {
            self.paint_settings(ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
    }
}

fn main() -> eframe::Result<()> {
    // 解析命令行参数：第一个非程序名参数作为图片路径

    let args: Vec<String> = std::env::args().collect();

    let initial_path: Option<PathBuf> = args.get(1).map(PathBuf::from);

    // ── 单实例检测：尝试连接已有管道 ──

    // 多窗口模式：不检测已有实例，每次双击都启动独立窗口

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("PhotoView")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true),

        ..Default::default()
    };

    eframe::run_native(
        "PhotoView",
        options,
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();

            for font_path in &[
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\simhei.ttf",
            ] {
                if let Ok(font_data) = std::fs::read(font_path) {
                    let name = "cjk";

                    fonts
                        .font_data
                        .insert(name.to_owned(), egui::FontData::from_owned(font_data));

                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push(name.to_owned());

                    fonts
                        .families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .push(name.to_owned());

                    break;
                }
            }

            cc.egui_ctx.set_fonts(fonts);

            let mut app = PhotoViewApp::default();

            app.initial_path = initial_path;

            Ok(Box::new(app))
        }),
    )
}
