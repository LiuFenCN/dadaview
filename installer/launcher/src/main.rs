#![windows_subsystem = "windows"]

// DaDaView 安装器（纯 exe，全程不调用 msiexec / Windows Installer，
// 因此不会出现 MSI 的 2502 / 2503 错误）。
//
// 自带 Win32 原生向导界面，流程：
//   欢迎 → 许可协议 → 选择安装目录 → 安装进度 → 完成
//
// 构建顺序：先构建 installer/uninstaller（dadaview-uninstall.exe），再构建本 crate
// （build.rs 会把主 exe / 卸载器 / README / LICENSE 打成 payload.zip 内嵌）。

use std::ffi::{c_void, OsStr};
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use windows::core::{w, Interface, PCWSTR, PWSTR, Result as WinResult};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SYSTEMTIME, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, EndPaint, FillRect, HBRUSH, HDC, HFONT, InvalidateRect,
    PAINTSTRUCT, SetBkColor, SetTextColor, UpdateWindow,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, INITCOMMONCONTROLSEX, INITCOMMONCONTROLSEX_ICC,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::{
    FileOpenDialog, FOS_PICKFOLDERS, IFileDialog, IShellLinkW, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW, ShellExecuteExW, ShellLink, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GDI_IMAGE_TYPE,
    GetClientRect, GetMessageW, GetWindowLongPtrW, GetWindowTextW, GWLP_USERDATA, HICON, HMENU,
    IMAGE_FLAGS, LoadCursorW, LoadImageW, MSG, PeekMessageW, PM_REMOVE, PostQuitMessage,
    RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow, SW_HIDE, SW_SHOW,
    TranslateMessage, IDC_ARROW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_PAINT, WNDCLASSW, WNDCLASS_STYLES,
};

const APP_NAME: &str = "DaDaView";
/// 版本号由 build.rs 从根 Cargo.toml 读出并注入，只维护一处。
const APP_VERSION: &str = env!("DADAVIEW_VERSION");
const PUBLISHER: &str = "LiuFenCN";
const APP_DESC: &str = "图片查看器 + 3D 模型预览";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\DaDaView";

// payload（build.rs 构建期打包的 deflate zip，见 README 构建顺序）
const PAYLOAD_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));
const LICENSE_TEXT: &str = include_str!("../../../LICENSE");
const LICENSE_ZH: &str = include_str!("../../../LICENSE.zh-CN.md");

// ---------- Win32 常量（直接写数值，避免不同 windows 版本常量类型差异） ----------

const WS_CHILD_U: u32 = 0x4000_0000;
const WS_VISIBLE_U: u32 = 0x1000_0000;
const WS_TABSTOP_U: u32 = 0x0001_0000;
const WS_VSCROLL_U: u32 = 0x0020_0000;
const WS_CAPTION_U: u32 = 0x00C0_0000;
const WS_SYSMENU_U: u32 = 0x0008_0000;
const WS_MINIMIZEBOX_U: u32 = 0x0002_0000;
const WS_EX_CLIENTEDGE_U: u32 = 0x0000_0200;

const BS_PUSHBUTTON_U: u32 = 0x0000_0000;
const BS_DEFPUSHBUTTON_U: u32 = 0x0000_0001;
const BS_AUTOCHECKBOX_U: u32 = 0x0000_0003;

const ES_MULTILINE_U: u32 = 0x0004;
const ES_AUTOHSCROLL_U: u32 = 0x0080;
const ES_AUTOVSCROLL_U: u32 = 0x0040;
const ES_READONLY_U: u32 = 0x0800;

const SS_RIGHT_U: u32 = 0x0000_0002;

const BM_GETCHECK_U: u32 = 0x00F0;
const BM_SETCHECK_U: u32 = 0x00F1;
const BST_CHECKED_U: usize = 1;

const PBM_SETPOS_U: u32 = 0x0402;
const PBM_SETRANGE32_U: u32 = 0x0406;

const WM_SETFONT_U: u32 = 0x0030;
const BN_CLICKED_U: usize = 0;
const COLOR_3DFACE_U: u32 = 15;
const ICC_PROGRESS_CLASS_U: u32 = 0x0000_0020;
const IMAGE_ICON_TYPE: GDI_IMAGE_TYPE = GDI_IMAGE_TYPE(1);

// 控件 ID
const ID_BACK: isize = 1001;
const ID_NEXT: isize = 1002;
const ID_CANCEL: isize = 1003;
const ID_BROWSE: isize = 1004;
const ID_ACCEPT: isize = 1005;
const ID_DESKTOP: isize = 1006;
const ID_STARTMENU: isize = 1007;
const ID_PROGRESS: isize = 1008;
const ID_PFILE: isize = 1009;
const ID_PPCT: isize = 1010;
const ID_RUN: isize = 1011;
const ID_CLEANSMI: isize = 1012;

// 布局（96 DPI 下的逻辑像素）
const BASE_W: i32 = 640;
const BASE_H: i32 = 480;
const HEADER_H: i32 = 62;
const FOOTER_H: i32 = 54;
const MARGIN: i32 = 14;
const BTN_W: i32 = 88;
const BTN_H: i32 = 28;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// EDIT 控件只认 CRLF，把内嵌文本的换行统一转换。
fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// u32 → WINDOW_STYLE（两者都是 #[repr(transparent)] 的新类型）
fn ws(v: u32) -> WINDOW_STYLE {
    unsafe { std::mem::transmute::<u32, WINDOW_STYLE>(v) }
}

fn wes(v: u32) -> WINDOW_EX_STYLE {
    unsafe { std::mem::transmute::<u32, WINDOW_EX_STYLE>(v) }
}

/// 整数资源名（IS_INTRESOURCE）：指针值即 ID
fn intres(id: u16) -> PCWSTR {
    PCWSTR::from_raw(id as usize as *const u16)
}

/// windows 0.58 的注册表函数返回 WIN32_ERROR 结构体（.0 为 u32），统一转 Result。
fn win32_result(code: windows::Win32::Foundation::WIN32_ERROR) -> WinResult<()> {
    if code.0 == 0 {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(
            windows::core::HRESULT::from_win32(code.0),
        ))
    }
}

fn fail(msg: &str) -> ! {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            PCWSTR(wide(msg).as_ptr()),
            w!("DaDaView 安装程序"),
            windows::Win32::UI::WindowsAndMessaging::MB_OK
                | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
        );
    }
    std::process::exit(1);
}

// ---------- 安装线程 → UI 线程的进度共享状态 ----------

static PROG_DONE: AtomicU64 = AtomicU64::new(0);
static PROG_TOTAL: AtomicU64 = AtomicU64::new(0);
static PROG_FILE: Mutex<String> = Mutex::new(String::new());
static INSTALLING: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

// ---------- 提权 ----------

fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// 以管理员身份重新启动自身，并等待子进程结束、回传退出码。
fn relaunch_elevated(args: &[String]) -> Option<i32> {
    let exe = std::env::current_exe().ok()?;
    let exe_w = wide(&exe.to_string_lossy());
    let params = args.join(" ");
    let params_w = wide(&params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe_w.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut info) }.is_err() {
        return None;
    }
    let mut code = 1u32;
    unsafe {
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Some(code as i32)
}

// ---------- 目录选择 ----------

fn pick_install_dir(default_dir: &Path) -> Option<PathBuf> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .ok()
            .or_else(|| fail("无法创建目录选择对话框。"))?;
        let _ = dialog.SetOptions(FOS_PICKFOLDERS);
        let title = wide("选择 DaDaView 的安装目录");
        let _ = dialog.SetTitle(PCWSTR(title.as_ptr()));
        let name = wide(&default_dir.to_string_lossy());
        let _ = dialog.SetFileName(PCWSTR(name.as_ptr()));
        if dialog.Show(None).is_err() {
            return None; // 用户取消
        }
        let item = dialog.GetResult().ok()?;
        let p: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let mut len = 0usize;
        while *p.0.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p.0, len));
        CoTaskMemFree(Some(p.0 as *const _));
        let dir = PathBuf::from(s);
        if dir.as_os_str().is_empty() {
            None
        } else {
            Some(dir)
        }
    }
}

// ---------- 快捷方式 ----------

fn create_shortcut(lnk_path: &Path, target: &Path, description: &str) -> WinResult<()> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let target_w = wide(&target.to_string_lossy());
        link.SetPath(PCWSTR(target_w.as_ptr()))?;
        let work_w = wide(&target.parent().unwrap_or(target).to_string_lossy());
        link.SetWorkingDirectory(PCWSTR(work_w.as_ptr()))?;
        let desc_w = wide(description);
        link.SetDescription(PCWSTR(desc_w.as_ptr()))?;
        link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;
        let persist: IPersistFile = link.cast()?;
        let lnk_w = wide(&lnk_path.to_string_lossy());
        persist.Save(PCWSTR(lnk_w.as_ptr()), true)
    }
}

// ---------- 注册表 ----------

fn reg_set_string(hkey: HKEY, name: &str, value: &str) -> WinResult<()> {
    let name_w = wide(name);
    let value_w = wide(value);
    let bytes =
        unsafe { std::slice::from_raw_parts(value_w.as_ptr() as *const u8, value_w.len() * 2) };
    win32_result(unsafe { RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), 0, REG_SZ, Some(bytes)) })
}

fn reg_set_dword(hkey: HKEY, name: &str, value: u32) -> WinResult<()> {
    let name_w = wide(name);
    win32_result(unsafe {
        RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), 0, REG_DWORD, Some(&value.to_le_bytes()))
    })
}

fn write_arp_entries(install_dir: &Path, payload_bytes: u64) -> WinResult<()> {
    unsafe {
        let mut hkey = HKEY::default();
        win32_result(RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(wide(UNINSTALL_KEY).as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_WOW64_64KEY,
            None,
            &mut hkey,
            None,
        ))?;
        let exe = install_dir.join("dadaview.exe");
        let uninstaller = install_dir.join("dadaview-uninstall.exe");
        let result = (|| {
            reg_set_string(hkey, "DisplayName", "DaDaView")?;
            reg_set_string(hkey, "DisplayIcon", &format!("{},0", exe.display()))?;
            reg_set_string(hkey, "DisplayVersion", APP_VERSION)?;
            reg_set_string(hkey, "Publisher", PUBLISHER)?;
            reg_set_string(hkey, "InstallLocation", &install_dir.to_string_lossy())?;
            reg_set_string(
                hkey,
                "UninstallString",
                &format!("\"{}\"", uninstaller.display()),
            )?;
            reg_set_string(
                hkey,
                "QuietUninstallString",
                &format!("\"{}\"", uninstaller.display()),
            )?;
            reg_set_string(
                hkey,
                "URLInfoAbout",
                "https://github.com/LiuFenCN/dadaview",
            )?;
            reg_set_string(
                hkey,
                "Comments",
                "DaDaView 图片查看器 + 3D 模型预览 / Image viewer with 3D model preview",
            )?;
            reg_set_string(hkey, "InstallDate", &today_yyyymmdd())?;
            reg_set_dword(hkey, "NoModify", 1)?;
            reg_set_dword(hkey, "NoRepair", 1)?;
            reg_set_dword(hkey, "EstimatedSize", (payload_bytes / 1024) as u32)?;
            WinResult::Ok(())
        })();
        let _ = RegCloseKey(hkey);
        result
    }
}

/// 「应用和功能」显示的安装日期格式：YYYYMMDD
fn today_yyyymmdd() -> String {
    let t: SYSTEMTIME = unsafe { GetLocalTime() };
    format!("{:04}{:02}{:02}", t.wYear, t.wMonth, t.wDay)
}

// ---------- 已有安装检测 ----------

const UNINSTALL_ROOT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const INSTALLER_PRODUCTS: &str = r"SOFTWARE\Classes\Installer\Products";
const INSTALLER_USERDATA: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData";

#[derive(Default, Clone)]
struct ExistingInstall {
    /// 本安装器（纯 exe 版）写入的条目版本
    version: Option<String>,
    location: Option<String>,
    /// 旧 MSI 版遗留的 Uninstall 子键名（形如 {GUID}）
    msi_subkey: Option<String>,
    msi_version: Option<String>,
    /// 旧 MSI 版在 Classes\Installer\Products 下的产品键名（packed GUID）
    msi_product_key: Option<String>,
    /// 旧 MSI 版在 Installer\UserData\<SID>\Products\<packed> 下的路径（相对 INSTALLER_USERDATA）
    msi_userdata_paths: Vec<String>,
}

impl ExistingInstall {
    /// 安装向导里展示的「当前已装版本」，优先取本安装器的条目
    fn display_version(&self) -> Option<&str> {
        self.version.as_deref().or(self.msi_version.as_deref())
    }
}

