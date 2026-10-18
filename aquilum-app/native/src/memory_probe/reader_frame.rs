//! Diagnostic-only reader first-content-present latency and process memory.
//! This does not measure compositor scanout or monitor photons.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::counting::{CURRENT, PEAK};

static TEXT_PAINTED: AtomicBool = AtomicBool::new(false);

pub fn reset_frame() {
    TEXT_PAINTED.store(false, Ordering::Relaxed);
}

pub fn mark_text_painted() {
    TEXT_PAINTED.store(true, Ordering::Relaxed);
}

pub fn take_text_painted() -> bool {
    TEXT_PAINTED.swap(false, Ordering::Relaxed)
}

#[derive(Clone, Copy, Default)]
struct Memory {
    rss_kib: u64,
    hwm_kib: u64,
}

#[cfg(target_os = "linux")]
fn process_memory() -> Memory {
    let mut memory = Memory::default();
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            let (value, target) = if let Some(rest) = line.strip_prefix("VmRSS:") {
                (rest, &mut memory.rss_kib)
            } else if let Some(rest) = line.strip_prefix("VmHWM:") {
                (rest, &mut memory.hwm_kib)
            } else {
                continue;
            };
            *target = value.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    memory
}

#[cfg(target_os = "windows")]
fn process_memory() -> Memory {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    if unsafe { GetProcessMemoryInfo(
        GetCurrentProcess(),
        (&raw mut counters).cast(),
        size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
    ) }.is_ok() {
        return Memory {
            rss_kib: (counters.WorkingSetSize / 1024) as u64,
            hwm_kib: (counters.PeakWorkingSetSize / 1024) as u64,
        };
    }
    Memory::default()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn process_memory() -> Memory {
    Memory::default()
}

// RSS and VmHWM are process-wide. VmHWM is cumulative since process start.
pub fn navigation_sample(label: &str, reader_state: Option<&str>) -> String {
    let memory = process_memory();
    let mib = |bytes: usize| bytes as f64 / 1_048_576.0;
    format!(
        "READER_NAV_SAMPLE label={} reader_active={} heap_current_mib={:.3} heap_peak_mib={:.3} rss_kib={} process_hwm_kib={} {}",
        label.replace(' ', "_"),
        reader_state.is_some(),
        mib(CURRENT.load(Ordering::Relaxed)),
        mib(PEAK.load(Ordering::Relaxed)),
        memory.rss_kib,
        memory.hwm_kib,
        reader_state.unwrap_or(""),
    )
}

pub struct ReaderFrame {
    began: Instant,
    sync_open_ms: f64,
    initial_heap: usize,
    initial_mem: Memory,
}

impl ReaderFrame {
    pub fn begin() -> Self {
        let initial_heap = CURRENT.load(Ordering::Relaxed);
        let initial_mem = process_memory();
        // Global allocation peaks are reset only in opt-in memory-probe mode.
        PEAK.store(initial_heap, Ordering::Relaxed);
        reset_frame();
        ReaderFrame {
            began: Instant::now(),
            sync_open_ms: 0.0,
            initial_heap,
            initial_mem,
        }
    }

    pub fn opened(&mut self) {
        self.sync_open_ms = self.began.elapsed().as_secs_f64() * 1000.0;
    }

    pub fn finish(self, scene_ms: f64, raster_ms: f64, present_ms: f64) -> String {
        let mem = process_memory();
        let mib = |b: usize| b as f64 / 1_048_576.0;
        format!(
            concat!(
                "READER_GUI_FRAME first_text_present_ms={:.3} sync_open_ms={:.3} ",
                "scene_ms={scene_ms:.3} raster_ms={raster_ms:.3} present_ms={present_ms:.3} ",
                "heap_before_mib={:.3} heap_at_present_mib={:.3} heap_peak_mib={:.3} ",
                "rss_before_kib={} rss_at_present_kib={} process_hwm_kib={} ",
                "pixel_scanout_measured=false"
            ),
            self.began.elapsed().as_secs_f64() * 1000.0,
            self.sync_open_ms,
            mib(self.initial_heap),
            mib(CURRENT.load(Ordering::Relaxed)),
            mib(PEAK.load(Ordering::Relaxed)),
            self.initial_mem.rss_kib,
            mem.rss_kib,
            mem.hwm_kib,
            scene_ms = scene_ms,
            raster_ms = raster_ms,
            present_ms = present_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_paint_marker_only_fires_once() {
        reset_frame();
        assert!(!take_text_painted());
        mark_text_painted();
        assert!(take_text_painted());
        assert!(!take_text_painted());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_process_rss_is_observable() {
        let m = process_memory();
        assert!(m.rss_kib > 0);
        assert!(m.hwm_kib >= m.rss_kib);
    }
}
