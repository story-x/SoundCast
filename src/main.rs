#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_icon;
mod audio;
mod config;
mod monitor;
mod tray;
mod ui;
mod window_hook;

use audio::{
    enumerate_playback_devices, exe_file_name_lower, get_process_exe_path, switch_process_audio,
    AudioDevice, AudioPolicyConfig, DeviceNotificationClient,
};
use config::{is_auto_start_enabled, open_config_in_editor, set_auto_start, AppConfig};
use monitor::{enumerate_monitors, monitor_for_window, MonitorInfo};
use muda::MenuEvent;
use std::cell::RefCell;
use std::rc::Rc;
use tray::{TrayAction, TrayController};
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use window_hook::WindowHookGuard;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, IMMNotificationClient, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowExW, FindWindowW, GetMessageW,
    GetWindowThreadProcessId, IsIconic, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetTimer, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    HWND_MESSAGE, MSG, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

const WM_APP_WINDOW_EVENT: u32 = WM_APP + 1;
const WM_APP_DEVICE_CHANGE: u32 = WM_APP + 2;
const WM_APP_MENU_EVENT: u32 = WM_APP + 3;
const WM_APP_TRAY_EVENT: u32 = WM_APP + 4;
const WM_APP_RELOAD_CONFIG: u32 = WM_APP + 5;
const WM_APP_EXIT_APP: u32 = WM_APP + 6;

const TIMER_DEBOUNCE_SWITCH: usize = 1;
const TIMER_DEBOUNCE_REFRESH: usize = 2;

struct AppState {
    config: AppConfig,
    monitors: Vec<MonitorInfo>,
    devices: Vec<AudioDevice>,
    policy: Option<AudioPolicyConfig>,
    tray: TrayController,
    pending_hwnd: Option<HWND>,
    self_pid: u32,
    last_status: String,
}

impl AppState {
    fn refresh_devices_and_menu(&mut self) {
        self.monitors = enumerate_monitors();
        self.devices = enumerate_playback_devices();
        self.tray
            .rebuild_menu(&self.config, &self.monitors, &self.devices);
    }

    fn reload_config_and_menu(&mut self) {
        self.config = AppConfig::load();
        self.refresh_devices_and_menu();
    }

    fn handle_pending_window(&mut self) {
        if !self.config.enabled {
            return;
        }
        let Some(hwnd) = self.pending_hwnd.take() else {
            return;
        };
        let Some(ref policy) = self.policy else {
            return;
        };

        let mut pid: u32 = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid == 0 || pid == self.self_pid {
            return;
        }

        let Some(monitor_name) = monitor_for_window(hwnd) else {
            return;
        };
        let Some(target_device_id) = self.config.monitor_audio_map.get(&monitor_name).cloned()
        else {
            return;
        };

        let exe_path = get_process_exe_path(pid);
        let file_name = exe_path
            .as_deref()
            .map(exe_file_name_lower)
            .unwrap_or_else(|| format!("PID:{pid}"));

        if let Some(ref path) = exe_path {
            let name_lower = exe_file_name_lower(path);
            if self.config.is_process_ignored(&name_lower) {
                return;
            }
        }

        let switched = switch_process_audio(policy, pid, exe_path.as_deref(), &target_device_id);
        if switched {
            let dev_name = self
                .devices
                .iter()
                .find(|d| d.id == target_device_id)
                .map(|d| d.name.as_str())
                .unwrap_or("目标设备");
            let short_mon = monitor_name
                .strip_prefix("\\\\.\\")
                .unwrap_or(&monitor_name);
            self.last_status = format!("最近切换: {file_name} -> {dev_name} [{short_mon}]");
        }
    }

