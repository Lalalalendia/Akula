use akula_music::TrackKey;
use akula_music::playback::{Effect, Error, Event, MAX_QUEUE, Phase, Player, Repeat, Ticket, Track};
use std::num::NonZeroU64;

fn track(id: &str) -> Track {
    Track::new(TrackKey::new("yandex", id).unwrap(), 180_000).unwrap()
}

fn player(id: u64) -> Player {
    Player::new(NonZeroU64::new(id).unwrap())
}

fn start(p: &mut Player) -> Ticket {
    let load = p.replace(vec![track("a"), track("b"), track("c")], 0).unwrap();
    let ready = p.receive(load.ticket(), Event::Ready).unwrap().unwrap();
    assert!(matches!(ready, Effect::SetPaused { paused: false, .. }));
    ready.ticket()
}

fn current(p: &Player) -> &str {
    p.current().unwrap().key().id()
}

#[test]
fn rapid_next_ignores_every_old_load_and_failure() {
    let mut p = player(1);
    let a = p.replace(vec![track("a"), track("b"), track("c")], 0).unwrap();
    let b = p.next_track().unwrap();
    let c = p.next_track().unwrap();
    for old in [a.ticket(), b.ticket()] {
        assert_eq!(p.receive(old, Event::Ready), Ok(None));
        assert_eq!(p.receive(old, Event::Failed), Ok(None));
        assert_eq!(p.receive(old, Event::Ended), Ok(None));
    }
    assert_eq!(current(&p), "c");
    assert_eq!(p.phase(), Phase::Loading);
    assert!(p.receive(c.ticket(), Event::Ready).unwrap().is_some());
    assert_eq!(p.receive(c.ticket(), Event::Ready), Ok(None));
    assert_eq!(p.phase(), Phase::Playing);
}

#[test]
fn pause_before_ready_and_resume_use_fresh_revision() {
    let mut p = player(1);
    let load = p.replace(vec![track("a")], 0).unwrap();
    assert_eq!(p.set_paused(true), Ok(None));
    let paused = p.receive(load.ticket(), Event::Ready).unwrap().unwrap();
    assert!(matches!(paused, Effect::SetPaused { paused: true, .. }));
    assert_eq!(p.phase(), Phase::Paused);
    assert_eq!(p.set_paused(true), Ok(None));
    let running = p.set_paused(false).unwrap().unwrap();
    assert_eq!(running.ticket().session(), load.ticket().session());
    assert!(running.ticket().revision() > paused.ticket().revision());
    assert_eq!(p.receive(paused.ticket(), Event::Ended), Ok(None));
    assert_eq!(p.phase(), Phase::Playing);
}

#[test]
fn seek_supersedes_old_progress_and_acknowledgements() {
    let mut p = player(1);
    let run = start(&mut p);
    p.receive(run, Event::Progress { position_ms: 12_000 }).unwrap();
    let a = p.seek(90_000).unwrap();
    let b = p.seek(1_000).unwrap();
    assert_eq!(p.position_ms(), 12_000);
    assert_eq!(p.receive(a.ticket(), Event::Seeked { position_ms: 90_000 }), Ok(None));
    assert_eq!(p.receive(run, Event::Progress { position_ms: 150_000 }), Ok(None));
    assert_eq!(p.receive(run, Event::Ended), Ok(None));
    p.set_paused(true).unwrap();
    let ready = p.receive(b.ticket(), Event::Seeked { position_ms: 1_100 }).unwrap().unwrap();
    assert!(matches!(ready, Effect::SetPaused { paused: true, .. }));
    assert_eq!(p.position_ms(), 1_100);
    assert_eq!(p.phase(), Phase::Paused);
    assert_eq!(ready.ticket().session(), run.session());
}

#[test]
fn reordered_progress_cannot_move_position_back() {
    let mut p = player(1);
    let run = start(&mut p);
    p.receive(run, Event::Progress { position_ms: 10_000 }).unwrap();
    p.receive(run, Event::Progress { position_ms: 5_000 }).unwrap();
    assert_eq!(p.position_ms(), 10_000);
    assert_eq!(p.receive(run, Event::Progress { position_ms: 180_001 }), Err(Error::InvalidPosition));
    assert_eq!(p.position_ms(), 10_000);
}

#[test]
fn repeat_one_creates_new_session_but_manual_next_advances() {
    let mut p = player(1);
    p.set_repeat(Repeat::One);
    let run = start(&mut p);
    let replay = p.receive(run, Event::Ended).unwrap().unwrap();
    assert_eq!(current(&p), "a");
    assert_ne!(replay.ticket().session(), run.session());
    assert_eq!(p.receive(run, Event::Ended), Ok(None));
    let next = p.next_track().unwrap();
    assert_eq!(current(&p), "b");
    assert_ne!(next.ticket().session(), replay.ticket().session());
}

#[test]
fn repeat_all_wraps_in_both_directions() {
    let mut p = player(1);
    p.set_repeat(Repeat::All);
    p.replace(vec![track("a"), track("b")], 1).unwrap();
    p.next_track().unwrap();
    assert_eq!(current(&p), "a");
    p.previous_track().unwrap();
    assert_eq!(current(&p), "b");
}

