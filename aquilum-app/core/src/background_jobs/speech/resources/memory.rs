use super::{Error, MIB, Result};

const RESERVE: u64 = 1024 * MIB;

pub(super) fn budget(sample: Option<(u64, u64)>, floor: u64, ceiling: u64) -> Result<u64> {
    let bytes = sample.map_or(1536 * MIB, |(total, available)| {
        (total / 2).min(available.saturating_sub(RESERVE))
    });
    if bytes < floor * MIB {
        return Err(Error::ResourcePressure);
    }
    Ok(bytes.min(ceiling * MIB))
}

#[cfg(target_os = "linux")]
pub(super) fn available() -> Option<(u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name| -> Option<u64> {
        let line = info.lines().find_map(|line| line.strip_prefix(name))?;
        line.split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()?
            .checked_mul(1024)
    };
    let mut sample = (field("MemTotal:")?, field("MemAvailable:")?);
    if let Ok(groups) = std::fs::read_to_string("/proc/self/cgroup") {
        if let Some(group) = groups.lines().find_map(|line| line.strip_prefix("0::/")) {
            let root = std::path::Path::new("/sys/fs/cgroup");
            for path in root
                .join(group)
                .ancestors()
                .take_while(|path| path.starts_with(root))
            {
                let read = |name| {
                    std::fs::read_to_string(path.join(name))
                        .ok()?
                        .trim()
                        .parse::<u64>()
                        .ok()
                };
                if let (Some(limit), Some(used)) = (read("memory.max"), read("memory.current")) {
                    sample.0 = sample.0.min(limit);
                    sample.1 = sample.1.min(limit.saturating_sub(used));
                }
            }
        }
    }
    Some(sample)
}

#[cfg(windows)]
pub(super) use super::windows::available;

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn available() -> Option<(u64, u64)> {
    None
}
