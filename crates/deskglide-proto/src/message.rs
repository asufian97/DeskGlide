//! Compact binary message codec.
//!
//! Reliable-stream frames are `[u16 BE length][body]`; QUIC datagrams carry a bare
//! `body`. A body is `[u8 tag][payload]` with big-endian integers.

use std::fmt;

/// Maximum encoded body length accepted on the reliable stream (`u16::MAX`).
pub const MAX_FRAME_LEN: usize = 65_535;
/// Maximum UTF-8 byte length sent in a single `ClipboardText` message.
pub const MAX_CLIPBOARD_LEN: usize = 60_000;
const MAX_STR: usize = 255;

const TAG_HELLO: u8 = 0x01;
const TAG_AUTH_CHALLENGE: u8 = 0x02;
const TAG_AUTH_RESPONSE: u8 = 0x03;
const TAG_AUTH_RESULT: u8 = 0x04;
const TAG_MOUSE_MOVE: u8 = 0x10;
const TAG_MOUSE_BUTTON: u8 = 0x11;
const TAG_MOUSE_SCROLL: u8 = 0x12;
const TAG_KEY: u8 = 0x13;
const TAG_RELEASE_ALL: u8 = 0x20;
const TAG_CONTROL: u8 = 0x21;
const TAG_EDGE_RETURN: u8 = 0x22;
const TAG_CLIPBOARD_TEXT: u8 = 0x23;
const TAG_HEARTBEAT: u8 = 0x30;

/// Screen edge used for seamless mouse-edge switching between machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum ScreenEdge {
    #[default]
    None = 0,
    Left = 1,
    Right = 2,
    Top = 3,
    Bottom = 4,
}

impl ScreenEdge {
    pub const ALL: [ScreenEdge; 5] = [
        ScreenEdge::None,
        ScreenEdge::Left,
        ScreenEdge::Right,
        ScreenEdge::Top,
        ScreenEdge::Bottom,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => ScreenEdge::None,
            1 => ScreenEdge::Left,
            2 => ScreenEdge::Right,
            3 => ScreenEdge::Top,
            4 => ScreenEdge::Bottom,
            _ => return None,
        })
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "disabled" | "hotkey" => ScreenEdge::None,
            "left" => ScreenEdge::Left,
            "right" => ScreenEdge::Right,
            "top" | "up" => ScreenEdge::Top,
            "bottom" | "down" => ScreenEdge::Bottom,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenEdge::None => "none",
            ScreenEdge::Left => "left",
            ScreenEdge::Right => "right",
            ScreenEdge::Top => "top",
            ScreenEdge::Bottom => "bottom",
        }
    }

    /// Opposite edge on the remote peer (leaving Host Right enters Client Left).
    pub fn opposite(self) -> Self {
        match self {
            ScreenEdge::None => ScreenEdge::None,
            ScreenEdge::Left => ScreenEdge::Right,
            ScreenEdge::Right => ScreenEdge::Left,
            ScreenEdge::Top => ScreenEdge::Bottom,
            ScreenEdge::Bottom => ScreenEdge::Top,
        }
    }
}

impl fmt::Display for ScreenEdge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Mouse buttons supported on every platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
    /// "Back" / side button (Windows XBUTTON1, Linux BTN_SIDE).
    Back = 3,
    /// "Forward" / extra button (Windows XBUTTON2, Linux BTN_EXTRA).
    Forward = 4,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => MouseButton::Left,
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            3 => MouseButton::Back,
            4 => MouseButton::Forward,
            _ => return None,
        })
    }

    /// Linux evdev `BTN_*` code.
    pub fn evdev_code(self) -> u16 {
        match self {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
            MouseButton::Back => 0x113,
            MouseButton::Forward => 0x114,
        }
    }

    /// Parse a Linux evdev `BTN_*` code (`BTN_FORWARD`/`BTN_BACK` are folded in).
    pub fn from_evdev(code: u16) -> Option<Self> {
        Some(match code {
            0x110 => MouseButton::Left,
            0x111 => MouseButton::Right,
            0x112 => MouseButton::Middle,
            0x113 | 0x116 => MouseButton::Back,
            0x114 | 0x115 => MouseButton::Forward,
            _ => return None,
        })
    }
}

/// Key transition, numerically identical to evdev `EV_KEY` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum KeyState {
    Release = 0,
    Press = 1,
    Repeat = 2,
}

impl KeyState {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => KeyState::Release,
            1 => KeyState::Press,
            2 => KeyState::Repeat,
            _ => return None,
        })
    }

    pub fn is_down(self) -> bool {
        !matches!(self, KeyState::Release)
    }
}

