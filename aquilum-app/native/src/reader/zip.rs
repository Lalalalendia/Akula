use std::collections::HashMap; mod limited;
#[cfg(test)] mod security_tests;
const MAX_LEGACY_READ_BYTES: usize = 64 * 1024 * 1024;
const END: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
struct Entry {
    method: u16,
    compressed: usize,
    size: usize,
    local: usize,
}

pub struct Zip {
    data: Vec<u8>,
    entries: HashMap<String, Entry>,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

impl Zip {
    pub fn new(data: Vec<u8>) -> Option<Zip> {
        let end = (0..data.len().saturating_sub(21)).rev().take(65_557).find(|&i| u32_at(&data, i) == Some(END))?;
        let count = u16_at(&data, end + 10)? as usize;
        let mut at = u32_at(&data, end + 16)? as usize;
        let mut entries = HashMap::with_capacity(count.min(1024));
        for _ in 0..count {
            if u32_at(&data, at)? != CENTRAL {
                return None;
            }
            let method = u16_at(&data, at + 10)?;
            let compressed = u32_at(&data, at + 20)? as usize;
            let size = u32_at(&data, at + 24)? as usize;
            let name_len = u16_at(&data, at + 28)? as usize;
            let extra_len = u16_at(&data, at + 30)? as usize;
            let comment_len = u16_at(&data, at + 32)? as usize;
            let local = u32_at(&data, at + 42)? as usize;
            let name_start = at.checked_add(46)?;
            let name_end = name_start.checked_add(name_len)?;
            let name = String::from_utf8_lossy(data.get(name_start..name_end)?).into_owned();
            entries.insert(name, Entry { method, compressed, size, local });
            at = name_end.checked_add(extra_len)?.checked_add(comment_len)?;
        }
        Some(Zip { data, entries })
    }

    pub fn size(&self, name: &str) -> Option<usize> {
        self.entries.get(name).map(|e| e.size)
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.read_limited(name, MAX_LEGACY_READ_BYTES)
    }
}
