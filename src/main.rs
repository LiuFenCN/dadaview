// 隐藏 Windows 控制台窗口

#![windows_subsystem = "windows"]

mod file_assoc;
mod gpu3d;
mod imaging;
mod mesh3d;
mod settings;
mod win32;

use eframe::{NativeOptions, egui};

use egui::{Color32, RichText, TextureHandle};

use rfd::FileDialog;

use std::collections::BTreeMap;

use std::path::PathBuf;

use std::sync::{Arc, Mutex};

use std::thread;

use file_assoc::register_file_association;
use imaging::{
    ARROW_ZONE_H, ARROW_ZONE_W, AnimFrame, ModelRenderResult, PendingImage, PreloadedImage,
    adaptive_arrow_color, downsample_for_tint, image_dimensions_robust, in_arrow_zone,
    load_gif_frames, load_image_async, preload_async,
};
use settings::{
    AppSettings, drag_binding_label, human_size, key_to_string, mini_size_pp, string_to_key,
};
use win32::apply_win11_effects;

const SUPPORTED_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif", "ico", "obj", "stl", "gltf", "glb",
    "ply", "off", "3ds", "dae", "vtk", "wrl", "vrml", "amf", "3mf", "gcode", "gco", "g", "vrm",
    "fbx", "pmx", "pmd", "usd", "usda", "usdc", "usdz",
];

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

    loaded_next2: Arc<Mutex<Option<PreloadedImage>>>,

    loaded_prev: Arc<Mutex<Option<PreloadedImage>>>,

    loaded_prev2: Arc<Mutex<Option<PreloadedImage>>>,

    is_loading: bool,

    show_info: bool,

    round_corners_applied: bool,

    // 新功能
    comic_mode: bool,

    show_settings: bool,

    file_dialog_result: Arc<Mutex<Option<Option<PathBuf>>>>,

    dialog_open: Arc<Mutex<bool>>,

    info_cache: Option<(String, String)>,

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

            loaded_next2: Arc::new(Mutex::new(None)),

            loaded_prev: Arc::new(Mutex::new(None)),

            loaded_prev2: Arc::new(Mutex::new(None)),

            is_loading: false,

            show_info: false,

            round_corners_applied: false,

            comic_mode: false,

            show_settings: false,
            file_dialog_result: Arc::new(Mutex::new(None)),
            dialog_open: Arc::new(Mutex::new(false)),
            info_cache: None,

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

impl PhotoViewApp {
    /// 在独立线程弹系统文件对话框，结果经 file_dialog_result 回传，不阻塞 UI
    fn spawn_file_dialog(&self) {
        let dest = self.file_dialog_result.clone();
        let flag = self.dialog_open.clone();
        *flag.lock().unwrap() = true;
        thread::spawn(move || {
            let picked = FileDialog::new()
                .add_filter(
                    "图片",
                    &[
                        "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif", "ico",
                    ],
                )
                .add_filter(
                    "3D 模型",
                    &[
                        "obj", "stl", "gltf", "glb", "ply", "off", "3ds", "dae", "vtk", "wrl",
                        "vrml", "amf", "vrm", "fbx", "pmx", "pmd", "usd", "usda", "usdc", "usdz",
                    ],
                )
                .add_filter("3D 打印", &["stl", "3mf", "amf", "gcode", "gco", "obj"])
                .add_filter(
                    "所有支持",
                    &[
                        "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif", "ico", "obj",
                        "stl", "gltf", "glb", "ply", "off", "3ds", "dae", "vtk", "wrl", "vrml",
                        "amf", "3mf", "gcode", "gco", "g", "vrm", "fbx", "pmx", "pmd", "usd",
                        "usda", "usdc", "usdz",
                    ],
                )
                .pick_file();
            *dest.lock().unwrap() = Some(picked);
            *flag.lock().unwrap() = false;
        });
    }

