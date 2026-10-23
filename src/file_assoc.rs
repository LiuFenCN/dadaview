// Windows 注册表文件关联

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

/// 我们的 ProgID（图片/模型查看器共用）
pub(crate) const PROG_ID: &str = "DaDaView.Image";

/// UserChoice 哈希里的"experience"字符串兜底值（与 shell32.dll 中一致）
const EXPERIENCE_FALLBACK: &str =
    "User Choice set via Windows User Experience {D18B6DD5-6124-4341-9318-804003BAFA0B}";

// ─── UserChoice 哈希（Windows 8+ 默认应用保护） ─────────────────────
//
// Windows 8 起，双击文件的默认打开方式不再看 HKCU\Software\Classes\.ext 的默认值，
// 而是看 FileExts\.ext\UserChoice（ProgId + Hash）。Hash 由以下要素计算：
//   Base64(MicrosoftHash(MD5(UTF16LE(ext + sid + progid + regdate + experience))))
// 其中 regdate 是 UserChoice 键的写入时间（FILETIME，分钟截断，16 位小写十六进制）。
// 算法与 SetUserFTA / PS-SFTA 使用的逆向实现一致，已在本机用现存 UserChoice
// 的 Hash 逐位复现验证（见 tests）。

/// MD5（RFC 1321），仅用于 UserChoice 哈希，不涉安全用途
fn md5_digest(msg: &[u8]) -> [u8; 16] {
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6,
        10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in data.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | (!b & d), i)
            } else if i < 32 {
                ((d & b) | (!d & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | !d), (7 * i) % 16)
            };
            let f = f
                .wrapping_add(a)
                .wrapping_add(K[i])
                .wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

/// 32 位逻辑右移（PS-SFTA 的 Get-ShiftRight：负数时先算术移再修高位）
#[inline]
fn shr_u32(v: i32, n: u32) -> i32 {
    ((v as u32) >> n) as i32
}

/// 小端读取 i32
#[inline]
fn get_long(bytes: &[u8], idx: usize) -> i32 {
    i32::from_le_bytes([bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3]])
}

