//! embed-icon — 把多尺寸 .ico 与 VERSIONINFO 写入 Windows PE 资源段。
//!
//! 用法：
//!   embed-icon.exe <exe路径> [ico路径]   嵌入图标 + 版本信息（缺省 ico：assets/dadaview.ico）
//!   embed-icon.exe --verify <exe路径>    加载 exe 中的图标组 #1 验证资源有效性
//!
//! 实现走 kernel32 的 BeginUpdateResourceW / UpdateResourceW / EndUpdateResourceW，
//! 不依赖 rc.exe / windres / winres，任何 Rust 工具链都能构建。
//!
//! 版本号从根 Cargo.toml 的 `version` 读取，因此发版只需改那一处，
//! exe 属性页里的「文件版本 / 产品版本」会自动跟着变。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, LoadLibraryExW, UpdateResourceW,
    LOAD_LIBRARY_AS_DATAFILE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GDI_IMAGE_TYPE, IMAGE_FLAGS, LoadImageW,
};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN_US: u16 = 1033;
/// GDI_IMAGE_TYPE: 0=BITMAP, 1=ICON, 2=CURSOR
const IMAGE_ICON_TYPE: GDI_IMAGE_TYPE = GDI_IMAGE_TYPE(1);

const PRODUCT_NAME: &str = "DaDaView";
const COMPANY_NAME: &str = "LiuFenCN";
const FILE_DESCRIPTION: &str = "DaDaView 图片查看器 + 3D 模型预览";
const COPYRIGHT: &str = "Copyright (C) 2026 LiuFenCN · AGPL-3.0";

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// 整数资源名（IS_INTRESOURCE）：指针值即 ID
fn intres(id: u16) -> PCWSTR {
    PCWSTR::from_raw(id as usize as *const u16)
}

struct IcoImage {
    dim: u16, // 0 表示 256（与 ICO/GRPICONDIR 表示法一致）
    data: Vec<u8>,
}

fn parse_ico(bytes: &[u8]) -> Result<Vec<IcoImage>, String> {
    if bytes.len() < 6 || u16::from_le_bytes([bytes[0], bytes[1]]) != 0 {
        return Err("不是合法的 ICO 文件".into());
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = 6 + 16 * i;
        let e = bytes
            .get(base..base + 16)
            .ok_or_else(|| "ICO 目录项越界".to_string())?;
        let w = e[0];
        let h = e[1];
        let size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let data = bytes
            .get(off..off + size)
            .ok_or_else(|| "ICO 图像数据越界".to_string())?
            .to_vec();
        // ICO 里 256 用 0 表示；统一存 0
        let dim = |b: u8| if b == 0 { 0 } else { b as u16 };
        out.push(IcoImage {
            dim: dim(w).max(dim(h)),
            data,
        });
    }
    if out.is_empty() {
        return Err("ICO 不含任何图像".into());
    }
    Ok(out)
}

// ---------- VERSIONINFO（RT_VERSION）----------

/// 补齐到 4 字节边界。注意 VS_VERSIONINFO 的 Padding 是相对**整个块起始**算的，
/// 块头占 6 字节，所以补零时要带上这个偏移。
fn pad_to_4(block_len_after: usize, body: &mut Vec<u8>) {
    while (block_len_after + body.len()) % 4 != 0 {
        body.push(0);
    }
}