    /// 文件切换时缓存大小/修改时间，信息面板不再每帧打磁盘
    fn refresh_info_cache(&mut self) {
        self.info_cache = self
            .current_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| {
                let size = human_size(m.len());
                let modified = m
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|| "—".into());
                (size, modified)
            });
    }

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
        self.refresh_info_cache();

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
        self.refresh_info_cache();

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
        self.refresh_info_cache();

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

    /// 重排一个方向的两个槽位：优先复用已命中新目标的预加载图，缺的再异步补
    fn refill_dir(
        near: &Arc<Mutex<Option<PreloadedImage>>>,
        far: &Arc<Mutex<Option<PreloadedImage>>>,
        t_near: Option<PathBuf>,
        t_far: Option<PathBuf>,
    ) {
        let mut cands: Vec<PreloadedImage> = near
            .lock()
            .unwrap()
            .take()
            .into_iter()
            .chain(far.lock().unwrap().take())
            .collect();

        let mut reuse = |t: &Option<PathBuf>| -> Option<PreloadedImage> {
            let t = t.as_ref()?;

            let idx = cands.iter().position(|c| &c.path == t)?;

            Some(cands.remove(idx))
        };

        let n = reuse(&t_near);

        let f = reuse(&t_far);

        let n_missing = n.is_none();

        let f_missing = f.is_none();

        let far_dup_of_near = t_far.is_some() && t_far.as_deref() == t_near.as_deref();

        *near.lock().unwrap() = n;

        *far.lock().unwrap() = f;

        if n_missing && let Some(p) = t_near {
            preload_async(p, near.clone());
        }

        if f_missing
            && !far_dup_of_near
            && let Some(p) = t_far
        {
            preload_async(p, far.clone());
        }
    }

    fn preload_neighbors(&mut self) {
        if self.image_files.is_empty() {
            return;
        }

        let l = self.image_files.len();

        let step = if self.comic_mode { 2 } else { 1 };

        let cur = self.current_path.clone();

        // 3D 模型与当前图不预加载
        let target_at = |delta: isize| -> Option<PathBuf> {
            let idx = (self.current_index as isize + delta * step as isize).rem_euclid(l as isize)
                as usize;

            let p = &self.image_files[idx];

            (!mesh3d::is_model_path(p) && Some(p.as_path()) != cur.as_deref()).then(|| p.clone())
        };

        let near_next = target_at(1);

        let far_next = target_at(2);

        let near_prev = target_at(-1);

        let far_prev = target_at(-2);

        Self::refill_dir(&self.loaded_next, &self.loaded_next2, near_next, far_next);

        Self::refill_dir(&self.loaded_prev, &self.loaded_prev2, near_prev, far_prev);
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

    /// 切换窗口置顶状态（走 egui 视口命令，作用于本窗口，不受同名窗口干扰）
    fn toggle_always_on_top(&mut self, ctx: &egui::Context) {
        self.always_on_top = !self.always_on_top;
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.always_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
    }

    fn paint_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let name = self
            .current_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let (file_size, modified) = self
            .info_cache
            .clone()
            .unwrap_or_else(|| ("—".into(), "—".into()));

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

        // 异步文件对话框结果回收
        let dialog_result = self.file_dialog_result.lock().unwrap().take();
        if let Some(picked) = dialog_result {
            if let Some(path) = picked {
                self.open_image(path, ctx);
            }
            ctx.request_repaint();
        } else if self.show_settings || *self.dialog_open.lock().unwrap() {
            // 设置面板或对话框打开期间保持低频轮询
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        //
        if self.should_close_next_frame
            && !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary))
        {
            if self.show_settings {
                // 设置面板打开时：右键只关面板，不退出程序
                self.show_settings = false;
                self.should_close_next_frame = false;
            } else {
                self.settings.save();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
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
                .is_none_or(|t| t.is_none());
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
                    .is_none_or(|tex| {
                        self.current_texture
                            .as_ref()
                            .is_none_or(|cur| cur.id() != tex.id())
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
            self.toggle_always_on_top(ctx);
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
                        {
                            self.spawn_file_dialog();
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
                        if let Some(tex2) = self.comic_second_texture.as_ref()
                            && self.comic_mode
                        {
                            // ── 漫画模式：双图并排 ──

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

            let app = PhotoViewApp {
                initial_path,
                ..Default::default()
            };

            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(path: &str) -> PreloadedImage {
        PreloadedImage {
            path: PathBuf::from(path),
            tex_w: 1,
            tex_h: 1,
            pixels: Vec::new(),
            orig_w: 1,
            orig_h: 1,
        }
    }

    fn slot(p: Option<PreloadedImage>) -> Arc<Mutex<Option<PreloadedImage>>> {
        Arc::new(Mutex::new(p))
    }

    #[test]
    fn take_slot_only_when_path_matches() {
        let s = slot(Some(pre("a.png")));
        let miss = PhotoViewApp::take_slot_if_match(&s, std::path::Path::new("b.png"), |p| &p.path);
        assert!(miss.is_none(), "不匹配必须返回 None");
        assert!(s.lock().unwrap().is_some(), "不匹配时槽位原样保留");

        let hit = PhotoViewApp::take_slot_if_match(&s, std::path::Path::new("a.png"), |p| &p.path);
        assert!(hit.is_some(), "匹配必须取出");
        assert!(s.lock().unwrap().is_none(), "取出后槽位清空");
    }

    #[test]
    fn refill_dir_promotes_far_candidate_to_near() {
        // 近槽是过期数据 x，远槽是 a；新目标近=a、远=None
        // 期望：a 从远槽升级到近槽，x 被丢弃，全程不触发异步加载
        let near = slot(Some(pre("x.png")));
        let far = slot(Some(pre("a.png")));

        PhotoViewApp::refill_dir(&near, &far, Some(PathBuf::from("a.png")), None);

        assert_eq!(
            near.lock().unwrap().as_ref().map(|p| p.path.as_path()),
            Some(std::path::Path::new("a.png")),
            "远槽命中新近目标应升级"
        );
        assert!(
            far.lock().unwrap().is_none(),
            "远目标为空则远槽清空且不加载"
        );
    }

    #[test]
    fn refill_dir_reuses_both_slots_without_reloading() {
        let near = slot(Some(pre("a.png")));
        let far = slot(Some(pre("b.png")));
        PhotoViewApp::refill_dir(
            &near,
            &far,
            Some(PathBuf::from("a.png")),
            Some(PathBuf::from("b.png")),
        );
        assert_eq!(
            near.lock().unwrap().as_ref().map(|p| p.path.as_path()),
            Some(std::path::Path::new("a.png"))
        );
        assert_eq!(
            far.lock().unwrap().as_ref().map(|p| p.path.as_path()),
            Some(std::path::Path::new("b.png"))
        );
    }

    #[test]
    fn refill_dir_skips_duplicate_far_target() {
        // 远近目标相同（小目录首尾重叠）：只应命中一个，另一个槽留空且不重复加载
        let near = slot(None);
        let far = slot(Some(pre("only.png")));
        PhotoViewApp::refill_dir(
            &near,
            &far,
            Some(PathBuf::from("only.png")),
            Some(PathBuf::from("only.png")),
        );
        assert_eq!(
            near.lock().unwrap().as_ref().map(|p| p.path.as_path()),
            Some(std::path::Path::new("only.png")),
            "唯一候选应进近槽"
        );
        assert!(far.lock().unwrap().is_none(), "重复远目标不应二次加载");
    }
}
