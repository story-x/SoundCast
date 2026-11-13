use std::mem::size_of;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, DISPLAY_DEVICEW,
    HDC, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};

const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x00000001;
const MONITORINFOF_PRIMARY: u32 = 0x00000001;

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// 系统设备名，例如 \\.\DISPLAY1
    pub device_name: String,
    /// 短名称，例如 DISPLAY1
    pub short_name: String,
    /// 型号与分辨率描述，例如 DELL U2720Q · 2560x1440
    pub detail: String,
    /// 友好显示名称，例如 "DISPLAY1 (主屏, 2560x1440)"
    pub display_label: String,
    pub is_primary: bool,
}

fn wide_slice_to_string(slice: &[u16]) -> String {
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..end])
}

/// 尝试获取显示器友好名称（显卡驱动/EDID 提供的名称）
fn get_monitor_friendly_name(adapter_name: &[u16; 32]) -> Option<String> {
    unsafe {
        let mut dd = DISPLAY_DEVICEW {
            cb: size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        if EnumDisplayDevicesW(
            PCWSTR(adapter_name.as_ptr()),
            0,
            &mut dd,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
        .as_bool()
        {
            let name = wide_slice_to_string(&dd.DeviceString);
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// 枚举当前所有连接且活跃的显示器
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let ptr = &mut monitors as *mut Vec<MonitorInfo>;
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_monitor_proc),
            LPARAM(ptr as isize),
        );
    }
    // 主屏排在最前面，其余按设备名排序
    monitors.sort_by(|a, b| {
        b.is_primary
            .cmp(&a.is_primary)
            .then_with(|| a.device_name.cmp(&b.device_name))
    });
    monitors
}

unsafe extern "system" fn enum_monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _lprect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmonitor, &mut info.monitorInfo as *mut _).as_bool() {
        let device_name = wide_slice_to_string(&info.szDevice);
        let is_primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;
        let rc = info.monitorInfo.rcMonitor;
        let width = rc.right - rc.left;
        let height = rc.bottom - rc.top;

        let short_id = device_name
            .strip_prefix("\\\\.\\")
            .unwrap_or(&device_name)
            .to_string();
        let model = get_monitor_friendly_name(&info.szDevice)
            .filter(|m| m != "Generic PnP Monitor" && m != "通用即插即用监视器");

        let mut tags = Vec::new();
        if is_primary {
            tags.push("主屏".to_string());
        }
        tags.push(format!("{width}x{height}"));

        let detail = match &model {
            Some(m) => format!("{m} · {width}x{height}"),
            None => format!("标准显示器 · {width}x{height}"),
        };

        let display_label = match model {
            Some(m) => format!("{short_id} - {m} ({})", tags.join(", ")),
            None => format!("{short_id} ({})", tags.join(", ")),
        };

        monitors.push(MonitorInfo {
            device_name,
            short_name: short_id,
            detail,
            display_label,
            is_primary,
        });
    }
    BOOL(1)
}

/// 获取指定窗口当前所在显示器的 device_name（如 \\.\DISPLAY1）
pub fn monitor_for_window(hwnd: HWND) -> Option<String> {
    unsafe {
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if hmonitor.0.is_null() {
            return None;
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info.monitorInfo as *mut _).as_bool() {
            let name = wide_slice_to_string(&info.szDevice);
            if !name.is_empty() {
                return Some(name);
            }
        }
        None
    }
}

