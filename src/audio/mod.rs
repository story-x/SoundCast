use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::Path;
use windows::core::{implement, w, Interface, GUID, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionState, AudioSessionStateActive,
    AudioSessionStateExpired, EDataFlow, ERole, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    DEVICE_STATE, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

const DEVINTERFACE_AUDIO_RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";
const HRESULT_PROCESS_NO_AUDIO: i32 = 0x80070490_u32 as i32; // E_NOT_FOUND / Element not found

/// 已知的 Windows.Media.Internal.AudioPolicyConfig 接口 GUID
const KNOWN_POLICY_GUIDS: [GUID; 3] = [
    // Pre-H1H2
    GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258),
    // H1H2 / Win11
    GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324),
    // Windows 10 1709 (16299)
    GUID::from_u128(0x32aa8e18_6496_4e24_9f94_b800e7eccc45),
];

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

type DllGetActivationFactoryFn =
    unsafe extern "system" fn(activatable_class_id: *mut c_void, factory: *mut *mut c_void) -> HRESULT;
type QueryInterfaceFn =
    unsafe extern "system" fn(this: *mut c_void, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT;
type ReleaseFn = unsafe extern "system" fn(this: *mut c_void) -> u32;
type GetIidsFn =
    unsafe extern "system" fn(this: *mut c_void, iid_count: *mut u32, iids: *mut *mut GUID) -> HRESULT;
type SetPersistedEndpointFn = unsafe extern "system" fn(
    this: *mut c_void,
    process_id: u32,
    flow: i32,
    role: i32,
    device_id: *mut c_void,
) -> HRESULT;
type GetPersistedEndpointFn = unsafe extern "system" fn(
    this: *mut c_void,
    process_id: u32,
    flow: i32,
    role: i32,
    device_id: *mut *mut c_void,
) -> HRESULT;

pub struct AudioPolicyConfig {
    ptr: *mut c_void,
    vtable: *const *const c_void,
}

impl AudioPolicyConfig {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let dll = LoadLibraryW(w!("AudioSes.dll"))
                .map_err(|e| format!("加载 AudioSes.dll 失败: {e}"))?;
            let proc = GetProcAddress(dll, windows::core::s!("DllGetActivationFactory"))
                .ok_or_else(|| "找不到 DllGetActivationFactory".to_string())?;
            let get_factory: DllGetActivationFactoryFn = std::mem::transmute(proc);

            let class_name = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
            let class_raw: *mut c_void = std::mem::transmute_copy(&class_name);
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = get_factory(class_raw, &mut factory);
            if hr.is_err() || factory.is_null() {
                return Err(format!("DllGetActivationFactory 失败: {hr:?}"));
            }

            let vtable = *(factory as *const *const *const c_void);
            // IInspectable::GetIids is at vtable index 3
            let get_iids: GetIidsFn = std::mem::transmute(*vtable.add(3));
            let mut count: u32 = 0;
            let mut iids: *mut GUID = std::ptr::null_mut();
            let hr = get_iids(factory, &mut count, &mut iids);
            if hr.is_err() || count == 0 || iids.is_null() {
                let release: ReleaseFn = std::mem::transmute(*vtable.add(2));
                release(factory);
                return Err(format!("GetIids 失败: {hr:?}"));
            }

            let iid_slice = std::slice::from_raw_parts(iids, count as usize);
            let target_guid = iid_slice
                .iter()
                .rev()
                .find(|g| KNOWN_POLICY_GUIDS.contains(g))
                .copied()
                .unwrap_or(iid_slice[iid_slice.len() - 1]);

            let query_interface: QueryInterfaceFn = std::mem::transmute(*vtable.add(0));
            let mut policy_ptr: *mut c_void = std::ptr::null_mut();
            let hr = query_interface(factory, &target_guid, &mut policy_ptr);

            CoTaskMemFree(Some(iids as *const c_void));
            let release: ReleaseFn = std::mem::transmute(*vtable.add(2));
            release(factory);

            if hr.is_err() || policy_ptr.is_null() {
                return Err(format!("QueryInterface AudioPolicyConfig 失败: {hr:?}"));
            }

            let policy_vtable = *(policy_ptr as *const *const *const c_void);
            Ok(Self {
                ptr: policy_ptr,
                vtable: policy_vtable,
            })
        }
    }

    pub fn get_persisted_endpoint(
        &self,
        process_id: u32,
        flow: EDataFlow,
        role: ERole,
    ) -> Option<String> {
        unsafe {
            let get_fn: GetPersistedEndpointFn = std::mem::transmute(*self.vtable.add(26));
            let mut raw_hstring: *mut c_void = std::ptr::null_mut();
            let hr = get_fn(self.ptr, process_id, flow.0, role.0, &mut raw_hstring);
            if hr.is_err() {
                if hr.0 != HRESULT_PROCESS_NO_AUDIO {
                    // 静默忽略未设置端点的进程错误
                }
                return None;
            }
            if raw_hstring.is_null() {
                return None;
            }
            let hstr: HSTRING = std::mem::transmute(raw_hstring);
            let s = hstr.to_string_lossy();
            unpack_device_id(&s)
        }
    }

    pub fn set_persisted_endpoint(
        &self,
        process_id: u32,
        flow: EDataFlow,
        role: ERole,
        raw_device_id: &str,
    ) -> Result<(), String> {
        unsafe {
            let set_fn: SetPersistedEndpointFn = std::mem::transmute(*self.vtable.add(25));
            let formatted = format_device_id(raw_device_id);
            let hstr = HSTRING::from(&formatted);
            let raw_hstr: *mut c_void = std::mem::transmute_copy(&hstr);
            let hr = set_fn(self.ptr, process_id, flow.0, role.0, raw_hstr);
            if hr.is_err() && hr.0 != HRESULT_PROCESS_NO_AUDIO {
                return Err(format!("SetPersistedDefaultAudioEndpoint 失败: {hr:?}"));
            }
            Ok(())
        }
    }
}

