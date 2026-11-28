use akula_music::TrackKey;
use akula_music::playback::worker::{
    BackendError, Command, Mailbox, MockBackend, Runtime, RuntimeError, SubmitError, COMMAND_CAPACITY,
};
use akula_music::playback::{Event, Phase, Player, Repeat, Ticket, Track, MAX_QUEUE};
use std::num::NonZeroU64;

fn owner(id: u64) -> NonZeroU64 {
    NonZeroU64::new(id).unwrap()
}

fn track(id: &str) -> Track {
    Track::new(TrackKey::new("mock", id).unwrap(), 180_000).unwrap()
}

fn replace() -> Command {
    Command::Replace { tracks: vec![track("a"), track("b"), track("c")], start: 0 }
}

fn send(rt: &mut Runtime, command: Command) {
    rt.submit(owner(1), command).unwrap();
    assert!(rt.step().unwrap());
}

fn playing(rt: &mut Runtime) -> Ticket {
    send(rt, replace());
    let load = rt.player().ticket();
    assert!(!rt.backend().playing());
    assert!(rt.complete(load, true).unwrap());
    assert!(rt.backend().playing());
    rt.player().ticket()
}

#[test]
fn bounded_mailbox_rejects_and_returns_command() {
    let mut m = Mailbox::new(owner(1));
    for _ in 0..COMMAND_CAPACITY {
        m.submit(owner(1), Command::Next).unwrap();
    }
    assert_eq!(m.submit(owner(1), Command::Seek(1)), Err((SubmitError::Full, Command::Seek(1))));
    assert_eq!(m.len(), COMMAND_CAPACITY);
    assert_eq!(m.submit(owner(2), Command::Stop), Err((SubmitError::WrongOwner, Command::Stop)));
    assert_eq!(m.len(), COMMAND_CAPACITY);
}

#[test]
fn stop_flushes_pending_commands_even_when_full() {
    let mut rt = Runtime::new(owner(1));
    let old = playing(&mut rt);
    for _ in 0..COMMAND_CAPACITY {
        rt.submit(owner(1), Command::Next).unwrap();
    }
    rt.submit(owner(1), Command::Stop).unwrap();
    assert_eq!(rt.queued(), 1);
    assert!(rt.step().unwrap());
    assert_eq!(rt.player().phase(), Phase::Idle);
    assert!(!rt.backend().playing());
    assert!(!rt.notify(old, Event::Ended).unwrap());
    assert!(!rt.step().unwrap());
}

#[test]
fn oversized_replace_does_not_consume_capacity() {
    let mut m = Mailbox::new(owner(1));
    let invalid = Command::Replace { tracks: vec![track("a"); MAX_QUEUE + 1], start: 0 };
    assert!(matches!(m.submit(owner(1), invalid), Err((SubmitError::InvalidQueue, _))));
    assert!(m.is_empty());
}

#[test]
fn rapid_loads_cancel_old_success_and_failure() {
    let mut rt = Runtime::new(owner(1));
    send(&mut rt, replace());
    let a = rt.player().ticket();
    send(&mut rt, Command::Next);
    let b = rt.player().ticket();
    send(&mut rt, Command::Next);
    let c = rt.player().ticket();
    assert!(!rt.complete(a, true).unwrap());
    assert!(!rt.complete(b, false).unwrap());
    assert!(rt.backend().pending());
    assert!(!rt.backend().playing());
    assert!(rt.complete(c, true).unwrap());
    assert!(!rt.complete(c, true).unwrap());
    assert!(rt.backend().playing());
    assert_eq!(rt.backend().track().unwrap().key().id(), "c");
}

#[test]
fn backend_holds_audio_between_ready_and_release() {
    let mut p = Player::new(owner(1));
    let mut b = MockBackend::new(owner(1));
    let load = p.replace(vec![track("a")], 0).unwrap();
    let t = load.ticket();
    b.accept(load).unwrap();
    assert_eq!(b.complete(t, true), Some(Event::Ready));
    assert!(!b.playing());
    let release = p.receive(t, Event::Ready).unwrap().unwrap();
    b.accept(release.clone()).unwrap();
    assert!(b.playing());
    assert_eq!(b.accept(release), Err(BackendError::StaleRevision));
    assert!(b.playing());
}

#[test]
fn stale_command_cannot_restart_audio() {
    let mut p = Player::new(owner(1));
    let mut b = MockBackend::new(owner(1));
    let load = p.replace(vec![track("a")], 0).unwrap();
    b.accept(load.clone()).unwrap();
    let stop = p.stop().unwrap();
    b.accept(stop).unwrap();
    assert_eq!(b.accept(load.clone()), Err(BackendError::StaleRevision));
    assert_eq!(b.complete(load.ticket(), true), None);
    assert!(!b.playing());
    assert!(!b.pending());
}

#[test]
fn newer_seek_replaces_previous_seek_and_preserves_pause_intent() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    send(&mut rt, Command::Seek(100_000));
    let a = rt.player().ticket();
    send(&mut rt, Command::Seek(5_000));
    let b = rt.player().ticket();
    send(&mut rt, Command::Pause(true));
    assert!(!rt.complete(a, true).unwrap());
    assert!(!rt.notify(run, Event::Ended).unwrap());
    assert!(!rt.backend().playing());
    assert!(rt.complete(b, true).unwrap());
    assert_eq!(rt.player().position_ms(), 5_000);
    assert_eq!(rt.player().phase(), Phase::Paused);
    assert!(!rt.backend().playing());
    send(&mut rt, Command::Pause(false));
    assert!(rt.backend().playing());
}

