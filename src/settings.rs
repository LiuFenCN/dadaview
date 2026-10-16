// 键位绑定与应用设置

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub(crate) fn labels(&self) -> Vec<(&'static str, String)> {
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

    pub(crate) fn get_mut(&mut self, label: &str) -> Option<&mut String> {
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

pub(crate) fn drag_binding_label(code: &str) -> String {
    match code {
        "LeftDrag" => "左键拖动".into(),

        "MiddleDrag" => "中键拖动".into(),

        "ShiftLeftDrag" => "Shift+左键拖动".into(),

        other => other.to_string(),
    }
}

/// 将 egui Key 转为可读字符串
pub(crate) fn key_to_string(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowLeft => "ArrowLeft".into(),

        egui::Key::ArrowRight => "ArrowRight".into(),

        egui::Key::ArrowUp => "ArrowUp".into(),

        egui::Key::ArrowDown => "ArrowDown".into(),

        k => format!("{:?}", k),
    }
}

/// 将字符串解析回 egui::Key（部分支持）
pub(crate) fn string_to_key(s: &str) -> Option<egui::Key> {
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
    pub(crate) fn load() -> Self {
        let path = settings_path();

        if path.exists()
            && let Ok(data) = std::fs::read_to_string(&path)
            && let Ok(s) = serde_json::from_str(&data)
        {
            return s;
        }

        Self::default()
    }

    pub(crate) fn save(&self) {
        let path = settings_path();

        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, data);
        }
    }
}

/// 计算 2cm 对应的像素值（适配 DPI 缩放）
pub(crate) fn mini_size_pp(ppi: f32) -> f32 {
    (2.0 * 96.0 / 2.54 * ppi).round()
}

/// 人类可读文件大小
pub(crate) fn human_size(bytes: u64) -> String {
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
pub(crate) fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf()
}

pub(crate) fn settings_path() -> PathBuf {
    exe_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trip() {
        for key in [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::C,
            egui::Key::R,
            egui::Key::Escape,
            egui::Key::F1,
            egui::Key::F5,
            egui::Key::F12,
        ] {
            let s = key_to_string(key);
            assert_eq!(string_to_key(&s), Some(key), "往返失败: {s}");
        }
        assert_eq!(string_to_key("NoSuchKey"), None);
    }

    #[test]
    fn bindings_labels_match_get_mut() {
        let mut kb = KeyBindings::default();

        for (label, _) in kb.labels() {
            assert!(kb.get_mut(label).is_some(), "标签 {label} 无法映射回字段");
        }

        *kb.get_mut("下一页").unwrap() = "N".into();

        assert_eq!(kb.next, "N");
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn mini_size_scales_linearly_with_dpi_factor() {
        let base = mini_size_pp(1.0);

        assert!((base - 75.6).abs() < 1.0, "2cm@96dpi 约 76px，实际 {base}");
        assert!((mini_size_pp(1.5) - base * 1.5).abs() < 2.0);
    }
}
