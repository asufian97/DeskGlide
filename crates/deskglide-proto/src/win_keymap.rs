//! Bidirectional translation between Windows PS/2 Set-1 scan codes (+ VK hints)
//! and canonical Linux evdev key codes.

use crate::keycode::*;

/// Windows Set-1 scan code pair used by `SendInput` (`wScan` + `KEYEVENTF_EXTENDEDKEY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinKey {
    pub scan_code: u16,
    pub extended: bool,
}

const VK_PAUSE: u32 = 0x13;
const VK_SNAPSHOT: u32 = 0x2C;
const VK_NUMLOCK: u32 = 0x90;
const VK_RSHIFT: u32 = 0xA1;
const VK_VOLUME_MUTE: u32 = 0xAD;
const VK_VOLUME_DOWN: u32 = 0xAE;
const VK_VOLUME_UP: u32 = 0xAF;
const VK_MEDIA_NEXT_TRACK: u32 = 0xB0;
const VK_MEDIA_PREV_TRACK: u32 = 0xB1;
const VK_MEDIA_STOP: u32 = 0xB2;
const VK_MEDIA_PLAY_PAUSE: u32 = 0xB3;

/// Base Set-1 scan codes (`0x01..=0x58`) -> Linux evdev key codes.
/// For indices `0x01..=0x53`, Linux evdev codes happen to equal the Set-1 scan code
/// (except `0x45` Pause/NumLock and `0x54` Alt+SysRq).
fn base_set1_to_evdev(scan_code: u16) -> Option<u16> {
    Some(match scan_code {
        0x01..=0x44 => scan_code,
        0x45 => KEY_PAUSE,
        0x46..=0x53 => scan_code,
        0x54 => KEY_SYSRQ,
        0x56 => KEY_102ND,
        0x57 => KEY_F11,
        0x58 => KEY_F12,
        0x59 => KEY_KPEQUAL,
        0x64 => KEY_F13,
        0x65 => KEY_F14,
        0x66 => KEY_F15,
        0x67 => KEY_F16,
        0x68 => KEY_F17,
        0x69 => KEY_F18,
        0x6A => KEY_F19,
        0x6B => KEY_F20,
        0x6C => KEY_F21,
        0x6D => KEY_F22,
        0x6E => KEY_F23,
        0x70 => KEY_KATAKANAHIRAGANA,
        0x73 => KEY_RO,
        0x76 => KEY_F24,
        0x79 => KEY_HENKAN,
        0x7B => KEY_MUHENKAN,
        0x7D => KEY_YEN,
        0x7E => KEY_KPCOMMA,
        0xF1 => KEY_HANJA,
        0xF2 => KEY_HANGEUL,
        _ => return None,
    })
}

fn extended_set1_to_evdev(scan_code: u16) -> Option<u16> {
    Some(match scan_code {
        0x10 => KEY_PREVIOUSSONG,
        0x19 => KEY_NEXTSONG,
        0x1C => KEY_KPENTER,
        0x1D => KEY_RIGHTCTRL,
        0x20 => KEY_MUTE,
        0x21 => KEY_CALC,
        0x22 => KEY_PLAYPAUSE,
        0x24 => KEY_STOPCD,
        0x2E => KEY_VOLUMEDOWN,
        0x30 => KEY_VOLUMEUP,
        0x32 => KEY_HOMEPAGE,
        0x35 => KEY_KPSLASH,
        0x37 => KEY_SYSRQ,
        0x38 => KEY_RIGHTALT,
        0x45 => KEY_NUMLOCK,
        0x46 => KEY_PAUSE, // Ctrl+Pause (Break)
        0x47 => KEY_HOME,
        0x48 => KEY_UP,
        0x49 => KEY_PAGEUP,
        0x4B => KEY_LEFT,
        0x4D => KEY_RIGHT,
        0x4F => KEY_END,
        0x50 => KEY_DOWN,
        0x51 => KEY_PAGEDOWN,
        0x52 => KEY_INSERT,
        0x53 => KEY_DELETE,
        0x5B => KEY_LEFTMETA,
        0x5C => KEY_RIGHTMETA,
        0x5D => KEY_COMPOSE,
        0x5E => KEY_POWER,
        0x5F => KEY_SLEEP,
        0x63 => KEY_WAKEUP,
        0x65 => KEY_SEARCH,
        0x66 => KEY_BOOKMARKS,
        0x67 => KEY_REFRESH,
        0x68 => KEY_STOP,
        0x69 => KEY_FORWARD,
        0x6A => KEY_BACK,
        0x6B => KEY_COMPUTER,
        0x6C => KEY_MAIL,
        0x6D => KEY_MEDIA,
        _ => return None,
    })
}

