//! Reproducible native UI timing probes. Run separately, not in the required
//! workspace correctness suite: cargo test --locked -p aquilum-native
//! --bin aquilum-native native_note_latency_matrix -- --ignored --nocapture
//! --test-threads=1
//!
//! Timings include native UI model/layout; no GPU presentation or OS compositor.
//! Report distributions instead of runner-sensitive hard performance budgets.

use super::*;
use std::io::{Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

fn fixture_bytes(marker: char, target: usize) -> String {
    let line = "Строка заметки: UTF-8 🌐, Markdown **жирный** и текст для переноса.\n";
    let mut content = String::with_capacity(target);
    content.push(marker);
    while content.len() + line.len() <= target {
        content.push_str(line);
    }
    content.push_str(&" ".repeat(target - content.len()));
    assert_eq!(content.len(), target);
    content
}

fn replace_first_byte(path: &std::path::Path, letter: u8) {
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&[letter]).unwrap();
    file.flush().unwrap();
}

fn assert_open_note(h: &Harness, path: &std::path::Path, expected: &str) {
    assert!(h.app.note.as_ref().is_some_and(|n| n.is(path)), "wrong active note");
    assert_eq!(h.app.text.byte_slice(..1).to_string(), expected, "stale worker replaced the active note");
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
fn rapid_tab_switches_never_adopt_a_previous_notes_background_reload() {
    let mut h = Harness::new();
    let first = h.write_note("first-latency.md", &fixture_bytes('A', 48 * 1024));
    let second = h.write_note("second-latency.md", &fixture_bytes('B', 48 * 1024));
    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    assert!(h.app.open_in(&mut h.root, second.clone(), true));
    let first_id = h.app.tabs.tabs().iter().find(|t| t.path.as_deref() == Some(first.as_path())).unwrap().id;
    let second_id = h.app.tabs.tabs().iter().find(|t| t.path.as_deref() == Some(second.as_path())).unwrap().id;

    for i in 0..20 {
        let (id, path, marker) = if i % 2 == 0 { (first_id, &first, "A") } else { (second_id, &second, "B") };
        h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(id), Size::new(f64::from(W), f64::from(H)));
        assert_open_note(&h, path, marker);
        h.app.workspace.core.emit(aquilum_core::CoreEvent::ContentChanged(vec![path.to_string_lossy().into_owned()]));
        h.app.on_core_events(&mut h.root);
        assert_open_note(&h, path, marker);
    }

    h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(second_id), Size::new(f64::from(W), f64::from(H)));
    for _ in 0..20 {
        h.app.on_core_events(&mut h.root);
        assert_open_note(&h, &second, "B");
        std::thread::sleep(Duration::from_millis(2));
    }
    h.app.shutdown(); // stop indexing before deleting temporary vault files
}

