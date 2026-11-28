use super::*;

#[test]
fn cpu_profiles_preserve_capacity_and_cap_large_hosts() {
    for (cpus, balanced, fast) in [
        (1, 1, 1),
        (2, 1, 1),
        (4, 2, 2),
        (8, 4, 6),
        (12, 6, 8),
        (128, 8, 8),
    ] {
        assert_eq!(CpuMode::Quiet.for_cpus(cpus), 1);
        assert_eq!(CpuMode::Balanced.for_cpus(cpus), balanced);
        assert_eq!(CpuMode::Fast.for_cpus(cpus), fast);
    }
}

#[test]
fn memory_budget_preserves_a_reserve_and_handles_unknown_resources() {
    let sample = |total, available| Some((total * MIB, available * MIB));
    assert_eq!(
        memory::budget(sample(16384, 8192), 512, 4096).unwrap(),
        4096 * MIB
    );
    assert_eq!(
        memory::budget(sample(8192, 2800), 512, 4096).unwrap(),
        1776 * MIB
    );
    assert!(matches!(
        memory::budget(sample(8192, 1400), 512, 4096),
        Err(Error::ResourcePressure)
    ));
    assert_eq!(memory::budget(None, 512, 4096).unwrap(), 1536 * MIB);
    assert_eq!(memory::budget(None, 256, 768).unwrap(), 768 * MIB);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_child_observes_single_cpu_background_priority_and_memory_ceiling() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "test \"$OMP_THREAD_LIMIT\" = 1 && test \"$OPENBLAS_NUM_THREADS\" = 1 && test \"$(ulimit -v)\" -le 786432 && sleep 0.2"]);
    let mut child = Limits::Media.spawn(&mut command).unwrap();
    let pid = rustix::process::Pid::from_raw(child.child.id() as i32);
    assert_eq!(rustix::process::sched_getaffinity(pid).unwrap().count(), 1);
    assert!(rustix::process::getpriority_process(pid).unwrap() >= 10);
    assert!(child.child.wait().unwrap().success());
}

#[cfg(windows)]
#[test]
fn windows_managed_child_runs_inside_a_job() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    let mut command = Command::new("cmd.exe");
    command.args([
        "/d",
        "/c",
        "if \"%OMP_THREAD_LIMIT%\"==\"1\" exit /b 0 & exit /b 1",
    ]);
    let mut child = Limits::Media.spawn(&mut command).unwrap();
    let mut in_job = 0;
    assert_ne!(
        unsafe {
            IsProcessInJob(
                child.child.as_raw_handle(),
                child._job.as_raw_handle(),
                &mut in_job,
            )
        },
        0
    );
    assert_ne!(in_job, 0);
    assert!(child.child.wait().unwrap().success());
}

#[test]
fn memory_pause_preserves_committed_windows_and_respects_worker_ownership() {
    use crate::background_jobs::{Queue, Segment, Spec, State};
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let mut queue = Queue::open(&dir.path().join("jobs.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: "speech".into(),
            source: "video.mp4".into(),
            revision: "a".repeat(64),
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim("speech", Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 2).unwrap();
    queue
        .append(
            &lease,
            &Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: 30_000,
                payload: b"[]".to_vec(),
            },
        )
        .unwrap();
    queue.park_for_memory(&lease).unwrap();
    let job = queue.get(id).unwrap().unwrap();
    assert_eq!((job.state, job.committed), (State::Paused, 1));
    queue.resume(id).unwrap();
    queue
        .claim("speech", Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert!(matches!(
        queue.park_for_memory(&lease),
        Err(crate::background_jobs::Error::Conflict)
    ));
    assert_eq!(queue.segments(id, 0, 1).unwrap()[0].payload, b"[]");
}