    fn handle_menu_event(&mut self, event: MenuEvent) {
        let Some(action) = self.tray.actions.get(&event.id).cloned() else {
            return;
        };
        match action {
            TrayAction::OpenSettings => {
                spawn_settings_process(Some(&self.last_status));
            }
            TrayAction::ToggleEnabled => {
                self.config.enabled = !self.config.enabled;
                let _ = self.config.save();
                self.tray
                    .rebuild_menu(&self.config, &self.monitors, &self.devices);
            }
            TrayAction::BindMonitor {
                monitor_device,
                audio_device_id,
            } => {
                match audio_device_id {
                    Some(id) => {
                        self.config.monitor_audio_map.insert(monitor_device, id);
                    }
                    None => {
                        self.config.monitor_audio_map.remove(&monitor_device);
                    }
                }
                let _ = self.config.save();
                self.tray
                    .rebuild_menu(&self.config, &self.monitors, &self.devices);
            }
            TrayAction::ToggleAutoStart => {
                let current = is_auto_start_enabled();
                let _ = set_auto_start(!current);
                self.tray
                    .rebuild_menu(&self.config, &self.monitors, &self.devices);
            }
            TrayAction::OpenConfig => {
                open_config_in_editor();
            }
            TrayAction::Refresh => {
                self.reload_config_and_menu();
            }
            TrayAction::Quit => unsafe {
                PostQuitMessage(0);
            },
        }
    }
}

thread_local! {
    static APP_STATE: RefCell<Option<Rc<RefCell<AppState>>>> = const { RefCell::new(None) };
}

fn activate_existing_settings_window() -> bool {
    unsafe {
        if let Ok(hwnd) = FindWindowW(None, w!("SoundCast 设置")) {
            if !hwnd.0.is_null() {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                } else {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                let _ = SetForegroundWindow(hwnd);
                return true;
            }
        }
    }
    false
}

fn spawn_settings_process(status: Option<&str>) {
    // 如果设置窗口已打开，直接前置激活，无需重新拉起进程
    if activate_existing_settings_window() {
        return;
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--settings");
        if let Some(st) = status {
            if !st.is_empty() {
                cmd.arg("--status").arg(st);
            }
        }
        let _ = cmd.spawn();
    }
}

fn run_settings_mode(args: &[String]) {
    unsafe {
        let mutex_res = CreateMutexW(None, false, w!("Local\\SoundCast_SettingsWindow_Mutex"));
        let last_err = GetLastError();
        if mutex_res.is_ok() && last_err == ERROR_ALREADY_EXISTS {
            activate_existing_settings_window();
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _mutex = mutex_res;

        let initial_status = args
            .windows(2)
            .find(|w| w[0] == "--status")
            .map(|w| w[1].clone());

        ui::run_settings_window(WM_APP_RELOAD_CONFIG, WM_APP_EXIT_APP, initial_status);

        CoUninitialize();
    }
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--settings") {
        run_settings_mode(&args);
        return;
    }

    unsafe {
        // 守护进程单实例保护：若已有实例在运行，则唤醒其设置界面并退出
        let mutex_res = CreateMutexW(None, false, w!("Local\\SoundCast_SingleInstance_Mutex"));
        let last_err = GetLastError();
        if mutex_res.is_ok() && last_err == ERROR_ALREADY_EXISTS {
            if let Ok(existing_hwnd) =
                FindWindowExW(HWND_MESSAGE, None, w!("SoundCastHiddenMsgWindow"), None)
            {
                if !existing_hwnd.0.is_null() {
                    let _ = PostMessageW(existing_hwnd, WM_APP_TRAY_EVENT, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        let _mutex = mutex_res;

        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        run_daemon_loop();
        CoUninitialize();
    }
}

fn run_daemon_loop() {
    let config = AppConfig::load();
    let first_run_unconfigured = config.monitor_audio_map.is_empty();
    let monitors = enumerate_monitors();
    let devices = enumerate_playback_devices();
    let policy = AudioPolicyConfig::new().ok();

    let tray = match TrayController::new(&config, &monitors, &devices) {
        Ok(t) => t,
        Err(_) => return,
    };

    let Some(msg_hwnd) = create_message_window() else {
        return;
    };

    let state = Rc::new(RefCell::new(AppState {
        config,
        monitors,
        devices,
        policy,
        tray,
        pending_hwnd: None,
        self_pid: unsafe { GetCurrentProcessId() },
        last_status: "就绪：等待应用窗口跨屏自动切换".to_string(),
    }));

    APP_STATE.with(|s| {
        *s.borrow_mut() = Some(state.clone());
    });

    // 菜单点击立即唤醒消息循环
    let notify_hwnd_raw = msg_hwnd.0 as isize;
    MenuEvent::set_event_handler(Some(move |_event| unsafe {
        let hwnd = HWND(notify_hwnd_raw as *mut _);
        let _ = PostMessageW(hwnd, WM_APP_MENU_EVENT, WPARAM(0), LPARAM(0));
    }));

    // 左键点击/双击托盘图标弹出 Slint 设置窗口
    let tray_hwnd_raw = msg_hwnd.0 as isize;
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        let open = matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        );
        if open {
            unsafe {
                let hwnd = HWND(tray_hwnd_raw as *mut _);
                let _ = PostMessageW(hwnd, WM_APP_TRAY_EVENT, WPARAM(0), LPARAM(0));
            }
        }
    }));

    // 安装窗口移动/前台切换钩子
    let _hook_guard = WindowHookGuard::install(msg_hwnd, WM_APP_WINDOW_EVENT);

    // 注册音频设备热插拔监听
    let _audio_notif_guard = register_audio_notification(msg_hwnd, WM_APP_DEVICE_CHANGE);

    // 首次启动尚未配置规则时，自动打开设置窗口
    if first_run_unconfigured {
        spawn_settings_process(Some("首次启动：请为每个显示器选择绑定的音频输出设备"));
    }

    // 标准 Win32 消息循环：无事件时完全阻塞挂起，CPU 0% 占用，后台内存仅数 MB
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    APP_STATE.with(|s| {
        *s.borrow_mut() = None;
    });
}

