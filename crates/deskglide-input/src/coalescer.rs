//! Batches high-polling-rate relative mouse deltas into smooth periodic updates.

/// Accumulates relative `(dx, dy)` mouse deltas between network flushes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MouseCoalescer {
    dx: i32,
    dy: i32,
}

impl MouseCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulate a raw relative mouse delta using saturating arithmetic.
    pub fn add(&mut self, dx: i32, dy: i32) {
        self.dx = self.dx.saturating_add(dx);
        self.dy = self.dy.saturating_add(dy);
    }

    /// Returns `Some((dx, dy))` if non-zero accumulated movement is pending and resets state.
    pub fn take(&mut self) -> Option<(i32, i32)> {
        if self.dx == 0 && self.dy == 0 {
            None
        } else {
            let out = (self.dx, self.dy);
            self.dx = 0;
            self.dy = 0;
            Some(out)
        }
    }

    /// Clear any pending deltas without sending.
    pub fn clear(&mut self) {
        self.dx = 0;
        self.dy = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.dx == 0 && self.dy == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_deltas_and_resets() {
        let mut c = MouseCoalescer::new();
        assert_eq!(c.take(), None);
        assert!(c.is_empty());

        for _ in 0..8 {
            c.add(3, -2);
        }
        c.add(-4, 1);
        assert!(!c.is_empty());
        assert_eq!(c.take(), Some((20, -15)));
        assert_eq!(c.take(), None);
    }

    #[test]
    fn canceling_deltas_yields_none() {
        let mut c = MouseCoalescer::new();
        c.add(5, -5);
        c.add(-5, 5);
        assert_eq!(c.take(), None);
    }

    #[test]
    fn saturates_on_extreme_overflow() {
        let mut c = MouseCoalescer::new();
        c.add(i32::MAX, i32::MIN);
        c.add(10, -10);
        assert_eq!(c.take(), Some((i32::MAX, i32::MIN)));
    }
}
