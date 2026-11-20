//! Linux evdev key codes (the canonical key namespace on the wire) and helpers.
#![allow(missing_docs)]

pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_W: u16 = 17;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_SEMICOLON: u16 = 39;
pub const KEY_APOSTROPHE: u16 = 40;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_COMMA: u16 = 51;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_KPASTERISK: u16 = 55;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_F1: u16 = 59;
pub const KEY_F2: u16 = 60;
pub const KEY_F3: u16 = 61;
pub const KEY_F4: u16 = 62;
pub const KEY_F5: u16 = 63;
pub const KEY_F6: u16 = 64;
pub const KEY_F7: u16 = 65;
pub const KEY_F8: u16 = 66;
pub const KEY_F9: u16 = 67;
pub const KEY_F10: u16 = 68;
pub const KEY_NUMLOCK: u16 = 69;
pub const KEY_SCROLLLOCK: u16 = 70;
pub const KEY_KP7: u16 = 71;
pub const KEY_KP8: u16 = 72;
pub const KEY_KP9: u16 = 73;
pub const KEY_KPMINUS: u16 = 74;
pub const KEY_KP4: u16 = 75;
pub const KEY_KP5: u16 = 76;
pub const KEY_KP6: u16 = 77;
pub const KEY_KPPLUS: u16 = 78;
pub const KEY_KP1: u16 = 79;
pub const KEY_KP2: u16 = 80;
pub const KEY_KP3: u16 = 81;
pub const KEY_KP0: u16 = 82;
pub const KEY_KPDOT: u16 = 83;
pub const KEY_ZENKAKUHANKAKU: u16 = 85;
pub const KEY_102ND: u16 = 86;
pub const KEY_F11: u16 = 87;
pub const KEY_F12: u16 = 88;
pub const KEY_RO: u16 = 89;
pub const KEY_HENKAN: u16 = 92;
pub const KEY_KATAKANAHIRAGANA: u16 = 93;
pub const KEY_MUHENKAN: u16 = 94;
pub const KEY_KPENTER: u16 = 96;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_KPSLASH: u16 = 98;
pub const KEY_SYSRQ: u16 = 99;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;
pub const KEY_MUTE: u16 = 113;
pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_VOLUMEUP: u16 = 115;
pub const KEY_POWER: u16 = 116;
pub const KEY_KPEQUAL: u16 = 117;
pub const KEY_PAUSE: u16 = 119;
pub const KEY_KPCOMMA: u16 = 121;
pub const KEY_HANGEUL: u16 = 122;
pub const KEY_HANJA: u16 = 123;
pub const KEY_YEN: u16 = 124;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;
pub const KEY_COMPOSE: u16 = 127;
pub const KEY_STOP: u16 = 128;
pub const KEY_CALC: u16 = 140;
pub const KEY_SLEEP: u16 = 142;
pub const KEY_WAKEUP: u16 = 143;
pub const KEY_MAIL: u16 = 155;
pub const KEY_BOOKMARKS: u16 = 156;
pub const KEY_COMPUTER: u16 = 157;
pub const KEY_BACK: u16 = 158;
pub const KEY_FORWARD: u16 = 159;
pub const KEY_NEXTSONG: u16 = 163;
pub const KEY_PLAYPAUSE: u16 = 164;
pub const KEY_PREVIOUSSONG: u16 = 165;
pub const KEY_STOPCD: u16 = 166;
pub const KEY_HOMEPAGE: u16 = 172;
pub const KEY_REFRESH: u16 = 173;
pub const KEY_F13: u16 = 183;
pub const KEY_F14: u16 = 184;
pub const KEY_F15: u16 = 185;
pub const KEY_F16: u16 = 186;
pub const KEY_F17: u16 = 187;
pub const KEY_F18: u16 = 188;
pub const KEY_F19: u16 = 189;
pub const KEY_F20: u16 = 190;
pub const KEY_F21: u16 = 191;
pub const KEY_F22: u16 = 192;
pub const KEY_F23: u16 = 193;
pub const KEY_F24: u16 = 194;
pub const KEY_SEARCH: u16 = 217;
pub const KEY_MEDIA: u16 = 226;

/// Highest code treated as a keyboard key (codes `0x100..` are buttons).
pub const KEY_MAX_KEYBOARD: u16 = 0xFF;

