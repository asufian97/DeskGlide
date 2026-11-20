//! Client-side screen-edge tracking for seamless pointer return to the host.

use deskglide_proto::ScreenEdge;

/// Tracks pointer position and outward edge pressure on the Client so pushing
/// past the entry edge smoothly hands control back to the Server.
#[derive(Debug)]
pub struct ClientEdgeTracker {
    entry_edge: ScreenEdge,
    barrier_px: i32,
    push_acc: i32,
    virt_x: i32,
    virt_y: i32,
    screen_w: i32,
    screen_h: i32,
}

impl Default for ClientEdgeTracker {
    fn default() -> Self {
        Self::new(32)
    }
}

impl ClientEdgeTracker {
    pub fn new(barrier_px: i32) -> Self {
        let (screen_w, screen_h) = detect_primary_resolution();
        Self {
            entry_edge: ScreenEdge::None,
            barrier_px: barrier_px.max(1),
            push_acc: 0,
            virt_x: screen_w / 2,
            virt_y: screen_h / 2,
            screen_w,
            screen_h,
        }
    }

    /// Reset state when remote control becomes active from `entry_edge`.
    pub fn on_activate(&mut self, entry_edge: ScreenEdge) {
        self.entry_edge = entry_edge;
        self.push_acc = 0;
        let (w, h) = detect_primary_resolution();
        self.screen_w = w;
        self.screen_h = h;

        match entry_edge {
            ScreenEdge::Left => {
                self.virt_x = 12;
                self.virt_y = h / 2;
            }
            ScreenEdge::Right => {
                self.virt_x = (w - 12).max(0);
                self.virt_y = h / 2;
            }
            ScreenEdge::Top => {
                self.virt_x = w / 2;
                self.virt_y = 12;
            }
            ScreenEdge::Bottom => {
                self.virt_x = w / 2;
                self.virt_y = (h - 12).max(0);
            }
            ScreenEdge::None => {
                self.virt_x = w / 2;
                self.virt_y = h / 2;
            }
        }

        #[cfg(target_os = "windows")]
        warp_win_cursor_to_entry(entry_edge);
    }

    pub fn on_deactivate(&mut self) {
        self.entry_edge = ScreenEdge::None;
        self.push_acc = 0;
    }

    /// Update after a relative `MouseMove { dx, dy }` was injected.
    /// Returns `true` when the user has pushed past `entry_edge` enough to return to the Host.
    pub fn on_mouse_move(&mut self, dx: i32, dy: i32) -> bool {
        if self.entry_edge == ScreenEdge::None {
            return false;
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(triggered) = self.check_windows_edge(dx, dy) {
                return triggered;
            }
        }

        self.advance_virtual_edge(dx, dy)
    }

    /// Pure coordinate tracker used on Linux (and cross-platform unit tests).
    pub fn advance_virtual_edge(&mut self, dx: i32, dy: i32) -> bool {
        if self.entry_edge == ScreenEdge::None {
            return false;
        }
        let max_x = (self.screen_w - 1).max(0);
        let max_y = (self.screen_h - 1).max(0);

        let next_x = self.virt_x.saturating_add(dx);
        let next_y = self.virt_y.saturating_add(dy);

        let overshoot = match self.entry_edge {
            ScreenEdge::Left if next_x < 0 => -next_x,
            ScreenEdge::Right if next_x > max_x => next_x - max_x,
            ScreenEdge::Top if next_y < 0 => -next_y,
            ScreenEdge::Bottom if next_y > max_y => next_y - max_y,
            _ => 0,
        };

        self.virt_x = next_x.clamp(0, max_x);
        self.virt_y = next_y.clamp(0, max_y);

        if overshoot > 0 {
            self.push_acc = self.push_acc.saturating_add(overshoot);
            if self.push_acc >= self.barrier_px {
                self.push_acc = 0;
                return true;
            }
        } else {
            self.push_acc = 0;
        }
        false
    }