impl Drop for AudioPolicyConfig {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                let release: ReleaseFn = std::mem::transmute(*self.vtable.add(2));
                release(self.ptr);
            }
        }
    }
}

fn format_device_id(raw_id: &str) -> String {
    if raw_id.is_empty() {
        return String::new();
    }
    format!("{MMDEVAPI_TOKEN}{raw_id}{DEVINTERFACE_AUDIO_RENDER}")
}

fn unpack_device_id(full_id: &str) -> Option<String> {
    if full_id.is_empty() {
        return None;
    }
    let s = full_id
        .strip_prefix(MMDEVAPI_TOKEN)
        .unwrap_or(full_id);
    let s = s
        .strip_suffix(DEVINTERFACE_AUDIO_RENDER)
        .unwrap_or(s);
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 枚举当前所有活跃的音频播放设备
pub fn enumerate_playback_devices() -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    unsafe {
        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return devices;
        };

        let default_id = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()
            .and_then(|d| d.GetId().ok())
            .and_then(|pwstr| {
                let s = pwstr.to_string().ok();
                CoTaskMemFree(Some(pwstr.0 as *const c_void));
                s
            });

        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return devices;
        };
        let Ok(count) = collection.GetCount() else {
            return devices;
        };

        for i in 0..count {
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let Ok(pwstr_id) = device.GetId() else {
                continue;
            };
            let id = pwstr_id.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pwstr_id.0 as *const c_void));
            if id.is_empty() {
                continue;
            }

            let mut friendly_name = id.clone();
            if let Ok(store) = device.OpenPropertyStore(STGM_READ) {
                if let Ok(prop) = store.GetValue(&PKEY_Device_FriendlyName) {
                    if let Ok(pwstr_name) = PropVariantToStringAlloc(&prop) {
                        if let Ok(name) = pwstr_name.to_string() {
                            if !name.is_empty() {
                                friendly_name = name;
                            }
                        }
                        CoTaskMemFree(Some(pwstr_name.0 as *const c_void));
                    }
                }
            }

            let is_default = default_id.as_deref() == Some(id.as_str());
            devices.push(AudioDevice {
                id,
                name: friendly_name,
                is_default,
            });
        }
    }

    devices.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    devices
}

/// 获取指定 PID 的可执行文件完整路径（小写）
pub fn get_process_exe_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok && len > 0 {
            Some(String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
        } else {
            None
        }
    }
}

pub fn exe_file_name_lower(exe_path: &str) -> String {
    Path::new(exe_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct SwitchResult {
    /// 进程是否符合音频触发条件（当仅切换有音频输出应用时，要求存在活跃音频会话）
    pub has_audio: bool,
    /// 是否实际执行了持久化音频端点切换
    pub switched: bool,
    /// 切换前的音频设备 ID（优先使用此前持久化设备，其次使用当前音频会话所在设备）
    pub from_device_id: Option<String>,
}

/// 遍历当前所有处于激活状态的播放端点上的非过期 WASAPI 音频会话
fn for_each_render_session(mut f: impl FnMut(&str, u32, AudioSessionState)) {
    unsafe {
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return;
        };
        let Ok(dev_count) = collection.GetCount() else {
            return;
        };

        for d in 0..dev_count {
            let Ok(device) = collection.Item(d) else {
                continue;
            };
            let Ok(pwstr_id) = device.GetId() else {
                continue;
            };
            let dev_id = pwstr_id.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pwstr_id.0 as *const c_void));
            if dev_id.is_empty() {
                continue;
            }

            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(session_enum) = manager.GetSessionEnumerator() else {
                continue;
            };
            let Ok(sess_count) = session_enum.GetCount() else {
                continue;
            };

            for s in 0..sess_count {
                let Ok(ctrl) = session_enum.GetSession(s) else {
                    continue;
                };
                let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let state = ctrl2.GetState().unwrap_or(AudioSessionStateExpired);
                if state == AudioSessionStateExpired {
                    continue;
                }
                if let Ok(pid) = ctrl2.GetProcessId() {
                    if pid != 0 {
                        f(&dev_id, pid, state);
                    }
                }
            }
        }
    }
}

