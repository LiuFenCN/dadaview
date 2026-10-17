// Windows 注册表文件关联

/// 注册文件关联到 Windows 注册表（仅 HKCU）
pub(crate) fn register_file_association(exts: &[String], set_default: bool) -> Result<(), String> {
    use winreg::enums::*;

    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let exe_path = std::env::current_exe().map_err(|e| format!("无法获取 exe 路径: {}", e))?;

    let exe_str = exe_path.to_str().ok_or("exe 路径包含非法字符")?;

    // 注册 ProgID

    let prog_id = "DaDaView.Image";

    let (prog_key, _) = hkcu
        .create_subkey(format!("SOFTWARE\\Classes\\{}", prog_id))
        .map_err(|e| format!("创建 ProgID 失败: {}", e))?;

    prog_key
        .set_value("", &"DaDaView 图片查看器")
        .map_err(|e| e.to_string())?;

    prog_key
        .set_value("FriendlyTypeName", &"DaDaView 图片查看器")
        .map_err(|e| e.to_string())?;

    let (shell_key, _) = prog_key
        .create_subkey("shell\\open\\command")
        .map_err(|e| format!("创建 command 失败: {}", e))?;

    shell_key
        .set_value("", &format!("\"{}\" \"%1\"", exe_str))
        .map_err(|e| e.to_string())?;

    let (icon_key, _) = prog_key
        .create_subkey("DefaultIcon")
        .map_err(|e| format!("创建 DefaultIcon 失败: {}", e))?;

    icon_key
        .set_value("", &format!("\"{}\",0", exe_str))
        .map_err(|e| e.to_string())?;

    // 旧版 PhotoView.Image 指向旧 exe，改写为当前程序，避免“打开方式”里残留失效项

    if let Ok(old_key) = hkcu.open_subkey("SOFTWARE\\Classes\\PhotoView.Image") {
        let _ = old_key.set_value("", &"DaDaView 图片查看器");

        if let Ok((cmd_key, _)) =
            hkcu.create_subkey("SOFTWARE\\Classes\\PhotoView.Image\\shell\\open\\command")
        {
            let _ = cmd_key.set_value("", &format!("\"{}\" \"%1\"", exe_str));
        }

        if let Ok((icon_key, _)) =
            hkcu.create_subkey("SOFTWARE\\Classes\\PhotoView.Image\\DefaultIcon")
        {
            let _ = icon_key.set_value("", &format!("\"{}\",0", exe_str));
        }
    }

    // 关联各扩展名

    for ext in exts {
        let ext_key_name = format!("SOFTWARE\\Classes\\.{}", ext);

        let (ext_key, _) = hkcu
            .create_subkey(&ext_key_name)
            .map_err(|e| format!("创建 .{} 键失败: {}", ext, e))?;

        // OpenWithProgids — 出现在"打开方式"列表

        let (open_with, _) = ext_key
            .create_subkey("OpenWithProgids")
            .map_err(|e| format!("创建 OpenWithProgids 失败: {}", e))?;

        open_with
            .set_value(prog_id, &"")
            .map_err(|e| e.to_string())?;

        if set_default {
            // 设置默认值 — 在 HKCU\SOFTWARE\Classes 下设置

            ext_key.set_value("", &prog_id).map_err(|e| e.to_string())?;

            // 同时写入 FileExts 以增强兼容性

            let file_exts_path = format!(
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{}",
                ext
            );

            if let Ok((fe_key, _)) = hkcu.create_subkey(&file_exts_path) {
                // 写入 ProgID 到 OpenWithList

                if let Ok((owl_key, _)) = fe_key.create_subkey("OpenWithList") {
                    let _ = owl_key.set_value("a", &"DaDaView.exe");

                    if let Ok(mrulist) = fe_key.open_subkey("OpenWithList") {
                        let _ = mrulist.set_value("MRUList", &"a");
                    }
                }

                // 写入 ProgID 到 OpenWithProgids

                if let Ok((owp_key, _)) = fe_key.create_subkey("OpenWithProgids") {
                    let _ = owp_key.set_value(prog_id, &0u32);
                }
            }

            // 修复旧版 ProgID（如 Photoview.png）的 command 路径和 DefaultIcon

            // 旧版安装可能注册了 Photoview.ext 格式的 ProgID，其 exe 路径可能已失效

            let old_prog_id = format!("Photoview.{}", ext);

            if let Ok(_old_key) = hkcu.open_subkey(format!("SOFTWARE\\Classes\\{}", old_prog_id)) {
                // 旧 ProgID 存在，更新 command 和 DefaultIcon 指向当前 exe

                if let Ok((fix_key, _)) = hkcu.create_subkey(format!(
                    "SOFTWARE\\Classes\\{}\\shell\\open\\command",
                    old_prog_id
                )) {
                    let _ = fix_key.set_value("", &format!("\"{}\" \"%1\"", exe_str));
                }

                if let Ok((icon_key, _)) =
                    hkcu.create_subkey(format!("SOFTWARE\\Classes\\{}\\DefaultIcon", old_prog_id))
                {
                    let _ = icon_key.set_value("", &format!("\"{}\",0", exe_str));
                }

                // 更新友好名称

                if let Ok(name_key) =
                    hkcu.open_subkey(format!("SOFTWARE\\Classes\\{}", old_prog_id))
                {
                    let _ = name_key.set_value("", &"DaDaView 图片查看器");

                    let _ = name_key.set_value("FriendlyTypeName", &"DaDaView 图片查看器");
                }
            }
        }
    }

    // 通知 shell 刷新

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::Shell::SHChangeNotify;

        use windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED;

        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            windows::Win32::UI::Shell::SHCNF_IDLIST,
            None,
            None,
        );
    }

    Ok(())
}