fn reg_query_string(hkey: HKEY, name: &str) -> Option<String> {
    let name_w = wide(name);
    unsafe {
        let mut size = 0u32;
        let mut ty = REG_VALUE_TYPE(0);
        let rc = RegQueryValueExW(
            hkey,
            PCWSTR(name_w.as_ptr()),
            None,
            Some(&mut ty),
            None,
            Some(&mut size),
        );
        if rc.0 != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let rc = RegQueryValueExW(
            hkey,
            PCWSTR(name_w.as_ptr()),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        );
        if rc.0 != 0 {
            return None;
        }
        let units: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        let s = String::from_utf16_lossy(&units);
        let s = s.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

fn open_key(root: HKEY, path: &str, flags: REG_SAM_FLAGS) -> Option<HKEY> {
    let path_w = wide(path);
    unsafe {
        let mut hkey = HKEY::default();
        let rc = RegOpenKeyExW(root, PCWSTR(path_w.as_ptr()), 0, flags, &mut hkey);
        if rc.0 == 0 { Some(hkey) } else { None }
    }
}

/// 枚举指定根键下的所有子键名
fn enum_subkeys(root: HKEY) -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let mut index = 0u32;
        loop {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let rc = RegEnumKeyExW(
                root,
                index,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
                None,
                PWSTR::null(),
                None,
                None,
            );
            if rc.0 != 0 {
                break;
            }
            index += 1;
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
    }
    names
}

fn detect_installed() -> ExistingInstall {
    let mut info = ExistingInstall::default();

    // 1) 本安装器写入的条目
    if let Some(hkey) = open_key(
        HKEY_LOCAL_MACHINE,
        UNINSTALL_KEY,
        KEY_READ | KEY_WOW64_64KEY,
    ) {
        info.version = reg_query_string(hkey, "DisplayVersion");
        info.location = reg_query_string(hkey, "InstallLocation");
        unsafe {
            let _ = RegCloseKey(hkey);
        }
    }

    // 2) 旧 MSI 版遗留：Uninstall 下任意子键中 DisplayName == DaDaView
    if let Some(root) = open_key(
        HKEY_LOCAL_MACHINE,
        UNINSTALL_ROOT,
        KEY_READ | KEY_WOW64_64KEY,
    ) {
        for sub in enum_subkeys(root) {
            if sub.eq_ignore_ascii_case("DaDaView") {
                continue; // 本安装器自己的条目，已处理
            }
            let Some(hk) = open_key(root, &sub, KEY_READ) else {
                continue;
            };
            let display = reg_query_string(hk, "DisplayName").unwrap_or_default();
            if display == APP_NAME {
                let uninstall_string = reg_query_string(hk, "UninstallString").unwrap_or_default();
                let is_msi = sub.starts_with('{')
                    || uninstall_string.to_ascii_lowercase().contains("msiexec");
                if is_msi {
                    info.msi_subkey = Some(sub.clone());
                    info.msi_version = reg_query_string(hk, "DisplayVersion");
                    if info.location.is_none() {
                        info.location = reg_query_string(hk, "InstallLocation");
                    }
                }
            }
            unsafe {
                let _ = RegCloseKey(hk);
            }
        }
        unsafe {
            let _ = RegCloseKey(root);
        }
    }

    // 3) 对应的 MSI 产品键（用于一并清理）
    if info.msi_subkey.is_some() {
        if let Some(root) = open_key(
            HKEY_LOCAL_MACHINE,
            INSTALLER_PRODUCTS,
            KEY_READ | KEY_WOW64_64KEY,
        ) {
            for sub in enum_subkeys(root) {
                let Some(hk) = open_key(root, &sub, KEY_READ) else {
                    continue;
                };
                let product_name = reg_query_string(hk, "ProductName").unwrap_or_default();
                unsafe {
                    let _ = RegCloseKey(hk);
                }
                if product_name == APP_NAME {
                    info.msi_product_key = Some(sub);
                    break;
                }
            }
            unsafe {
                let _ = RegCloseKey(root);
            }
        }
    }

    // 4) MSI 每用户安装属性：Installer\UserData\<SID>\Products\<packed>
    //    只删前两处的话 MSI 内部仍认为产品已安装，重装/修复时会出现幽灵记录
    if let Some(packed) = &info.msi_product_key {
        if let Some(root) = open_key(
            HKEY_LOCAL_MACHINE,
            INSTALLER_USERDATA,
            KEY_READ | KEY_WOW64_64KEY,
        ) {
            for sid in enum_subkeys(root) {
                let products = format!("{}\\Products", sid);
                let Some(pr) = open_key(root, &products, KEY_READ) else {
                    continue;
                };
                for sub in enum_subkeys(pr) {
                    if sub == *packed {
                        info.msi_userdata_paths.push(format!("{}\\{}", products, sub));
                    }
                }
                unsafe {
                    let _ = RegCloseKey(pr);
                }
            }
            unsafe {
                let _ = RegCloseKey(root);
            }
        }
    }

    info
}

/// 清掉旧 MSI 版在「应用和功能」里的注册项（只删注册表记录，不动任何已安装文件）。
fn remove_msi_leftover(info: &ExistingInstall) -> Result<(), String> {
    let mut first_err: Option<String> = None;
    unsafe {
        if let Some(sub) = &info.msi_subkey {
            let path = format!(r"{}\{}", UNINSTALL_ROOT, sub);
            let rc = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(wide(&path).as_ptr()));
            if rc.0 != 0 {
                first_err = Some(format!("删除 {} 失败（错误码 {}）", path, rc.0));
            }
        }
        if let Some(sub) = &info.msi_product_key {
            let path = format!(r"{}\{}", INSTALLER_PRODUCTS, sub);
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(wide(&path).as_ptr()));
        }
        for p in &info.msi_userdata_paths {
            let path = format!(r"{}\{}", INSTALLER_USERDATA, p);
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(wide(&path).as_ptr()));
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// 按点分数字段比较版本号（"0.1.10" > "0.1.9"）
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| c == '.' || c == '-' || c == '+')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (va, vb) = (parse(a), parse(b));
    let n = va.len().max(vb.len());
    for i in 0..n {
        let x = *va.get(i).unwrap_or(&0);
        let y = *vb.get(i).unwrap_or(&0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 已有版本 → 本次安装的性质
fn install_kind(existing: &str, new: &str) -> &'static str {
    match version_cmp(new, existing) {
        std::cmp::Ordering::Greater => "升级安装",
        std::cmp::Ordering::Equal => "修复安装（覆盖同版本）",
        std::cmp::Ordering::Less => "降级安装",
    }
}

// ---------- payload ----------

/// payload 解压后的总字节数（用于显示所需空间与进度百分比）。
fn payload_total_bytes() -> u64 {
    match zip::ZipArchive::new(std::io::Cursor::new(PAYLOAD_ZIP)) {
        Ok(mut archive) => {
            let mut total = 0u64;
            for i in 0..archive.len() {
                if let Ok(e) = archive.by_index(i) {
                    total += e.size();
                }
            }
            total
        }
        Err(_) => 0,
    }
}

fn extract_payload(dir: &Path, on_file: &dyn Fn(&str, u64)) -> Result<u64, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(PAYLOAD_ZIP))
        .map_err(|e| format!("安装包数据损坏：{}", e))?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("无法读取安装包条目：{}", e))?;
        let name = entry.name().to_string();
        let out_path = dir.join(&name);
        if !out_path.starts_with(dir) {
            return Err(format!("安装包含有非法路径：{}", name));
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("无法创建目录 {}：{}", parent.display(), e))?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("无法解压 {}：{}", name, e))?;
        std::fs::write(&out_path, &buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "无法写入 {}：{}。\n如果 DaDaView 正在运行，请先完全退出再重试。",
                    out_path.display(),
                    e
                )
            } else {
                format!("无法写入文件 {}：{}", out_path.display(), e)
            }
        })?;
        total += buf.len() as u64;
        on_file(&name, buf.len() as u64);
    }
    Ok(total)
}

