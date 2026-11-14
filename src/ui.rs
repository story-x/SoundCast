use crate::app_icon::{default_app_icon, enumerate_running_apps, load_exe_icon, RunningAppInfo};
use crate::audio::{enumerate_playback_devices, get_device_volume, set_device_volume, AudioDevice};
use crate::config::{is_auto_start_enabled, set_auto_start, AppConfig, CloseAction, ThemeMode};
use crate::monitor::{enumerate_monitors, MonitorInfo};
use crate::switch_log::{clear_switch_logs, log_signature, open_log_in_editor, read_switch_logs};
use slint::winit_030::{
    winit::{
        dpi::PhysicalPosition,
        event::WindowEvent,
        window::{Theme as WinitTheme, Window as WinitWindow},
    },
    EventResult, WinitWindowAccessor,
};
use slint::{
    CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode,
    VecModel,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, PostMessageW, HWND_MESSAGE, SW_SHOWNORMAL,
};

slint::include_modules!();

struct SharedUiData {
    config: AppConfig,
    saved_config: AppConfig,
    saved_auto_start: bool,
    monitors: Vec<MonitorInfo>,
    devices: Vec<AudioDevice>,
    device_ids: Vec<Option<String>>,
    selected_indices: Vec<i32>,
    ignored_list: Vec<String>,
    all_running_apps: Vec<RunningAppInfo>,
    available_apps: Vec<RunningAppInfo>,
    selected_app_index: usize,
    icon_cache: HashMap<String, Image>,
    fallback_icon: Image,
}

impl SharedUiData {
    fn get_icon_for_path(&mut self, exe_path: &str) -> Image {
        if exe_path.is_empty() {
            return self.fallback_icon.clone();
        }
        if let Some(cached) = self.icon_cache.get(exe_path) {
            return cached.clone();
        }
        let img = load_exe_icon(exe_path).unwrap_or_else(|| self.fallback_icon.clone());
        self.icon_cache.insert(exe_path.to_string(), img.clone());
        img
    }

