//! Win32 input capture using `WH_KEYBOARD_LL` + `WH_MOUSE_LL` (swallowing/blocking)
//! paired with Raw Input (`WM_INPUT`) for unaccelerated hardware mouse deltas.

use std::cell::RefCell;
use std::collections::HashSet;
use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use deskglide_proto::win_keymap::win_to_evdev;
use deskglide_proto::{InputEvent, KeyState, MouseButton, ScreenEdge};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};
use windows_sys::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, GetSystemMetrics, PostMessageW, PostThreadMessageW,
    RegisterClassExW, SetCursorPos, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    HC_ACTION, HHOOK, HWND_MESSAGE, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP,
    MSG, MSLLHOOKSTRUCT, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, UnregisterClassW, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, XBUTTON1, XBUTTON2,
};

use crate::hotkey::{Hotkey, HotkeyStateMachine};
use crate::{CaptureEvent, InputCapture};

const WM_INPUT: u32 = 0x00FF;
const WM_SYNC_ACTIVE: u32 = WM_APP + 1;
const WM_RESTORE_EDGE: u32 = WM_APP + 2;
const EDGE_BARRIER_PX: i32 = 24;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_GENERIC_MOUSE: u16 = 0x02;
const LLMHF_INJECTED: u32 = 0x0000_0001;
const LLMHF_LOWER_IL_INJECTED: u32 = 0x0000_0002;
const LLKHF_LOWER_IL_INJECTED: u32 = 0x0000_0002;

struct HookContext {
    tx: mpsc::Sender<CaptureEvent>,
    remote_flag: Arc<AtomicBool>,
    switch_edge: Arc<AtomicU8>,
    edge_push_acc: i32,
    hotkey_sm: HotkeyStateMachine,
    down_keys: HashSet<u16>,
    saved_cursor: Option<POINT>,
}

impl HookContext {
    fn enter_remote(&mut self) {
        self.remote_flag.store(true, Ordering::SeqCst);
        self.edge_push_acc = 0;
        unsafe {
            let mut pt: POINT = zeroed();
            if GetCursorPos(&mut pt) != 0 {
                self.saved_cursor = Some(pt);
            }
            let cx = GetSystemMetrics(SM_CXSCREEN) / 2;
            let cy = GetSystemMetrics(SM_CYSCREEN) / 2;
            let _ = SetCursorPos(cx, cy);
        }
    }

    fn leave_remote(&mut self) {
        self.remote_flag.store(false, Ordering::SeqCst);
        self.edge_push_acc = 0;
        if let Some(pt) = self.saved_cursor.take() {
            unsafe {
                let _ = SetCursorPos(pt.x, pt.y);
            }
        }
    }

    fn leave_remote_at_edge(&mut self, edge: ScreenEdge) {
        self.remote_flag.store(false, Ordering::SeqCst);
        self.edge_push_acc = 0;
        if edge == ScreenEdge::None {
            self.leave_remote();
            return;
        }
        let saved = self.saved_cursor.take();
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(32);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(32);
            let right = left + w - 1;
            let bottom = top + h - 1;
            let base_x = saved.map(|p| p.x).unwrap_or(left + w / 2).clamp(left + 16, right - 16);
            let base_y = saved.map(|p| p.y).unwrap_or(top + h / 2).clamp(top + 16, bottom - 16);
            let (x, y) = match edge {
                ScreenEdge::Left => (left + 16, base_y),
                ScreenEdge::Right => (right - 16, base_y),
                ScreenEdge::Top => (base_x, top + 16),
                ScreenEdge::Bottom => (base_x, bottom - 16),
                ScreenEdge::None => (base_x, base_y),
            };
            let _ = SetCursorPos(x, y);
        }
    }
}

