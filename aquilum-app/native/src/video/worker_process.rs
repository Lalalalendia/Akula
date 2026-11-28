//! Process cleanup and bounded diagnostics for the two video download routes.
//! Keep child reaping before the caller drops its private staging directory.
use std::io::Read;
use std::process::Child;

pub(super) struct ChildGuard(pub(super) Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(super) fn diagnostic_tail(mut reader: impl Read, trim: bool) -> String {
    let mut bytes = [0_u8; 4096];
    let mut recent = Vec::<u8>::new();
    while let Ok(n) = reader.read(&mut bytes) {
        if n == 0 { break; }
        recent.extend_from_slice(&bytes[..n]);
        if recent.len() > 8192 {
            recent.drain(..recent.len() - 8192);
        }
    }
    let text = String::from_utf8_lossy(&recent);
    let text = if trim { text.trim() } else { text.as_ref() };
    text.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keep_last_bytes_and_existing_whitespace_policy() {
        let mut bytes = vec![b'x'; 100];
        bytes.extend(vec![b' '; 8190]);
        bytes.extend_from_slice(b"ok");
        assert_eq!(diagnostic_tail(bytes.as_slice(), true), "ok");
        assert_eq!(diagnostic_tail(bytes.as_slice(), false), " ".repeat(500));
        assert_eq!(diagnostic_tail(&b"  hi \n"[..], true), "hi");
        assert_eq!(diagnostic_tail(&b"  hi \n"[..], false), "  hi \n");
        assert_eq!(diagnostic_tail(&[0xff, b'x'][..], false), "\u{fffd}x");
    }
}