pub fn is_ctrl(code: u16) -> bool {
    code == KEY_LEFTCTRL || code == KEY_RIGHTCTRL
}
pub fn is_alt(code: u16) -> bool {
    code == KEY_LEFTALT || code == KEY_RIGHTALT
}
pub fn is_shift(code: u16) -> bool {
    code == KEY_LEFTSHIFT || code == KEY_RIGHTSHIFT
}
pub fn is_meta(code: u16) -> bool {
    code == KEY_LEFTMETA || code == KEY_RIGHTMETA
}
pub fn is_modifier(code: u16) -> bool {
    is_ctrl(code) || is_alt(code) || is_shift(code) || is_meta(code)
}

const LETTERS: [u16; 26] = [
    KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I, KEY_J, KEY_K, KEY_L, KEY_M,
    KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R, KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X, KEY_Y, KEY_Z,
];
const DIGITS: [u16; 10] = [KEY_0, KEY_1, KEY_2, KEY_3, KEY_4, KEY_5, KEY_6, KEY_7, KEY_8, KEY_9];
const FKEYS: [u16; 24] = [
    KEY_F1, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9, KEY_F10, KEY_F11,
    KEY_F12, KEY_F13, KEY_F14, KEY_F15, KEY_F16, KEY_F17, KEY_F18, KEY_F19, KEY_F20, KEY_F21,
    KEY_F22, KEY_F23, KEY_F24,
];

/// Parse a human key name (case-insensitive), e.g. `S`, `F12`, `ScrollLock`, `Pause`.
pub fn from_name(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    let b = n.as_bytes();
    if b.len() == 1 {
        let c = b[0];
        if c.is_ascii_lowercase() {
            return Some(LETTERS[(c - b'a') as usize]);
        }
        if c.is_ascii_digit() {
            return Some(DIGITS[(c - b'0') as usize]);
        }
    }
    if let Some(num) = n.strip_prefix('f') {
        if let Ok(i) = num.parse::<usize>() {
            if (1..=24).contains(&i) {
                return Some(FKEYS[i - 1]);
            }
        }
    }
    Some(match n.as_str() {
        "esc" | "escape" => KEY_ESC,
        "tab" => KEY_TAB,
        "space" => KEY_SPACE,
        "enter" | "return" => KEY_ENTER,
        "backspace" => KEY_BACKSPACE,
        "insert" | "ins" => KEY_INSERT,
        "delete" | "del" => KEY_DELETE,
        "home" => KEY_HOME,
        "end" => KEY_END,
        "pageup" | "pgup" => KEY_PAGEUP,
        "pagedown" | "pgdn" => KEY_PAGEDOWN,
        "up" => KEY_UP,
        "down" => KEY_DOWN,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "scrolllock" | "scroll" => KEY_SCROLLLOCK,
        "pause" | "break" => KEY_PAUSE,
        "printscreen" | "prtsc" | "print" | "sysrq" => KEY_SYSRQ,
        "capslock" => KEY_CAPSLOCK,
        "numlock" => KEY_NUMLOCK,
        "grave" | "`" | "backtick" => KEY_GRAVE,
        "minus" | "-" => KEY_MINUS,
        "equal" | "=" => KEY_EQUAL,
        "comma" | "," => KEY_COMMA,
        "period" | "dot" | "." => KEY_DOT,
        "slash" | "/" => KEY_SLASH,
        "backslash" | "\\" => KEY_BACKSLASH,
        "semicolon" | ";" => KEY_SEMICOLON,
        "apostrophe" | "quote" | "'" => KEY_APOSTROPHE,
        "leftbracket" | "[" => KEY_LEFTBRACE,
        "rightbracket" | "]" => KEY_RIGHTBRACE,
        "menu" | "apps" | "compose" => KEY_COMPOSE,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(from_name("s"), Some(KEY_S));
        assert_eq!(from_name("S"), Some(KEY_S));
        assert_eq!(from_name("z"), Some(KEY_Z));
        assert_eq!(from_name("0"), Some(KEY_0));
        assert_eq!(from_name("F12"), Some(KEY_F12));
        assert_eq!(from_name("f13"), Some(KEY_F13));
        assert_eq!(from_name("ScrollLock"), Some(KEY_SCROLLLOCK));
        assert_eq!(from_name("f25"), None);
        assert_eq!(from_name("nope"), None);
    }

    #[test]
    fn modifiers() {
        assert!(is_modifier(KEY_LEFTMETA) && is_modifier(KEY_RIGHTALT));
        assert!(!is_modifier(KEY_S));
    }
}