thread_local! {
    static HOOK_CTX: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

pub struct WindowsCapture {
    rx: mpsc::Receiver<CaptureEvent>,
    remote_flag: Arc<AtomicBool>,
    switch_edge: Arc<AtomicU8>,
    thread_id: u32,
    hwnd_raw: isize,
    thread: Option<JoinHandle<()>>,
}

impl WindowsCapture {
    pub fn start(hotkey: Hotkey, switch_edge: ScreenEdge) -> Result<Self> {
        let (tx, rx) = mpsc::channel(4096);
        let remote_flag = Arc::new(AtomicBool::new(false));
        let remote_flag_thread = Arc::clone(&remote_flag);
        let switch_edge_atomic = Arc::new(AtomicU8::new(switch_edge as u8));
        let switch_edge_thread = Arc::clone(&switch_edge_atomic);
        let (ready_tx, ready_rx) = std_mpsc::sync_channel::<Result<(u32, isize)>>(1);

        let thread = thread::Builder::new()
            .name("deskglide-win-capture".into())
            .spawn(move || {
                run_hook_thread(hotkey, tx, remote_flag_thread, switch_edge_thread, ready_tx);
            })
            .context("failed to spawn Windows input capture thread")?;

        let (thread_id, hwnd_raw) = ready_rx
            .recv()
            .context("Windows capture thread exited before completing initialization")??;

        Ok(Self {
            rx,
            remote_flag,
            switch_edge: switch_edge_atomic,
            thread_id,
            hwnd_raw,
            thread: Some(thread),
        })
    }
}

impl InputCapture for WindowsCapture {
    fn set_remote_active(&self, active: bool) -> Result<()> {
        let prev = self.remote_flag.swap(active, Ordering::SeqCst);
        if prev != active {
            unsafe {
                let _ = PostMessageW(
                    self.hwnd_raw as HWND,
                    WM_SYNC_ACTIVE,
                    usize::from(active),
                    0,
                );
            }
        }
        Ok(())
    }

    fn set_switch_edge(&self, edge: ScreenEdge) {
        self.switch_edge.store(edge as u8, Ordering::SeqCst);
    }

    fn restore_from_edge(&self, edge: ScreenEdge) -> Result<()> {
        self.remote_flag.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostMessageW(
                self.hwnd_raw as HWND,
                WM_RESTORE_EDGE,
                usize::from(edge as u8),
                0,
            );
        }
        Ok(())
    }

    fn is_remote_active(&self) -> bool {
        self.remote_flag.load(Ordering::SeqCst)
    }

    fn events_rx(&mut self) -> &mut mpsc::Receiver<CaptureEvent> {
        &mut self.rx
    }
}

impl Drop for WindowsCapture {
    fn drop(&mut self) {
        self.remote_flag.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run_hook_thread(
    hotkey: Hotkey,
    tx: mpsc::Sender<CaptureEvent>,
    remote_flag: Arc<AtomicBool>,
    switch_edge: Arc<AtomicU8>,
    ready_tx: std_mpsc::SyncSender<Result<(u32, isize)>>,
) {
    unsafe {
        let tid = GetCurrentThreadId();
        let hinst: HINSTANCE = GetModuleHandleW(std::ptr::null());

        let class_name: Vec<u16> = "DeskGlideRawInputSink\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                let _ =
                    ready_tx.send(Err(anyhow::anyhow!("RegisterClassExW failed: win32={err}")));
                return;
            }
        }

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            let err = GetLastError();
            let _ = ready_tx.send(Err(anyhow::anyhow!("CreateWindowExW failed: win32={err}")));
            return;
        }

        let rid = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: HID_USAGE_GENERIC_MOUSE,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        if RegisterRawInputDevices(&rid, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            let err = GetLastError();
            let _ = DestroyWindow(hwnd);
            let _ = ready_tx.send(Err(anyhow::anyhow!(
                "RegisterRawInputDevices failed: win32={err}"
            )));
            return;
        }

        HOOK_CTX.with(|cell| {
            *cell.borrow_mut() = Some(HookContext {
                tx,
                remote_flag,
                switch_edge,
                edge_push_acc: 0,
                hotkey_sm: HotkeyStateMachine::new(hotkey),
                down_keys: HashSet::new(),
                saved_cursor: None,
            });
        });