/// 计算 UserChoice 哈希（Windows 内部 MicrosoftHash 的复刻）
///
/// `ext` 带点（如 ".png"）；`regdate` 为 16 位小写十六进制 FILETIME（分钟截断）。
fn user_choice_hash(ext: &str, sid: &str, prog_id: &str, regdate: &str, experience: &str) -> String {
    let base = format!("{ext}{sid}{prog_id}{regdate}{experience}").to_lowercase();

    // UTF-16LE + 2 个零字节
    let mut bytes: Vec<u8> = base
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    bytes.extend_from_slice(&[0, 0]);

    let md5 = md5_digest(&bytes);
    let md5_0 = get_long(&md5, 0);
    let md5_4 = get_long(&md5, 4);

    let length_base = bytes.len() as i32;
    let length = (((length_base & 4) <= 1) as i32) + shr_u32(length_base, 2) - 1;
    if length <= 1 {
        return String::new();
    }
    let iterations = (shr_u32(length - 2, 1) + 1) as u32;

    // ── 第一轮混合 ──
    let md51 = (md5_0 as i64 | 1) + 0x69FB0000;
    let md52 = (md5_4 as i64 | 1) + 0x13DB0000;
    let mut cache: i64 = 0;
    let mut out_hash1: i32 = 0;
    let mut out_hash2: i32 = 0;
    let mut pdata = 0usize;
    for _ in 0..iterations {
        let r0 = (get_long(&bytes, pdata) as i64 + out_hash1 as i64) as i32;
        let r1 = get_long(&bytes, pdata + 4);
        pdata += 8;
        let r2 = ((r0 as i64 * md51)
            .wrapping_sub(0x10FA9605i64 * shr_u32(r0, 16) as i64)) as i32;
        let r3 = ((0x79F8A395i64 * r2 as i64)
            .wrapping_add(0x689B6B9Fi64 * shr_u32(r2, 16) as i64)) as i32;
        let r4 = ((0xEA970001i64 * r3 as i64)
            .wrapping_sub(0x3C101569i64 * shr_u32(r3, 16) as i64)) as i32;
        let r5 = r4.wrapping_add(r1);
        let r6 = (cache + r4 as i64) as i32;
        let r7 = ((r5 as i64 * md52)
            .wrapping_sub(0x3CE8EC25i64 * shr_u32(r5, 16) as i64)) as i32;
        out_hash2 = ((0x59C3AF2Di64 * r7 as i64)
            .wrapping_sub(0x2232E0F1i64 * shr_u32(r7, 16) as i64)) as i32;
        out_hash1 = ((0x1EC90001i64 * out_hash2 as i64)
            .wrapping_add(0x35BD1EC9i64 * shr_u32(out_hash2, 16) as i64)) as i32;
        out_hash2 = (r6 as i64 + out_hash1 as i64) as i32;
        cache = out_hash2 as i64;
    }

    // ── 第二轮混合 ──
    let md51 = md5_0 as i64 | 1;
    let md52 = md5_4 as i64 | 1;
    let mut cache: i64 = 0;
    let mut out_hash1_b: i32 = 0;
    let mut out_hash2_b: i32 = 0;
    let mut pdata = 0usize;
    for _ in 0..iterations {
        let r0 = (get_long(&bytes, pdata) as i64 + out_hash1_b as i64) as i32;
        pdata += 8;
        let r1 = (r0 as i64).wrapping_mul(md51) as i32;
        let r2 = ((0xB1110000i64 * r1 as i64)
            .wrapping_sub(0x30674EEFi64 * shr_u32(r1, 16) as i64)) as i32;
        let r3 = ((0x5B9F0000i64 * r2 as i64)
            .wrapping_sub(0x78F7A461i64 * shr_u32(r2, 16) as i64)) as i32;
        let r4 = ((0x12CEB96Di64 * shr_u32(r3, 16) as i64)
            .wrapping_sub(0x46930000i64 * r3 as i64)) as i32;
        let r5 = ((0x1D830000i64 * r4 as i64)
            .wrapping_add(0x257E1D83i64 * shr_u32(r4, 16) as i64)) as i32;
        let r6 = (md52.wrapping_mul(r5 as i64 + get_long(&bytes, pdata - 4) as i64)) as i32;
        let r7 = ((0x16F50000i64 * r6 as i64)
            .wrapping_sub(0x5D8BE90Bi64 * shr_u32(r6, 16) as i64)) as i32;
        out_hash2_b = ((0x96FF0000i64 * r7 as i64)
            .wrapping_sub(0x2C7C6901i64 * shr_u32(r7, 16) as i64)) as i32;
        out_hash1_b = ((0x2B890000i64 * out_hash2_b as i64)
            .wrapping_add(0x7C932B89i64 * shr_u32(out_hash2_b, 16) as i64)) as i32;
        // OUTHASH1 被 0x9F690000 行覆盖，OUTHASH2 用覆盖后的新值
        let out_hash1_final = ((0x9F690000i64 * out_hash1_b as i64)
            .wrapping_sub(0x405B6097i64 * shr_u32(out_hash1_b, 16) as i64)) as i32;
        out_hash2_b = (out_hash1_final as i64 + cache + r5 as i64) as i32;
        cache = out_hash2_b as i64;
        out_hash1_b = out_hash1_final;
    }

    // 16 字节折叠为 8 字节后 Base64
    let hv1 = out_hash1_b ^ out_hash1;
    let hv2 = out_hash2_b ^ out_hash2;
    let mut out = [0u8; 8];
    out[0..4].copy_from_slice(&hv1.to_le_bytes());
    out[4..8].copy_from_slice(&hv2.to_le_bytes());
    base64_std(&out)
}

/// 标准 Base64（带 = 补齐），仅用于 8 字节以内的 UserChoice 哈希
fn base64_std(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            s.push(T[(n >> 6) as usize & 63] as char);
        } else {
            s.push('=');
        }
        if chunk.len() > 2 {
            s.push(T[n as usize & 63] as char);
        } else {
            s.push('=');
        }
    }
    s
}

