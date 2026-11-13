use crate::audio::{enumerate_active_audio_pids, exe_file_name_lower, get_process_exe_path};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::HashMap;
use std::mem::size_of;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetIconInfo, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, HICON, ICONINFO,
};

#[derive(Debug, Clone)]
pub struct RunningAppInfo {
    pub exe_name: String,
    pub exe_path: String,
    pub display_name: String,
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn get_window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..copied as usize])
            .trim()
            .to_string()
    }
}

/// 枚举当前运行中的可见窗口应用与正在发声的音频进程（按进程名去重，附带路径与窗口标题）
pub fn enumerate_running_apps() -> Vec<RunningAppInfo> {
    let mut map: HashMap<String, RunningAppInfo> = HashMap::new();

    // 1. 活跃音频会话进程
    for pid in enumerate_active_audio_pids() {
        if let Some(path) = get_process_exe_path(pid) {
            let exe_name = exe_file_name_lower(&path);
            if exe_name.is_empty() || exe_name == "soundcast.exe" {
                continue;
            }
            let fallback_title = exe_name
                .strip_suffix(".exe")
                .unwrap_or(&exe_name)
                .to_string();
            map.entry(exe_name.clone()).or_insert(RunningAppInfo {
                exe_name,
                exe_path: path,
                display_name: fallback_title,
            });
        }
    }

    // 2. 桌面可见窗口进程（优先补充友好的窗口标题）
    unsafe extern "system" fn enum_win_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd).as_bool() {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0 {
                if let Some(path) = get_process_exe_path(pid) {
                    let exe_name = exe_file_name_lower(&path);
                    if !exe_name.is_empty() && exe_name != "soundcast.exe" {
                        let win_title = get_window_title(hwnd);
                        let map = &mut *(lparam.0 as *mut HashMap<String, RunningAppInfo>);
                        let fallback = exe_name
                            .strip_suffix(".exe")
                            .unwrap_or(&exe_name)
                            .to_string();

                        match map.get_mut(&exe_name) {
                            Some(existing) => {
                                if !win_title.is_empty()
                                    && (existing.display_name == fallback
                                        || win_title.chars().count()
                                            < existing.display_name.chars().count())
                                {
                                    existing.display_name = win_title;
                                }
                            }
                            None => {
                                let display_name = if win_title.is_empty() {
                                    fallback
                                } else {
                                    win_title
                                };
                                map.insert(
                                    exe_name.clone(),
                                    RunningAppInfo {
                                        exe_name,
                                        exe_path: path,
                                        display_name,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        BOOL(1)
    }

    unsafe {
        let ptr = &mut map as *mut HashMap<String, RunningAppInfo>;
        let _ = EnumWindows(Some(enum_win_proc), LPARAM(ptr as isize));
    }

    let mut list: Vec<RunningAppInfo> = map.into_values().collect();
    list.sort_by(|a, b| a.exe_name.cmp(&b.exe_name));
    list
}

/// 从可执行文件路径提取图标并转为 Slint Image
pub fn load_exe_icon(exe_path: &str) -> Option<Image> {
    unsafe {
        let wide = to_wide_null(exe_path);
        let mut shfi = SHFILEINFOW::default();
        let res = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut shfi),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if res == 0 || shfi.hIcon.0.is_null() {
            return None;
        }
        let img = hicon_to_slint_image(shfi.hIcon);
        let _ = DestroyIcon(shfi.hIcon);
        img
    }
}

unsafe fn hicon_to_slint_image(hicon: HICON) -> Option<Image> {
    let mut icon_info = ICONINFO::default();
    if GetIconInfo(hicon, &mut icon_info).is_err() {
        return None;
    }

    let result = extract_bitmap_rgba(icon_info.hbmColor, icon_info.hbmMask);

    if !icon_info.hbmColor.0.is_null() {
        let _ = DeleteObject(icon_info.hbmColor);
    }
    if !icon_info.hbmMask.0.is_null() {
        let _ = DeleteObject(icon_info.hbmMask);
    }
    result
}

unsafe fn extract_bitmap_rgba(hbm_color: HBITMAP, hbm_mask: HBITMAP) -> Option<Image> {
    if hbm_color.0.is_null() {
        return None;
    }

    let mut bmp = BITMAP::default();
    if GetObjectW(
        hbm_color,
        size_of::<BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    ) == 0
    {
        return None;
    }

    let width = bmp.bmWidth.max(1) as u32;
    let height = bmp.bmHeight.max(1) as u32;
    let pixel_count = (width * height) as usize;

    let hdc = CreateCompatibleDC(None);
    if hdc.0.is_null() {
        return None;
    }

    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bgra = vec![0u8; pixel_count * 4];
    let lines = GetDIBits(
        hdc,
        hbm_color,
        0,
        height,
        Some(bgra.as_mut_ptr() as *mut _),
        &mut bi,
        DIB_RGB_COLORS,
    );

    if lines == 0 {
        let _ = DeleteDC(hdc);
        return None;
    }

    let has_alpha = bgra.chunks_exact(4).any(|px| px[3] != 0);
    let mut mask_bgra = Vec::new();
    if !has_alpha && !hbm_mask.0.is_null() {
        mask_bgra.resize(pixel_count * 4, 0);
        let _ = GetDIBits(
            hdc,
            hbm_mask,
            0,
            height,
            Some(mask_bgra.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
    }

    let _ = DeleteDC(hdc);

    let mut rgba = vec![0u8; pixel_count * 4];
    for i in 0..pixel_count {
        let b = bgra[i * 4];
        let g = bgra[i * 4 + 1];
        let r = bgra[i * 4 + 2];
        let mut a = bgra[i * 4 + 3];
        if !has_alpha && !mask_bgra.is_empty() {
            // 掩码为白色(255)表示透明，黑色(0)表示不透明
            a = if mask_bgra[i * 4] > 128 { 0 } else { 255 };
        }
        rgba[i * 4] = r;
        rgba[i * 4 + 1] = g;
        rgba[i * 4 + 2] = b;
        rgba[i * 4 + 3] = a;
    }

    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, width, height);
    Some(Image::from_rgba8(buffer))
}

/// 生成简洁的默认占位应用图标（当无法提取进程图标时使用）
pub fn default_app_icon() -> Image {
    let size = 24usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let dx = (fx - 12.0).abs() - 7.0;
            let dy = (fy - 12.0).abs() - 7.0;
            let dist = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt()
                + dx.max(dy).min(0.0)
                - 3.5;
            if dist <= 0.0 {
                rgba[idx] = 88;
                rgba[idx + 1] = 101;
                rgba[idx + 2] = 122;
                rgba[idx + 3] = 255;

                // 中间小圆点作为装饰
                let r = ((fx - 12.0).powi(2) + (fy - 12.0).powi(2)).sqrt();
                if r <= 3.0 {
                    rgba[idx] = 220;
                    rgba[idx + 1] = 228;
                    rgba[idx + 2] = 240;
                }
            }
        }
    }
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, size as u32, size as u32);
    Image::from_rgba8(buffer)
}

