//! Client-side held key and mouse button tracker to prevent stuck keys.

use std::collections::HashSet;

use anyhow::Result;
use deskglide_proto::{keycode::is_modifier, InputEvent, KeyState, MouseButton};

use crate::InputInjector;

/// Tracks currently depressed keys and mouse buttons on the client so they can
/// all be cleanly released on switch-back, disconnect, or heartbeat timeout.
#[derive(Debug, Default)]
pub struct HeldStateTracker {
    keys: HashSet<u16>,
    buttons: HashSet<MouseButton>,
}

impl HeldStateTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject `event` while keeping held state synchronized.
    pub fn apply(&mut self, injector: &mut dyn InputInjector, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::Key { code, state } => {
                if state.is_down() {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
            }
            InputEvent::MouseButton { button, pressed } => {
                if pressed {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
            }
            InputEvent::MouseMove { .. } | InputEvent::MouseScroll { .. } => {}
        }
        injector.inject(event)
    }

    /// Synthesize release events for every currently held mouse button and key.
    /// Releases mouse buttons first, then normal keys, then modifier keys last.
    pub fn release_all(&mut self, injector: &mut dyn InputInjector) -> Result<()> {
        let mut first_err: Option<anyhow::Error> = None;

        let buttons: Vec<MouseButton> = self.buttons.drain().collect();
        for button in buttons {
            if let Err(e) = injector.inject(InputEvent::MouseButton {
                button,
                pressed: false,
            }) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }

        let all_keys: Vec<u16> = self.keys.drain().collect();
        let (mods, non_mods): (Vec<u16>, Vec<u16>) =
            all_keys.into_iter().partition(|&c| is_modifier(c));

        for code in non_mods.into_iter().chain(mods) {
            if let Err(e) = injector.inject(InputEvent::Key {
                code,
                state: KeyState::Release,
            }) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskglide_proto::keycode;

    #[derive(Default)]
    struct MockInjector {
        events: Vec<InputEvent>,
    }

    impl InputInjector for MockInjector {
        fn inject(&mut self, event: InputEvent) -> Result<()> {
            self.events.push(event);
            Ok(())
        }
    }

    #[test]
    fn releases_held_buttons_normal_keys_then_modifiers() {
        let mut tracker = HeldStateTracker::new();
        let mut inj = MockInjector::default();

        tracker
            .apply(&mut inj, InputEvent::Key { code: keycode::KEY_LEFTCTRL, state: KeyState::Press })
            .unwrap();
        tracker
            .apply(&mut inj, InputEvent::Key { code: keycode::KEY_C, state: KeyState::Press })
            .unwrap();
        tracker
            .apply(&mut inj, InputEvent::Key { code: keycode::KEY_A, state: KeyState::Press })
            .unwrap();
        tracker
            .apply(&mut inj, InputEvent::Key { code: keycode::KEY_A, state: KeyState::Release })
            .unwrap();
        tracker
            .apply(&mut inj, InputEvent::MouseButton { button: MouseButton::Left, pressed: true })
            .unwrap();

        inj.events.clear();
        tracker.release_all(&mut inj).unwrap();

        assert!(tracker.is_empty());
        assert_eq!(
            inj.events,
            vec![
                InputEvent::MouseButton { button: MouseButton::Left, pressed: false },
                InputEvent::Key { code: keycode::KEY_C, state: KeyState::Release },
                InputEvent::Key { code: keycode::KEY_LEFTCTRL, state: KeyState::Release },
            ]
        );
    }
}