/// 当前用户 SID（小写）。优先进程 Token，失败回退 whoami
fn current_user_sid() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn OpenProcessToken(
                process_handle: isize,
                desired_access: u32,
                token_handle: *mut isize,
            ) -> i32;
            fn GetTokenInformation(
                token_handle: isize,
                token_information_class: u32,
                token_information: *mut core::ffi::c_void,
                token_information_length: u32,
                return_length: *mut u32,
            ) -> i32;
            fn ConvertSidToStringSidW(
                sid: *const core::ffi::c_void,
                string_sid: *mut *mut u16,
            ) -> i32;
            fn LocalFree(hmem: isize) -> isize;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CloseHandle(hobject: isize) -> i32;
        }

        const TOKEN_QUERY: u32 = 0x0008;
        const TOKEN_USER: u32 = 1;

        // TOKEN_USER { SID_AND_ATTRIBUTES { PSID Sid; DWORD Attributes; } User; }
        unsafe {
            let mut token: isize = 0;
            if OpenProcessToken(-1, TOKEN_QUERY, &mut token) == 0 {
                return sid_from_whoami();
            }
            let mut buf = [0u8; 64];
            let mut ret_len: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TOKEN_USER,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                &mut ret_len,
            );
            CloseHandle(token);
            if ok == 0 {
                return sid_from_whoami();
            }
            let sid_ptr = *(buf.as_ptr() as *const *const core::ffi::c_void);
            if sid_ptr.is_null() {
                return sid_from_whoami();
            }
            let mut sid_str: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(sid_ptr, &mut sid_str) == 0 || sid_str.is_null() {
                return sid_from_whoami();
            }
            let mut len = 0usize;
            while *sid_str.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(sid_str, len));
            LocalFree(sid_str as isize);
            if s.is_empty() {
                return sid_from_whoami();
            }
            return Some(s.to_lowercase());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        sid_from_whoami()
    }
}

/// whoami /user 兜底取 SID
fn sid_from_whoami() -> Option<String> {
    let out = std::process::Command::new("whoami").arg("/user").output().ok()?;
    // whoami 输出 UTF-16LE
    let u16s: Vec<u16> = out
        .stdout
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let text = String::from_utf16_lossy(&u16s);
    text.split_whitespace()
        .find(|t| t.starts_with("S-1-"))
        .map(|t| t.to_lowercase())
}

/// 从 shell32.dll 提取 experience 字符串（不同 Windows 版本 GUID 可能变化），失败用兜底常量
fn user_experience_string() -> String {
    const SEARCH: &str = "User Choice set via Windows User Experience";
    if let Some(root) = std::env::var_os("SYSTEMROOT") {
        for sub in ["System32", "SysWOW64"] {
            let path = std::path::Path::new(&root).join(sub).join("Shell32.dll");
            if let Ok(data) = std::fs::read(&path) {
                let u16s: Vec<u16> = data
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let text = String::from_utf16_lossy(&u16s);
                if let Some(start) = text.find(SEARCH)
                    && let Some(end) = text[start..].find('}')
                {
                    return text[start..start + end + 1].to_string();
                }
            }
        }
    }
    EXPERIENCE_FALLBACK.to_string()
}

/// 当前时间 → FILETIME（1601 起 100ns 计数）
fn now_filetime() -> i64 {
    let now = chrono::Utc::now();
    let secs = now.timestamp() as i64;
    secs.saturating_mul(10_000_000)
        .saturating_add(11_644_473_600_i64 * 10_000_000)
        .saturating_add(now.timestamp_subsec_nanos() as i64 / 100)
}

/// FILETIME → 16 位小写十六进制，截断到分钟（与 Windows 计算 UserChoice 哈希时一致）
fn filetime_hex_minute(ft: i64) -> String {
    let ft = ft - ft.rem_euclid(600_000_000);
    format!("{:08x}{:08x}", (ft >> 32) as u32, ft as u32)
}

/// 读取注册表键的最后写入时间（FILETIME）
fn key_last_write_time(path: &str) -> Option<i64> {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegOpenKeyExW(
                hkey: isize,
                lpsubkey: *const u16,
                uloptions: u32,
                samdesired: u32,
                phkresult: *mut isize,
            ) -> i32;
            fn RegQueryInfoKeyW(
                hkey: isize,
                lpclass: *mut u16,
                lpcchclass: *mut u32,
                lpreserved: *const u32,
                lpcsubkeys: *mut u32,
                lpcbmaxsubkeylen: *mut u32,
                lpcbmaxclasslen: *mut u32,
                lpcvalues: *mut u32,
                lpcbmaxvaluenamelen: *mut u32,
                lpcbmaxvaluelen: *mut u32,
                lpcbsecuritydescriptor: *mut u32,
                lpftlastwritetime: *mut i64,
            ) -> i32;
            fn RegCloseKey(hkey: isize) -> i32;
        }
        const HKEY_CURRENT_USER_HANDLE: isize = 0x8000_0001;
        const KEY_QUERY_VALUE: u32 = 0x1;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let mut hkey: isize = 0;
            if RegOpenKeyExW(
                HKEY_CURRENT_USER_HANDLE,
                wide.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            ) != 0
            {
                return None;
            }
            let mut ft: i64 = 0;
            let rc = RegQueryInfoKeyW(
                hkey,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ft,
            );
            RegCloseKey(hkey);
            if rc != 0 {
                return None;
            }
            Some(ft)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