fn create_message_window() -> Option<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let class_name = w!("SoundCastHiddenMsgWindow");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("SoundCast"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            HINSTANCE(hinstance.0),
            None,
        )
        .ok()?;

        Some(hwnd)
    }
}

struct AudioNotifGuard {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl Drop for AudioNotifGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

fn register_audio_notification(hwnd: HWND, msg_id: u32) -> Option<AudioNotifGuard> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let client: IMMNotificationClient = DeviceNotificationClient::new(hwnd, msg_id).into();
        enumerator
            .RegisterEndpointNotificationCallback(&client)
            .ok()?;
        Some(AudioNotifGuard { enumerator, client })
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_WINDOW_EVENT => {
            let target_hwnd = HWND(wparam.0 as *mut _);
            let debounce = APP_STATE.with(|s| {
                if let Some(rc) = s.borrow().as_ref() {
                    let mut st = rc.borrow_mut();
                    st.pending_hwnd = Some(target_hwnd);
                    st.config.debounce_ms.max(50) as u32
                } else {
                    150
                }
            });
            let _ = SetTimer(hwnd, TIMER_DEBOUNCE_SWITCH, debounce, None);
            LRESULT(0)
        }
        WM_APP_DEVICE_CHANGE | WM_DISPLAYCHANGE => {
            let _ = SetTimer(hwnd, TIMER_DEBOUNCE_REFRESH, 400, None);
            LRESULT(0)
        }
        WM_APP_MENU_EVENT => {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                APP_STATE.with(|s| {
                    if let Some(rc) = s.borrow().as_ref() {
                        rc.borrow_mut().handle_menu_event(event);
                    }
                });
            }
            LRESULT(0)
        }
        WM_APP_TRAY_EVENT => {
            while TrayIconEvent::receiver().try_recv().is_ok() {}
            let status = APP_STATE.with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|rc| rc.borrow().last_status.clone())
            });
            spawn_settings_process(status.as_deref());
            LRESULT(0)
        }
        WM_APP_RELOAD_CONFIG => {
            APP_STATE.with(|s| {
                if let Some(rc) = s.borrow().as_ref() {
                    rc.borrow_mut().reload_config_and_menu();
                }
            });
            LRESULT(0)
        }
        WM_APP_EXIT_APP => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_TIMER => {
            let timer_id = wparam.0;
            let _ = KillTimer(hwnd, timer_id);
            if timer_id == TIMER_DEBOUNCE_SWITCH {
                APP_STATE.with(|s| {
                    if let Some(rc) = s.borrow().as_ref() {
                        rc.borrow_mut().handle_pending_window();
                    }
                });
            } else if timer_id == TIMER_DEBOUNCE_REFRESH {
                APP_STATE.with(|s| {
                    if let Some(rc) = s.borrow().as_ref() {
                        rc.borrow_mut().refresh_devices_and_menu();
                    }
                });
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[allow(dead_code)]
fn _unused(_: PCWSTR) {}
