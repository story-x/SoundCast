use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_REG_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_RUN_NAME: &str = "SoundCast";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    #[default]
    Ask,
    HideToTray,
    ExitApp,
}

impl CloseAction {
    pub fn to_index(self) -> i32 {
        match self {
            CloseAction::Ask => 0,
            CloseAction::HideToTray => 1,
            CloseAction::ExitApp => 2,
        }
    }

    pub fn from_index(idx: i32) -> Self {
        match idx {
            1 => CloseAction::HideToTray,
            2 => CloseAction::ExitApp,
            _ => CloseAction::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn to_index(self) -> i32 {
        match self {
            ThemeMode::System => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
        }
    }

    pub fn from_index(idx: i32) -> Self {
        match idx {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 是否开启自动切换
    pub enabled: bool,
    /// 窗口移动防抖延迟（毫秒）
    pub debounce_ms: u64,
    /// 点击关闭按钮时的行为（每次询问 / 隐藏到托盘 / 退出应用）
    pub close_action: CloseAction,
    /// 设置界面主题模式（跟随系统 / 浅色 / 深色）
    pub theme_mode: ThemeMode,
    /// 显示器设备名 (如 \\.\DISPLAY1) -> 音频播放设备 ID
    pub monitor_audio_map: BTreeMap<String, String>,
    /// 显示器设备名 (如 \\.\DISPLAY1) -> 目标主音量百分比 (0..=100)
    pub monitor_volume_map: BTreeMap<String, u8>,
    /// 忽略的进程名（小写，不含路径）
    pub ignored_processes: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            debounce_ms: 150,
            close_action: CloseAction::Ask,
            theme_mode: ThemeMode::System,
            monitor_audio_map: BTreeMap::new(),
            monitor_volume_map: BTreeMap::new(),
            ignored_processes: Vec::new(),
        }
    }
}

impl AppConfig {
    /// 获取配置文件路径：如果 exe 同目录已存在 config.toml，则使用便携模式；否则存放在 %APPDATA%\SoundCast\config.toml
    pub fn config_path() -> PathBuf {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                let portable = dir.join("config.toml");
                if portable.exists() {
                    return portable;
                }
            }
        }

        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        appdata.join("SoundCast").join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&content) {
                return cfg;
            }
        }
        let default_cfg = Self::default();
        let _ = default_cfg.save();
        default_cfg
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, content).map_err(|e| e.to_string())
    }

    pub fn is_process_ignored(&self, exe_name_lower: &str) -> bool {
        self.ignored_processes
            .iter()
            .any(|p| p.eq_ignore_ascii_case(exe_name_lower))
    }
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 查询是否已开启开机自启动
pub fn is_auto_start_enabled() -> bool {
    unsafe {
        let subkey = to_wide_null(RUN_REG_PATH);
        let value_name = to_wide_null(APP_RUN_NAME);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return false;
        }

        let mut size: u32 = 0;
        let exists = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        )
        .is_ok();
        let _ = RegCloseKey(hkey);
        exists
    }
}

/// 设置或取消开机自启动
pub fn set_auto_start(enable: bool) -> Result<(), String> {
    unsafe {
        let subkey = to_wide_null(RUN_REG_PATH);
        let value_name = to_wide_null(APP_RUN_NAME);
        let mut hkey = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_WRITE,
            &mut hkey,
        )
        .ok()
        .map_err(|e| format!("无法打开注册表项: {e}"))?;

        let result = if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let quoted = format!("\"{}\"", exe.display());
            let data = to_wide_null(&quoted);
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            RegSetValueExW(
                hkey,
                PCWSTR(value_name.as_ptr()),
                0,
                REG_SZ,
                Some(bytes),
            )
            .ok()
            .map_err(|e| format!("写入开机自启失败: {e}"))
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()));
            Ok(())
        };

        let _ = RegCloseKey(hkey);
        result
    }
}

pub fn open_config_in_editor() {
    let path = AppConfig::config_path();
    if !path.exists() {
        let _ = AppConfig::default().save();
    }
    let _ = std::process::Command::new("notepad.exe")
        .arg(&path)
        .spawn();
}

#[allow(dead_code)]
fn _unused_pwstr(_: PWSTR) {}