/// Peer operating system (informational).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OsKind {
    Unknown = 0,
    Windows = 1,
    Linux = 2,
    MacOs = 3,
}

impl OsKind {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            OsKind::Windows
        } else if cfg!(target_os = "linux") {
            OsKind::Linux
        } else if cfg!(target_os = "macos") {
            OsKind::MacOs
        } else {
            OsKind::Unknown
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => OsKind::Windows,
            2 => OsKind::Linux,
            3 => OsKind::MacOs,
            _ => OsKind::Unknown,
        }
    }
}

impl fmt::Display for OsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OsKind::Unknown => "unknown OS",
            OsKind::Windows => "Windows",
            OsKind::Linux => "Linux",
            OsKind::MacOs => "macOS",
        })
    }
}

/// An OS-neutral input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Relative pointer motion.
    MouseMove { dx: i32, dy: i32 },
    MouseButton { button: MouseButton, pressed: bool },
    /// 120 units per detent; `dy > 0` scrolls up, `dx > 0` scrolls right.
    MouseScroll { dx: i32, dy: i32 },
    /// `code` is a Linux evdev key code.
    Key { code: u16, state: KeyState },
}

/// Every message exchanged between server and client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Client -> server, first message.
    Hello { version: u16, os: OsKind, nonce: [u8; 32], name: String },
    /// Server -> client.
    AuthChallenge { nonce: [u8; 32] },
    /// Client -> server: HMAC proof of the pairing secret.
    AuthResponse { proof: [u8; 32] },
    /// Server -> client: verdict plus the server's own proof.
    AuthResult { accepted: bool, proof: [u8; 32], reason: String },
    Input(InputEvent),
    /// Release every key and mouse button currently held on the client.
    ReleaseAllKeys,
    /// Server tells the client whether it is now being controlled and which edge entered from.
    Control { active: bool, entry_edge: ScreenEdge },
    /// Client notifies the server that the pointer crossed back over its return edge.
    EdgeReturn,
    /// Synchronized plain UTF-8 clipboard text.
    ClipboardText(String),
    Heartbeat { timestamp_ms: u64 },
}