#[test]
fn provider_failure_and_retry_do_not_reuse_session() {
    let mut rt = Runtime::new(owner(1));
    send(&mut rt, replace());
    let first = rt.player().ticket();
    assert!(rt.complete(first, false).unwrap());
    assert_eq!(rt.player().phase(), Phase::Failed);
    assert!(!rt.backend().playing());
    send(&mut rt, Command::Retry);
    let retry = rt.player().ticket();
    assert_ne!(retry.session(), first.session());
    assert!(!rt.complete(first, true).unwrap());
    assert!(rt.complete(retry, true).unwrap());
    assert!(rt.backend().playing());
}

#[test]
fn owner_switch_drops_queue_and_stale_device_events() {
    let mut rt = Runtime::new(owner(1));
    let old = playing(&mut rt);
    rt.submit(owner(1), Command::Next).unwrap();
    assert_eq!(rt.switch_owner(owner(1)), Err(RuntimeError::OwnerReuse));
    rt.switch_owner(owner(2)).unwrap();
    assert_eq!(rt.queued(), 0);
    assert!(!rt.backend().playing());
    assert!(!rt.notify(old, Event::Ended).unwrap());
    assert_eq!(rt.submit(owner(1), Command::Stop), Err((SubmitError::WrongOwner, Command::Stop)));
    rt.submit(owner(2), replace()).unwrap();
    rt.step().unwrap();
    assert!(rt.complete(rt.player().ticket(), true).unwrap());
    assert!(rt.backend().playing());
}

#[test]
fn invalid_device_progress_poison_fails_closed() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    assert!(matches!(rt.notify(run, Event::Progress { position_ms: 180_001 }), Err(RuntimeError::Controller(_))));
    assert!(!rt.backend().playing());
    assert_eq!(rt.step(), Err(RuntimeError::Poisoned));
    assert_eq!(rt.complete(run, true), Err(RuntimeError::Poisoned));
}

#[test]
fn invalid_user_seek_does_not_poison_or_change_ticket() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    rt.submit(owner(1), Command::Seek(180_001)).unwrap();
    assert!(matches!(rt.step(), Err(RuntimeError::Controller(_))));
    assert_eq!(rt.player().ticket(), run);
    assert!(rt.backend().playing());
    send(&mut rt, Command::Pause(true));
    assert!(!rt.backend().playing());
}

#[test]
fn end_repeats_once_per_current_revision() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    send(&mut rt, Command::Repeat(Repeat::One));
    assert!(rt.notify(run, Event::Ended).unwrap());
    let new = rt.player().ticket();
    assert_ne!(run.session(), new.session());
    assert!(!rt.notify(run, Event::Ended).unwrap());
    assert!(!rt.backend().playing());
    assert!(rt.complete(new, true).unwrap());
    assert!(rt.backend().playing());
}

#[test]
fn foreign_owner_command_is_rejected_before_backend_mutation() {
    let mut p = Player::new(owner(2));
    let mut b = MockBackend::new(owner(1));
    let e = p.replace(vec![track("a")], 0).unwrap();
    assert_eq!(b.accept(e), Err(BackendError::WrongOwner));
    assert!(b.track().is_none());
    assert!(b.ticket().is_none());
}

#[test]
fn explicit_stop_during_seek_cancels_pending_io() {
    let mut rt = Runtime::new(owner(1));
    playing(&mut rt);
    send(&mut rt, Command::Seek(5_000));
    let seek = rt.player().ticket();
    send(&mut rt, Command::Stop);
    assert!(!rt.complete(seek, true).unwrap());
    assert!(!rt.backend().pending());
    assert!(!rt.backend().playing());
}

#[test]
fn queued_stop_precedes_racing_load_completion() {
    let mut rt = Runtime::new(owner(1));
    send(&mut rt, replace());
    let load = rt.player().ticket();
    rt.submit(owner(1), Command::Stop).unwrap();
    assert!(!rt.complete(load, true).unwrap());
    assert_eq!(rt.player().phase(), Phase::Idle);
    assert!(!rt.backend().playing());
    assert!(!rt.step().unwrap());
}

#[test]
fn queued_stop_precedes_racing_terminal_notification() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    send(&mut rt, Command::Repeat(Repeat::One));
    rt.submit(owner(1), Command::Stop).unwrap();
    assert!(!rt.notify(run, Event::Ended).unwrap());
    assert_eq!(rt.player().phase(), Phase::Idle);
    assert!(!rt.backend().pending());
    assert!(!rt.backend().playing());
}

#[test]
fn owner_ids_must_increase_and_poison_rejects_submission() {
    let mut rt = Runtime::new(owner(1));
    let run = playing(&mut rt);
    assert!(rt.notify(run, Event::Progress { position_ms: 180_001 }).is_err());
    assert_eq!(rt.submit(owner(1), Command::Next), Err((SubmitError::Poisoned, Command::Next)));
    rt.switch_owner(owner(2)).unwrap();
    assert_eq!(rt.switch_owner(owner(1)), Err(RuntimeError::OwnerReuse));
    assert_eq!(rt.queued(), 0);
    assert!(!rt.backend().playing());
}
