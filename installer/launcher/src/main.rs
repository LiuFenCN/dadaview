#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK, SW_SHOWNORMAL};

const MSI: &[u8] = include_bytes!("../../../dist/DaDaView-0.1.0-x64-setup.msi");

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn fail(msg: &str) -> ! {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(msg).as_ptr()),
            w!("DaDaView 安装程序"),
            MB_OK | MB_ICONERROR,
        );
    }
    std::process::exit(1);
}

fn main() {
    let quiet = std::env::args().any(|a| a == "--quiet" || a == "/quiet");
    let msi_path =
        std::env::temp_dir().join(format!("DaDaView-setup-{}.msi", std::process::id()));
    let written = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&msi_path)
        .and_then(|mut f| f.write_all(MSI));
    if written.is_err() {
        let _ = std::fs::remove_file(&msi_path);
        fail("无法释放安装包，请检查临时目录权限后重试。");
    }

    let params = format!(
        "/i \"{}\"{}",
        msi_path.display(),
        if quiet { " /qn" } else { "" }
    );
    let params_wide = wide(&params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: w!("msiexec.exe"),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    if unsafe { ShellExecuteExW(&mut info) }.is_err() {
        let _ = std::fs::remove_file(&msi_path);
        fail("安装需要管理员权限。请重试，并在用户账户控制提示中选择“是”。");
    }

    let mut code = 1u32;
    unsafe {
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
        }
    }
    let _ = std::fs::remove_file(&msi_path);
    std::process::exit(code as i32);
}