impl Message {
    /// Encode the message body (used directly for datagrams).
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(16);
        match self {
            Message::Hello { version, os, nonce, name } => {
                b.push(TAG_HELLO);
                b.extend_from_slice(&version.to_be_bytes());
                b.push(*os as u8);
                b.extend_from_slice(nonce);
                put_str(&mut b, name);
            }
            Message::AuthChallenge { nonce } => {
                b.push(TAG_AUTH_CHALLENGE);
                b.extend_from_slice(nonce);
            }
            Message::AuthResponse { proof } => {
                b.push(TAG_AUTH_RESPONSE);
                b.extend_from_slice(proof);
            }
            Message::AuthResult { accepted, proof, reason } => {
                b.push(TAG_AUTH_RESULT);
                b.push(*accepted as u8);
                b.extend_from_slice(proof);
                put_str(&mut b, reason);
            }
            Message::Input(InputEvent::MouseMove { dx, dy }) => {
                b.push(TAG_MOUSE_MOVE);
                b.extend_from_slice(&dx.to_be_bytes());
                b.extend_from_slice(&dy.to_be_bytes());
            }
            Message::Input(InputEvent::MouseButton { button, pressed }) => {
                b.push(TAG_MOUSE_BUTTON);
                b.push(*button as u8);
                b.push(*pressed as u8);
            }
            Message::Input(InputEvent::MouseScroll { dx, dy }) => {
                b.push(TAG_MOUSE_SCROLL);
                b.extend_from_slice(&dx.to_be_bytes());
                b.extend_from_slice(&dy.to_be_bytes());
            }
            Message::Input(InputEvent::Key { code, state }) => {
                b.push(TAG_KEY);
                b.extend_from_slice(&code.to_be_bytes());
                b.push(*state as u8);
            }
            Message::ReleaseAllKeys => b.push(TAG_RELEASE_ALL),
            Message::Control { active, entry_edge } => {
                b.push(TAG_CONTROL);
                b.push(*active as u8);
                b.push(*entry_edge as u8);
            }
            Message::EdgeReturn => b.push(TAG_EDGE_RETURN),
            Message::ClipboardText(text) => {
                b.push(TAG_CLIPBOARD_TEXT);
                put_long_str(&mut b, text, MAX_CLIPBOARD_LEN);
            }
            Message::Heartbeat { timestamp_ms } => {
                b.push(TAG_HEARTBEAT);
                b.extend_from_slice(&timestamp_ms.to_be_bytes());
            }
        }
        b
    }

    /// Encode as a length-prefixed frame for the reliable stream.
    pub fn encode_frame(&self) -> Vec<u8> {
        let body = self.encode();
        debug_assert!(body.len() <= MAX_FRAME_LEN);
        let mut out = Vec::with_capacity(2 + body.len());
        out.extend_from_slice(&(body.len() as u16).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decode a message body (no length prefix). Rejects trailing bytes.
    pub fn decode(buf: &[u8]) -> Result<Message, ProtoError> {
        if buf.len() > MAX_FRAME_LEN {
            return Err(ProtoError::TooLarge(buf.len()));
        }
        let mut r = Reader { buf, pos: 0 };
        let tag = r.u8()?;
        let msg = match tag {
            TAG_HELLO => Message::Hello {
                version: r.u16()?,
                os: OsKind::from_u8(r.u8()?),
                nonce: r.arr32()?,
                name: r.string()?,
            },
            TAG_AUTH_CHALLENGE => Message::AuthChallenge { nonce: r.arr32()? },
            TAG_AUTH_RESPONSE => Message::AuthResponse { proof: r.arr32()? },
            TAG_AUTH_RESULT => Message::AuthResult {
                accepted: r.bool()?,
                proof: r.arr32()?,
                reason: r.string()?,
            },
            TAG_MOUSE_MOVE => Message::Input(InputEvent::MouseMove { dx: r.i32()?, dy: r.i32()? }),
            TAG_MOUSE_BUTTON => Message::Input(InputEvent::MouseButton {
                button: MouseButton::from_u8(r.u8()?).ok_or(ProtoError::InvalidValue("mouse button"))?,
                pressed: r.bool()?,
            }),
            TAG_MOUSE_SCROLL => Message::Input(InputEvent::MouseScroll { dx: r.i32()?, dy: r.i32()? }),
            TAG_KEY => Message::Input(InputEvent::Key {
                code: r.u16()?,
                state: KeyState::from_u8(r.u8()?).ok_or(ProtoError::InvalidValue("key state"))?,
            }),
            TAG_RELEASE_ALL => Message::ReleaseAllKeys,
            TAG_CONTROL => Message::Control {
                active: r.bool()?,
                entry_edge: ScreenEdge::from_u8(r.u8()?)
                    .ok_or(ProtoError::InvalidValue("screen edge"))?,
            },
            TAG_EDGE_RETURN => Message::EdgeReturn,
            TAG_CLIPBOARD_TEXT => Message::ClipboardText(r.long_string()?),
            TAG_HEARTBEAT => Message::Heartbeat { timestamp_ms: r.u64()? },
            other => return Err(ProtoError::UnknownTag(other)),
        };
        if r.pos != buf.len() {
            return Err(ProtoError::TrailingBytes);
        }
        Ok(msg)
    }
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let mut end = s.len().min(MAX_STR);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    b.push(end as u8);
    b.extend_from_slice(&s.as_bytes()[..end]);
}

fn put_long_str(b: &mut Vec<u8>, s: &str, max_bytes: usize) {
    let mut end = s.len().min(max_bytes);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    b.extend_from_slice(&(end as u16).to_be_bytes());
    b.extend_from_slice(&s.as_bytes()[..end]);
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtoError> {
        let end = self.pos.checked_add(n).ok_or(ProtoError::Truncated)?;
        let s = self.buf.get(self.pos..end).ok_or(ProtoError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, ProtoError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtoError::InvalidValue("bool")),
        }
    }
    fn u16(&mut self) -> Result<u16, ProtoError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, ProtoError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ProtoError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn arr32(&mut self) -> Result<[u8; 32], ProtoError> {
        Ok(self.take(32)?.try_into().unwrap())
    }
    fn string(&mut self) -> Result<String, ProtoError> {
        let n = self.u8()? as usize;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtoError::InvalidValue("utf-8 string"))
    }
    fn long_string(&mut self) -> Result<String, ProtoError> {
        let n = self.u16()? as usize;
        if n > MAX_CLIPBOARD_LEN {
            return Err(ProtoError::TooLarge(n));
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtoError::InvalidValue("utf-8 string"))
    }
}

