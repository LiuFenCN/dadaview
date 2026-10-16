// Win32 窗口特效

#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::Graphics::Dwm::{
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::core::PCWSTR;

#[cfg(target_os = "windows")]
pub(crate) fn apply_win11_effects() {
    unsafe {
        let title: Vec<u16> = "PhotoView\0".encode_utf16().collect();

        if let Ok(hwnd) = FindWindowW(None, PCWSTR(title.as_ptr())) {
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
}

#[cfg(not(target_os = "windows"))]

pub(crate) fn apply_win11_effects() {}