/// 写 UserChoice 所需的用户上下文（每次注册只取一次）
struct UserChoiceCtx {
    sid: String,
    experience: String,
}

impl UserChoiceCtx {
    fn fetch() -> Result<Self, String> {
        let sid = current_user_sid().ok_or("无法获取当前用户 SID")?;
        Ok(Self {
            sid,
            experience: user_experience_string(),
        })
    }
}

/// 为单个扩展名写入 UserChoice（ProgId + 合法 Hash），使其成为默认打开方式。
///
/// 要点：regdate 必须是键的真实写入时间（分钟截断）。写入 Hash 会刷新键时间戳，
/// 因此写完后读回校验，跨分钟边界时重试（最多 3 次）。
fn set_user_choice(ext: &str, ctx: &UserChoiceCtx, prog_id: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{ext}\\UserChoice"
    );

    for _ in 0..3 {
        // 删旧键再建：保证时间戳全新，且旧 ProgId 不会残留
        let _ = hkcu.delete_subkey(&path);
        let (key, _) = hkcu
            .create_subkey(&path)
            .map_err(|e| format!("创建 UserChoice 键失败: {e}"))?;
        key.set_value("ProgId", &prog_id)
            .map_err(|e| format!("写入 ProgId 失败: {e}"))?;

        let ft = key_last_write_time(&path).unwrap_or_else(now_filetime);
        let regdate = filetime_hex_minute(ft);
        let hash = user_choice_hash(
            &format!(".{ext}"),
            &ctx.sid,
            prog_id,
            &regdate,
            &ctx.experience,
        );
        key.set_value("Hash", &hash)
            .map_err(|e| format!("写入 Hash 失败: {e}"))?;

        // 校验：键当前时间的分钟须与 regdate 一致，否则 Hash 会被 Windows 判无效
        let ft2 = key_last_write_time(&path).unwrap_or_else(now_filetime);
        if filetime_hex_minute(ft2) == regdate {
            return Ok(());
        }
    }
    // 极端情况（连续 3 次跨分钟边界）：保留最后一次写入，静默接受
    Ok(())
}

// ─── 注册表文件关联 ───────────────────────────────────────

/// 每个扩展名一个专属 ProgID（`DaDaView.<ext>`），从而每种格式能有自己的
/// DefaultIcon（指向 exe 内嵌的对应文档图标资源）与 FriendlyTypeName。
fn per_ext_prog_id(ext: &str) -> String {
    format!("DaDaView.{}", ext)
}

