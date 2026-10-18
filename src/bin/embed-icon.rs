//! embed-icon — 把多尺寸 .ico 写入 Windows PE 的图标资源（RT_ICON / RT_GROUP_ICON）。
//!
//! 用法：
//!   embed-icon.exe <exe路径> [ico路径]   嵌入图标（缺省 ico：assets/dadaview.ico）
//!   embed-icon.exe --verify <exe路径>    加载 exe 中的图标组 #1 验证资源有效性
//!
//! 实现走 kernel32 的 BeginUpdateResourceW / UpdateResourceW / EndUpdateResourceW，
//! 不依赖 rc.exe / windres / winres，任何 Rust 工具链都能构建。

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
const LANG_EN_US: u16 = 1033;
/// GDI_IMAGE_TYPE: 0=BITMAP, 1=ICON, 2=CURSOR
const IMAGE_ICON_TYPE: GDI_IMAGE_TYPE = GDI_IMAGE_TYPE(1);

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

fn embed(exe: &str, ico: &str) -> Result<(), String> {
    let ico_bytes = std::fs::read(ico).map_err(|e| format!("读取 {} 失败: {}", ico, e))?;
    let images = parse_ico(&ico_bytes)?;

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

    unsafe { EndUpdateResourceW(h, false) }.map_err(|e| format!("EndUpdateResourceW 失败: {}", e))?;

    let dims: Vec<String> = images
        .iter()
        .map(|im| if im.dim == 0 { "256".into() } else { im.dim.to_string() })
        .collect();
    println!(
        "已嵌入 {} 张图标（{} px）-> {}",
        images.len(),
        dims.join(" / "),
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