/// Convert a Windows `WH_KEYBOARD_LL` / Raw Input `(scan_code, extended, vk_code)`
/// triple to a Linux evdev key code.
pub fn win_to_evdev(scan_code: u16, extended: bool, vk_code: u32) -> Option<u16> {
    // Disambiguate Windows keys whose scan code / extended flag is shared or quirky.
    match vk_code {
        VK_PAUSE => return Some(KEY_PAUSE),
        VK_NUMLOCK => return Some(KEY_NUMLOCK),
        VK_SNAPSHOT => return Some(KEY_SYSRQ),
        VK_RSHIFT => return Some(KEY_RIGHTSHIFT),
        _ => {}
    }

    if scan_code == 0 {
        return match vk_code {
            VK_VOLUME_MUTE => Some(KEY_MUTE),
            VK_VOLUME_DOWN => Some(KEY_VOLUMEDOWN),
            VK_VOLUME_UP => Some(KEY_VOLUMEUP),
            VK_MEDIA_NEXT_TRACK => Some(KEY_NEXTSONG),
            VK_MEDIA_PREV_TRACK => Some(KEY_PREVIOUSSONG),
            VK_MEDIA_STOP => Some(KEY_STOPCD),
            VK_MEDIA_PLAY_PAUSE => Some(KEY_PLAYPAUSE),
            _ => None,
        };
    }

    if extended {
        extended_set1_to_evdev(scan_code)
    } else {
        base_set1_to_evdev(scan_code)
    }
}

