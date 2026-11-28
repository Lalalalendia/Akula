use akula_music::TrackKey;
use akula_music::playback::worker::threaded::{Admission, Request, ThreadedRuntime};
use akula_music::playback::worker::{COMMAND_CAPACITY, Command};
use akula_music::playback::{Event, Phase, Track};
use std::num::NonZeroU64;
use std::time::Duration;

fn owner(id: u64) -> NonZeroU64 {
    NonZeroU64::new(id).unwrap()
}

fn ready(rt: &ThreadedRuntime) {
    assert!(rt.wait_idle(Duration::from_secs(5)).unwrap());
}

fn replace() -> Request {
    let track = Track::new(TrackKey::new("mock", "a").unwrap(), 180_000).unwrap();
    Request::Intent(Command::Replace { tracks: vec![track], start: 0 })
}

#[test]
fn real_thread_load_play_seek_pause_shutdown() {
    let rt = ThreadedRuntime::spawn(owner(1)).unwrap();
    rt.submit(owner(1), replace()).unwrap();
    ready(&rt);
    let load = rt.snapshot().unwrap();
    assert_eq!(load.phase, Phase::Loading);
    assert!(!load.playing);
    rt.submit(owner(1), Request::Complete { ticket: load.ticket, success: true }).unwrap();
    ready(&rt);
    assert!(rt.snapshot().unwrap().playing);
    rt.submit(owner(1), Request::Intent(Command::Seek(42_000))).unwrap();
    ready(&rt);
    let seek = rt.snapshot().unwrap();
    assert!(!seek.playing);
    rt.submit(owner(1), Request::Intent(Command::Pause(true))).unwrap();
    rt.submit(owner(1), Request::Complete { ticket: seek.ticket, success: true }).unwrap();
    ready(&rt);
    let paused = rt.snapshot().unwrap();
    assert_eq!(paused.phase, Phase::Paused);
    assert_eq!(paused.position_ms, 42_000);
    assert!(!paused.playing);
    rt.shutdown().unwrap();
}

#[test]
fn stop_fences_late_completion_and_new_owner_rejects_old_ticket() {
    let rt = ThreadedRuntime::spawn(owner(1)).unwrap();
    rt.submit(owner(1), replace()).unwrap();
    ready(&rt);
    let old = rt.snapshot().unwrap().ticket;
    rt.submit(owner(1), Request::Intent(Command::Stop)).unwrap();
    rt.submit(owner(1), Request::Complete { ticket: old, success: true }).unwrap();
    ready(&rt);
    assert_eq!(rt.snapshot().unwrap().phase, Phase::Idle);
    assert!(!rt.snapshot().unwrap().playing);
    rt.shutdown().unwrap();

    let other = ThreadedRuntime::spawn(owner(2)).unwrap();
    let stale = Request::Notify { ticket: old, event: Event::Ended };
    assert_eq!(other.submit(owner(2), stale.clone()), Err((Admission::WrongOwner, stale)));
    assert_eq!(other.submit(owner(1), replace()), Err((Admission::WrongOwner, replace())));
    other.shutdown().unwrap();
}

#[test]
fn competing_producers_never_exceed_capacity_and_stop_is_admitted() {
    let rt = ThreadedRuntime::spawn(owner(1)).unwrap();
    rt.submit(owner(1), replace()).unwrap();
    ready(&rt);
    let load = rt.snapshot().unwrap().ticket;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let rt = &rt;
            scope.spawn(move || {
                for _ in 0..500 {
                    let request = Request::Complete { ticket: load, success: true };
                    match rt.submit(owner(1), request.clone()) {
                        Ok(()) => {}
                        Err((Admission::Full, rejected)) => assert_eq!(rejected, request),
                        Err(other) => panic!("unexpected admission: {other:?}"),
                    }
                    assert!(rt.snapshot().unwrap().queued <= COMMAND_CAPACITY);
                }
            });
        }
    });
    rt.submit(owner(1), Request::Intent(Command::Stop)).unwrap();
    ready(&rt);
    let stopped = rt.snapshot().unwrap();
    assert_eq!(stopped.queued, 0);
    assert_eq!(stopped.phase, Phase::Idle);
    assert!(!stopped.playing);
    assert!(stopped.last_error.is_none());
    rt.shutdown().unwrap();
}
