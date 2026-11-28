use akula_music::TrackKey;
use akula_music::playback::worker::frames::{FrameError, RenderedClock};
use akula_music::playback::{Event, Player, Ticket, Track};
use std::num::NonZeroU64;

fn start(id: u64) -> (Player, Ticket) {
    let mut p = Player::new(NonZeroU64::new(id).unwrap());
    let track = Track::new(TrackKey::new("mock", "a").unwrap(), 180_000).unwrap();
    let load = p.replace(vec![track], 0).unwrap().ticket();
    (p, load)
}

#[test]
fn frame_time_uses_output_rate_not_channel_count_or_buffer_rounding() {
    let (mut p, load) = start(1);
    let mut clock = RenderedClock::new(load, 44_100).unwrap();
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    clock.authorize(run, true).unwrap();
    for frame in 1..=44_100 {
        clock.observe(run, frame).unwrap();
    }
    assert_eq!(clock.frames(), 44_100);
    assert_eq!(clock.elapsed_ms(), 1_000);
}

#[test]
fn held_output_does_not_count_and_pause_resume_preserves_total() {
    let (mut p, load) = start(1);
    let mut clock = RenderedClock::new(load, 48_000).unwrap();
    assert_eq!(clock.observe(load, 1), Err(FrameError::RenderedWhileHeld));
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    clock.authorize(run, true).unwrap();
    clock.observe(run, 48_000).unwrap();
    let pause = p.set_paused(true).unwrap().unwrap().ticket();
    clock.authorize(pause, false).unwrap();
    assert_eq!(clock.observe(run, 96_000), Ok(false));
    assert_eq!(clock.observe(pause, 48_001), Err(FrameError::RenderedWhileHeld));
    let resume = p.set_paused(false).unwrap().unwrap().ticket();
    clock.authorize(resume, true).unwrap();
    clock.observe(resume, 96_000).unwrap();
    assert_eq!(clock.elapsed_ms(), 2_000);
}

#[test]
fn seek_position_never_becomes_listening_time() {
    let (mut p, load) = start(1);
    let mut clock = RenderedClock::new(load, 48_000).unwrap();
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    clock.authorize(run, true).unwrap();
    clock.observe(run, 24_000).unwrap();
    let seek = p.seek(150_000).unwrap().ticket();
    clock.authorize(seek, false).unwrap();
    let new = p.receive(seek, Event::Seeked { position_ms: 150_000 }).unwrap().unwrap().ticket();
    clock.authorize(new, true).unwrap();
    assert_eq!(clock.elapsed_ms(), 500);
    clock.observe(new, 48_000).unwrap();
    assert_eq!(clock.elapsed_ms(), 1_000);
}

#[test]
fn replay_and_other_owner_cannot_modify_previous_session() {
    let (mut p, load) = start(1);
    let mut clock = RenderedClock::new(load, 48_000).unwrap();
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    clock.authorize(run, true).unwrap();
    let next = p.previous_track().unwrap().ticket();
    let (_, foreign) = start(2);
    assert_eq!(clock.authorize(next, true), Err(FrameError::InvalidSession));
    assert_eq!(clock.authorize(foreign, true), Err(FrameError::InvalidSession));
    assert_eq!(clock.observe(next, 48_000), Ok(false));
    assert_eq!(clock.elapsed_ms(), 0);
}

#[test]
fn duplicate_regressed_and_large_counters_are_safe() {
    let (mut p, load) = start(1);
    let mut clock = RenderedClock::new(load, 8_000).unwrap();
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    clock.authorize(run, true).unwrap();
    clock.observe(run, u64::MAX).unwrap();
    assert_eq!(clock.observe(run, u64::MAX), Ok(false));
    assert_eq!(clock.observe(run, u64::MAX - 1), Err(FrameError::CounterRegressed));
    assert_eq!(clock.elapsed_ms(), u64::MAX / 8);
    assert_eq!(clock.authorize(run, false), Err(FrameError::InvalidRevision));
}

#[test]
fn invalid_rate_or_nonload_ticket_is_rejected() {
    let (mut p, load) = start(1);
    assert!(matches!(RenderedClock::new(load, 0), Err(FrameError::InvalidRate)));
    assert!(matches!(RenderedClock::new(load, 384_001), Err(FrameError::InvalidRate)));
    let run = p.receive(load, Event::Ready).unwrap().unwrap().ticket();
    assert!(matches!(RenderedClock::new(run, 48_000), Err(FrameError::InvalidSession)));
}