fn utf16z(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// 组装一个 VS_VERSIONINFO 风格的块：
/// `wLength | wValueLength | wType | szKey(NUL) | pad | Value | pad | Children`
///
/// - `value_words`：字符串值为字符数（含结尾 NUL），二进制值为字节数
/// - `is_text`：wType，字符串块为 1，二进制为 0
fn ver_block(key: &str, value: &[u8], value_words: u16, is_text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    const HDR: usize = 6;
    let mut body = utf16z(key);
    pad_to_4(HDR, &mut body);
    body.extend_from_slice(value);
    pad_to_4(HDR, &mut body);
    for c in children {
        body.extend_from_slice(c);
    }

    let mut out = Vec::with_capacity(6 + body.len());
    out.extend_from_slice(&((6 + body.len()) as u16).to_le_bytes()); // wLength
    out.extend_from_slice(&value_words.to_le_bytes()); // wValueLength
    out.extend_from_slice(&u16::from(is_text).to_le_bytes()); // wType
    out.extend_from_slice(&body);
    out
}

fn parse_version(v: &str) -> (u16, u16, u16, u16) {
    let mut it = v
        .split(|c: char| c == '.' || c == '-' || c == '+')
        .map(|p| p.trim().parse::<u16>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

fn build_version_resource(version: &str, original_filename: &str, internal_name: &str) -> Vec<u8> {
    let (major, minor, build, rev) = parse_version(version);
    let ms = ((major as u32) << 16) | minor as u32;
    let ls = ((build as u32) << 16) | rev as u32;

    // VS_FIXEDFILEINFO：13 个 DWORD，共 52 字节
    let mut ffi = Vec::with_capacity(52);
    for v in [
        0xFEEF_04BDu32, // dwSignature
        0x0001_0000,    // dwStrucVersion
        ms,             // dwFileVersionMS
        ls,             // dwFileVersionLS
        ms,             // dwProductVersionMS
        ls,             // dwProductVersionLS
        0x0000_003F,    // dwFileFlagsMask
        0x0000_0000,    // dwFileFlags
        0x0004_0004,    // dwFileOS = VOS_NT_WINDOWS32
        0x0000_0001,    // dwFileType = VFT_APP
        0x0000_0000,    // dwFileSubtype
        0x0000_0000,    // dwFileDateMS
        0x0000_0000,    // dwFileDateLS
    ] {
        ffi.extend_from_slice(&v.to_le_bytes());
    }

    let pairs: [(&str, &str); 8] = [
        ("CompanyName", COMPANY_NAME),
        ("FileDescription", FILE_DESCRIPTION),
        ("FileVersion", version),
        ("InternalName", internal_name),
        ("LegalCopyright", COPYRIGHT),
        ("OriginalFilename", original_filename),
        ("ProductName", PRODUCT_NAME),
        ("ProductVersion", version),
    ];
    let mut strings = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let words = v.encode_utf16().count() as u16 + 1;
        strings.push(ver_block(k, &utf16z(v), words, true, &[]));
    }

    // 语言 0x0409（en-US）、代码页 0x04B0（Unicode）
    let table = ver_block("040904B0", &[], 0, true, &strings);
    let sfi = ver_block("StringFileInfo", &[], 0, true, &[table]);

    let mut translation = Vec::with_capacity(4);
    translation.extend_from_slice(&0x0409u16.to_le_bytes());
    translation.extend_from_slice(&0x04B0u16.to_le_bytes());
    let var = ver_block("Translation", &translation, 4, false, &[]);
    let vfi = ver_block("VarFileInfo", &[], 0, true, &[var]);

    ver_block("VS_VERSION_INFO", &ffi, 52, false, &[sfi, vfi])
}

/// 从根 Cargo.toml 读取 [package] version
fn read_cargo_version() -> Result<String, String> {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("读取 {} 失败: {}", manifest, e))?;
    text.lines()
        .find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("version")?.trim().strip_prefix('=')?;
            Some(rest.trim().trim_matches('"').to_string())
        })
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "Cargo.toml 中未找到 version".to_string())
}