/// 注册（或修正）单个扩展名的专属 ProgID：command 指向当前 exe、
/// DefaultIcon 指向该格式的文档图标资源、FriendlyTypeName 带格式名。
fn register_per_ext_progid(
    hkcu: &RegKey,
    ext: &str,
    exe_str: &str,
) -> Result<(), String> {
    let prog_id = per_ext_prog_id(ext);

    let (prog_key, _) = hkcu
        .create_subkey(format!("SOFTWARE\\Classes\\{}", prog_id))
        .map_err(|e| format!("创建 ProgID {} 失败: {}", prog_id, e))?;

    let friendly = format!(
        "{} 文件 (DaDaView)",
        crate::ext_icons::ext_category(ext).trim_end_matches("文件")
    );
    prog_key
        .set_value("", &friendly)
        .map_err(|e| e.to_string())?;
    prog_key
        .set_value("FriendlyTypeName", &friendly)
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
    // Explorer 图标序号：0 = 应用图标，1.. = 各格式文档图标（见 ext_icons.rs）
    let icon_index = crate::ext_icons::doc_icon_shell_index(ext).unwrap_or(0);
    icon_key
        .set_value("", &format!("\"{}\",{}", exe_str, icon_index))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// 注册文件关联到 Windows 注册表（仅 HKCU）
pub(crate) fn register_file_association(exts: &[String], set_default: bool) -> Result<(), String> {
    use winreg::enums::*;

    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let exe_path = std::env::current_exe().map_err(|e| format!("无法获取 exe 路径: {}", e))?;

    let exe_str = exe_path.to_str().ok_or("exe 路径包含非法字符")?;

    // 设为默认查看器需要 UserChoice 哈希，先取用户上下文（SID 等），失败则整体放弃
    let user_ctx = if set_default && !exts.is_empty() {
        Some(UserChoiceCtx::fetch()?)
    } else {
        None
    };

    // 通用 ProgID（DaDaView.Image）：保留——「打开方式」列表与旧关联的兼容项

    let prog_id = PROG_ID;

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

    // 旧版 PhotoView.Image 指向旧 exe，改写为当前程序，避免"打开方式"里残留失效项

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

    // UserChoice 失败收集（单个扩展名失败不影响其余）

    let mut uc_failures: Vec<String> = Vec::new();

    // 关联各扩展名

    for ext in exts {
        // 每个扩展名一个专属 ProgID，携带该格式自己的文档图标
        register_per_ext_progid(&hkcu, ext, exe_str)?;

        let ext_prog_id = per_ext_prog_id(ext);

        let ext_key_name = format!("SOFTWARE\\Classes\\.{}", ext);

        let (ext_key, _) = hkcu
            .create_subkey(&ext_key_name)
            .map_err(|e| format!("创建 .{} 键失败: {}", ext, e))?;

        // OpenWithProgids — 出现在"打开方式"列表（专属 + 通用都列上）

        let (open_with, _) = ext_key
            .create_subkey("OpenWithProgids")
            .map_err(|e| format!("创建 OpenWithProgids 失败: {}", e))?;

        open_with
            .set_value(&ext_prog_id, &"")
            .map_err(|e| e.to_string())?;
        open_with
            .set_value(prog_id, &"")
            .map_err(|e| e.to_string())?;

        if set_default {
            // 设置默认值 — 在 HKCU\SOFTWARE\Classes 下设置（指向专属 ProgID）

            ext_key.set_value("", &ext_prog_id).map_err(|e| e.to_string())?;

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
                    let _ = owp_key.set_value(&ext_prog_id, &0u32);
                    let _ = owp_key.set_value(prog_id, &0u32);
                }
            }

            // 关键：写 UserChoice（ProgId + 合法 Hash），否则 Windows 8+ 不认 Classes 默认值

            if let Some(ctx) = &user_ctx
                && let Err(e) = set_user_choice(ext, ctx, &ext_prog_id)
            {
                uc_failures.push(format!(".{}: {}", ext, e));
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

    if !uc_failures.is_empty() {
        return Err(format!(
            "部分格式的默认查看器设置失败: {}",
            uc_failures.join("；")
        ));
    }

    Ok(())
}

/// 启动时的静默迁移：把仍指向通用 ProgID（`DaDaView.Image`）的旧关联
/// 切换到每扩展名专属 ProgID，让各格式立刻获得自己的文档图标。
///
/// 只在「当前默认仍是我们旧的通用 ProgID」时才改写——用户后来手动改成了
/// 其它程序的关联则完全不动。已指向专属 ProgID 的也跳过（幂等）。
/// 每次启动都调用是安全的：注册 ProgID 键是幂等覆盖，UserChoice 仅在
/// 命中旧值时重写一次。
pub(crate) fn migrate_legacy_progids(exts: &[String]) {
    use winreg::enums::*;

    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let exe_str = match std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
    {
        Some(s) => s,
        None => return,
    };

    let mut changed = false;

    for ext in exts {
        // 1) 确保专属 ProgID 存在（幂等）
        if register_per_ext_progid(&hkcu, ext, &exe_str).is_err() {
            continue;
        }
        let ext_prog_id = per_ext_prog_id(ext);

        // 2) HKCU\.\ext 默认值若还是通用 ProgID → 换成专属
        if let Ok(k) = hkcu.open_subkey_with_flags(format!("SOFTWARE\\Classes\\.{}", ext), KEY_READ)
            && k.get_value::<String, _>("").ok().as_deref() == Some(PROG_ID)
        {
            if let Ok((k2, _)) = hkcu.create_subkey(format!("SOFTWARE\\Classes\\.{}", ext)) {
                let _ = k2.set_value("", &ext_prog_id);
                changed = true;
            }
        }

        // 3) UserChoice 若还是通用 ProgID → 用合法 Hash 重写为专属 ProgID
        let uc_path = format!(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{}\\UserChoice",
            ext
        );
        let current = hkcu
            .open_subkey_with_flags(&uc_path, KEY_READ)
            .and_then(|k| k.get_value::<String, _>("ProgId"));
        if current.ok().as_deref() == Some(PROG_ID)
            && let Ok(ctx) = UserChoiceCtx::fetch()
            && set_user_choice(ext, &ctx, &ext_prog_id).is_ok()
        {
            changed = true;
        }
    }

    if changed {
        // 通知 shell 刷新图标缓存
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实向量 1：本机 .png 的 UserChoice 现存值。
    /// 输入由 PowerShell 参照实现（PS-SFTA 算法）用键的真实时间戳复现，输出 SJJBRSamaCo=
    /// 与注册表中存储的 Hash 完全一致 —— 验证 Rust 移植逐位正确。
    #[test]
    fn user_choice_hash_matches_real_vector() {
        let hash = user_choice_hash(
            ".png",
            "s-1-5-21-2065622552-3875877111-564435868-1001",
            "PhotoView.Image",
            "01dd540fa1655c00",
            "User Choice set via Windows User Experience {D18B6DD5-6124-4341-9318-804003BAFA0B}",
        );
        assert_eq!(hash, "SJJBRSamaCo=");
    }

    /// 真实向量 2：DaDaView.Image + .obj + 另一个 regdate
    #[test]
    fn user_choice_hash_matches_second_vector() {
        let hash = user_choice_hash(
            ".obj",
            "s-1-5-21-2065622552-3875877111-564435868-1001",
            "DaDaView.Image",
            "01dd5e0780c236aa",
            "User Choice set via Windows User Experience {D18B6DD5-6124-4341-9318-804003BAFA0B}",
        );
        assert_eq!(hash, "Ue15TeWpaBQ=");
    }

    /// 输入过短时与参照实现一致返回空串
    #[test]
    fn user_choice_hash_tiny_input() {
        assert_eq!(user_choice_hash(".", "", "", "", ""), "");
    }

    #[test]
    fn base64_padding() {
        assert_eq!(base64_std(&[1, 2, 3, 4, 5]), "AQIDBAU=");
        assert_eq!(base64_std(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn md5_known_vectors() {
        assert_eq!(
            md5_digest(b""),
            [
                0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8,
                0x42, 0x7e
            ]
        );
        assert_eq!(
            md5_digest(b"abc"),
            [
                0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
                0x7f, 0x72
            ]
        );
    }

    #[test]
    fn filetime_hex_minute_truncates() {
        // 与 PowerShell 验证输出一致：.png 键的 FILETIME → 分钟截断
        assert_eq!(filetime_hex_minute(0x01dd540fbcbde70c), "01dd540fa1655c00");
        // 已对齐分钟的值不变
        assert_eq!(filetime_hex_minute(0x01dd540fa1655c00), "01dd540fa1655c00");
        assert_eq!(filetime_hex_minute(0x01dd5e0780c236aa), "01dd5e0780393e00");
    }

    /// 冒烟测试（会写注册表，默认不运行）：
    ///   cargo test -- --ignored set_user_choice_smoke
    /// 验证完整链路：写 UserChoice → 读回 → 用键时间戳重算哈希应自洽（Windows 的校验逻辑相同）
    #[test]
    #[ignore = "writes to HKCU registry"]
    fn set_user_choice_smoke() {
        let ctx = UserChoiceCtx::fetch().expect("用户上下文");
        let ext = "vtk";
        let prog_id = per_ext_prog_id(ext);
        set_user_choice(ext, &ctx, &prog_id).expect("set_user_choice");

        let path = format!(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{ext}\\UserChoice"
        );
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey(&path).expect("打开 UserChoice 键");
        let prog: String = key.get_value("ProgId").expect("读 ProgId");
        let hash: String = key.get_value("Hash").expect("读 Hash");
        assert_eq!(prog, prog_id);
        assert!(!hash.is_empty());

        let ft = key_last_write_time(&path).expect("读键时间戳");
        let expect = user_choice_hash(
            &format!(".{ext}"),
            &ctx.sid,
            &prog_id,
            &filetime_hex_minute(ft),
            &ctx.experience,
        );
        eprintln!("DEBUG recomputed_hash={expect}");
        assert_eq!(hash, expect, "Hash 与键时间戳不自洽，Windows 会判无效");
    }
}
