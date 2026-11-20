//! Cross-platform input capture, injection, mouse coalescing, edge switching,
//! hotkey detection, and held-state tracking.

pub mod coalescer;
pub mod edge;
pub mod hotkey;
pub mod state;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

use anyhow::Result;
use deskglide_proto::{InputEvent, ScreenEdge};
use tokio::sync::mpsc;

pub use coalescer::MouseCoalescer;
pub use edge::ClientEdgeTracker;
pub use hotkey::{Hotkey, HotkeyDecision, HotkeyStateMachine};
pub use state::HeldStateTracker;

/// Events emitted by an [`InputCapture`] backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    /// Captured input intended for the remote client (only emitted when remote is active).
    Input(InputEvent),
    /// Active target changed via hotkey or screen edge (`true` = remote, `false` = local).
    ActiveChanged(bool),
}

/// Platform-neutral local input capture & trap interface (Server / Host side).
pub trait InputCapture: Send {
    /// Toggle whether local input is trapped and forwarded to the remote client.
    fn set_remote_active(&self, active: bool) -> Result<()>;

    /// Update the host screen edge that switches control to the remote client.
    fn set_switch_edge(&self, edge: ScreenEdge);

    /// Return control to local host placing the pointer just inside `edge` (on edge return).
    fn restore_from_edge(&self, edge: ScreenEdge) -> Result<()>;

    /// Current active state (`true` = trapped/remote, `false` = local).
    fn is_remote_active(&self) -> bool;

    /// Receive stream of captured events.
    fn events_rx(&mut self) -> &mut mpsc::Receiver<CaptureEvent>;
}

/// Platform-neutral input injection interface (Client side).
pub trait InputInjector: Send {
    /// Inject a single OS-neutral input event into the local OS.
    fn inject(&mut self, event: InputEvent) -> Result<()>;
}

/// Open the native input capture backend for the current OS.
pub fn open_capture(hotkey: Hotkey, switch_edge: ScreenEdge) -> Result<Box<dyn InputCapture>> {
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsCapture::start(hotkey, switch_edge)?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxCapture::start(hotkey, switch_edge)?))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (hotkey, switch_edge);
        anyhow::bail!("input capture is only implemented on Windows and Linux");
    }
}

/// Open the native input injection backend for the current OS.
pub fn open_injector() -> Result<Box<dyn InputInjector>> {
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsInjector::new()))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxInjector::new()?))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        anyhow::bail!("input injection is only implemented on Windows and Linux");
    }
}
