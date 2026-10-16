// 图片解码、异步加载与预加载

use eframe::egui;
use egui::Color32;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

pub const MAX_TEXTURE_DIM: u32 = 2048;

pub(crate) const ARROW_ZONE_W: f32 = 80.0;

pub(crate) const ARROW_ZONE_H: f32 = 100.0;

// ─── 图片数据 ───────────────────────────────────────

pub(crate) struct PendingImage {
    pub(crate) path: PathBuf,

    pub(crate) pixels: Vec<Color32>,

    pub(crate) tex_w: usize,

    pub(crate) tex_h: usize,

    pub(crate) orig_w: usize,

    pub(crate) orig_h: usize,
}

#[derive(Clone)]

pub(crate) struct PreloadedImage {
    pub(crate) path: PathBuf,

    pub(crate) tex_w: usize,

    pub(crate) tex_h: usize,

    pub(crate) pixels: Vec<Color32>,

    pub(crate) orig_w: usize,

    pub(crate) orig_h: usize,
}

/// 动画帧
pub(crate) struct AnimFrame {
    pub(crate) pixels: Vec<Color32>,

    pub(crate) tex_w: usize,

    pub(crate) tex_h: usize,

    pub(crate) delay_ms: u32, // 帧延迟（毫秒）
}

pub(crate) type ModelRenderResult = (usize, usize, Vec<Color32>, bool);

pub(crate) fn resize_for_texture(img: DynamicImage) -> DynamicImage {
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

pub(crate) fn rgba_to_color32(rgba: &[u8]) -> Vec<Color32> {
    let mut pixels = Vec::with_capacity(rgba.len() / 4);

    for chunk in rgba.as_chunks::<4>().0 {
        pixels.push(Color32::from_rgba_unmultiplied(
            chunk[0], chunk[1], chunk[2], chunk[3],
        ));
    }

    pixels
}

/// 把整帧抽成小图，仅用于箭头/UI 自适应配色
pub(crate) fn downsample_for_tint(
    src: &[Color32],
    w: usize,
    h: usize,
    side: usize,
) -> Vec<Color32> {
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
pub(crate) fn open_image_robust(
    path: &std::path::Path,
) -> Result<image::DynamicImage, image::ImageError> {
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
pub(crate) fn image_dimensions_robust(
    path: &std::path::Path,
) -> Result<(u32, u32), image::ImageError> {
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

pub(crate) fn load_image_async(
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
pub(crate) fn load_gif_frames(path: &PathBuf, dest: Arc<Mutex<Option<Vec<AnimFrame>>>>) {
    use std::fs::File;

    use std::io::BufReader;

    if let Ok(file) = File::open(path) {
        let reader = BufReader::new(file);

        if let Ok(decoder) = image::codecs::gif::GifDecoder::new(reader) {
            let mut frames = Vec::new();

            for frame in decoder.into_frames().flatten() {
                let delay_ms = {
                    let (n, d) = frame.delay().numer_denom_ms();

                    n.checked_div(d).unwrap_or(100)
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

            if frames.len() > 1 {
                *dest.lock().unwrap() = Some(frames);
            }
        }
    }
}

pub(crate) fn preload_async(path: PathBuf, dest: Arc<Mutex<Option<PreloadedImage>>>) {
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
pub(crate) fn adaptive_arrow_color(
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

pub(crate) fn in_arrow_zone(pos: egui::Pos2, screen: egui::Rect) -> bool {
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