/// Protocol decoding errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    Truncated,
    TrailingBytes,
    TooLarge(usize),
    UnknownTag(u8),
    InvalidValue(&'static str),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::Truncated => write!(f, "truncated message"),
            ProtoError::TrailingBytes => write!(f, "trailing bytes after message"),
            ProtoError::TooLarge(n) => write!(f, "message too large ({n} bytes)"),
            ProtoError::UnknownTag(t) => write!(f, "unknown message tag 0x{t:02x}"),
            ProtoError::InvalidValue(what) => write!(f, "invalid {what}"),
        }
    }
}

impl std::error::Error for ProtoError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_messages() -> Vec<Message> {
        vec![
            Message::Hello { version: 1, os: OsKind::Linux, nonce: [7; 32], name: "zorin-laptop".into() },
            Message::AuthChallenge { nonce: [9; 32] },
            Message::AuthResponse { proof: [1; 32] },
            Message::AuthResult { accepted: true, proof: [2; 32], reason: String::new() },
            Message::AuthResult { accepted: false, proof: [0; 32], reason: "bad secret".into() },
            Message::Input(InputEvent::MouseMove { dx: -12345, dy: 678 }),
            Message::Input(InputEvent::MouseButton { button: MouseButton::Forward, pressed: true }),
            Message::Input(InputEvent::MouseScroll { dx: 0, dy: -240 }),
            Message::Input(InputEvent::Key { code: 125, state: KeyState::Repeat }),
            Message::ReleaseAllKeys,
            Message::Control { active: true, entry_edge: ScreenEdge::Left },
            Message::EdgeReturn,
            Message::ClipboardText("Hello DeskGlide clipboard!".into()),
            Message::Heartbeat { timestamp_ms: u64::MAX - 3 },
        ]
    }

    #[test]
    fn screen_edge_helpers() {
        assert_eq!(ScreenEdge::Left.opposite(), ScreenEdge::Right);
        assert_eq!(ScreenEdge::Right.opposite(), ScreenEdge::Left);
        assert_eq!(ScreenEdge::Top.opposite(), ScreenEdge::Bottom);
        assert_eq!(ScreenEdge::Bottom.opposite(), ScreenEdge::Top);
        assert_eq!(ScreenEdge::None.opposite(), ScreenEdge::None);
        assert_eq!(ScreenEdge::from_name("Right"), Some(ScreenEdge::Right));
        assert_eq!(ScreenEdge::from_name("hotkey"), Some(ScreenEdge::None));
    }

    #[test]
    fn clipboard_truncates_on_char_boundary() {
        let huge = "😀".repeat(20_000); // 80,000 bytes (> MAX_CLIPBOARD_LEN)
        let msg = Message::ClipboardText(huge);
        match Message::decode(&msg.encode()).unwrap() {
            Message::ClipboardText(text) => {
                assert!(text.len() <= MAX_CLIPBOARD_LEN);
                assert!(text.chars().all(|c| c == '😀'));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn roundtrip_all() {
        for m in all_messages() {
            let enc = m.encode();
            assert_eq!(Message::decode(&enc).unwrap(), m);
            let frame = m.encode_frame();
            assert_eq!(u16::from_be_bytes([frame[0], frame[1]]) as usize, enc.len());
        }
    }

    #[test]
    fn mouse_move_datagram_is_compact() {
        assert_eq!(Message::Input(InputEvent::MouseMove { dx: 1, dy: 2 }).encode().len(), 9);
    }

    #[test]
    fn rejects_truncated_and_trailing() {
        for m in all_messages() {
            let enc = m.encode();
            if enc.len() > 1 {
                assert!(Message::decode(&enc[..enc.len() - 1]).is_err(), "{m:?}");
            }
            let mut longer = enc.clone();
            longer.push(0);
            assert_eq!(Message::decode(&longer), Err(ProtoError::TrailingBytes));
        }
        assert_eq!(Message::decode(&[]), Err(ProtoError::Truncated));
        assert_eq!(Message::decode(&[0xEE]), Err(ProtoError::UnknownTag(0xEE)));
        assert!(Message::decode(&vec![0x10; MAX_FRAME_LEN + 1]).is_err());
    }

    #[test]
    fn long_names_truncate_on_char_boundary() {
        let name = "é".repeat(200); // 400 bytes
        let m = Message::Hello { version: 1, os: OsKind::Windows, nonce: [0; 32], name };
        match Message::decode(&m.encode()).unwrap() {
            Message::Hello { name, .. } => assert!(name.len() <= 255 && name.chars().all(|c| c == 'é')),
            _ => unreachable!(),
        }
    }

    #[test]
    fn button_evdev_roundtrip() {
        for b in MouseButton::ALL {
            assert_eq!(MouseButton::from_evdev(b.evdev_code()), Some(b));
        }
    }
}
