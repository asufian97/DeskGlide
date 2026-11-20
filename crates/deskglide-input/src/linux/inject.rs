//! Linux `uinput` virtual input injection (works identically on Wayland and X11).

use anyhow::{Context, Result};
use deskglide_proto::{InputEvent, KeyState, MouseButton, WHEEL_DETENT};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent as EvInputEvent, KeyCode, RelativeAxisCode};

use crate::InputInjector;

/// Uses TWO separate virtual devices (keyboard + mouse) so udev and libinput
/// classify both cleanly under Wayland (GNOME/Mutter on Zorin OS) and X11.
pub struct LinuxInjector {
    keyboard: VirtualDevice,
    mouse: VirtualDevice,
    scroll_v_acc: i32,
    scroll_h_acc: i32,
}

impl LinuxInjector {
    pub fn new() -> Result<Self> {
        let mut kbd_keys = AttributeSet::<KeyCode>::new();
        for code in 1u16..=248 {
            kbd_keys.insert(KeyCode::new(code));
        }

        let keyboard = VirtualDevice::builder()
            .context("failed to open /dev/uinput (run scripts/linux/setup-uinput.sh once)")?
            .name("deskglide-virtual-keyboard")
            .with_keys(&kbd_keys)?
            .build()
            .context("failed to create uinput virtual keyboard")?;

        let mut mouse_buttons = AttributeSet::<KeyCode>::new();
        for b in MouseButton::ALL {
            mouse_buttons.insert(KeyCode::new(b.evdev_code()));
        }

        let mut rel_axes = AttributeSet::<RelativeAxisCode>::new();
        rel_axes.insert(RelativeAxisCode::REL_X);
        rel_axes.insert(RelativeAxisCode::REL_Y);
        rel_axes.insert(RelativeAxisCode::REL_WHEEL);
        rel_axes.insert(RelativeAxisCode::REL_HWHEEL);
        rel_axes.insert(RelativeAxisCode::REL_WHEEL_HI_RES);
        rel_axes.insert(RelativeAxisCode::REL_HWHEEL_HI_RES);

        let mouse = VirtualDevice::builder()
            .context("failed to open /dev/uinput for mouse")?
            .name("deskglide-virtual-mouse")
            .with_keys(&mouse_buttons)?
            .with_relative_axes(&rel_axes)?
            .build()
            .context("failed to create uinput virtual mouse")?;

        Ok(Self {
            keyboard,
            mouse,
            scroll_v_acc: 0,
            scroll_h_acc: 0,
        })
    }
}

impl InputInjector for LinuxInjector {
    fn inject(&mut self, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::MouseMove { dx, dy } => {
                let mut evs = Vec::with_capacity(2);
                if dx != 0 {
                    evs.push(EvInputEvent::new(
                        EventType::RELATIVE.0,
                        RelativeAxisCode::REL_X.0,
                        dx,
                    ));
                }
                if dy != 0 {
                    evs.push(EvInputEvent::new(
                        EventType::RELATIVE.0,
                        RelativeAxisCode::REL_Y.0,
                        dy,
                    ));
                }
                if !evs.is_empty() {
                    self.mouse.emit(&evs)?;
                }
            }
            InputEvent::MouseButton { button, pressed } => {
                let ev = EvInputEvent::new(
                    EventType::KEY.0,
                    button.evdev_code(),
                    i32::from(pressed),
                );
                self.mouse.emit(&[ev])?;
            }
            InputEvent::MouseScroll { dx, dy } => {
                let mut evs = Vec::with_capacity(4);
                if dy != 0 {
                    evs.push(EvInputEvent::new(
                        EventType::RELATIVE.0,
                        RelativeAxisCode::REL_WHEEL_HI_RES.0,
                        dy,
                    ));
                    self.scroll_v_acc += dy;
                    let notches = self.scroll_v_acc / WHEEL_DETENT;
                    if notches != 0 {
                        self.scroll_v_acc -= notches * WHEEL_DETENT;
                        evs.push(EvInputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_WHEEL.0,
                            notches,
                        ));
                    }
                }
                if dx != 0 {
                    evs.push(EvInputEvent::new(
                        EventType::RELATIVE.0,
                        RelativeAxisCode::REL_HWHEEL_HI_RES.0,
                        dx,
                    ));
                    self.scroll_h_acc += dx;
                    let notches = self.scroll_h_acc / WHEEL_DETENT;
                    if notches != 0 {
                        self.scroll_h_acc -= notches * WHEEL_DETENT;
                        evs.push(EvInputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_HWHEEL.0,
                            notches,
                        ));
                    }
                }
                if !evs.is_empty() {
                    self.mouse.emit(&evs)?;
                }
            }
            InputEvent::Key { code, state } => {
                // On Linux Wayland/X11, holding a key down triggers compositor auto-repeat
                // natively from Press (1) until Release (0).
                let val = match state {
                    KeyState::Press => 1,
                    KeyState::Release => 0,
                    KeyState::Repeat => return Ok(()),
                };
                let ev = EvInputEvent::new(EventType::KEY.0, code, val);
                self.keyboard.emit(&[ev])?;
            }
        }
        Ok(())
    }
}
