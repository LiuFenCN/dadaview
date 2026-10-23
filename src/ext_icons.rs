//! 文档图标：扩展名 → exe 内图标资源的映射（唯一权威定义）。
//!
//! `assets/ext-icons/<ext>.ico`（由 `gen_icons.py` 生成）由 `embed-icon` 按
//! 本清单顺序嵌入 dadaview.exe：
//!
//!   资源 ID 1            = 应用图标（assets/dadaview.ico，Explorer 序号 0）
//!   资源 ID 2 + ordinal  = 各格式文档图标（Explorer 序号 = 1 + ordinal）
//!
//! `DefaultIcon` 注册表值里的序号即 Explorer 序号，见 [`doc_icon_shell_index`]。

// 资源 ID 常量与 resource_id 函数仅 embed-icon 使用；embed-icon 以
// `#[path]` 方式 include 本模块并开启了 allow(unused)，主程序编译时
// 保留这三个导出（带 allow）以维持清单的唯一权威定义
#![allow(dead_code)]

/// 文档图标覆盖的全部扩展名（顺序即资源序号，勿改；与 `gen_icons.py` 一致）
pub(crate) const DOC_ICON_EXTS: &[&str] = &[
    // 图像（粉）
    "png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif", "gif", "ico",
    // 模型（蓝）
    "obj", "stl", "gltf", "glb", "ply", "off", "3ds", "dae", "vtk", "wrl", "vrml",
    "vrm", "fbx", "pmx", "pmd", "usd", "usda", "usdc", "usdz",
    // 3D 打印（绿）
    "amf", "3mf", "gcode", "gco", "g",
];

/// 应用图标占用的资源 ID 与 Explorer 序号
pub(crate) const APP_ICON_ID: u16 = 1;
/// 文档图标组的起始资源 ID（Explorer 序号 1 对应资源 ID 2）
pub(crate) const DOC_ICON_BASE_ID: u16 = 2;

/// 扩展名在文档图标序列中的 0 起始序号
pub(crate) fn doc_icon_ordinal(ext: &str) -> Option<usize> {
    DOC_ICON_EXTS.iter().position(|e| *e == ext)
}

/// 扩展名对应的 RT_GROUP_ICON 资源 ID
pub(crate) fn doc_icon_resource_id(ext: &str) -> Option<u16> {
    doc_icon_ordinal(ext).map(|o| DOC_ICON_BASE_ID + o as u16)
}

/// 注册表 `DefaultIcon = "<exe>,<index>` 中使用的图标序号
pub(crate) fn doc_icon_shell_index(ext: &str) -> Option<u16> {
    doc_icon_ordinal(ext).map(|o| o as u16 + 1)
}

/// 格式的类别名（用于注册表的 FriendlyTypeName）
pub(crate) fn ext_category(ext: &str) -> &'static str {
    match ext {
        "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tiff" | "tif" | "gif" | "ico" => "图像",
        "amf" | "3mf" | "gcode" | "gco" | "g" => "打印文件",
        _ => "模型",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_unique_and_in_range() {
        let mut seen = std::collections::BTreeSet::new();
        for ext in DOC_ICON_EXTS {
            let id = doc_icon_resource_id(ext).expect("ext must have icon");
            let idx = doc_icon_shell_index(ext).expect("ext must have icon");
            assert_eq!(id, idx + 1, "shell index = 资源 ID - 1（应用图标占 0）");
            assert!(seen.insert(idx), "序号冲突: {ext}");
        }
        assert_eq!(seen.len(), DOC_ICON_EXTS.len());
    }
}
