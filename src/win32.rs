// Win32 窗口特效

#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::Graphics::Dwm::{
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::System::Threading::GetCurrentProcessId;
#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
};

/// 找到属于当前进程的第一个可见顶层窗口；多窗口同名时不会误伤其他实例
#[cfg(target_os = "windows")]
pub(crate) fn find_own_window() -> Option<HWND> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let slot = lparam.0 as *mut HWND;

            let mut pid = 0u32;

            GetWindowThreadProcessId(hwnd, Some(&mut pid));

            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() {
                *slot = hwnd;

                BOOL(0)
            } else {
                BOOL(1)
            }
        }
    }

    unsafe {
        let mut found = HWND::default();

        let _ = EnumWindows(Some(callback), LPARAM(&mut found as *mut HWND as isize));

        (!found.0.is_null()).then_some(found)
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn apply_win11_effects() {
    let Some(hwnd) = find_own_window() else {
        return;
    };

    unsafe {
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

#[cfg(not(target_os = "windows"))]
pub(crate) fn apply_win11_effects() {}