    #[cfg(target_os = "windows")]
    fn check_windows_edge(&mut self, dx: i32, dy: i32) -> Option<bool> {
        use std::mem::zeroed;
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        };

        unsafe {
            let mut pt: POINT = zeroed();
            if GetCursorPos(&mut pt) == 0 {
                return None;
            }
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let right = left + GetSystemMetrics(SM_CXVIRTUALSCREEN) - 1;
            let bottom = top + GetSystemMetrics(SM_CYVIRTUALSCREEN) - 1;

            let push = match self.entry_edge {
                ScreenEdge::Left if pt.x <= left + 1 && dx < 0 => -dx,
                ScreenEdge::Right if pt.x >= right - 1 && dx > 0 => dx,
                ScreenEdge::Top if pt.y <= top + 1 && dy < 0 => -dy,
                ScreenEdge::Bottom if pt.y >= bottom - 1 && dy > 0 => dy,
                _ => 0,
            };

            if push > 0 {
                self.push_acc = self.push_acc.saturating_add(push);
                if self.push_acc >= self.barrier_px {
                    self.push_acc = 0;
                    return Some(true);
                }
            } else {
                self.push_acc = 0;
            }
            Some(false)
        }
    }
}

#[cfg(target_os = "windows")]
fn warp_win_cursor_to_entry(entry_edge: ScreenEdge) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SetCursorPos, SM_CXSCREEN, SM_CYSCREEN,
    };
    if entry_edge == ScreenEdge::None {
        return;
    }
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN).max(1);
        let h = GetSystemMetrics(SM_CYSCREEN).max(1);
        let (x, y) = match entry_edge {
            ScreenEdge::Left => (12, h / 2),
            ScreenEdge::Right => ((w - 12).max(0), h / 2),
            ScreenEdge::Top => (w / 2, 12),
            ScreenEdge::Bottom => (w / 2, (h - 12).max(0)),
            ScreenEdge::None => return,
        };
        let _ = SetCursorPos(x, y);
    }
}

fn detect_primary_resolution() -> (i32, i32) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        };
        unsafe {
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if w > 0 && h > 0 {
                return (w, h);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let modes = entry.path().join("modes");
                if let Ok(contents) = std::fs::read_to_string(modes) {
                    if let Some(first) = contents.lines().next() {
                        if let Some((ws, hs)) = first.trim().split_once('x') {
                            let hs = hs.trim_end_matches(|c: char| !c.is_ascii_digit());
                            if let (Ok(w), Ok(h)) = (ws.parse::<i32>(), hs.parse::<i32>()) {
                                if w > 0 && h > 0 {
                                    return (w, h);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    (1920, 1080)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_edge_requires_outward_barrier_and_resets_inward() {
        let mut t = ClientEdgeTracker {
            entry_edge: ScreenEdge::Left,
            barrier_px: 30,
            push_acc: 0,
            virt_x: 12,
            virt_y: 500,
            screen_w: 1920,
            screen_h: 1080,
        };

        // Moving right enters screen normally
        assert!(!t.advance_virtual_edge(20, 0));
        assert_eq!(t.virt_x, 32);

        // Moving left reaches 0 and overshoots by 8 (below 30)
        assert!(!t.advance_virtual_edge(-40, 0));
        assert_eq!(t.virt_x, 0);
        assert_eq!(t.push_acc, 8);

        // Moving back inward resets push accumulator
        assert!(!t.advance_virtual_edge(5, 0));
        assert_eq!(t.push_acc, 0);

        // Push left out past barrier
        assert!(!t.advance_virtual_edge(-25, 0)); // virt_x=5 -> 0, overshoot 20
        assert!(t.advance_virtual_edge(-15, 0)); // overshoot 15 -> 35 >= 30 -> return!
    }

    #[test]
    fn disabled_edge_never_triggers() {
        let mut t = ClientEdgeTracker::default();
        t.on_activate(ScreenEdge::None);
        assert!(!t.advance_virtual_edge(-5000, -5000));
    }
}
