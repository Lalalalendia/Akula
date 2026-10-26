//! Inflation-limited ZIP entry read for untrusted DOCX preview.
use super::{Zip, LOCAL, u16_at, u32_at};
use std::io::Read;

impl Zip {
    /// The declared and actual sizes must both be within the cap.
    /// The legacy, unbounded read() remains unchanged for EPUB callers.
    pub fn read_limited(&self, name: &str, limit: usize) -> Option<Vec<u8>> {
        let entry = self.entries.get(name)?;
        if entry.size > limit {
            return None;
        }
        let at = entry.local;
        if u32_at(&self.data, at)? != LOCAL {
            return None;
        }
        let start = at.checked_add(30)?
            .checked_add(u16_at(&self.data, at + 26)? as usize)?
            .checked_add(u16_at(&self.data, at + 28)? as usize)?;
        let end = start.checked_add(entry.compressed)?;
        let raw = self.data.get(start..end)?;
        match entry.method {
            0 if raw.len() == entry.size => Some(raw.to_vec()),
            8 => {
                let mut out = Vec::new();
                let cap = u64::try_from(limit).ok()?.checked_add(1)?;
                flate2::read::DeflateDecoder::new(raw).take(cap).read_to_end(&mut out).ok()?;
                (out.len() == entry.size).then_some(out)
            }
            _ => None,
        }
    }
}