#[test]
fn final_track_ends_and_invalidates_old_session() {
    let mut p = player(1);
    let load = p.replace(vec![track("a")], 0).unwrap();
    let run = p.receive(load.ticket(), Event::Ready).unwrap().unwrap();
    let stop = p.receive(run.ticket(), Event::Ended).unwrap().unwrap();
    assert!(matches!(stop, Effect::Stop { .. }));
    assert_eq!(p.phase(), Phase::Idle);
    assert_eq!(p.queue_len(), 0);
    assert!(p.current().is_none());
    assert_eq!(p.receive(run.ticket(), Event::Ready), Ok(None));
}

#[test]
fn explicit_stop_during_seek_fences_late_ack() {
    let mut p = player(1);
    start(&mut p);
    let seek = p.seek(5_000).unwrap();
    let stop = p.stop().unwrap();
    assert!(stop.ticket().revision() > seek.ticket().revision());
    assert_eq!(p.receive(seek.ticket(), Event::Seeked { position_ms: 5_000 }), Ok(None));
    assert_eq!(p.phase(), Phase::Idle);
    assert_eq!(p.position_ms(), 0);
}

#[test]
fn failure_stops_output_and_retry_gets_fresh_session() {
    let mut p = player(1);
    let run = start(&mut p);
    let stop = p.receive(run, Event::Failed).unwrap().unwrap();
    assert!(matches!(stop, Effect::Stop { .. }));
    assert_eq!(p.phase(), Phase::Failed);
    assert_eq!(p.receive(run, Event::Ready), Ok(None));
    let retry = p.retry().unwrap();
    assert_eq!(current(&p), "a");
    assert_ne!(retry.ticket().session(), run.session());
    assert_eq!(p.phase(), Phase::Loading);
}

#[test]
fn different_controller_cannot_consume_old_owner_event() {
    let mut a = player(1);
    let mut b = player(2);
    let old = start(&mut a);
    let now = start(&mut b);
    assert_eq!(old.revision(), now.revision());
    assert_ne!(old.owner(), now.owner());
    assert_eq!(b.receive(old, Event::Ended), Ok(None));
    assert_eq!(current(&b), "a");
}

#[test]
fn queue_and_snapshot_are_bounded_and_invalid_replace_is_atomic() {
    let mut p = player(1);
    let queue = vec![track("a"); MAX_QUEUE];
    p.replace(queue, 0).unwrap();
    assert_eq!(p.queue_len(), MAX_QUEUE);
    assert_eq!(p.upcoming(usize::MAX).len(), 30);
    let prior = p.ticket();
    assert_eq!(p.replace(vec![track("b"); MAX_QUEUE + 1], 0), Err(Error::InvalidQueue));
    assert_eq!(p.replace(vec![], 0), Err(Error::InvalidQueue));
    assert_eq!(p.replace(vec![track("b")], 1), Err(Error::InvalidQueue));
    assert_eq!(p.ticket(), prior);
    assert_eq!(current(&p), "a");
}

#[test]
fn invalid_seek_does_not_change_revision_or_position() {
    let mut p = player(1);
    let run = start(&mut p);
    assert_eq!(p.seek(180_001), Err(Error::InvalidPosition));
    assert_eq!(p.ticket(), run);
    let seek = p.seek(10_000).unwrap();
    assert_eq!(p.receive(seek.ticket(), Event::Seeked { position_ms: 180_001 }), Err(Error::InvalidPosition));
    assert_eq!(p.ticket(), seek.ticket());
    assert_eq!(p.position_ms(), 0);
    assert_eq!(p.phase(), Phase::Seeking);
}

#[test]
fn track_validation_rejects_zero_duration_and_oversized_identity() {
    let key = TrackKey::new("yandex", "a").unwrap();
    assert_eq!(Track::new(key, 0), Err(Error::InvalidTrack));
    let key = TrackKey::new("yandex", &"x".repeat(513)).unwrap();
    assert_eq!(Track::new(key, 1), Err(Error::InvalidTrack));
}

#[test]
fn repeated_rapid_commands_keep_invariants() {
    let mut p = player(1);
    p.set_repeat(Repeat::All);
    p.replace(vec![track("a"), track("b"), track("c")], 0).unwrap();
    let mut seed = 7_u64;
    let mut stale = Vec::new();
    for _ in 0..5_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let old = p.ticket();
        match (seed >> 32) % 7 {
            0 => { p.next_track().unwrap(); }
            1 => { p.previous_track().unwrap(); }
            2 => { let _ = p.set_paused(true); }
            3 => { let _ = p.set_paused(false); }
            4 => { let _ = p.seek(seed % 180_001); }
            5 => {
                let event = match p.phase() {
                    Phase::Loading => Event::Ready,
                    Phase::Seeking => Event::Seeked { position_ms: 0 },
                    _ => Event::Progress { position_ms: seed % 180_001 },
                };
                p.receive(old, event).unwrap();
            }
            _ => {
                if let Some(t) = stale.first().copied() {
                    assert_eq!(p.receive(t, Event::Failed), Ok(None));
                }
            }
        }
        if old != p.ticket() {
            stale.push(old);
            assert!(p.ticket().revision() > old.revision());
        }
        assert_eq!(p.queue_len(), 3);
        assert!(p.current().is_some());
        assert!(p.position_ms() <= 180_000);
        assert!(p.upcoming(usize::MAX).len() <= 2);
    }
}
