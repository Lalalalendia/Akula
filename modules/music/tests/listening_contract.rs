use akula_music::{ListenRecord, ListenSession, Outcome, SessionError, Summary, SummaryError, TrackKey};

fn session(id: &str, provider: &str, track: &str, duration: u64) -> ListenSession {
    ListenSession::new(id, TrackKey::new(provider, track).unwrap(), duration).unwrap()
}

fn record(id: &str, provider: &str, track: &str, played: u64, outcome: Outcome) -> ListenRecord {
    let mut s = session(id, provider, track, 180_000);
    s.observe_rendered(played).unwrap();
    s.finish(outcome)
}

#[test]
fn rejects_empty_identity_and_zero_duration() {
    assert_eq!(TrackKey::new(" ", "a"), Err(SessionError::EmptyIdentity));
    assert_eq!(TrackKey::new("yandex", ""), Err(SessionError::EmptyIdentity));
    let key = TrackKey::new("yandex", "a").unwrap();
    assert!(matches!(
        ListenSession::new("", key.clone(), 1),
        Err(SessionError::EmptyIdentity)
    ));
    assert!(matches!(
        ListenSession::new("s", key, 0),
        Err(SessionError::InvalidDuration)
    ));
}

#[test]
fn pause_and_seek_do_not_create_listening_time() {
    let mut s = session("s", "yandex", "a", 180_000);
    assert_eq!(s.observe_rendered(5_000), Ok(true));
    s.set_paused(true);
    s.set_paused(true);
    s.note_seek();
    assert_eq!(s.observe_rendered(5_000), Ok(false));
    assert_eq!(
        s.observe_rendered(6_000),
        Err(SessionError::RenderedWhilePaused)
    );
    s.set_paused(false);
    assert_eq!(s.observe_rendered(8_000), Ok(true));
    let r = s.finish(Outcome::Skipped);
    assert_eq!(r.rendered_ms(), 8_000);
    assert_eq!(r.pauses(), 1);
    assert_eq!(r.seeks(), 1);
    assert!(!r.qualifies());
}

#[test]
fn duplicates_and_regression_leave_time_unchanged() {
    let mut s = session("s", "yandex", "a", 60_000);
    assert_eq!(s.observe_rendered(10_000), Ok(true));
    assert_eq!(s.observe_rendered(10_000), Ok(false));
    assert_eq!(s.observe_rendered(9_000), Err(SessionError::CounterRegressed));
    assert_eq!(s.finish(Outcome::Stopped).rendered_ms(), 10_000);
}

#[test]
fn repeated_region_can_exceed_media_duration() {
    let mut s = session("s", "yandex", "a", 10_000);
    s.observe_rendered(8_000).unwrap();
    s.note_seek();
    s.observe_rendered(15_000).unwrap();
    assert_eq!(s.finish(Outcome::NaturalEnd).rendered_ms(), 15_000);
}

#[test]
fn qualification_boundary_and_short_tracks() {
    for (duration, played, qualifies) in [
        (180_000, 29_999, false),
        (180_000, 30_000, true),
        (10_000, 9_999, false),
        (10_000, 10_000, true),
        (10_000, 0, false),
    ] {
        let mut s = session("s", "yandex", "a", duration);
        s.observe_rendered(played).unwrap();
        assert_eq!(s.finish(Outcome::Stopped).qualifies(), qualifies);
    }
}

#[test]
fn seek_to_end_is_not_a_qualified_play() {
    let mut s = session("s", "yandex", "a", 180_000);
    s.note_seek();
    s.observe_rendered(1_000).unwrap();
    let r = s.finish(Outcome::NaturalEnd);
    let summary = Summary::aggregate([&r]).unwrap();
    assert_eq!(summary.natural_ends, 1);
    assert_eq!(summary.qualified_plays, 0);
}

#[test]
fn repeated_tracks_and_provider_identity_remain_distinct() {
    let records = [
        record("1", "yandex", "a", 30_000, Outcome::NaturalEnd),
        record("2", "yandex", "a", 40_000, Outcome::Skipped),
        record("3", "local", "a", 50_000, Outcome::Stopped),
        record("4", "yandex", "b", 1_000, Outcome::Failed),
    ];
    let s = Summary::aggregate(&records).unwrap();
    assert_eq!(s.sessions, 4);
    assert_eq!(s.track_audio_ms, 121_000);
    assert_eq!(s.qualified_plays, 3);
    assert_eq!(s.unique_qualified_tracks, 2);
    assert_eq!(s.natural_ends, 1);
    assert_eq!(s.skipped, 1);
    assert_eq!(s.failed, 1);
}

#[test]
fn duplicate_delivery_does_not_double_count() {
    let r = record("1", "yandex", "a", 30_000, Outcome::Stopped);
    let s = Summary::aggregate([&r, &r.clone()]).unwrap();
    assert_eq!(s.sessions, 1);
    assert_eq!(s.track_audio_ms, 30_000);
}

#[test]
fn conflicting_identity_is_rejected() {
    let a = record("1", "yandex", "a", 30_000, Outcome::Stopped);
    let b = record("1", "yandex", "a", 40_000, Outcome::Stopped);
    assert_eq!(Summary::aggregate([&a, &b]), Err(SummaryError::ConflictingSession));
}

#[test]
fn large_counters_do_not_wrap() {
    let a = record("1", "yandex", "a", u64::MAX, Outcome::Stopped);
    let b = record("2", "yandex", "a", u64::MAX, Outcome::Stopped);
    assert_eq!(
        Summary::aggregate([&a, &b]).unwrap().track_audio_ms,
        u128::from(u64::MAX) * 2
    );
}