fn start_menu_dir() -> PathBuf {
    let program_data = std::env::var_os("ProgramData")
        .unwrap_or_else(|| std::ffi::OsString::from(r"C:\ProgramData"));
    PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs\DaDaView")
}

fn desktop_lnk() -> PathBuf {
    let public = std::env::var_os("PUBLIC")
        .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Users\Public"));
    PathBuf::from(public).join(r"Desktop\DaDaView.lnk")
}

fn human_size(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", v as u64, UNITS[i])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// 真正的安装动作（在后台线程执行）。返回安装目录。
fn install_to(
    dir: &Path,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    msi_leftover: Option<ExistingInstall>,
    on_file: &dyn Fn(&str, u64),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建安装目录 {}：{}", dir.display(), e))?;

    let payload_bytes = extract_payload(dir, on_file)?;

    let exe = dir.join("dadaview.exe");

    if start_menu_shortcut {
        let sm_dir = start_menu_dir();
        std::fs::create_dir_all(&sm_dir).map_err(|e| format!("无法创建开始菜单目录：{}", e))?;
        create_shortcut(&sm_dir.join("DaDaView.lnk"), &exe, APP_DESC)
            .map_err(|e| format!("无法创建开始菜单快捷方式：{}", e))?;
    }

    if desktop_shortcut {
        create_shortcut(&desktop_lnk(), &exe, APP_DESC)
            .map_err(|e| format!("无法创建桌面快捷方式：{}", e))?;
    }

    write_arp_entries(dir, payload_bytes).map_err(|e| format!("无法写入注册表：{}", e))?;

    // 清理旧 MSI 遗留条目：只删注册表记录，不动已安装文件；失败不算致命
    if let Some(info) = msi_leftover {
        let _ = remove_msi_leftover(&info);
    }

    Ok(dir.to_path_buf())
}

fn launch_app(install_dir: &Path) {
    let exe = install_dir.join("dadaview.exe");
    let exe_w = wide(&exe.to_string_lossy());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: w!("open"),
        lpFile: PCWSTR(exe_w.as_ptr()),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let _ = unsafe { ShellExecuteExW(&mut info) };
}

// ============================================================================
// 向导界面
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Welcome,
    License,
    Dir,
    Progress,
    Done,
}

impl Page {
    fn subtitle(self) -> &'static str {
        match self {
            Page::Welcome => "欢迎使用 DaDaView 安装向导",
            Page::License => "许可协议",
            Page::Dir => "选择安装位置",
            Page::Progress => "正在安装 DaDaView",
            Page::Done => "安装完成",
        }
    }
}

struct Wizard {
    hwnd: HWND,
    page: Page,
    scale: f32,

    /// 安装前检测到的已有安装（用于升级 / 修复提示与默认目录）
    existing: ExistingInstall,

    back: HWND,
    next: HWND,
    cancel: HWND,

    header_title: HWND,
    header_sub: HWND,

    // 欢迎
    w_title: HWND,
    w_body: HWND,

    // 许可协议
    l_hint: HWND,
    l_edit: HWND,
    l_accept: HWND,

    // 安装目录
    d_hint: HWND,
    d_edit: HWND,
    d_browse: HWND,
    d_need: HWND,
    d_desktop: HWND,
    d_start: HWND,
    /// 仅当检测到旧 MSI 安装记录时创建
    d_cleansmi: HWND,

    // 进度
    p_hint: HWND,
    p_bar: HWND,
    p_file: HWND,
    p_pct: HWND,
    p_note: HWND,

    // 完成
    f_title: HWND,
    f_body: HWND,
    f_run: HWND,

    font_title: HFONT,
    font_body: HFONT,
    brush_white: HBRUSH,
    brush_line: HBRUSH,

    result_dir: PathBuf,
    finished: bool,
}

impl Wizard {
    fn new() -> Wizard {
        Wizard {
            hwnd: HWND::default(),
            page: Page::Welcome,
            scale: 1.0,
            existing: ExistingInstall::default(),
            back: HWND::default(),
            next: HWND::default(),
            cancel: HWND::default(),
            header_title: HWND::default(),
            header_sub: HWND::default(),
            w_title: HWND::default(),
            w_body: HWND::default(),
            l_hint: HWND::default(),
            l_edit: HWND::default(),
            l_accept: HWND::default(),
            d_hint: HWND::default(),
            d_edit: HWND::default(),
            d_browse: HWND::default(),
            d_need: HWND::default(),
            d_desktop: HWND::default(),
            d_start: HWND::default(),
            d_cleansmi: HWND::default(),
            p_hint: HWND::default(),
            p_bar: HWND::default(),
            p_file: HWND::default(),
            p_pct: HWND::default(),
            p_note: HWND::default(),
            f_title: HWND::default(),
            f_body: HWND::default(),
            f_run: HWND::default(),
            font_title: HFONT::default(),
            font_body: HFONT::default(),
            brush_white: HBRUSH::default(),
            brush_line: HBRUSH::default(),
            result_dir: PathBuf::new(),
            finished: false,
        }
    }

