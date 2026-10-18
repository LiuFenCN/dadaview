#![windows_subsystem = "windows"]

// DaDaView 卸载器：由安装器复制到安装目录，「应用和功能」的 UninstallString 调用。
// 自身位于安装目录内，通过父目录推断安装位置；删除文件后以隐藏 cmd 延迟移除整个目录。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_LOCAL_MACHINE};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\DaDaView";

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn info(msg: &str) {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(msg).as_ptr()),
            w!("DaDaView 卸载程序"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn fail(msg: &str) -> ! {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(msg).as_ptr()),
            w!("DaDaView 卸载程序"),
            MB_OK | MB_ICONERROR,
        );
    }
    std::process::exit(1);
}

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

fn relaunch_elevated() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let exe_w = wide(&exe.to_string_lossy());
    let mut info = windows::Win32::UI::Shell::SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<windows::Win32::UI::Shell::SHELLEXECUTEINFOW>() as u32,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe_w.as_ptr()),
        ..Default::default()
    };
    unsafe { windows::Win32::UI::Shell::ShellExecuteExW(&mut info).is_ok() }
}

fn start_menu_dir() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| {
        PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\DaDaView")
    })
}

fn desktop_lnk() -> Option<PathBuf> {
    std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join(r"Desktop\DaDaView.lnk"))
}

fn main() {
    if !is_elevated() {
        if relaunch_elevated() {
            return;
        }
        fail("卸载需要管理员权限。请重试，并在用户账户控制提示中选择“是”。");
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => fail(&format!("无法定位卸载程序：{}", e)),
    };
    let install_dir = match exe.parent() {
        Some(p) => p.to_path_buf(),
        None => fail("无法确定安装目录。"),
    };

    // 1. 快捷方式
    if let Some(sm) = start_menu_dir() {
        let _ = std::fs::remove_dir_all(&sm);
    }
    if let Some(dl) = desktop_lnk() {
        let _ = std::fs::remove_file(&dl);
    }

    // 2. 注册表（「应用和功能」项）
    unsafe {
        let key_w = wide(UNINSTALL_KEY);
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()));
    }

    // 3. 延迟删除安装目录（等本进程退出后由隐藏 cmd 执行，以移除正在运行的卸载器自身）
    let dir_arg = format!("\"{}\"", install_dir.display());
    let spawned = std::process::Command::new("cmd")
        .args([
            "/C",
            "timeout",
            "/t",
            "2",
            "/nobreak",
            ">",
            "nul",
            "&",
            "rmdir",
            "/S",
            "/Q",
            &dir_arg,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if spawned.is_err() {
        fail(&format!(
            "文件已删除，但安装目录 {} 未能自动移除，请手动删除。",
            install_dir.display()
        ));
    }

    info("DaDaView 已卸载。");
}
