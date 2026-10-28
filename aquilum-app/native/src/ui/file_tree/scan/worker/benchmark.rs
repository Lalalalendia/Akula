use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS};

use super::{CountScan, Counts, Wake};

#[derive(Clone, Copy)]
struct ResourceSample {
    cpu_ticks_100ns: u64,
    read_ops: u64,
    read_bytes: u64,
    other_ops: u64,
}

fn filetime_ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn sample() -> ResourceSample {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut io = IO_COUNTERS::default();
    unsafe {
        let process = GetCurrentProcess();
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).expect("GetProcessTimes");
        GetProcessIoCounters(process, &mut io).expect("GetProcessIoCounters");
    }
    ResourceSample {
        cpu_ticks_100ns: filetime_ticks(kernel) + filetime_ticks(user),
        read_ops: io.ReadOperationCount,
        read_bytes: io.ReadTransferCount,
        other_ops: io.OtherOperationCount,
    }
}

fn make_tree(notes: usize, nested: bool) -> tempfile::TempDir {
    let temp = crate::test_dir::tempdir();
    let root = temp.path();
    let mut previous_folder = PathBuf::new();
    for i in 0..notes {
        let folder = if nested { root.join(format!("folder-{:04}", i / 200)) } else { root.to_path_buf() };
        if folder != previous_folder {
            fs::create_dir_all(&folder).expect("create benchmark folder");
            previous_folder = folder.clone();
        }
        fs::write(folder.join(format!("note-{i:05}.md")), b"synthetic").expect("create benchmark note");
    }
    temp
}

// Simulate the old reload() behavior: one independent full walker per request.
// Keep handles only to account for all work before sampling process CPU/IO.
fn legacy_burst(root: &Path, requests: usize) -> Counts {
    let mut handles = Vec::with_capacity(requests);
    let mut newest = None;
    for _ in 0..requests {
        let path = root.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        handles.push(thread::spawn(move || {
            let _ = sender.send(Counts::scan(&path));
        }));
        newest = Some(receiver); // Dropping older receivers does not stop the earlier scans.
    }
    let final_counts = newest.expect("latest scan receiver").recv_timeout(Duration::from_secs(90)).expect("latest scan");
    for handle in handles {
        handle.join().expect("legacy scan thread");
    }
    final_counts
}

fn coalesced_burst(root: &Path, requests: usize) -> Counts {
    let wake: Wake = Arc::new(|| {});
    let mut counting = CountScan::default();
    for _ in 0..requests {
        counting.request(root.to_path_buf(), Arc::clone(&wake));
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(counts) = counting.poll(root, Arc::clone(&wake)) {
            assert!(counting.idle(), "final result must leave coordinator idle");
            return counts;
        }
        assert!(Instant::now() < deadline, "coalesced scan did not finish");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
#[ignore = "diagnostic Windows Release paired CPU/IO A/B, not a CI speed threshold"]
fn benchmark_filetree_countscan_cpu_io_abba() {
    const REQUESTS: usize = 24;
    for (notes, nested) in [(1000, false), (10000, false), (10000, true)] {
        let dir = make_tree(notes, nested);
        let expected = Counts::scan(dir.path()).total(dir.path());
        assert!(expected.is_some());
        let layout = if nested { "nested" } else { "flat" };
        // Counterbalance cache/order effects: legacy -> coalesced -> coalesced -> legacy.
        for (round, coalesced) in [false, true, true, false].into_iter().enumerate() {
            let before = sample();
            let start = Instant::now();
            let counts = if coalesced {
                coalesced_burst(dir.path(), REQUESTS)
            } else {
                legacy_burst(dir.path(), REQUESTS)
            };
            let wall = start.elapsed();
            let after = sample();
            assert_eq!(counts.total(dir.path()), expected, "count parity");
            let mode = if coalesced { "coalesced" } else { "legacy" };
            println!(
                "FILETREE_COUNTSCAN_AB notes={notes} layout={layout} mode={mode} round={} requests={REQUESTS} wall_us={} cpu_us={} io_read_ops={} io_read_bytes={} io_other_ops={} parity=PASS",
                round + 1,
                wall.as_micros(),
                after.cpu_ticks_100ns.saturating_sub(before.cpu_ticks_100ns) / 10,
                after.read_ops.saturating_sub(before.read_ops),
                after.read_bytes.saturating_sub(before.read_bytes),
                after.other_ops.saturating_sub(before.other_ops)
            );
        }
    }
}
