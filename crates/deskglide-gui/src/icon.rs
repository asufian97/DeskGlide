//! Procedural 32x32 application icon (used for both window titlebar and system tray).

use eframe::egui;

pub const ICON_SIZE: usize = 32;

/// Generate RGBA8 pixel buffer for a clean DeskGlide dual-screen icon.
pub fn rgba_32x32() -> Vec<u8> {
    let mut px = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
    let set = |px: &mut [u8], x: usize, y: usize, r: u8, g: u8, b: u8, a: u8| {
        if x < ICON_SIZE && y < ICON_SIZE {
            let idx = (y * ICON_SIZE + x) * 4;
            px[idx] = r;
            px[idx + 1] = g;
            px[idx + 2] = b;
            px[idx + 3] = a;
        }
    };

    // Dark slate rounded square background (#0F172A with #1E293B border)
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let dx = if x < 6 { 6 - x } else if x >= ICON_SIZE - 6 { x - (ICON_SIZE - 7) } else { 0 };
            let dy = if y < 6 { 6 - y } else if y >= ICON_SIZE - 6 { y - (ICON_SIZE - 7) } else { 0 };
            let dist2 = dx * dx + dy * dy;
            if dist2 <= 36 {
                let border = x <= 1 || x >= ICON_SIZE - 2 || y <= 1 || y >= ICON_SIZE - 2 || dist2 >= 25;
                if border {
                    set(&mut px, x, y, 45, 212, 191, 255); // #2DD4BF teal rim
                } else {
                    set(&mut px, x, y, 15, 23, 42, 255); // #0F172A slate fill
                }
            }
        }
    }

    // Left monitor silhouette (#E2E8F0)
    for y in 9..21 {
        for x in 5..14 {
            let edge = x == 5 || x == 13 || y == 9 || y == 20;
            if edge {
                set(&mut px, x, y, 226, 232, 240, 255);
            } else {
                set(&mut px, x, y, 30, 41, 59, 255);
            }
        }
    }
    for x in 7..12 {
        set(&mut px, x, 22, 148, 163, 184, 255);
    }

    // Right monitor silhouette (#E2E8F0)
    for y in 9..21 {
        for x in 18..27 {
            let edge = x == 18 || x == 26 || y == 9 || y == 20;
            if edge {
                set(&mut px, x, y, 226, 232, 240, 255);
            } else {
                set(&mut px, x, y, 30, 41, 59, 255);
            }
        }
    }
    for x in 20..25 {
        set(&mut px, x, 22, 148, 163, 184, 255);
    }

    // Glowing teal link bridge across screens (#2DD4BF)
    for x in 11..21 {
        set(&mut px, x, 14, 45, 212, 191, 255);
        set(&mut px, x, 15, 45, 212, 191, 255);
    }
    set(&mut px, 19, 13, 45, 212, 191, 255);
    set(&mut px, 19, 16, 45, 212, 191, 255);
    set(&mut px, 12, 13, 45, 212, 191, 255);
    set(&mut px, 12, 16, 45, 212, 191, 255);

    px
}

pub fn egui_icon() -> egui::IconData {
    egui::IconData {
        rgba: rgba_32x32(),
        width: ICON_SIZE as u32,
        height: ICON_SIZE as u32,
    }
}