#[test]
#[ignore = "opt-in perf matrix: run on an isolated CI runner with --ignored"]
fn native_note_latency_matrix() {
    let cycles: usize = std::env::var("AQ_LATENCY_SWITCHES").ok().and_then(|x| x.parse().ok()).unwrap_or(16);
    assert!((2..=100).contains(&cycles));
    for kib in [64, 512, 4096] {
        let mut h = Harness::new();
        let target = kib * 1024;
        let first = h.write_note("latency-first.md", &fixture_bytes('A', target));
        let second = h.write_note("latency-second.md", &fixture_bytes('B', target));

        let timer = Instant::now();
        assert!(h.app.open_in(&mut h.root, first.clone(), true));
        let cold_open_ms = milliseconds(timer.elapsed());
        let timer = Instant::now();
        let _ = h.root.redraw();
        let first_scene_ms = milliseconds(timer.elapsed());
        assert_open_note(&h, &first, "A");

        assert!(h.app.open_in(&mut h.root, second.clone(), true));
        let _ = h.root.redraw();
        let first_id = h.app.tabs.tabs().iter().find(|t| t.path.as_deref() == Some(first.as_path())).unwrap().id;
        let second_id = h.app.tabs.tabs().iter().find(|t| t.path.as_deref() == Some(second.as_path())).unwrap().id;

        let mut switches = Vec::with_capacity(cycles);
        let mut scenes = Vec::with_capacity(cycles);
        for i in 0..cycles {
            let (id, path, letter) = if i % 2 == 0 { (first_id, &first, "A") } else { (second_id, &second, "B") };
            let timer = Instant::now();
            h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(id), Size::new(f64::from(W), f64::from(H)));
            let switched_ms = milliseconds(timer.elapsed());
            let timer = Instant::now();
            let _ = h.root.redraw();
            let scene_ms = milliseconds(timer.elapsed());
            assert_open_note(&h, path, letter);
            switches.push(switched_ms);
            scenes.push(scene_ms);
            h.app.on_core_events(&mut h.root);
        }

        // Event burst from an unchanged file: time only the UI-thread
        // dispatch. The actual file hashing runs on an asynchronous worker.
        h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(first_id), Size::new(f64::from(W), f64::from(H)));
        let mut dispatches = Vec::new();
        for _ in 0..12 {
            h.app.workspace.core.emit(aquilum_core::CoreEvent::ContentChanged(vec![first.to_string_lossy().into_owned()]));
            let timer = Instant::now();
            h.app.on_core_events(&mut h.root);
            dispatches.push(milliseconds(timer.elapsed()));
        }

        // End-to-end changed-file delivery includes background IO, scheduling,
        // and adoption into the currently visible note.
        replace_first_byte(&first, b'C');
        h.app.workspace.core.emit(aquilum_core::CoreEvent::ContentChanged(vec![first.to_string_lossy().into_owned()]));
        let timer = Instant::now();
        let delivered = super::super::support::wait_until(2000, 2, || {
            h.app.on_core_events(&mut h.root);
            h.app.text.byte_slice(..1).to_string() == "C"
        });
        assert!(delivered, "{kib} KiB external change was not adopted within four seconds");
        let external_visible_ms = milliseconds(timer.elapsed());
        assert_open_note(&h, &first, "C");

        // Deliberately switch while a newer worker can still be in flight.
        replace_first_byte(&first, b'D');
        h.app.workspace.core.emit(aquilum_core::CoreEvent::ContentChanged(vec![first.to_string_lossy().into_owned()]));
        h.app.on_core_events(&mut h.root);
        h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(second_id), Size::new(f64::from(W), f64::from(H)));
        for _ in 0..12 {
            h.app.on_core_events(&mut h.root);
            assert_open_note(&h, &second, "B");
            std::thread::sleep(Duration::from_millis(2));
        }
        h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(first_id), Size::new(f64::from(W), f64::from(H)));
        assert_open_note(&h, &first, "D");

        let [switch_p50, switch_p95, switch_max] = percentiles(&switches, [0.5, 0.95, 1.0]);
        let [_, scene_p95, scene_max] = percentiles(&scenes, [0.5, 0.95, 1.0]);
        let [_, dispatch_p95, dispatch_max] = percentiles(&dispatches, [0.5, 0.95, 1.0]);
        eprintln!(
            "NOTE_LATENCY kib={kib} cycles={cycles} cold_open_ms={cold_open_ms:.3} first_scene_ms={first_scene_ms:.3} \
             switch_p50_ms={switch_p50:.3} switch_p95_ms={switch_p95:.3} switch_max_ms={switch_max:.3} \
             scene_p95_ms={scene_p95:.3} scene_max_ms={scene_max:.3} \
             dispatch_p95_ms={dispatch_p95:.3} dispatch_max_ms={dispatch_max:.3} \
             external_visible_ms={external_visible_ms:.3} stale_overwrites=0"
        );
        // Cleanup is deliberately excluded from the measured operation times.
        // Shutdown joins background search/index workers before TempDir removal.
        h.app.shutdown();
    }
}
