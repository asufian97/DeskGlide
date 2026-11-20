//! Encrypted UTF-8 clipboard text synchronization with loop suppression.

use arboard::Clipboard;
use deskglide_proto::MAX_CLIPBOARD_LEN;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

pub struct ClipboardSync {
    clipboard: Option<Clipboard>,
    last_hash: Option<[u8; 32]>,
}

impl Default for ClipboardSync {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardSync {
    pub fn new() -> Self {
        let mut clipboard = match Clipboard::new() {
            Ok(cb) => Some(cb),
            Err(e) => {
                warn!("clipboard unavailable on this system ({e}); clipboard sync disabled");
                None
            }
        };
        // Seed initial hash so starting a session doesn't immediately blast old clipboard text.
        let last_hash = clipboard
            .as_mut()
            .and_then(|cb| cb.get_text().ok())
            .map(|t| hash_text(&t));

        Self {
            clipboard,
            last_hash,
        }
    }

    /// Returns `Some(text)` if local clipboard text changed since last check/apply.
    pub fn poll_local_change(&mut self) -> Option<String> {
        let cb = self.clipboard.as_mut()?;
        let Ok(mut text) = cb.get_text() else {
            return None;
        };
        if text.is_empty() {
            return None;
        }
        if text.len() > MAX_CLIPBOARD_LEN {
            let mut end = MAX_CLIPBOARD_LEN;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        let h = hash_text(&text);
        if self.last_hash == Some(h) {
            return None;
        }
        self.last_hash = Some(h);
        Some(text)
    }

    /// Apply text received from the remote peer without echoing it back.
    pub fn apply_remote(&mut self, text: &str) {
        let Some(cb) = self.clipboard.as_mut() else {
            return;
        };
        let h = hash_text(text);
        if self.last_hash == Some(h) {
            return;
        }
        self.last_hash = Some(h);
        if let Err(e) = cb.set_text(text) {
            debug!("failed updating local clipboard: {e}");
        }
    }
}

fn hash_text(s: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().into()
}
