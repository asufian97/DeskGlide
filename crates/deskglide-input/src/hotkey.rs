//! Safe toggle-hotkey state machine with modifier-release guard.

use std::collections::HashSet;
use std::fmt;

use anyhow::{bail, Result};
use deskglide_proto::keycode::{self, is_alt, is_ctrl, is_meta, is_shift};
use deskglide_proto::KeyState;

/// A keyboard hotkey combination (modifiers + trigger key in Linux evdev codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: u16,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: false,
            meta: false,
            key: keycode::KEY_S,
        }
    }
}

impl Hotkey {
    /// Parse strings like `"Ctrl+Alt+S"`, `"Ctrl+Shift+F12"`, `"Super+Alt+ScrollLock"`.
    pub fn parse(spec: &str) -> Result<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut meta = false;
        let mut trigger: Option<u16> = None;

        for raw in spec.split('+') {
            let token = raw.trim();
            if token.is_empty() {
                bail!("invalid empty token in hotkey '{spec}'");
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "win" | "super" | "meta" | "cmd" => meta = true,
                _ => {
                    if trigger.is_some() {
                        bail!("multiple trigger keys in hotkey '{spec}'");
                    }
                    let code = keycode::from_name(token)
                        .ok_or_else(|| anyhow::anyhow!("unknown key '{token}' in hotkey '{spec}'"))?;
                    trigger = Some(code);
                }
            }
        }

        let key = trigger.ok_or_else(|| anyhow::anyhow!("missing trigger key in hotkey '{spec}'"))?;
        Ok(Self {
            ctrl,
            alt,
            shift,
            meta,
            key,
        })
    }

    fn matches_modifiers(&self, held: &HashSet<u16>) -> bool {
        let has_ctrl = held.iter().copied().any(is_ctrl);
        let has_alt = held.iter().copied().any(is_alt);
        let has_shift = held.iter().copied().any(is_shift);
        let has_meta = held.iter().copied().any(is_meta);
        has_ctrl == self.ctrl && has_alt == self.alt && has_shift == self.shift && has_meta == self.meta
    }

    fn is_required_modifier(&self, code: u16) -> bool {
        (self.ctrl && is_ctrl(code))
            || (self.alt && is_alt(code))
            || (self.shift && is_shift(code))
            || (self.meta && is_meta(code))
    }

    fn has_any_held_hotkey_key(&self, held: &HashSet<u16>) -> bool {
        held.contains(&self.key) || held.iter().copied().any(|k| self.is_required_modifier(k))
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.meta {
            parts.push("Super".to_string());
        }
        parts.push(format!("key({})", self.key));
        f.write_str(&parts.join("+"))
    }
}

/// Output of [`HotkeyStateMachine::on_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HotkeyDecision {
    /// True if the backend should swallow this key event instead of letting it pass/forward.
    pub swallow: bool,
    /// Present when the active control state transitioned (`true` = Remote, `false` = Local).
    pub new_remote_active: Option<bool>,
}

/// Tracks key presses/releases to safely switch between Local and Remote modes
/// without leaking stuck modifier keys on either machine.
#[derive(Debug)]
pub struct HotkeyStateMachine {
    hotkey: Hotkey,
    held_keys: HashSet<u16>,
    remote_active: bool,
    /// Armed while waiting for user to physically release hotkey keys before activating Remote.
    pending_activate: bool,
    /// Suppresses physical release events of the hotkey keys right after deactivating Remote.
    draining_deactivate: bool,
}

impl HotkeyStateMachine {
    pub fn new(hotkey: Hotkey) -> Self {
        Self {
            hotkey,
            held_keys: HashSet::new(),
            remote_active: false,
            pending_activate: false,
            draining_deactivate: false,
        }
    }

    pub fn is_remote_active(&self) -> bool {
        self.remote_active
    }

    /// Externally synchronize active state (e.g., on client disconnect).
    pub fn set_remote_active(&mut self, active: bool) {
        self.remote_active = active;
        self.pending_activate = false;
        self.draining_deactivate = false;
        if !active {
            self.held_keys.clear();
        }
    }