    fn resolve_system_exe_path(exe_name: &str) -> Option<String> {
        let windir = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        let candidates = [
            windir.join(exe_name),
            windir.join("System32").join(exe_name),
            windir
                .join("SystemApps")
                .join("Microsoft.Windows.Search_cw5n1h2txyewy")
                .join(exe_name),
        ];
        for p in candidates {
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
        None
    }

    fn resolve_app_display(&mut self, exe_name: &str, exe_path: &str) -> AppOptionData {
        if let Some(running) = self
            .all_running_apps
            .iter()
            .find(|a| a.exe_name.eq_ignore_ascii_case(exe_name))
            .cloned()
        {
            let icon = if !exe_path.is_empty() {
                self.get_icon_for_path(exe_path)
            } else {
                self.get_icon_for_path(&running.exe_path)
            };
            return AppOptionData {
                exe_name: SharedString::from(running.exe_name),
                display_name: SharedString::from(running.display_name),
                icon,
            };
        }

        let resolved_path = if !exe_path.is_empty() {
            exe_path.to_string()
        } else {
            Self::resolve_system_exe_path(exe_name).unwrap_or_default()
        };
        let icon = self.get_icon_for_path(&resolved_path);
        AppOptionData {
            exe_name: SharedString::from(exe_name),
            display_name: SharedString::from(exe_name),
            icon,
        }
    }

    fn build_app_option(&mut self, exe_name: &str) -> AppOptionData {
        self.resolve_app_display(exe_name, "")
    }
}

fn build_models(
    cfg: &AppConfig,
    monitors: &[MonitorInfo],
    devices: &[AudioDevice],
) -> (
    Vec<SharedString>,
    Vec<Option<String>>,
    Vec<MonitorRowData>,
    Vec<i32>,
) {
    let mut device_names = vec![SharedString::from("不自动切换（保持系统默认）")];
    let mut device_ids: Vec<Option<String>> = vec![None];

    for dev in devices {
        let label = if dev.is_default {
            format!("{} [系统默认]", dev.name)
        } else {
            dev.name.clone()
        };
        device_names.push(SharedString::from(label));
        device_ids.push(Some(dev.id.clone()));
    }

    let mut rows = Vec::new();
    let mut selected_indices = Vec::new();

    for mon in monitors {
        let bound_id = cfg.monitor_audio_map.get(&mon.device_name);
        let mut sel: i32 = 0;
        let mut resolved_dev_id: Option<&str> = None;
        if let Some(target_id) = bound_id {
            if let Some(pos) = device_ids
                .iter()
                .position(|id| id.as_deref() == Some(target_id.as_str()))
            {
                sel = pos as i32;
                resolved_dev_id = Some(target_id.as_str());
            }
        }
        let configured_vol = cfg.monitor_volume_map.get(&mon.device_name).copied();
        let volume_enabled = sel > 0 && configured_vol.is_some();
        let volume_percent = configured_vol
            .or_else(|| resolved_dev_id.and_then(get_device_volume))
            .unwrap_or(50)
            .min(100) as i32;

        selected_indices.push(sel);
        rows.push(MonitorRowData {
            device_name: SharedString::from(mon.device_name.as_str()),
            short_name: SharedString::from(mon.short_name.as_str()),
            detail: SharedString::from(mon.detail.as_str()),
            is_primary: mon.is_primary,
            selected_index: sel,
            volume_enabled,
            volume_percent,
        });
    }

    (device_names, device_ids, rows, selected_indices)
}

fn sync_ignored_and_available(win: &SettingsWindow, data: &mut SharedUiData) {
    let ignored_names = data.ignored_list.clone();
    let ignored_rows: Vec<AppOptionData> = ignored_names
        .iter()
        .map(|name| data.build_app_option(name))
        .collect();
    win.set_ignored_processes(ModelRc::new(VecModel::from(ignored_rows)));

    // 过滤掉已存在于忽略列表的进程，防止重复添加
    data.available_apps = data
        .all_running_apps
        .iter()
        .filter(|app| {
            !data
                .ignored_list
                .iter()
                .any(|ignored_name| ignored_name.eq_ignore_ascii_case(&app.exe_name))
        })
        .cloned()
        .collect();

    if data.available_apps.is_empty() {
        data.selected_app_index = 0;
        win.set_available_processes(ModelRc::new(VecModel::from(Vec::<AppOptionData>::new())));
        win.set_selected_process_index(0);
        win.set_can_add_ignored(false);
        win.set_current_selected_app(AppOptionData {
            exe_name: SharedString::new(),
            display_name: SharedString::new(),
            icon: data.fallback_icon.clone(),
        });
    } else {
        if data.selected_app_index >= data.available_apps.len() {
            data.selected_app_index = 0;
        }
        let available_clone = data.available_apps.clone();
        let available_rows: Vec<AppOptionData> = available_clone
            .iter()
            .map(|app| {
                let icon = data.get_icon_for_path(&app.exe_path);
                AppOptionData {
                    exe_name: SharedString::from(app.exe_name.as_str()),
                    display_name: SharedString::from(app.display_name.as_str()),
                    icon,
                }
            })
            .collect();

        let current_opt = available_rows[data.selected_app_index].clone();
        win.set_available_processes(ModelRc::new(VecModel::from(available_rows)));
        win.set_selected_process_index(data.selected_app_index as i32);
        win.set_current_selected_app(current_opt);
        win.set_can_add_ignored(true);
    }
}

fn sync_models_to_window(win: &SettingsWindow, data: &mut SharedUiData) {
    let (dev_names, dev_ids, rows, selected_indices) =
        build_models(&data.config, &data.monitors, &data.devices);
    data.device_ids = dev_ids;
    data.selected_indices = selected_indices;

    win.set_audio_device_names(ModelRc::new(VecModel::from(dev_names)));
    win.set_monitor_rows(ModelRc::new(VecModel::from(rows)));

    sync_ignored_and_available(win, data);
}

fn normalized_ignored_set(list: &[String]) -> Vec<String> {
    let mut items: Vec<String> = list.iter().map(|s| s.to_ascii_lowercase()).collect();
    items.sort();
    items.dedup();
    items
}

fn update_dirty_state(win: &SettingsWindow, data: &SharedUiData) {
    let current_enabled = win.get_auto_switch_enabled();
    let current_auto_start = win.get_auto_start_enabled();
    let current_debounce = (win.get_debounce_ms().clamp(50, 2000)) as u64;
    let current_close_action = CloseAction::from_index(win.get_close_action_index());
    let current_theme_mode = ThemeMode::from_index(win.get_theme_mode_index());

    let is_dirty = current_enabled != data.saved_config.enabled
        || current_auto_start != data.saved_auto_start
        || current_debounce != data.saved_config.debounce_ms.clamp(50, 2000)
        || current_close_action != data.saved_config.close_action
        || current_theme_mode != data.saved_config.theme_mode
        || data.config.monitor_audio_map != data.saved_config.monitor_audio_map
        || data.config.monitor_volume_map != data.saved_config.monitor_volume_map
        || normalized_ignored_set(&data.ignored_list)
            != normalized_ignored_set(&data.saved_config.ignored_processes);

    win.set_has_unsaved_changes(is_dirty);
}

fn post_daemon_message(msg_id: u32) {
    unsafe {
        if let Ok(daemon_hwnd) =
            FindWindowExW(HWND_MESSAGE, None, w!("SoundCastHiddenMsgWindow"), None)
        {
            if !daemon_hwnd.0.is_null() {
                let _ = PostMessageW(daemon_hwnd, msg_id, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn sync_switch_logs_to_window(win: &SettingsWindow, data: &mut SharedUiData) {
    let entries = read_switch_logs(200);
    let count = entries.len() as i32;
    let rows: Vec<SwitchLogRowData> = entries
        .into_iter()
        .map(|e| {
            let app = data.resolve_app_display(&e.process_name, &e.exe_path);
            SwitchLogRowData {
                timestamp: SharedString::from(e.timestamp),
                process_name: SharedString::from(e.process_name),
                display_name: app.display_name,
                icon: app.icon,
                from_device: SharedString::from(e.from_device),
                to_device: SharedString::from(e.to_device),
                monitor_name: SharedString::from(e.monitor_name),
                volume_text: e
                    .volume_percent
                    .map(|v| SharedString::from(format!("{v}%")))
                    .unwrap_or_default(),
            }
        })
        .collect();
    win.set_switch_log_count(count);
    win.set_switch_logs(ModelRc::new(VecModel::from(rows)));
}

fn apply_theme_to_window(win: &SettingsWindow, mode: ThemeMode) {
    let mut resolved_dark = None;
    let _ = win.window().with_winit_window(|winit_win| {
        let target_winit_theme = match mode {
            ThemeMode::System => None,
            ThemeMode::Light => Some(WinitTheme::Light),
            ThemeMode::Dark => Some(WinitTheme::Dark),
        };
        winit_win.set_theme(target_winit_theme);
        let effective = match mode {
            ThemeMode::System => winit_win.theme().unwrap_or(WinitTheme::Light),
            ThemeMode::Light => WinitTheme::Light,
            ThemeMode::Dark => WinitTheme::Dark,
        };
        resolved_dark = Some(effective == WinitTheme::Dark);
    });
    if let Some(is_dark) = resolved_dark {
        win.invoke_apply_color_scheme(is_dark);
        let next = win.get_redraw_nonce().wrapping_add(1);
        win.set_redraw_nonce(next);
        win.window().request_redraw();
    }
}

pub fn run_settings_window(
    reload_msg_id: u32,
    exit_msg_id: u32,
    initial_status: Option<String>,
    initial_tab: i32,
) {
    // 强制指定 Fluent 风格（build.rs 已编译期指定，此处双保险）
    std::env::set_var("SLINT_STYLE", "fluent");

    let Ok(win) = SettingsWindow::new() else {
        return;
    };

    let config = AppConfig::load();
    let initial_theme = config.theme_mode;
    let initial_auto_start = is_auto_start_enabled();
    let monitors = enumerate_monitors();
    let devices = enumerate_playback_devices();
    let ignored_list = config.ignored_processes.clone();
    let all_running_apps = enumerate_running_apps();

    win.set_app_version(SharedString::from(concat!("v", env!("CARGO_PKG_VERSION"))));
    win.set_active_tab(initial_tab.clamp(0, 1));
    win.set_theme_mode_index(initial_theme.to_index());
    win.set_auto_switch_enabled(config.enabled);
    win.set_auto_start_enabled(initial_auto_start);
    win.set_debounce_ms(config.debounce_ms as i32);
    win.set_close_action_index(config.close_action.to_index());
    if let Some(st) = initial_status {
        win.set_status_text(SharedString::from(st));
    }

    let data = Rc::new(RefCell::new(SharedUiData {
        saved_config: config.clone(),
        saved_auto_start: initial_auto_start,
        config,
        monitors,
        devices,
        device_ids: Vec::new(),
        selected_indices: Vec::new(),
        ignored_list,
        all_running_apps,
        available_apps: Vec::new(),
        selected_app_index: 0,
        icon_cache: HashMap::new(),
        fallback_icon: default_app_icon(),
    }));

    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_mark_dirty(move || {
            if let Some(w) = weak.upgrade() {
                let d = data.borrow();
                update_dirty_state(&w, &d);
            }
        });
    }

    {
        let mut d = data.borrow_mut();
        sync_models_to_window(&win, &mut d);
        sync_switch_logs_to_window(&win, &mut d);
    }

    // 轻量轮询切换日志文件变更，实时更新日志列表
    let last_log_sig = Rc::new(Cell::new(log_signature()));
    let log_timer = Timer::default();
    {
        let data = data.clone();
        let weak = win.as_weak();
        let last_sig = last_log_sig.clone();
        log_timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            let sig = log_signature();
            if sig != last_sig.get() {
                last_sig.set(sig);
                if let Some(w) = weak.upgrade() {
                    let mut d = data.borrow_mut();
                    sync_switch_logs_to_window(&w, &mut d);
                }
            }
        });
    }

    // 日志页操作回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        let last_sig = last_log_sig.clone();
        win.on_refresh_logs_clicked(move || {
            last_sig.set(log_signature());
            if let Some(w) = weak.upgrade() {
                let mut d = data.borrow_mut();
                d.all_running_apps = enumerate_running_apps();
                sync_switch_logs_to_window(&w, &mut d);
            }
        });
    }

    {
        let data = data.clone();
        let weak = win.as_weak();
        let last_sig = last_log_sig.clone();
        win.on_clear_logs_clicked(move || {
            if let Some(w) = weak.upgrade() {
                match clear_switch_logs() {
                    Ok(_) => {
                        last_sig.set(log_signature());
                        let mut d = data.borrow_mut();
                        sync_switch_logs_to_window(&w, &mut d);
                        w.set_status_text(SharedString::from("已清空切换日志"));
                    }
                    Err(e) => {
                        w.set_status_text(SharedString::from(format!("清空日志失败: {e}")));
                    }
                }
            }
        });
    }

    {
        win.on_open_log_file_clicked(|| {
            open_log_in_editor();
        });
    }

    // 下拉框变更回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_monitor_selection_changed(move |row_idx, sel_idx| {
            let mut d = data.borrow_mut();
            let r = row_idx as usize;
            let mut updated_dev_id: Option<String> = None;
            let mut mon_dev_opt: Option<String> = None;
            if r < d.selected_indices.len() {
                d.selected_indices[r] = sel_idx;
                let mon_dev = d.monitors[r].device_name.clone();
                if let Some(opt_id) = d.device_ids.get(sel_idx as usize).cloned() {
                    match opt_id {
                        Some(id) => {
                            d.config.monitor_audio_map.insert(mon_dev.clone(), id.clone());
                            updated_dev_id = Some(id);
                        }
                        None => {
                            d.config.monitor_audio_map.remove(&mon_dev);
                            d.config.monitor_volume_map.remove(&mon_dev);
                        }
                    }
                }
                mon_dev_opt = Some(mon_dev);
            }
            if let Some(w) = weak.upgrade() {
                let model = w.get_monitor_rows();
                if let Some(mut row) = model.row_data(r) {
                    row.selected_index = sel_idx;
                    if let Some(dev_id) = updated_dev_id {
                        let has_vol = mon_dev_opt
                            .as_ref()
                            .is_some_and(|m| d.config.monitor_volume_map.contains_key(m));
                        if !has_vol {
                            row.volume_percent = get_device_volume(&dev_id).unwrap_or(50) as i32;
                        }
                    } else {
                        row.volume_enabled = false;
                    }
                    model.set_row_data(r, row);
                }
                update_dirty_state(&w, &d);
            }
        });
    }

    // 单显示器音量开关切换回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_monitor_volume_toggled(move |row_idx, enabled| {
            let mut d = data.borrow_mut();
            let r = row_idx as usize;
            let Some(mon) = d.monitors.get(r).cloned() else {
                return;
            };
            let sel_idx = d.selected_indices.get(r).copied().unwrap_or(0);
            let dev_id = d.device_ids.get(sel_idx as usize).cloned().flatten();

            let Some(w) = weak.upgrade() else {
                return;
            };
            let model = w.get_monitor_rows();
            let Some(mut row) = model.row_data(r) else {
                return;
            };

            if enabled && sel_idx > 0 {
                let current_vol = dev_id
                    .as_deref()
                    .and_then(get_device_volume)
                    .unwrap_or_else(|| row.volume_percent.clamp(0, 100) as u8);
                d.config
                    .monitor_volume_map
                    .insert(mon.device_name, current_vol);
                row.volume_enabled = true;
                row.volume_percent = current_vol as i32;
            } else {
                d.config.monitor_volume_map.remove(&mon.device_name);
                row.volume_enabled = false;
            }
            model.set_row_data(r, row);
            update_dirty_state(&w, &d);
        });
    }

    // 单显示器音量滑块拖拽/释放回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_monitor_volume_changed(move |row_idx, percent, apply_live| {
            let mut d = data.borrow_mut();
            let r = row_idx as usize;
            let Some(mon) = d.monitors.get(r).cloned() else {
                return;
            };
            let clamped = percent.clamp(0, 100) as u8;
            let sel_idx = d.selected_indices.get(r).copied().unwrap_or(0);
            let dev_id = d.device_ids.get(sel_idx as usize).cloned().flatten();

            if sel_idx > 0 && d.config.monitor_volume_map.contains_key(&mon.device_name) {
                d.config
                    .monitor_volume_map
                    .insert(mon.device_name, clamped);
            }

            if let Some(w) = weak.upgrade() {
                let model = w.get_monitor_rows();
                if let Some(mut row) = model.row_data(r) {
                    row.volume_percent = clamped as i32;
                    model.set_row_data(r, row);
                }
                update_dirty_state(&w, &d);
            }

            if apply_live {
                if let Some(id) = dev_id {
                    let _ = set_device_volume(&id, clamped);
                }
            }
        });
    }

    // 刷新设备与进程列表回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_refresh_clicked(move || {
            let mut d = data.borrow_mut();
            d.monitors = enumerate_monitors();
            d.devices = enumerate_playback_devices();
            d.all_running_apps = enumerate_running_apps();
            if let Some(w) = weak.upgrade() {
                sync_models_to_window(&w, &mut d);
                w.set_status_text(SharedString::from("已刷新显示器、音频设备及运行中应用列表"));
            }
        });
    }

    // 选择弹出层中的某项应用
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_select_available_app(move |idx| {
            let mut d = data.borrow_mut();
            let i = idx as usize;
            if let Some(app) = d.available_apps.get(i).cloned() {
                d.selected_app_index = i;
                let icon = d.get_icon_for_path(&app.exe_path);
                if let Some(w) = weak.upgrade() {
                    w.set_selected_process_index(idx);
                    w.set_current_selected_app(AppOptionData {
                        exe_name: SharedString::from(app.exe_name),
                        display_name: SharedString::from(app.display_name),
                        icon,
                    });
                }
            }
        });
    }

    // 添加忽略进程（带图标选择，且校验已存在不能重复添加）
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_add_ignored_clicked(move |sel_idx| {
            let mut d = data.borrow_mut();
            let idx = sel_idx as usize;
            let Some(chosen) = d.available_apps.get(idx).cloned() else {
                return;
            };
            if d.ignored_list
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&chosen.exe_name))
            {
                return;
            }
            d.ignored_list.push(chosen.exe_name);
            if let Some(w) = weak.upgrade() {
                sync_ignored_and_available(&w, &mut d);
                update_dirty_state(&w, &d);
            }
        });
    }

    // 移除忽略进程（移除后重新出现在下拉可选项中）
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_remove_ignored_clicked(move |idx| {
            let mut d = data.borrow_mut();
            let i = idx as usize;
            if i < d.ignored_list.len() {
                d.ignored_list.remove(i);
            }
            if let Some(w) = weak.upgrade() {
                sync_ignored_and_available(&w, &mut d);
                update_dirty_state(&w, &d);
            }
        });
    }

    // 保存并应用回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_save_clicked(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let mut d = data.borrow_mut();
            d.config.enabled = w.get_auto_switch_enabled();
            d.config.theme_mode = ThemeMode::from_index(w.get_theme_mode_index());
            d.config.debounce_ms = (w.get_debounce_ms().clamp(50, 2000)) as u64;
            d.config.close_action = CloseAction::from_index(w.get_close_action_index());
            d.config.ignored_processes = d.ignored_list.clone();

            let auto_start_ok = set_auto_start(w.get_auto_start_enabled()).is_ok();
            match d.config.save() {
                Ok(_) => {
                    // 立即将各显示器已启用的固定音量应用到对应设备
                    for (mon_dev, vol) in &d.config.monitor_volume_map {
                        if let Some(dev_id) = d.config.monitor_audio_map.get(mon_dev) {
                            let _ = set_device_volume(dev_id, *vol);
                        }
                    }
                    d.saved_config = d.config.clone();
                    if auto_start_ok {
                        d.saved_auto_start = w.get_auto_start_enabled();
                    }
                    // 通知后台守护进程热加载配置
                    post_daemon_message(reload_msg_id);
                    update_dirty_state(&w, &d);
                    w.set_status_text(SharedString::from("设置已保存并热加载生效！"));
                }
                Err(e) => {
                    w.set_status_text(SharedString::from(format!("保存失败: {e}")));
                }
            }
        });
    }

    // 顶部主题切换回调（自动 / 浅色 / 深色）
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_theme_mode_changed(move |idx| {
            let mode = ThemeMode::from_index(idx);
            let mut disk_cfg = AppConfig::load();
            disk_cfg.theme_mode = mode;
            let _ = disk_cfg.save();
            let mut d = data.borrow_mut();
            d.config.theme_mode = mode;
            d.saved_config.theme_mode = mode;
            if let Some(w) = weak.upgrade() {
                apply_theme_to_window(&w, mode);
                update_dirty_state(&w, &d);
            }
        });
    }

    // 在默认浏览器中打开项目地址
    {
        win.on_open_project_url(|| unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                w!("https://github.com/story-x/SoundCast"),
                None,
                None,
                SW_SHOWNORMAL,
            );
        });
    }

    // 隐藏到托盘（仅退出当前 UI 子进程）
    {
        win.on_hide_to_tray_clicked(|| {
            let _ = slint::quit_event_loop();
        });
    }

    // 彻底退出应用（通知后台守护退出 + 关闭 UI）
    {
        win.on_exit_app_clicked(move || {
            post_daemon_message(exit_msg_id);
            let _ = slint::quit_event_loop();
        });
    }

    // 关闭询问弹窗操作回调
    {
        let data = data.clone();
        let weak = win.as_weak();
        win.on_close_dialog_action(move |action_idx, remember| {
            let action = CloseAction::from_index(action_idx);
            if let Some(w) = weak.upgrade() {
                if remember {
                    w.set_close_action_index(action.to_index());
                    let mut d = data.borrow_mut();
                    d.config.close_action = action;
                    d.saved_config.close_action = action;
                    let mut disk_cfg = AppConfig::load();
                    disk_cfg.close_action = action;
                    if disk_cfg.save().is_ok() {
                        post_daemon_message(reload_msg_id);
                    }
                }
                w.set_close_dialog_visible(false);
            }
            if action == CloseAction::ExitApp {
                post_daemon_message(exit_msg_id);
            }
            let _ = slint::quit_event_loop();
        });
    }

    // 拦截窗口右上角关闭按钮（×）
    {
        let weak = win.as_weak();
        win.window().on_close_requested(move || {
            let Some(w) = weak.upgrade() else {
                return CloseRequestResponse::HideWindow;
            };
            match CloseAction::from_index(w.get_close_action_index()) {
                CloseAction::HideToTray => {
                    let _ = slint::quit_event_loop();
                    CloseRequestResponse::HideWindow
                }
                CloseAction::ExitApp => {
                    post_daemon_message(exit_msg_id);
                    let _ = slint::quit_event_loop();
                    CloseRequestResponse::HideWindow
                }
                CloseAction::Ask => {
                    w.set_remember_close_choice(false);
                    w.set_close_dialog_visible(true);
                    let next = w.get_redraw_nonce().wrapping_add(1);
                    w.set_redraw_nonce(next);
                    w.window().request_redraw();
                    CloseRequestResponse::KeepWindowShown
                }
            }
        });
    }

    // 监听最小化还原/窗口大小变化/重新激活，强制触发全窗口重绘防止客户区透明，并确保首次启动居中
    let centered = Rc::new(Cell::new(false));
    {
        let weak = win.as_weak();
        let centered = centered.clone();
        win.window().on_winit_window_event(move |slint_win, event| {
            if !centered.get() && matches!(event, WindowEvent::Resized(_)) {
                let _ = slint_win.with_winit_window(|winit_win| {
                    if center_winit_window(winit_win) {
                        centered.set(true);
                    }
                });
            }
            if let WindowEvent::ThemeChanged(new_theme) = event {
                if let Some(w) = weak.upgrade() {
                    if ThemeMode::from_index(w.get_theme_mode_index()) == ThemeMode::System {
                        w.invoke_apply_color_scheme(*new_theme == WinitTheme::Dark);
                    }
                }
            }
            let should_redraw = matches!(
                event,
                WindowEvent::Resized(_)
                    | WindowEvent::Focused(true)
                    | WindowEvent::Occluded(false)
                    | WindowEvent::ThemeChanged(_)
            );
            if should_redraw {
                if let Some(w) = weak.upgrade() {
                    let next = w.get_redraw_nonce().wrapping_add(1);
                    w.set_redraw_nonce(next);
                }
                slint_win.request_redraw();
            }
            EventResult::Propagate
        });
    }

    let _ = win.show();
    apply_theme_to_window(&win, initial_theme);
    let _ = win.window().with_winit_window(|winit_win| {
        if !centered.get() && center_winit_window(winit_win) {
            centered.set(true);
        }
    });

    let _ = win.run();
}

fn center_winit_window(winit_win: &WinitWindow) -> bool {
    let outer = winit_win.outer_size();
    if outer.width == 0 || outer.height == 0 {
        return false;
    }
    let Some(monitor) = winit_win
        .current_monitor()
        .or_else(|| winit_win.primary_monitor())
    else {
        return false;
    };
    let mon_pos = monitor.position();
    let mon_size = monitor.size();
    let x = mon_pos.x + ((mon_size.width as i32 - outer.width as i32) / 2).max(0);
    let y = mon_pos.y + ((mon_size.height as i32 - outer.height as i32) / 2).max(0);
    winit_win.set_outer_position(PhysicalPosition::new(x, y));
    true
}