        let kb_hook: HHOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0);
        if kb_hook.is_null() {
            let err = GetLastError();
            let _ = DestroyWindow(hwnd);
            HOOK_CTX.with(|c| *c.borrow_mut() = None);
            let _ = ready_tx.send(Err(anyhow::anyhow!(
                "SetWindowsHookExW(WH_KEYBOARD_LL) failed: win32={err}"
            )));
            return;
        }

        let mouse_hook: HHOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0);
        if mouse_hook.is_null() {
            let err = GetLastError();
            let _ = UnhookWindowsHookEx(kb_hook);
            let _ = DestroyWindow(hwnd);
            HOOK_CTX.with(|c| *c.borrow_mut() = None);
            let _ = ready_tx.send(Err(anyhow::anyhow!(
                "SetWindowsHookExW(WH_MOUSE_LL) failed: win32={err}"
            )));
            return;
        }

        debug!(tid, "Windows input hooks & Raw Input registered");
        let _ = ready_tx.send(Ok((tid, hwnd as isize)));

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWindowsHookEx(mouse_hook);
        let _ = UnhookWindowsHookEx(kb_hook);
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name.as_ptr(), hinst);
        HOOK_CTX.with(|c| {
            if let Some(mut ctx) = c.borrow_mut().take() {
                ctx.leave_remote();
            }
        });
        debug!("Windows input capture thread stopped");
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => {
            HOOK_CTX.with(|cell| {
                let Ok(mut borrow) = cell.try_borrow_mut() else {
                    return;
                };
                let Some(ctx) = borrow.as_mut() else {
                    return;
                };

                let remote = ctx.remote_flag.load(Ordering::Relaxed);
                let edge = ScreenEdge::from_u8(ctx.switch_edge.load(Ordering::Relaxed))
                    .unwrap_or(ScreenEdge::None);
                if !remote && edge == ScreenEdge::None {
                    return;
                }

                let mut buf = [0u64; 32]; // 256 bytes, 8-byte aligned
                let mut size = size_of_val(&buf) as u32;
                let written = GetRawInputData(
                    lparam as HRAWINPUT,
                    RID_INPUT,
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    size_of::<RAWINPUTHEADER>() as u32,
                );
                if written == u32::MAX || (written as usize) < size_of::<RAWINPUTHEADER>() {
                    return;
                }

                let raw = &*(buf.as_ptr().cast::<RAWINPUT>());
                if raw.header.dwType != RIM_TYPEMOUSE || raw.header.hDevice.is_null() {
                    return;
                }
                let mouse = raw.data.mouse;
                if (mouse.usFlags & MOUSE_MOVE_ABSOLUTE) != 0 {
                    return;
                }
                let dx = mouse.lLastX;
                let dy = mouse.lLastY;
                if dx == 0 && dy == 0 {
                    return;
                }

                if remote {
                    let _ = ctx
                        .tx
                        .try_send(CaptureEvent::Input(InputEvent::MouseMove { dx, dy }));
                } else {
                    let mut pt: POINT = zeroed();
                    if GetCursorPos(&mut pt) == 0 {
                        return;
                    }
                    let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
                    let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
                    let right = left + GetSystemMetrics(SM_CXVIRTUALSCREEN) - 1;
                    let bottom = top + GetSystemMetrics(SM_CYVIRTUALSCREEN) - 1;

                    let push = match edge {
                        ScreenEdge::Left if pt.x <= left + 1 && dx < 0 => -dx,
                        ScreenEdge::Right if pt.x >= right - 1 && dx > 0 => dx,
                        ScreenEdge::Top if pt.y <= top + 1 && dy < 0 => -dy,
                        ScreenEdge::Bottom if pt.y >= bottom - 1 && dy > 0 => dy,
                        _ => 0,
                    };

                    if push > 0 {
                        ctx.edge_push_acc = ctx.edge_push_acc.saturating_add(push);
                        if ctx.edge_push_acc >= EDGE_BARRIER_PX {
                            ctx.edge_push_acc = 0;
                            ctx.hotkey_sm.set_remote_active(true);
                            ctx.enter_remote();
                            let _ = ctx.tx.try_send(CaptureEvent::ActiveChanged(true));
                        }
                    } else {
                        ctx.edge_push_acc = 0;
                    }
                }
            });
            0
        }
        WM_SYNC_ACTIVE => {
            let active = wparam != 0;
            HOOK_CTX.with(|cell| {
                if let Ok(mut borrow) = cell.try_borrow_mut() {
                    if let Some(ctx) = borrow.as_mut() {
                        ctx.hotkey_sm.set_remote_active(active);
                        ctx.down_keys.clear();
                        if active {
                            ctx.enter_remote();
                        } else {
                            ctx.leave_remote();
                        }
                    }
                }
            });
            0
        }
        WM_RESTORE_EDGE => {
            let edge = ScreenEdge::from_u8(wparam as u8).unwrap_or(ScreenEdge::None);
            HOOK_CTX.with(|cell| {
                if let Ok(mut borrow) = cell.try_borrow_mut() {
                    if let Some(ctx) = borrow.as_mut() {
                        ctx.hotkey_sm.set_remote_active(false);
                        ctx.down_keys.clear();
                        ctx.leave_remote_at_edge(edge);
                    }
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
    if (kb.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED)) != 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let msg = wparam as u32;
    let is_up = (kb.flags & LLKHF_UP) != 0 || msg == WM_KEYUP || msg == WM_SYSKEYUP;
    let is_down = !is_up && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN);
    if !is_up && !is_down {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let extended = (kb.flags & LLKHF_EXTENDED) != 0;
    let Some(evdev_code) = win_to_evdev(kb.scanCode as u16, extended, kb.vkCode) else {
        trace!(scan = kb.scanCode, vk = kb.vkCode, extended, "unmapped Windows key");
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    };

    let swallow = HOOK_CTX.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(ctx) = borrow.as_mut() else {
            return false;
        };

        let key_state = if is_up {
            ctx.down_keys.remove(&evdev_code);
            KeyState::Release
        } else if ctx.down_keys.insert(evdev_code) {
            KeyState::Press
        } else {
            KeyState::Repeat
        };

        let decision = ctx.hotkey_sm.on_key(evdev_code, key_state);
        if let Some(active) = decision.new_remote_active {
            if active {
                ctx.enter_remote();
            } else {
                ctx.leave_remote();
            }
            if ctx.tx.try_send(CaptureEvent::ActiveChanged(active)).is_err() {
                warn!("capture event channel full on ActiveChanged");
            }
            return true;
        }

        if decision.swallow && ctx.hotkey_sm.is_remote_active() {
            let _ = ctx.tx.try_send(CaptureEvent::Input(InputEvent::Key {
                code: evdev_code,
                state: key_state,
            }));
        }

        decision.swallow
    });

    if swallow {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let ms = &*(lparam as *const MSLLHOOKSTRUCT);
    if (ms.flags & (LLMHF_INJECTED | LLMHF_LOWER_IL_INJECTED)) != 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let swallow = HOOK_CTX.with(|cell| {
        let Ok(borrow) = cell.try_borrow() else {
            return false;
        };
        let Some(ctx) = borrow.as_ref() else {
            return false;
        };
        if !ctx.remote_flag.load(Ordering::Relaxed) {
            return false;
        }

        let msg = wparam as u32;
        let event = match msg {
            WM_MOUSEMOVE => None, // Delta already captured unaccelerated from Raw Input (WM_INPUT).
            WM_LBUTTONDOWN => Some(InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            }),
            WM_LBUTTONUP => Some(InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: false,
            }),
            WM_RBUTTONDOWN => Some(InputEvent::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            }),
            WM_RBUTTONUP => Some(InputEvent::MouseButton {
                button: MouseButton::Right,
                pressed: false,
            }),
            WM_MBUTTONDOWN => Some(InputEvent::MouseButton {
                button: MouseButton::Middle,
                pressed: true,
            }),
            WM_MBUTTONUP => Some(InputEvent::MouseButton {
                button: MouseButton::Middle,
                pressed: false,
            }),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let xbtn = ((ms.mouseData >> 16) & 0xFFFF) as u16;
                let button = if xbtn == XBUTTON1 {
                    Some(MouseButton::Back)
                } else if xbtn == XBUTTON2 {
                    Some(MouseButton::Forward)
                } else {
                    None
                };
                button.map(|button| InputEvent::MouseButton {
                    button,
                    pressed: msg == WM_XBUTTONDOWN,
                })
            }
            WM_MOUSEWHEEL => {
                let dy = ((ms.mouseData >> 16) as i16) as i32;
                (dy != 0).then_some(InputEvent::MouseScroll { dx: 0, dy })
            }
            WM_MOUSEHWHEEL => {
                let dx = ((ms.mouseData >> 16) as i16) as i32;
                (dx != 0).then_some(InputEvent::MouseScroll { dx, dy: 0 })
            }
            _ => None,
        };

        if let Some(ev) = event {
            let _ = ctx.tx.try_send(CaptureEvent::Input(ev));
        }

        true // Swallow all physical mouse activity locally while controlling remote.
    });

    if swallow {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}
