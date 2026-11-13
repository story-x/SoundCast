use std::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetWindowRect, IsWindowVisible, PostMessageW,
    CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MOVESIZEEND, GA_ROOT, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

thread_local! {
    static HOOK_TARGET_HWND: Cell<isize> = const { Cell::new(0) };
    static HOOK_MSG_ID: Cell<u32> = const { Cell::new(0) };
    static LAST_FG_HWND: Cell<isize> = const { Cell::new(0) };
    static LAST_FG_RECT: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
}

pub struct WindowHookGuard {
    hooks: Vec<HWINEVENTHOOK>,
}

impl WindowHookGuard {
    pub fn install(notify_hwnd: HWND, msg_id: u32) -> Self {
        HOOK_TARGET_HWND.with(|c| c.set(notify_hwnd.0 as isize));
        HOOK_MSG_ID.with(|c| c.set(msg_id));

        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let mut hooks = Vec::new();

        unsafe {
            let h1 = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_proc),
                0,
                0,
                flags,
            );
            if !h1.0.is_null() {
                hooks.push(h1);
            }

            let h2 = SetWinEventHook(
                EVENT_SYSTEM_MOVESIZEEND,
                EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(win_event_proc),
                0,
                0,
                flags,
            );
            if !h2.0.is_null() {
                hooks.push(h2);
            }

            let h3 = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(win_event_proc),
                0,
                0,
                flags,
            );
            if !h3.0.is_null() {
                hooks.push(h3);
            }
        }

        Self { hooks }
    }
}

impl Drop for WindowHookGuard {
    fn drop(&mut self) {
        unsafe {
            for hook in self.hooks.drain(..) {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.0.is_null() {
        return;
    }

    let root = GetAncestor(hwnd, GA_ROOT);
    if root.0.is_null() || !IsWindowVisible(root).as_bool() {
        return;
    }

    let mut rect = RECT::default();
    if GetWindowRect(root, &mut rect).is_err() {
        return;
    }
    // 忽略尺寸为 0 的隐藏/幽灵窗口
    if rect.right - rect.left <= 0 || rect.bottom - rect.top <= 0 {
        return;
    }

    if event == EVENT_OBJECT_LOCATIONCHANGE {
        let fg = GetForegroundWindow();
        let fg_root = if fg.0.is_null() {
            HWND::default()
        } else {
            GetAncestor(fg, GA_ROOT)
        };
        if root != fg_root {
            return;
        }
        let same_rect = LAST_FG_HWND.with(|h| h.get() == root.0 as isize)
            && LAST_FG_RECT.with(|r| {
                let prev = r.get();
                prev.left == rect.left
                    && prev.top == rect.top
                    && prev.right == rect.right
                    && prev.bottom == rect.bottom
            });
        if same_rect {
            return;
        }
    }

    LAST_FG_HWND.with(|h| h.set(root.0 as isize));
    LAST_FG_RECT.with(|r| r.set(rect));

    let target = HWND(HOOK_TARGET_HWND.with(|c| c.get()) as *mut _);
    let msg_id = HOOK_MSG_ID.with(|c| c.get());
    if !target.0.is_null() && msg_id != 0 {
        let _ = PostMessageW(target, msg_id, WPARAM(root.0 as usize), LPARAM(0));
    }
}