fn embed(exe: &str, ico: &str) -> Result<(), String> {
    let ico_bytes = std::fs::read(ico).map_err(|e| format!("读取 {} 失败: {}", ico, e))?;
    let images = parse_ico(&ico_bytes)?;
    let version = read_cargo_version()?;

    let exe_w = wide(exe);
    // false = 不清除既有资源（增量更新）
    let h: HANDLE = unsafe { BeginUpdateResourceW(PCWSTR(exe_w.as_ptr()), false) }
        .map_err(|e| format!("BeginUpdateResourceW 失败（文件被占用?）: {}", e))?;

    // GRPICONDIR：头 + 每张图的目录项
    let mut grp: Vec<u8> = Vec::new();
    grp.extend_from_slice(&0u16.to_le_bytes()); // reserved
    grp.extend_from_slice(&1u16.to_le_bytes()); // type = 1 (icon)
    grp.extend_from_slice(&(images.len() as u16).to_le_bytes());

    for (i, img) in images.iter().enumerate() {
        let id = (i + 1) as u16;
        unsafe {
            UpdateResourceW(
                h,
                intres(RT_ICON),
                intres(id),
                LANG_EN_US,
                Some(img.data.as_ptr() as *const _),
                img.data.len() as u32,
            )
        }
        .map_err(|e| format!("写入 RT_ICON #{} 失败: {}", id, e))?;
        // GRPICONDIRENTRY（宽高为 0 表示 256）
        grp.push(if img.dim == 0 { 0 } else { img.dim as u8 });
        grp.push(if img.dim == 0 { 0 } else { img.dim as u8 });
        grp.push(0); // colorCount
        grp.push(0); // reserved
        grp.extend_from_slice(&1u16.to_le_bytes()); // planes
        grp.extend_from_slice(&32u16.to_le_bytes()); // bitCount
        grp.extend_from_slice(&(img.data.len() as u32).to_le_bytes()); // bytesInRes
        grp.extend_from_slice(&id.to_le_bytes()); // 资源 ID
    }

    unsafe {
        UpdateResourceW(
            h,
            intres(RT_GROUP_ICON),
            intres(1),
            LANG_EN_US,
            Some(grp.as_ptr() as *const _),
            grp.len() as u32,
        )
    }
    .map_err(|e| format!("写入 RT_GROUP_ICON 失败: {}", e))?;

    // VERSIONINFO：属性页里的「文件版本 / 产品版本 / 描述 / 版权」
    let file_name = std::path::Path::new(exe)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "dadaview.exe".to_string());
    let internal = std::path::Path::new(exe)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "dadaview".to_string());
    let ver_bytes = build_version_resource(&version, &file_name, &internal);
    unsafe {
        UpdateResourceW(
            h,
            intres(RT_VERSION),
            intres(1),
            LANG_EN_US,
            Some(ver_bytes.as_ptr() as *const _),
            ver_bytes.len() as u32,
        )
    }
    .map_err(|e| format!("写入 RT_VERSION 失败: {}", e))?;

    unsafe { EndUpdateResourceW(h, false) }.map_err(|e| format!("EndUpdateResourceW 失败: {}", e))?;

    let dims: Vec<String> = images
        .iter()
        .map(|im| if im.dim == 0 { "256".into() } else { im.dim.to_string() })
        .collect();
    println!(
        "已嵌入 {} 张图标（{} px）+ 版本信息 {} -> {}",
        images.len(),
        dims.join(" / "),
        version,
        exe
    );
    Ok(())
}

fn verify(exe: &str) -> Result<(), String> {
    let exe_w = wide(exe);
    let hmod = unsafe { LoadLibraryExW(PCWSTR(exe_w.as_ptr()), None, LOAD_LIBRARY_AS_DATAFILE) }
        .map_err(|e| format!("LoadLibraryExW 失败: {}", e))?;
    let loaded = unsafe {
        LoadImageW(
            hmod,
            intres(1),
            IMAGE_ICON_TYPE,
            256,
            256,
            IMAGE_FLAGS(0),
        )
    };
    let ok = match loaded {
        Ok(g) if !g.is_invalid() => {
            println!("OK: {} 含有效图标组 #1（256x256 可加载）", exe);
            true
        }
        _ => {
            println!("FAIL: {} 中未找到可加载的图标组 #1", exe);
            false
        }
    };
    if ok {
        Ok(())
    } else {
        Err("图标资源缺失".into())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let default_ico = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/dadaview.ico").to_string();

    let result = match args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_slice() {
        [_ , "--verify", exe] => verify(exe),
        [_ , exe] => embed(exe, &default_ico),
        [_ , exe, ico] => embed(exe, ico),
        _ => {
            eprintln!("用法: embed-icon <exe路径> [ico路径] | embed-icon --verify <exe路径>");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("错误: {}", e);
        std::process::exit(1);
    }
}
