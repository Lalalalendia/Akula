use akula_game_wave_model::{ConfigError, Event, Group, Scheduler, Spawn, Wave};

fn wave(name: &str, groups: Vec<Group>) -> Wave {
    Wave { name: name.into(), timeout: None, groups }
}

#[test]
fn empty_config_is_rejected() {
    assert!(matches!(Scheduler::new(vec![]), Err(ConfigError::NoWaves)));
}

#[test]
fn first_spawn_is_immediate_then_frequency_is_strict() {
    let mut engine = Scheduler::new(vec![wave("first", vec![
        Group::Batch { frequency: 2, items: vec![Spawn::enemy("basic"), Spawn::power_up("health")] },
    ])]).unwrap();
    let first = engine.tick().unwrap();
    assert!(matches!(&first[..], [Event::Spawned { id: 1, item, .. }] if item.archetype == "basic"));
    assert!(engine.tick().unwrap().is_empty());
    assert!(engine.tick().unwrap().is_empty());
    let second = engine.tick().unwrap();
    assert!(matches!(&second[..], [Event::Spawned { id: 2, item, .. }] if item.archetype == "health"));
}

#[test]
fn next_batch_starts_after_last_spawn_without_waiting_for_kill() {
    let mut engine = Scheduler::new(vec![wave("w", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("a")] },
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("b")] },
    ])]).unwrap();
    assert!(matches!(&engine.tick().unwrap()[..], [Event::Spawned { group: 0, .. }]));
    assert!(matches!(&engine.tick().unwrap()[..], [Event::Spawned { group: 1, .. }]));
    assert_eq!(engine.active_count(), 2);
}

#[test]
fn killed_objects_allow_wave_to_finish_after_transition_delay() {
    let mut engine = Scheduler::new(vec![
        wave("a", vec![Group::Batch { frequency: 0, items: vec![Spawn::enemy("a")] }]),
        wave("b", vec![Group::Batch { frequency: 0, items: vec![Spawn::enemy("b")] }]),
    ]).unwrap();
    let first = engine.tick().unwrap();
    let id = match first.as_slice() {
        [Event::Spawned { id, .. }] => *id,
        _ => panic!("first tick must spawn"),
    };
    assert!(engine.mark_gone(id));
    assert!(!engine.mark_gone(id));
    // The source consumes one update to advance past the last group,
    // then starts its 0..150 transition counter on a later update.
    for _ in 0..151 {
        assert!(engine.tick().unwrap().is_empty());
    }
    let events = engine.tick().unwrap();
    assert!(matches!(&events[..], [Event::WaveChanged { wave: 1, .. }, Event::Spawned { wave: 1, .. }]));
}

#[test]
fn stopped_wave_waits_for_live_objects() {
    let mut engine = Scheduler::new(vec![wave("solo", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("a")] },
    ])]).unwrap();
    engine.tick().unwrap();
    for _ in 0..155 {
        assert!(engine.tick().unwrap().is_empty());
    }
    assert!(!engine.is_finished());
    assert_eq!(engine.active_count(), 1);
}

#[test]
fn pause_uses_strict_threshold() {
    let mut engine = Scheduler::new(vec![wave("p", vec![
        Group::Pause { ticks: 2 },
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("a")] },
    ])]).unwrap();
    for _ in 0..3 {
        assert!(engine.tick().unwrap().is_empty());
    }
    assert!(matches!(&engine.tick().unwrap()[..], [Event::Spawned { group: 1, .. }]));
}

#[test]
fn chooser_produces_repeatable_spawn_order() {
    let groups = || vec![Group::Batch {
        frequency: 0,
        items: vec![Spawn::enemy("a"), Spawn::enemy("b"), Spawn::enemy("c")],
    }];
    let mut one = Scheduler::with_shuffle(vec![wave("x", groups())], |_| 0).unwrap();
    let mut two = Scheduler::with_shuffle(vec![wave("x", groups())], |_| 0).unwrap();
    for _ in 0..3 {
        assert_eq!(one.tick().unwrap(), two.tick().unwrap());
    }
}

#[test]
fn invalid_shuffle_index_is_rejected() {
    let result = Scheduler::with_shuffle(vec![wave("x", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("a"), Spawn::enemy("b")] },
    ])], |n| n);
    assert!(matches!(result, Err(ConfigError::BadShuffleIndex { .. })));
}

#[test]
fn cycle_with_zero_repeats_keeps_emitting_until_prior_groups_complete() {
    let mut engine = Scheduler::new(vec![wave("cycle", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("anchor")] },
        Group::Cycle { frequency: 0, rest: 0, repeats: 0, items: vec![Spawn::enemy("repeat")] },
    ])]).unwrap();
    let first = engine.tick().unwrap();
    let anchor = match first.as_slice() { [Event::Spawned { id, .. }] => *id, _ => panic!("missing anchor") };
    let e2 = engine.tick().unwrap();
    assert!(matches!(&e2[..], [Event::Spawned { group: 1, .. }]));
    assert!(engine.tick().unwrap().is_empty());
    let e4 = engine.tick().unwrap();
    assert!(matches!(&e4[..], [Event::Spawned { group: 1, .. }]));
    assert!(engine.mark_gone(anchor));
}

#[test]
fn earlier_cycled_objects_remain_live_when_slot_is_reused() {
    let mut engine = Scheduler::new(vec![wave("cycle", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("anchor")] },
        Group::Cycle { frequency: 0, rest: 0, repeats: 0, items: vec![Spawn::enemy("repeat")] },
    ])]).unwrap();
    engine.tick().unwrap();
    engine.tick().unwrap();
    engine.tick().unwrap();
    engine.tick().unwrap();
    assert_eq!(engine.active_count(), 3);
    assert!(engine.mark_gone(2));
    assert_eq!(engine.active_count(), 2);
}

#[test]
fn aggregate_group_budget_rejects_many_small_waves() {
    let many = (0..5).map(|_| wave("x", (0..900).map(|_| Group::Pause { ticks: 0 }).collect())).collect();
    assert!(matches!(Scheduler::new(many), Err(ConfigError::TooManyTotalGroups)));
}

#[test]
fn empty_wave_ends_after_150_transition_updates() {
    let mut engine = Scheduler::new(vec![wave("empty", vec![])]).unwrap();
    for _ in 0..150 {
        assert!(engine.tick().unwrap().is_empty());
        assert!(!engine.is_finished());
    }
    assert_eq!(engine.tick().unwrap(), vec![Event::Finished]);
    assert!(engine.is_finished());
    assert!(engine.tick().unwrap().is_empty());
}

#[test]
fn no_active_object_is_removed_by_unknown_or_repeated_removal() {
    let mut engine = Scheduler::new(vec![wave("group", vec![
        Group::Batch { frequency: 0, items: vec![Spawn::enemy("x")] },
    ])]).unwrap();
    let id = match engine.tick().unwrap().as_slice() {
        [Event::Spawned { id, .. }] => *id,
        _ => panic!("expected object"),
    };
    assert!(!engine.mark_gone(id + 1));
    assert_eq!(engine.active_count(), 1);
    assert!(engine.mark_gone(id));
    assert!(!engine.mark_gone(id));
    assert_eq!(engine.active_count(), 0);
}

#[test]
fn finished_state_emits_final_event_only_once() {
    let mut engine = Scheduler::new(vec![wave("empty", vec![])]).unwrap();
    let mut finishes = 0;
    for _ in 0..500 {
        finishes += engine.tick().unwrap().iter()
            .filter(|event| matches!(event, Event::Finished)).count();
    }
    assert_eq!(finishes, 1);
}
