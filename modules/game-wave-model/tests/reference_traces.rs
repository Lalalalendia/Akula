//! Source-derived frame expectations for a tiny, synthetic game.
//!
//! Timing was reconstructed independently from Wave.as / WaveController.as
//! (Tetraform original author: Tyler Glaiel). These are NOT traces captured
//! from Flash Player. No third-party code or game data are embedded here.

use akula_game_wave_model::{Event, Group, Scheduler, Spawn, Wave};

struct Case {
    name: &'static str,
    frames: u32,
    waves: Vec<Wave>,
    kills_after_frame: &'static [(u32, u64)],
    end_alive: usize,
}

fn wave(name: &str, groups: Vec<Group>) -> Wave {
    Wave { name: name.into(), timeout: None, groups }
}

fn batch(frequency: u64, names: &[&str]) -> Group {
    Group::Batch {
        frequency,
        items: names.iter().map(|name| Spawn::enemy(name)).collect(),
    }
}

fn cycle(frequency: u64, rest: u64, repeats: u32, names: &[&str]) -> Group {
    Group::Cycle {
        frequency,
        rest,
        repeats,
        items: names.iter().map(|name| Spawn::enemy(name)).collect(),
    }
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "first_two", frames: 8,
            waves: vec![wave("one", vec![batch(2, &["a", "b"])])],
            kills_after_frame: &[], end_alive: 2,
        },
        Case {
            name: "pause_two", frames: 8,
            waves: vec![wave("p", vec![Group::Pause { ticks: 2 }, batch(0, &["a"])])],
            kills_after_frame: &[], end_alive: 1,
        },
        Case {
            name: "next_wave", frames: 160,
            waves: vec![
                wave("a", vec![batch(0, &["a"])]),
                wave("b", vec![batch(0, &["b"])]),
            ],
            kills_after_frame: &[(1, 1)], end_alive: 1,
        },
        Case {
            name: "forced_timeout", frames: 165,
            waves: vec![Wave {
                name: "a".into(), timeout: Some(2),
                groups: vec![batch(0, &["a"])],
            }],
            kills_after_frame: &[], end_alive: 1,
        },
        Case {
            name: "empty", frames: 160,
            waves: vec![wave("empty", vec![])],
            kills_after_frame: &[], end_alive: 0,
        },
        Case {
            name: "cycle_unbounded", frames: 16,
            waves: vec![wave("c", vec![
                batch(0, &["anchor"]), cycle(1, 2, 0, &["repeat"]),
            ])],
            kills_after_frame: &[], end_alive: 4,
        },
        Case {
            name: "cycle_stops", frames: 12,
            waves: vec![wave("c", vec![
                batch(0, &["anchor"]), cycle(1, 2, 0, &["repeat"]),
            ])],
            kills_after_frame: &[(3, 1)], end_alive: 1,
        },
        Case {
            name: "pause_with_kill", frames: 160,
            waves: vec![wave("p", vec![Group::Pause { ticks: 2 }, batch(0, &["a"])])],
            kills_after_frame: &[(4, 1)], end_alive: 0,
        },
    ]
}

#[test]
fn source_derived_frame_sequences_match() {
    let mut observed = Vec::new();
    for case in cases() {
        // Identity permutation gives a fully deterministic synthetic test.
        // No claim is made about the random sequence of a real Flash Player.
        let mut scheduler = Scheduler::with_shuffle(case.waves, |n| n - 1)
            .expect("fixture configuration must be valid");
        for frame in 1..=case.frames {
            for event in scheduler.tick().expect("fixture must remain within budgets") {
                let row = match event {
                    Event::Spawned { wave, group, id, item } => format!(
                        "{}\t{}\tspawn\t{}\t{}\t{}\t{}",
                        case.name, frame, wave, group, id, item.archetype,
                    ),
                    Event::WaveChanged { wave, name } => format!(
                        "{}\t{}\twave\t{}\t{}", case.name, frame, wave, name,
                    ),
                    Event::Finished => format!("{}\t{}\tfinish", case.name, frame),
                };
                observed.push(row);
            }
            for &(kill_frame, id) in case.kills_after_frame {
                if frame == kill_frame {
                    assert!(scheduler.mark_gone(id), "{}: cannot remove {id}", case.name);
                }
            }
        }
        assert_eq!(scheduler.active_count(), case.end_alive, "{}: alive count", case.name);
    }
    let expected: Vec<String> = include_str!("fixtures/source_derived_traces.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert_eq!(observed, expected, "source-derived per-frame trace differs");
}