    fn px(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn dir_string(&self) -> String {
        let mut buf = [0u16; 1024];
        let n = unsafe { GetWindowTextW(self.d_edit, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn is_checked(&self, ctl: HWND) -> bool {
        unsafe { SendMessageW(ctl, BM_GETCHECK_U, WPARAM(0), LPARAM(0)).0 as usize == BST_CHECKED_U }
    }

    fn check(&self, ctl: HWND, checked: bool) {
        let v = if checked { BST_CHECKED_U } else { 0 };
        unsafe {
            SendMessageW(ctl, BM_SETCHECK_U, WPARAM(v), LPARAM(0));
        }
    }
}

// ---------- 控件创建 ----------

#[allow(clippy::too_many_arguments)]
fn make_ctl(
    class: PCWSTR,
    text: &str,
    style: u32,
    ex: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    id: isize,
) -> HWND {
    let t = wide(text);
    let menu = if id == 0 {
        HMENU::default()
    } else {
        HMENU(id as *mut c_void)
    };
    unsafe {
        CreateWindowExW(
            wes(ex),
            class,
            PCWSTR(t.as_ptr()),
            ws(style | WS_CHILD_U),
            x,
            y,
            w,
            h,
            parent,
            menu,
            current_hinstance(),
            None,
        )
    }
    .unwrap_or_default()
}

fn current_hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

fn set_font(ctl: HWND, font: HFONT) {
    if ctl.is_invalid() || font.is_invalid() {
        return;
    }
    unsafe {
        SendMessageW(ctl, WM_SETFONT_U, WPARAM(font.0 as usize), LPARAM(1));
    }
}

fn enable(ctl: HWND, on: bool) {
    if ctl.is_invalid() {
        return;
    }
    unsafe {
        let _ = EnableWindow(ctl, BOOL::from(on));
    }
}

fn set_text(ctl: HWND, text: &str) {
    if ctl.is_invalid() {
        return;
    }
    let t = wide(text);
    unsafe {
        let _ = SetWindowTextW(ctl, PCWSTR(t.as_ptr()));
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, None, BOOL::from(true));
    }
}

/// 创建字体；height 为逻辑像素（负值 = 按字符高度而非行高）。
fn make_font(px: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,         // DEFAULT_CHARSET
            0,         // OUT_DEFAULT_PRECIS
            0,         // CLIP_DEFAULT_PRECIS
            0,         // DEFAULT_QUALITY
            0,         // DEFAULT_PITCH | FF_DONTCARE
            w!("Microsoft YaHei UI"),
        )
    }
}

/// 欢迎页正文：有旧版本时插入升级/修复说明
fn welcome_text(ex: &ExistingInstall) -> String {
    let mut s = String::new();
    if let Some(v) = ex.display_version() {
        s.push_str(&format!(
            "检测到已安装的 DaDaView {}。\n本次将执行「{}」，程序文件会被替换，设置保持不变。\n",
            v,
            install_kind(v, APP_VERSION)
        ));
        if let Some(loc) = &ex.location {
            s.push_str(&format!("原安装位置：{}\n", loc));
        }
        s.push('\n');
    }
    if ex.msi_subkey.is_some() {
        s.push_str(
            "另检测到一条旧的 MSI（Windows Installer）安装记录。新安装器已不再使用 MSI，\
             这条残留会让「已安装的应用」里出现两个 DaDaView。\n\
             可在下一步勾选清理——只删除列表记录，不会删除任何文件。\n\n",
        );
    }
    s.push_str(
        "DaDaView 是一款轻量图片查看器，同时支持 3D 模型预览（glTF / GLB / OBJ 等常见格式）。\n\n\
         建议在安装前关闭正在运行的其它程序。\n\n\
         单击“下一步”继续，或单击“取消”退出安装程序。",
    );
    s
}

/// 许可协议页内容：中文要点导读在前，英文原文在后（法律效力以英文为准）
fn license_display() -> String {
    format!(
        "{}\n\n{}\n\n{}",
        LICENSE_ZH.replace("\r\n", "\n").trim_end(),
        "— — — — — — — — — 以下为具有法律效力的英文原文 — — — — — — — — —",
        LICENSE_TEXT.replace("\r\n", "\n").trim_end()
    )
}

unsafe fn build_controls(w: &mut Wizard) {
    let mut rc = RECT::default();
    let _ = GetClientRect(w.hwnd, &mut rc);
    let cw = rc.right - rc.left;
    let ch = rc.bottom - rc.top;
    let (mx, my) = (w.px(MARGIN), w.px(MARGIN));

    let content_x = mx;
    let content_w = cw - 2 * mx;
    let content_top = w.px(HEADER_H) + my;
    let content_bottom = ch - w.px(FOOTER_H);
    let content_h = content_bottom - content_top;

    // ---- 页头（直接画在窗口上）----
    w.header_title = make_ctl(
        w!("STATIC"),
        &format!("{} {}", APP_NAME, APP_VERSION),
        WS_VISIBLE_U,
        0,
        mx,
        w.px(12),
        content_w,
        w.px(24),
        w.hwnd,
        0,
    );
    w.header_sub = make_ctl(
        w!("STATIC"),
        Page::Welcome.subtitle(),
        WS_VISIBLE_U,
        0,
        mx + w.px(2),
        w.px(36),
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.header_title, w.font_title);
    set_font(w.header_sub, w.font_body);

    // ---- 欢迎页 ----
    w.w_title = make_ctl(
        w!("STATIC"),
        "欢迎使用 DaDaView",
        0,
        0,
        content_x,
        content_top,
        content_w,
        w.px(22),
        w.hwnd,
        0,
    );
    set_font(w.w_title, w.font_title);
    // 正文用只读多行 EDIT：STATIC 控件不解析换行，EDIT 才能按段落排版并支持滚动
    w.w_body = make_ctl(
        w!("EDIT"),
        &crlf(&welcome_text(&w.existing)),
        ES_MULTILINE_U | ES_READONLY_U | ES_AUTOVSCROLL_U,
        0,
        content_x,
        content_top + w.px(28),
        content_w,
        content_h - w.px(34),
        w.hwnd,
        0,
    );
    set_font(w.w_body, w.font_body);

    // ---- 许可协议页 ----
    w.l_hint = make_ctl(
        w!("STATIC"),
        "请仔细阅读以下许可协议。接受协议后才能继续安装。",
        0,
        0,
        content_x,
        content_top,
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.l_hint, w.font_body);

    let edit_h = content_h - w.px(30) - w.px(34);
    w.l_edit = make_ctl(
        w!("EDIT"),
        &crlf(&license_display()),
        ES_MULTILINE_U | ES_READONLY_U | ES_AUTOVSCROLL_U | WS_VSCROLL_U,
        WS_EX_CLIENTEDGE_U,
        content_x,
        content_top + w.px(26),
        content_w,
        edit_h,
        w.hwnd,
        0,
    );
    set_font(w.l_edit, w.font_body);

    w.l_accept = make_ctl(
        w!("BUTTON"),
        "我接受许可协议中的条款(&A)",
        BS_AUTOCHECKBOX_U | WS_TABSTOP_U,
        0,
        content_x,
        content_bottom - w.px(26),
        content_w,
        w.px(22),
        w.hwnd,
        ID_ACCEPT,
    );
    set_font(w.l_accept, w.font_body);

    // ---- 安装目录页 ----
    w.d_hint = make_ctl(
        w!("STATIC"),
        "请选择 DaDaView 的安装目录：",
        0,
        0,
        content_x,
        content_top,
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.d_hint, w.font_body);

    let browse_w = w.px(96);
    // 升级场景下预填原安装位置，避免装出两份
    let initial_dir = w
        .existing
        .location
        .clone()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(default_install_dir);
    w.d_edit = make_ctl(
        w!("EDIT"),
        &initial_dir.to_string_lossy(),
        ES_AUTOHSCROLL_U,
        WS_EX_CLIENTEDGE_U,
        content_x,
        content_top + w.px(26),
        content_w - browse_w - w.px(8),
        w.px(24),
        w.hwnd,
        0,
    );
    set_font(w.d_edit, w.font_body);

    w.d_browse = make_ctl(
        w!("BUTTON"),
        "浏览(&B)...",
        BS_PUSHBUTTON_U | WS_TABSTOP_U,
        0,
        content_x + content_w - browse_w,
        content_top + w.px(26),
        browse_w,
        w.px(26),
        w.hwnd,
        ID_BROWSE,
    );
    set_font(w.d_browse, w.font_body);

    w.d_need = make_ctl(
        w!("STATIC"),
        &format!("所需磁盘空间：{}", human_size(payload_total_bytes() as f64)),
        0,
        0,
        content_x,
        content_top + w.px(62),
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.d_need, w.font_body);

    w.d_desktop = make_ctl(
        w!("BUTTON"),
        "创建桌面快捷方式(&D)",
        BS_AUTOCHECKBOX_U | WS_TABSTOP_U,
        0,
        content_x,
        content_top + w.px(96),
        content_w,
        w.px(22),
        w.hwnd,
        ID_DESKTOP,
    );
    set_font(w.d_desktop, w.font_body);
    w.check(w.d_desktop, true);

    w.d_start = make_ctl(
        w!("BUTTON"),
        "创建开始菜单快捷方式(&S)",
        BS_AUTOCHECKBOX_U | WS_TABSTOP_U,
        0,
        content_x,
        content_top + w.px(122),
        content_w,
        w.px(22),
        w.hwnd,
        ID_STARTMENU,
    );
    set_font(w.d_start, w.font_body);
    w.check(w.d_start, true);

    // 旧 MSI 遗留记录：仅在检测到时才创建这个勾选项
    if w.existing.msi_subkey.is_some() {
        w.d_cleansmi = make_ctl(
            w!("BUTTON"),
            "清理旧的 MSI 安装记录（避免「已安装的应用」出现两个 DaDaView）",
            BS_AUTOCHECKBOX_U | WS_TABSTOP_U,
            0,
            content_x,
            content_top + w.px(148),
            content_w,
            w.px(22),
            w.hwnd,
            ID_CLEANSMI,
        );
        set_font(w.d_cleansmi, w.font_body);
        w.check(w.d_cleansmi, true);
    }

    // ---- 进度页 ----
    w.p_hint = make_ctl(
        w!("STATIC"),
        "正在安装 DaDaView，请稍候。",
        0,
        0,
        content_x,
        content_top,
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.p_hint, w.font_body);

    w.p_bar = make_ctl(
        w!("msctls_progress32"),
        "",
        0,
        0,
        content_x,
        content_top + w.px(30),
        content_w,
        w.px(22),
        w.hwnd,
        ID_PROGRESS,
    );
    unsafe {
        SendMessageW(w.p_bar, PBM_SETRANGE32_U, WPARAM(0), LPARAM(100));
        SendMessageW(w.p_bar, PBM_SETPOS_U, WPARAM(0), LPARAM(0));
    }

    w.p_file = make_ctl(
        w!("STATIC"),
        "",
        0,
        0,
        content_x,
        content_top + w.px(62),
        content_w - w.px(80),
        w.px(20),
        w.hwnd,
        ID_PFILE,
    );
    set_font(w.p_file, w.font_body);

    w.p_pct = make_ctl(
        w!("STATIC"),
        "0%",
        SS_RIGHT_U,
        0,
        content_x + content_w - w.px(72),
        content_top + w.px(62),
        w.px(72),
        w.px(20),
        w.hwnd,
        ID_PPCT,
    );
    set_font(w.p_pct, w.font_body);

    w.p_note = make_ctl(
        w!("STATIC"),
        "安装过程中请不要关闭本窗口。",
        0,
        0,
        content_x,
        content_top + w.px(92),
        content_w,
        w.px(20),
        w.hwnd,
        0,
    );
    set_font(w.p_note, w.font_body);

    // ---- 完成页 ----
    w.f_title = make_ctl(
        w!("STATIC"),
        "DaDaView 安装完成",
        0,
        0,
        content_x,
        content_top,
        content_w,
        w.px(24),
        w.hwnd,
        0,
    );
    set_font(w.f_title, w.font_title);

    w.f_body = make_ctl(
        w!("EDIT"),
        "",
        ES_MULTILINE_U | ES_READONLY_U | ES_AUTOVSCROLL_U,
        0,
        content_x,
        content_top + w.px(32),
        content_w,
        w.px(80),
        w.hwnd,
        0,
    );
    set_font(w.f_body, w.font_body);

    w.f_run = make_ctl(
        w!("BUTTON"),
        "运行 DaDaView(&R)",
        BS_AUTOCHECKBOX_U | WS_TABSTOP_U,
        0,
        content_x,
        content_top + w.px(126),
        content_w,
        w.px(22),
        w.hwnd,
        ID_RUN,
    );
    set_font(w.f_run, w.font_body);
    w.check(w.f_run, true);

    // ---- 底部按钮 ----
    let bw = w.px(BTN_W);
    let bh = w.px(BTN_H);
    let by = ch - my - bh;
    let cancel_x = cw - mx - bw;
    let next_x = cancel_x - w.px(8) - bw;
    let back_x = next_x - w.px(8) - bw;

    w.back = make_ctl(
        w!("BUTTON"),
        "上一步(&B)",
        BS_PUSHBUTTON_U | WS_TABSTOP_U | WS_VISIBLE_U,
        0,
        back_x,
        by,
        bw,
        bh,
        w.hwnd,
        ID_BACK,
    );
    w.next = make_ctl(
        w!("BUTTON"),
        "下一步(&N)",
        BS_DEFPUSHBUTTON_U | WS_TABSTOP_U | WS_VISIBLE_U,
        0,
        next_x,
        by,
        bw,
        bh,
        w.hwnd,
        ID_NEXT,
    );
    w.cancel = make_ctl(
        w!("BUTTON"),
        "取消",
        BS_PUSHBUTTON_U | WS_TABSTOP_U | WS_VISIBLE_U,
        0,
        cancel_x,
        by,
        bw,
        bh,
        w.hwnd,
        ID_CANCEL,
    );
    set_font(w.back, w.font_body);
    set_font(w.next, w.font_body);
    set_font(w.cancel, w.font_body);
}

// ---------- 页面切换 ----------

unsafe fn show(hwnd: HWND, visible: bool) {
    if hwnd.is_invalid() {
        return;
    }
    let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
}

unsafe fn set_page(w: &mut Wizard, page: Page) {
    w.page = page;
    set_text(w.header_sub, page.subtitle());

    show(w.w_title, page == Page::Welcome);
    show(w.w_body, page == Page::Welcome);

    show(w.l_hint, page == Page::License);
    show(w.l_edit, page == Page::License);
    show(w.l_accept, page == Page::License);

    show(w.d_hint, page == Page::Dir);
    show(w.d_edit, page == Page::Dir);
    show(w.d_browse, page == Page::Dir);
    show(w.d_need, page == Page::Dir);
    show(w.d_desktop, page == Page::Dir);
    show(w.d_start, page == Page::Dir);
    show(w.d_cleansmi, page == Page::Dir);

    show(w.p_hint, page == Page::Progress);
    show(w.p_bar, page == Page::Progress);
    show(w.p_file, page == Page::Progress);
    show(w.p_pct, page == Page::Progress);
    show(w.p_note, page == Page::Progress);

    show(w.f_title, page == Page::Done);
    show(w.f_body, page == Page::Done);
    show(w.f_run, page == Page::Done);

    match page {
        Page::Welcome => {
            enable(w.back, false);
            enable(w.next, true);
            enable(w.cancel, true);
            set_text(w.next, "下一步(&N)");
        }
        Page::License => {
            enable(w.back, true);
            enable(w.next, w.is_checked(w.l_accept));
            enable(w.cancel, true);
        }
        Page::Dir => {
            enable(w.back, true);
            enable(w.next, true);
            enable(w.cancel, true);
        }
        Page::Progress => {
            enable(w.back, false);
            enable(w.next, false);
            enable(w.cancel, false);
        }
        Page::Done => {
            enable(w.back, false);
            enable(w.next, true);
            enable(w.cancel, false);
            set_text(w.next, "完成(&F)");
            let _ = SetFocus(w.next);
        }
    }

    invalidate(w.hwnd);
}

unsafe fn paint(w: &Wizard) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(w.hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(w.hwnd, &mut rc);

    // 顶部白色页头
    let header = RECT {
        left: 0,
        top: 0,
        right: rc.right,
        bottom: w.px(HEADER_H),
    };
    let _ = FillRect(hdc, &header, w.brush_white);

    // 页头下沿分割线
    let line = RECT {
        left: 0,
        top: w.px(HEADER_H),
        right: rc.right,
        bottom: w.px(HEADER_H) + 1,
    };
    let _ = FillRect(hdc, &line, w.brush_line);

    // 底部分割线
    let footer_line = RECT {
        left: 0,
        top: rc.bottom - w.px(FOOTER_H),
        right: rc.right,
        bottom: rc.bottom - w.px(FOOTER_H) + 1,
    };
    let _ = FillRect(hdc, &footer_line, w.brush_line);

    let _ = EndPaint(w.hwnd, &ps);
}

unsafe fn pump_pending() {
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
        if msg.message == 0x0012 {
            // WM_QUIT：记录后重新投递，交还给主消息循环统一退出
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            PostQuitMessage(msg.wParam.0 as i32);
            return;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe fn pump_messages() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if QUIT_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

// ---------- 事件 ----------

impl Wizard {
    unsafe fn on_command(&mut self, wparam: WPARAM, _lparam: LPARAM) {
        let id = (wparam.0 & 0xffff) as isize;
        let code = (wparam.0 >> 16) & 0xffff;
        if code != BN_CLICKED_U {
            return; // 只处理点击通知，忽略焦点等其它通知
        }
        match id {
            ID_BACK => {
                let target = match self.page {
                    Page::License => Some(Page::Welcome),
                    Page::Dir => Some(Page::License),
                    _ => None,
                };
                if let Some(p) = target {
                    set_page(self, p);
                }
            }
            ID_NEXT => self.on_next(),
            ID_CANCEL => {
                if !INSTALLING.load(Ordering::SeqCst) {
                    let _ = DestroyWindow(self.hwnd);
                }
            }
            ID_BROWSE => self.on_browse(),
            ID_ACCEPT => {
                enable(self.next, self.is_checked(self.l_accept));
            }
            _ => {}
        }
    }

    unsafe fn on_next(&mut self) {
        match self.page {
            Page::Welcome => set_page(self, Page::License),
            Page::License => {
                if self.is_checked(self.l_accept) {
                    set_page(self, Page::Dir);
                }
            }
            Page::Dir => {
                let dir = PathBuf::from(self.dir_string());
                if dir.as_os_str().is_empty() {
                    message_box("请先选择安装目录。");
                    return;
                }
                // 安装期间会再次进入消息泵，改用裸指针避免 &mut self 与窗口过程里的
                // &mut Wizard 形成别名（窗口过程同样只经裸指针访问）。
                let ptr = self as *mut Wizard;
                run_install(ptr, dir);
            }
            Page::Done => {
                if self.is_checked(self.f_run) {
                    launch_app(&self.result_dir.clone());
                }
                let _ = DestroyWindow(self.hwnd);
            }
            Page::Progress => {}
        }
    }

    unsafe fn on_browse(&mut self) {
        let current = PathBuf::from(self.dir_string());
        let start = if current.as_os_str().is_empty() {
            default_install_dir()
        } else {
            current
        };
        if let Some(dir) = pick_install_dir(&start) {
            set_text(self.d_edit, &dir.to_string_lossy());
        }
    }

    unsafe fn refresh_progress(&self) {
        let done = PROG_DONE.load(Ordering::SeqCst);
        let total = PROG_TOTAL.load(Ordering::SeqCst).max(1);
        let pct = ((done as f64 / total as f64) * 100.0).round() as usize;
        let pct = pct.min(100);
        let _ = SendMessageW(self.p_bar, PBM_SETPOS_U, WPARAM(pct), LPARAM(0));
        set_text(self.p_pct, &format!("{}%", pct));

        let file = PROG_FILE.lock().map(|f| f.clone()).unwrap_or_default();
        let text = if file.is_empty() {
            "正在准备…".to_string()
        } else {
            format!("正在释放：{}", file)
        };
        set_text(self.p_file, &text);
    }

    unsafe fn refresh_done_text(&self) {
        let text = format!(
            "DaDaView {} 已成功安装到：\n{}\n\n您可以从开始菜单或桌面快捷方式启动程序。",
            APP_VERSION,
            self.result_dir.display()
        );
        set_text(self.f_body, &text);
    }
}

/// 切到进度页并在后台线程执行安装，期间保持消息泵运转以刷新界面。
///
/// 全程只经裸指针访问 Wizard：泵消息时窗口过程也会拿到同一指针，
/// 用引用会与这里的 &mut Wizard 形成别名。
unsafe fn run_install(w: *mut Wizard, dir: PathBuf) {
    let desktop = (*w).is_checked((*w).d_desktop);
    let start_menu = (*w).is_checked((*w).d_start);
    let clean_msi = (*w).existing.msi_subkey.is_some() && (*w).is_checked((*w).d_cleansmi);
    let leftover = if clean_msi {
        Some((*w).existing.clone())
    } else {
        None
    };

    PROG_DONE.store(0, Ordering::SeqCst);
    PROG_TOTAL.store(payload_total_bytes(), Ordering::SeqCst);
    if let Ok(mut f) = PROG_FILE.lock() {
        *f = String::new();
    }
    INSTALLING.store(true, Ordering::SeqCst);

    set_page(&mut *w, Page::Progress);

    let handle = std::thread::spawn(move || {
        install_to(&dir, desktop, start_menu, leftover, &|name: &str, n: u64| {
            PROG_DONE.fetch_add(n, Ordering::SeqCst);
            if let Ok(mut f) = PROG_FILE.lock() {
                *f = name.to_string();
            }
        })
    });

    // 安装期间的消息泵：一边处理界面消息，一边刷新进度
    loop {
        pump_pending();
        (*w).refresh_progress();
        if QUIT_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        if handle.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    let result = handle.join();
    INSTALLING.store(false, Ordering::SeqCst);

    match result {
        Ok(Ok(installed)) => {
            (*w).result_dir = installed;
            (*w).refresh_done_text();
            (*w).finished = true;
            if !QUIT_REQUESTED.load(Ordering::SeqCst) {
                set_page(&mut *w, Page::Done);
            }
        }
        Ok(Err(e)) => {
            message_box(&format!("安装失败：{}", e));
            set_page(&mut *w, Page::Dir);
        }
        Err(_) => {
            message_box("安装过程中发生意外错误。");
            set_page(&mut *w, Page::Dir);
        }
    }
}

fn message_box(msg: &str) {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            PCWSTR(wide(msg).as_ptr()),
            w!("DaDaView 安装程序"),
            windows::Win32::UI::WindowsAndMessaging::MB_OK
                | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
        );
    }
}

// ---------- 窗口过程 ----------

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let wiz = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Wizard;
    match msg {
        WM_COMMAND => {
            if let Some(w) = wiz.as_mut() {
                w.on_command(wparam, lparam);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            if let Some(w) = wiz.as_mut() {
                paint(w);
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            // 页头上的静态文本要画在白色背景上，其余交给系统默认处理
            let ctl = HWND(lparam.0 as *mut c_void);
            if let Some(w) = wiz.as_ref() {
                if (ctl == w.header_title || ctl == w.header_sub) && !w.brush_white.is_invalid() {
                    let hdc = HDC(wparam.0 as *mut c_void);
                    let _ = SetBkColor(hdc, COLORREF(0x00FFFFFF));
                    let _ = SetTextColor(hdc, COLORREF(0x00333333));
                    return LRESULT(w.brush_white.0 as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            if INSTALLING.load(Ordering::SeqCst) {
                return LRESULT(0); // 安装中不允许关闭
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn default_install_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles")
        .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Program Files"));
    PathBuf::from(pf).join("DaDaView")
}

fn run_wizard() -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let mut icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: INITCOMMONCONTROLSEX_ICC(ICC_PROGRESS_CLASS_U),
        };
        let _ = InitCommonControlsEx(&mut icc);

        let hinstance: HINSTANCE = match GetModuleHandleW(None) {
            Ok(h) => HINSTANCE(h.0),
            Err(_) => return 1,
        };

        let class_name = w!("DaDaViewSetupWindowClass");
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: load_icon(hinstance),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_3DFACE_U + 1) as *mut c_void),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };
        let _ = RegisterClassW(&wc);

        let scale = GetDpiForSystem() as f32 / 96.0;
        let mut wiz = Box::new(Wizard::new());
        wiz.scale = if scale.is_finite() && scale > 0.5 {
            scale
        } else {
            1.0
        };
        // 安装前先看看这台机器上装过什么（旧 exe 版 / 旧 MSI 版）
        wiz.existing = detect_installed();
        wiz.font_title = make_font(wiz.px(17), 700);
        wiz.font_body = make_font(wiz.px(12), 400);
        wiz.brush_white = CreateSolidBrush(COLORREF(0x00FFFFFF));
        wiz.brush_line = CreateSolidBrush(COLORREF(0x00D0D0D0));
        let ptr: *mut Wizard = &mut *wiz;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(wide("DaDaView 安装向导").as_ptr()),
            ws(WS_CAPTION_U | WS_SYSMENU_U | WS_MINIMIZEBOX_U),
            i32::MIN, // CW_USEDEFAULT
            i32::MIN,
            wiz.px(BASE_W),
            wiz.px(BASE_H),
            HWND::default(),
            HMENU::default(),
            hinstance,
            Some(ptr as *const c_void),
        )
        .unwrap_or_default();
        if hwnd.is_invalid() {
            return 1;
        }

        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        wiz.hwnd = hwnd;

        build_controls(&mut wiz);
        set_page(&mut wiz, Page::Welcome);

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        pump_messages();
        drop(wiz);
        0
    }
}

unsafe fn load_icon(hinstance: HINSTANCE) -> HICON {
    match LoadImageW(
        hinstance,
        intres(1),
        IMAGE_ICON_TYPE,
        32,
        32,
        IMAGE_FLAGS(0),
    ) {
        Ok(h) if !h.is_invalid() => HICON(h.0),
        _ => HICON::default(),
    }
}

// ============================================================================
// 入口
// ============================================================================

/// 静默安装（--quiet）：不经界面，沿用原安装位置（若有）或默认目录，并清理旧 MSI 记录。
fn install_quiet() -> i32 {
    let existing = detect_installed();
    let dir = existing
        .location
        .clone()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(default_install_dir);
    let leftover = if existing.msi_subkey.is_some() {
        Some(existing.clone())
    } else {
        None
    };
    match install_to(&dir, true, true, leftover, &|_name: &str, _n: u64| {}) {
        Ok(_) => 0,
        Err(e) => {
            message_box(&format!("安装失败：{}", e));
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a == "/quiet");

    if !is_elevated() {
        match relaunch_elevated(&args) {
            Some(code) => std::process::exit(code),
            None => fail("安装需要管理员权限。请重试，并在用户账户控制提示中选择“是”。"),
        }
    }

    let code = if quiet {
        install_quiet()
    } else {
        run_wizard()
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// payload zip 能完整解压，且 dadaview.exe 与源构建逐字节一致。
    #[test]
    fn payload_extracts_intact() {
        let dir =
            std::env::temp_dir().join(format!("dadaview-setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let total = extract_payload(&dir, &|_name: &str, _n: u64| {}).unwrap();
        assert!(total > 1_000_000, "payload 应超过 1MB，实际 {}", total);

        for name in [
            "dadaview.exe",
            "dadaview-uninstall.exe",
            "README.md",
            "LICENSE",
            "LICENSE.zh-CN.md",
        ] {
            assert!(dir.join(name).exists(), "缺少 {}", name);
        }

        let extracted = std::fs::read(dir.join("dadaview.exe")).unwrap();
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/release/dadaview.exe");
        let original = std::fs::read(source).unwrap();
        assert_eq!(extracted.len(), original.len(), "dadaview.exe 长度不一致");
        assert!(extracted == original, "dadaview.exe 与源构建不一致");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 许可协议文本已随安装包内嵌，且非空。
    #[test]
    fn license_embedded() {
        assert!(!LICENSE_TEXT.trim().is_empty(), "LICENSE 不应为空");
    }
}