/// Convert a canonical Linux evdev key code to a Windows Set-1 `(scan_code, extended)` pair.
pub fn evdev_to_win(code: u16) -> Option<WinKey> {
    let wk = match code {
        1..=68 => WinKey { scan_code: code, extended: false }, // Esc..F10 (incl. RShift 0x36 non-extended)
        KEY_NUMLOCK => WinKey { scan_code: 0x45, extended: true },
        70..=83 => WinKey { scan_code: code, extended: false }, // ScrollLock..KP Dot
        KEY_102ND => WinKey { scan_code: 0x56, extended: false },
        KEY_F11 => WinKey { scan_code: 0x57, extended: false },
        KEY_F12 => WinKey { scan_code: 0x58, extended: false },
        KEY_KPEQUAL => WinKey { scan_code: 0x59, extended: false },
        KEY_F13 => WinKey { scan_code: 0x64, extended: false },
        KEY_F14 => WinKey { scan_code: 0x65, extended: false },
        KEY_F15 => WinKey { scan_code: 0x66, extended: false },
        KEY_F16 => WinKey { scan_code: 0x67, extended: false },
        KEY_F17 => WinKey { scan_code: 0x68, extended: false },
        KEY_F18 => WinKey { scan_code: 0x69, extended: false },
        KEY_F19 => WinKey { scan_code: 0x6A, extended: false },
        KEY_F20 => WinKey { scan_code: 0x6B, extended: false },
        KEY_F21 => WinKey { scan_code: 0x6C, extended: false },
        KEY_F22 => WinKey { scan_code: 0x6D, extended: false },
        KEY_F23 => WinKey { scan_code: 0x6E, extended: false },
        KEY_F24 => WinKey { scan_code: 0x76, extended: false },
        KEY_KATAKANAHIRAGANA => WinKey { scan_code: 0x70, extended: false },
        KEY_RO => WinKey { scan_code: 0x73, extended: false },
        KEY_HENKAN => WinKey { scan_code: 0x79, extended: false },
        KEY_MUHENKAN => WinKey { scan_code: 0x7B, extended: false },
        KEY_YEN => WinKey { scan_code: 0x7D, extended: false },
        KEY_KPCOMMA => WinKey { scan_code: 0x7E, extended: false },
        KEY_HANJA => WinKey { scan_code: 0xF1, extended: false },
        KEY_HANGEUL => WinKey { scan_code: 0xF2, extended: false },

        KEY_PREVIOUSSONG => WinKey { scan_code: 0x10, extended: true },
        KEY_NEXTSONG => WinKey { scan_code: 0x19, extended: true },
        KEY_KPENTER => WinKey { scan_code: 0x1C, extended: true },
        KEY_RIGHTCTRL => WinKey { scan_code: 0x1D, extended: true },
        KEY_MUTE => WinKey { scan_code: 0x20, extended: true },
        KEY_CALC => WinKey { scan_code: 0x21, extended: true },
        KEY_PLAYPAUSE => WinKey { scan_code: 0x22, extended: true },
        KEY_STOPCD => WinKey { scan_code: 0x24, extended: true },
        KEY_VOLUMEDOWN => WinKey { scan_code: 0x2E, extended: true },
        KEY_VOLUMEUP => WinKey { scan_code: 0x30, extended: true },
        KEY_HOMEPAGE => WinKey { scan_code: 0x32, extended: true },
        KEY_KPSLASH => WinKey { scan_code: 0x35, extended: true },
        KEY_SYSRQ => WinKey { scan_code: 0x37, extended: true },
        KEY_RIGHTALT => WinKey { scan_code: 0x38, extended: true },
        KEY_PAUSE => WinKey { scan_code: 0x45, extended: false },
        KEY_HOME => WinKey { scan_code: 0x47, extended: true },
        KEY_UP => WinKey { scan_code: 0x48, extended: true },
        KEY_PAGEUP => WinKey { scan_code: 0x49, extended: true },
        KEY_LEFT => WinKey { scan_code: 0x4B, extended: true },
        KEY_RIGHT => WinKey { scan_code: 0x4D, extended: true },
        KEY_END => WinKey { scan_code: 0x4F, extended: true },
        KEY_DOWN => WinKey { scan_code: 0x50, extended: true },
        KEY_PAGEDOWN => WinKey { scan_code: 0x51, extended: true },
        KEY_INSERT => WinKey { scan_code: 0x52, extended: true },
        KEY_DELETE => WinKey { scan_code: 0x53, extended: true },
        KEY_LEFTMETA => WinKey { scan_code: 0x5B, extended: true },
        KEY_RIGHTMETA => WinKey { scan_code: 0x5C, extended: true },
        KEY_COMPOSE => WinKey { scan_code: 0x5D, extended: true },
        KEY_POWER => WinKey { scan_code: 0x5E, extended: true },
        KEY_SLEEP => WinKey { scan_code: 0x5F, extended: true },
        KEY_WAKEUP => WinKey { scan_code: 0x63, extended: true },
        KEY_SEARCH => WinKey { scan_code: 0x65, extended: true },
        KEY_BOOKMARKS => WinKey { scan_code: 0x66, extended: true },
        KEY_REFRESH => WinKey { scan_code: 0x67, extended: true },
        KEY_STOP => WinKey { scan_code: 0x68, extended: true },
        KEY_FORWARD => WinKey { scan_code: 0x69, extended: true },
        KEY_BACK => WinKey { scan_code: 0x6A, extended: true },
        KEY_COMPUTER => WinKey { scan_code: 0x6B, extended: true },
        KEY_MAIL => WinKey { scan_code: 0x6C, extended: true },
        KEY_MEDIA => WinKey { scan_code: 0x6D, extended: true },
        _ => return None,
    };
    Some(wk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_standard_and_extended_keys() {
        let codes = [
            KEY_ESC, KEY_1, KEY_0, KEY_Q, KEY_A, KEY_Z, KEY_S, KEY_ENTER, KEY_SPACE, KEY_TAB,
            KEY_BACKSPACE, KEY_CAPSLOCK, KEY_F1, KEY_F10, KEY_F11, KEY_F12, KEY_F13, KEY_F24,
            KEY_LEFTCTRL, KEY_RIGHTCTRL, KEY_LEFTSHIFT, KEY_RIGHTSHIFT, KEY_LEFTALT, KEY_RIGHTALT,
            KEY_LEFTMETA, KEY_RIGHTMETA, KEY_COMPOSE,
            KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_HOME, KEY_END, KEY_PAGEUP, KEY_PAGEDOWN,
            KEY_INSERT, KEY_DELETE, KEY_SYSRQ, KEY_SCROLLLOCK, KEY_NUMLOCK, KEY_PAUSE,
            KEY_KP0, KEY_KP9, KEY_KPDOT, KEY_KPENTER, KEY_KPSLASH, KEY_KPASTERISK, KEY_KPEQUAL,
            KEY_102ND, KEY_MUTE, KEY_VOLUMEDOWN, KEY_VOLUMEUP, KEY_PLAYPAUSE, KEY_NEXTSONG,
            KEY_PREVIOUSSONG,
        ];

        for code in codes {
            let wk = evdev_to_win(code).unwrap_or_else(|| panic!("missing WinKey for {code}"));
            let back = win_to_evdev(wk.scan_code, wk.extended, 0)
                .unwrap_or_else(|| panic!("failed reverse map for {code} -> {wk:?}"));
            assert_eq!(back, code, "mismatch for key {code}");
        }
    }

    #[test]
    fn special_disambiguation_rules() {
        // Right shift must be non-extended 0x36.
        assert_eq!(evdev_to_win(KEY_RIGHTSHIFT), Some(WinKey { scan_code: 0x36, extended: false }));
        assert_eq!(win_to_evdev(0x36, false, VK_RSHIFT), Some(KEY_RIGHTSHIFT));
        assert_eq!(win_to_evdev(0x36, true, VK_RSHIFT), Some(KEY_RIGHTSHIFT));

        // NumLock is extended 0x45; Pause is non-extended 0x45 (or VK_PAUSE).
        assert_eq!(evdev_to_win(KEY_NUMLOCK), Some(WinKey { scan_code: 0x45, extended: true }));
        assert_eq!(win_to_evdev(0x45, true, VK_NUMLOCK), Some(KEY_NUMLOCK));
        assert_eq!(win_to_evdev(0x45, false, VK_PAUSE), Some(KEY_PAUSE));
        assert_eq!(win_to_evdev(0x46, true, VK_PAUSE), Some(KEY_PAUSE));
        assert_eq!(win_to_evdev(0x00, false, VK_PAUSE), Some(KEY_PAUSE));

        // PrintScreen (VK_SNAPSHOT)
        assert_eq!(win_to_evdev(0x37, true, VK_SNAPSHOT), Some(KEY_SYSRQ));
        assert_eq!(win_to_evdev(0x54, false, VK_SNAPSHOT), Some(KEY_SYSRQ));

        // Media VK fallback when scan_code == 0
        assert_eq!(win_to_evdev(0, true, VK_VOLUME_UP), Some(KEY_VOLUMEUP));
        assert_eq!(win_to_evdev(0, false, VK_MEDIA_PLAY_PAUSE), Some(KEY_PLAYPAUSE));
    }
}
