//! DeskGlide wire protocol.
//!
//! * All key codes on the wire are **Linux evdev key codes** (`u16`), which are a
//!   superset of USB-HID keyboard usages and map 1:1 to PC scancode set 1 for the
//!   classic keys. Windows translates to/from them via [`win_keymap`].
//! * Scroll values use **120 units per wheel detent** (Windows `WHEEL_DELTA` and
//!   Linux `REL_WHEEL_HI_RES` both use 120), positive = up / right.
//! * Mouse motion is relative (`dx`, `dy`) in device counts.

pub mod keycode;
pub mod message;
pub mod win_keymap;

pub use message::*;

/// Bumped whenever the wire format changes incompatibly.
pub const PROTOCOL_VERSION: u16 = 2;
/// Default UDP port (QUIC).
pub const DEFAULT_PORT: u16 = 24910;
/// QUIC/TLS ALPN identifier.
pub const ALPN: &[u8] = b"deskglide/2";
/// Scroll units per wheel detent.
pub const WHEEL_DETENT: i32 = 120;