    /// Process a raw key transition and decide whether to swallow/toggle.
    pub fn on_key(&mut self, code: u16, state: KeyState) -> HotkeyDecision {
        match state {
            KeyState::Press => {
                let first_press = self.held_keys.insert(code);

                if self.pending_activate {
                    return HotkeyDecision {
                        swallow: true,
                        new_remote_active: None,
                    };
                }

                if first_press && code == self.hotkey.key && self.hotkey.matches_modifiers(&self.held_keys) {
                    if !self.remote_active {
                        // Local -> Remote: wait until hotkey keys are released so neither host nor
                        // remote sees half-held modifiers after switching.
                        self.pending_activate = true;
                        return HotkeyDecision {
                            swallow: true,
                            new_remote_active: None,
                        };
                    } else {
                        // Remote -> Local: switch immediately and swallow remaining hotkey releases.
                        self.remote_active = false;
                        self.draining_deactivate = true;
                        return HotkeyDecision {
                            swallow: true,
                            new_remote_active: Some(false),
                        };
                    }
                }

                HotkeyDecision {
                    swallow: self.remote_active,
                    new_remote_active: None,
                }
            }
            KeyState::Repeat => {
                if self.pending_activate || self.draining_deactivate {
                    return HotkeyDecision {
                        swallow: true,
                        new_remote_active: None,
                    };
                }
                HotkeyDecision {
                    swallow: self.remote_active,
                    new_remote_active: None,
                }
            }
            KeyState::Release => {
                self.held_keys.remove(&code);

                if self.pending_activate {
                    let done = !self.hotkey.has_any_held_hotkey_key(&self.held_keys);
                    if done {
                        self.pending_activate = false;
                        self.remote_active = true;
                        return HotkeyDecision {
                            swallow: true,
                            new_remote_active: Some(true),
                        };
                    }
                    return HotkeyDecision {
                        swallow: true,
                        new_remote_active: None,
                    };
                }

                if self.draining_deactivate {
                    let is_hotkey_part =
                        code == self.hotkey.key || self.hotkey.is_required_modifier(code);
                    if !self.hotkey.has_any_held_hotkey_key(&self.held_keys) {
                        self.draining_deactivate = false;
                    }
                    return HotkeyDecision {
                        swallow: is_hotkey_part,
                        new_remote_active: None,
                    };
                }

                HotkeyDecision {
                    swallow: self.remote_active,
                    new_remote_active: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkeys() {
        let def = Hotkey::parse("Ctrl+Alt+S").unwrap();
        assert_eq!(def, Hotkey::default());

        let custom = Hotkey::parse("super + shift + F12").unwrap();
        assert!(custom.meta && custom.shift && !custom.ctrl && !custom.alt);
        assert_eq!(custom.key, keycode::KEY_F12);

        assert!(Hotkey::parse("Ctrl+Alt").is_err());
        assert!(Hotkey::parse("Ctrl+A+B").is_err());
        assert!(Hotkey::parse("Ctrl+Bogus").is_err());
    }

    #[test]
    fn toggle_to_remote_waits_for_hotkey_release() {
        let mut sm = HotkeyStateMachine::new(Hotkey::default());

        // Ctrl down, Alt down (pass through locally)
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTCTRL, KeyState::Press),
            HotkeyDecision { swallow: false, new_remote_active: None }
        );
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTALT, KeyState::Press),
            HotkeyDecision { swallow: false, new_remote_active: None }
        );
        // S down -> swallowed, pending activation
        assert_eq!(
            sm.on_key(keycode::KEY_S, KeyState::Press),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        assert!(!sm.is_remote_active());

        // Release S, Ctrl (still waiting for Alt)
        assert_eq!(
            sm.on_key(keycode::KEY_S, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTCTRL, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        // Release Alt -> activates Remote!
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTALT, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: Some(true) }
        );
        assert!(sm.is_remote_active());
    }

    #[test]
    fn toggle_back_to_local_switches_immediately_and_drains_releases() {
        let mut sm = HotkeyStateMachine::new(Hotkey::default());
        sm.set_remote_active(true);

        assert_eq!(
            sm.on_key(keycode::KEY_RIGHTCTRL, KeyState::Press),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTALT, KeyState::Press),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        // Pressing S immediately returns to Local
        assert_eq!(
            sm.on_key(keycode::KEY_S, KeyState::Press),
            HotkeyDecision { swallow: true, new_remote_active: Some(false) }
        );
        assert!(!sm.is_remote_active());

        // Hotkey key releases are swallowed so local OS doesn't see stray ups
        assert_eq!(
            sm.on_key(keycode::KEY_S, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        assert_eq!(
            sm.on_key(keycode::KEY_LEFTALT, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );
        assert_eq!(
            sm.on_key(keycode::KEY_RIGHTCTRL, KeyState::Release),
            HotkeyDecision { swallow: true, new_remote_active: None }
        );

        // Next normal key passes to local
        assert_eq!(
            sm.on_key(keycode::KEY_A, KeyState::Press),
            HotkeyDecision { swallow: false, new_remote_active: None }
        );
    }

    #[test]
    fn extra_modifiers_do_not_trigger_hotkey() {
        let mut sm = HotkeyStateMachine::new(Hotkey::default());
        sm.on_key(keycode::KEY_LEFTCTRL, KeyState::Press);
        sm.on_key(keycode::KEY_LEFTALT, KeyState::Press);
        sm.on_key(keycode::KEY_LEFTSHIFT, KeyState::Press);
        let d = sm.on_key(keycode::KEY_S, KeyState::Press);
        assert_eq!(d, HotkeyDecision { swallow: false, new_remote_active: None });
    }
}