/// 收集目标应用正在输出音频（`AudioSessionStateActive`）的 PID 及其当前所在音频设备。
fn collect_matching_audio_targets(
    window_pid: u32,
    window_exe_lower: Option<&str>,
) -> (Vec<u32>, HashMap<u32, String>) {
    let mut pids = HashSet::new();
    let mut session_devices: HashMap<u32, String> = HashMap::new();
    let mut exe_cache: HashMap<u32, Option<String>> = HashMap::new();

    for_each_render_session(|dev_id, sess_pid, state| {
        if state != AudioSessionStateActive {
            return;
        }

        let matches_app = if sess_pid == window_pid {
            true
        } else if let Some(target_exe) = window_exe_lower {
            let sess_exe = exe_cache
                .entry(sess_pid)
                .or_insert_with(|| get_process_exe_path(sess_pid));
            sess_exe.as_deref() == Some(target_exe)
        } else {
            false
        };

        if matches_app {
            pids.insert(sess_pid);
            session_devices
                .entry(sess_pid)
                .or_insert_with(|| dev_id.to_string());
        }
    });

    (pids.into_iter().collect(), session_devices)
}

/// 枚举当前所有存在非过期音频会话的进程 PID
pub fn enumerate_active_audio_pids() -> Vec<u32> {
    let mut pids = HashSet::new();
    for_each_render_session(|_, pid, _| {
        pids.insert(pid);
    });
    pids.into_iter().collect()
}

/// 将指定应用（窗口 PID + 同路径正在输出音频的子进程）的音频输出切换到 target_device_id
pub fn switch_process_audio(
    policy: &AudioPolicyConfig,
    window_pid: u32,
    exe_path_lower: Option<&str>,
    target_device_id: &str,
) -> SwitchResult {
    let (target_pids, session_devices) =
        collect_matching_audio_targets(window_pid, exe_path_lower);
    if target_pids.is_empty() {
        return SwitchResult::default();
    }

    let roles = [eConsole, eMultimedia, eCommunications];
    let mut switched = false;
    let mut from_device_id: Option<String> = None;

    for pid in target_pids {
        // 先检查是否已经是目标设备，避免重复切换造成爆音
        let persisted = policy.get_persisted_endpoint(pid, eRender, eMultimedia);
        if persisted.as_deref() == Some(target_device_id) {
            continue;
        }
        let candidate_from = persisted.or_else(|| session_devices.get(&pid).cloned());
        if from_device_id.is_none() {
            from_device_id = candidate_from;
        } else if from_device_id.as_deref() == Some(target_device_id) && candidate_from.is_some() {
            from_device_id = candidate_from;
        }

        for role in roles {
            if policy
                .set_persisted_endpoint(pid, eRender, role, target_device_id)
                .is_ok()
            {
                switched = true;
            }
        }
    }

    SwitchResult {
        has_audio: true,
        switched,
        from_device_id,
    }
}

/// 音频设备变更通知客户端，用于在耳机/显示器音频设备插拔时刷新菜单
#[implement(IMMNotificationClient)]
pub struct DeviceNotificationClient {
    hwnd: HWND,
    msg_id: u32,
}

impl DeviceNotificationClient {
    pub fn new(hwnd: HWND, msg_id: u32) -> Self {
        Self { hwnd, msg_id }
    }

    fn notify(&self) {
        unsafe {
            let _ = PostMessageW(self.hwnd, self.msg_id, WPARAM(0), LPARAM(0));
        }
    }
}

impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl {
    fn OnDeviceStateChanged(&self, _pwstrdeviceid: &PCWSTR, _dwnewstate: DEVICE_STATE) -> windows::core::Result<()> {
        self.notify();
        Ok(())
    }

    fn OnDeviceAdded(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        self.notify();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        self.notify();
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _pwstrdefaultdeviceid: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.notify();
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _key: &windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

fn open_endpoint_volume(device_id: &str) -> Option<IAudioEndpointVolume> {
    if device_id.is_empty() {
        return None;
    }
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDevice(&HSTRING::from(device_id)).ok()?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// 读取指定音频播放设备当前的主音量百分比 (0..=100)
pub fn get_device_volume(device_id: &str) -> Option<u8> {
    let ep_vol = open_endpoint_volume(device_id)?;
    let level = unsafe { ep_vol.GetMasterVolumeLevelScalar().ok()? };
    Some((level.clamp(0.0, 1.0) * 100.0).round() as u8)
}

/// 设置指定音频播放设备的主音量百分比 (0..=100)
pub fn set_device_volume(device_id: &str, percent: u8) -> bool {
    let Some(ep_vol) = open_endpoint_volume(device_id) else {
        return false;
    };
    let scalar = (percent.min(100) as f32) / 100.0;
    unsafe { ep_vol.SetMasterVolumeLevelScalar(scalar, std::ptr::null()).is_ok() }
}

