use crate::audio::AudioDevice;
use crate::config::{is_auto_start_enabled, AppConfig};
use crate::monitor::MonitorInfo;
use muda::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use std::collections::HashMap;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone)]
pub enum TrayAction {
    OpenSettings,
    ToggleEnabled,
    BindMonitor {
        monitor_device: String,
        audio_device_id: Option<String>,
    },
    ToggleAutoStart,
    OpenConfig,
    Refresh,
    Quit,
}

pub struct TrayController {
    pub tray: TrayIcon,
    pub actions: HashMap<MenuId, TrayAction>,
}

impl TrayController {
    pub fn new(
        cfg: &AppConfig,
        monitors: &[MonitorInfo],
        devices: &[AudioDevice],
    ) -> Result<Self, String> {
        let icon = build_tray_icon()?;
        let (menu, actions) = build_menu(cfg, monitors, devices);
        let tooltip = if cfg.enabled {
            "SoundCast - 自动按显示器切换应用音频 (运行中)"
        } else {
            "SoundCast - 已暂停"
        };

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip)
            .with_icon(icon)
            .build()
            .map_err(|e| format!("创建系统托盘失败: {e}"))?;

        Ok(Self { tray, actions })
    }

    pub fn rebuild_menu(
        &mut self,
        cfg: &AppConfig,
        monitors: &[MonitorInfo],
        devices: &[AudioDevice],
    ) {
        let (menu, actions) = build_menu(cfg, monitors, devices);
        self.actions = actions;
        self.tray.set_menu(Some(Box::new(menu)));
        let tooltip = if cfg.enabled {
            "SoundCast - 自动按显示器切换应用音频 (运行中)"
        } else {
            "SoundCast - 已暂停"
        };
        let _ = self.tray.set_tooltip(Some(tooltip));
    }
}

fn build_menu(
    cfg: &AppConfig,
    monitors: &[MonitorInfo],
    devices: &[AudioDevice],
) -> (Menu, HashMap<MenuId, TrayAction>) {
    let menu = Menu::new();
    let mut actions = HashMap::new();

    let title_item = MenuItem::new(
        if cfg.enabled {
            "SoundCast · 运行中"
        } else {
            "SoundCast · 已暂停"
        },
        false,
        None,
    );
    let _ = menu.append(&title_item);

    let open_settings = MenuItem::new("打开设置界面...", true, None);
    actions.insert(open_settings.id().clone(), TrayAction::OpenSettings);
    let _ = menu.append(&open_settings);

    let toggle_enabled = CheckMenuItem::new("启用自动切换", true, cfg.enabled, None);
    actions.insert(toggle_enabled.id().clone(), TrayAction::ToggleEnabled);
    let _ = menu.append(&toggle_enabled);

    let _ = menu.append(&PredefinedMenuItem::separator());

    if monitors.is_empty() {
        let empty = MenuItem::new("未检测到显示器", false, None);
        let _ = menu.append(&empty);
    } else {
        for mon in monitors {
            let bound_id = cfg.monitor_audio_map.get(&mon.device_name);
            let bound_name = bound_id.and_then(|id| {
                devices
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|d| d.name.as_str())
            });
            let submenu_title = match bound_name {
                Some(name) => format!("{}  ->  {}", mon.display_label, shorten(name, 24)),
                None => format!("{}  (未绑定)", mon.display_label),
            };
            let submenu = Submenu::new(submenu_title, true);

            let none_item = CheckMenuItem::new(
                "不自动切换（保持默认）",
                true,
                bound_id.is_none(),
                None,
            );
            actions.insert(
                none_item.id().clone(),
                TrayAction::BindMonitor {
                    monitor_device: mon.device_name.clone(),
                    audio_device_id: None,
                },
            );
            let _ = submenu.append(&none_item);
            let _ = submenu.append(&PredefinedMenuItem::separator());

            for dev in devices {
                let checked = bound_id.map(|id| id == &dev.id).unwrap_or(false);
                let label = if dev.is_default {
                    format!("{} [系统默认]", dev.name)
                } else {
                    dev.name.clone()
                };
                let dev_item = CheckMenuItem::new(label, true, checked, None);
                actions.insert(
                    dev_item.id().clone(),
                    TrayAction::BindMonitor {
                        monitor_device: mon.device_name.clone(),
                        audio_device_id: Some(dev.id.clone()),
                    },
                );
                let _ = submenu.append(&dev_item);
            }

            let _ = menu.append(&submenu);
        }
    }

    let _ = menu.append(&PredefinedMenuItem::separator());

    let auto_start = CheckMenuItem::new("开机自启动", true, is_auto_start_enabled(), None);
    actions.insert(auto_start.id().clone(), TrayAction::ToggleAutoStart);
    let _ = menu.append(&auto_start);

    let open_cfg = MenuItem::new("打开配置文件 (config.toml)", true, None);
    actions.insert(open_cfg.id().clone(), TrayAction::OpenConfig);
    let _ = menu.append(&open_cfg);

    let refresh = MenuItem::new("刷新显示器与音频设备", true, None);
    actions.insert(refresh.id().clone(), TrayAction::Refresh);
    let _ = menu.append(&refresh);

    let _ = menu.append(&PredefinedMenuItem::separator());

    let quit = MenuItem::new("退出 SoundCast", true, None);
    actions.insert(quit.id().clone(), TrayAction::Quit);
    let _ = menu.append(&quit);

    (menu, actions)
}

fn shorten(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}...")
    } else {
        collected
    }
}

/// 加载内置的 32x32 RGBA 专属托盘图标
fn build_tray_icon() -> Result<Icon, String> {
    let rgba = include_bytes!("../assets/tray_32.rgba").to_vec();
    Icon::from_rgba(rgba, 32, 32).map_err(|e| format!("生成图标失败: {e}"))
}

