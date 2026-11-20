//! Win32 `SendInput` backend for injecting keyboard and mouse events on Windows 11.

use std::mem::{size_of, zeroed};

use anyhow::Result;
use deskglide_proto::keycode::KEY_PAUSE;
use deskglide_proto::win_keymap::evdev_to_win;
use deskglide_proto::{InputEvent, KeyState, MouseButton};
use tracing::trace;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT, VK_PAUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{XBUTTON1, XBUTTON2};

use crate::InputInjector;

#[derive(Default)]
pub struct WindowsInjector;

impl WindowsInjector {
    pub fn new() -> Self {
        Self
    }

    fn send_one(input: INPUT) -> Result<()> {
        let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
        if sent == 0 {
            let err = unsafe { GetLastError() };
            anyhow::bail!("SendInput failed (win32={err})");
        }
        Ok(())
    }

    fn mouse_input(dx: i32, dy: i32, mouse_data: u32, dw_flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: mouse_data,
                    dwFlags: dw_flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

impl InputInjector for WindowsInjector {
    fn inject(&mut self, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::MouseMove { dx, dy } => {
                if dx != 0 || dy != 0 {
                    Self::send_one(Self::mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE))?;
                }
            }
            InputEvent::MouseButton { button, pressed } => {
                let (flag, data) = match (button, pressed) {
                    (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1)),
                    (MouseButton::Back, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
                    (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON2)),
                    (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
                };
                Self::send_one(Self::mouse_input(0, 0, data, flag))?;
            }
            InputEvent::MouseScroll { dx, dy } => {
                if dy != 0 {
                    Self::send_one(Self::mouse_input(0, 0, dy as u32, MOUSEEVENTF_WHEEL))?;
                }
                if dx != 0 {
                    Self::send_one(Self::mouse_input(0, 0, dx as u32, MOUSEEVENTF_HWHEEL))?;
                }
            }
            InputEvent::Key { code, state } => {
                let mut flags = match state {
                    KeyState::Press | KeyState::Repeat => 0u32,
                    KeyState::Release => KEYEVENTF_KEYUP,
                };

                let (vk, scan) = if code == KEY_PAUSE {
                    (VK_PAUSE, 0u16)
                } else {
                    let Some(wk) = evdev_to_win(code) else {
                        trace!(code, "skipping unmapped evdev key on Windows");
                        return Ok(());
                    };
                    flags |= KEYEVENTF_SCANCODE;
                    if wk.extended {
                        flags |= KEYEVENTF_EXTENDEDKEY;
                    }
                    (0u16, wk.scan_code)
                };

                let mut input: INPUT = unsafe { zeroed() };
                input.r#type = INPUT_KEYBOARD;
                input.Anonymous.ki = KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                };
                Self::send_one(input)?;
            }
        }
        Ok(())
    }
}
