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
    // This profiler is intentionally opt-in: aq_trace adds synchronization
    // and timestamping overhead, so ordinary latency measurements must not
    // use the trace as a speedup baseline.
    let trace_output = std::env::var_os("AQ_REWRITE_TRACE");
    if trace_output.is_some() {
        aq_trace::start();
    }
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
        let mut stages = Vec::with_capacity(cycles);
        let mut editor_profiles = Vec::with_capacity(cycles);
        let mut parley_restored = 0usize;
        let _ = crate::ui::editor::take_ui_restore_hits();
        if trace_output.is_some() {
            aq_trace::mark(match kib {
                64 => "akula: matrix 64 begin",
                512 => "akula: matrix 512 begin",
                4096 => "akula: matrix 4096 begin",
                _ => unreachable!(),
            });
        }
        for i in 0..cycles {
            let (id, path, letter) = if i % 2 == 0 { (first_id, &first, "A") } else { (second_id, &second, "B") };
            let timer = Instant::now();
            h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(id), Size::new(f64::from(W), f64::from(H)));
            let switched_ms = milliseconds(timer.elapsed());
            stages.push(crate::app::tabs::open::take_tab_switch_stages());
            editor_profiles.push(crate::ui::note::page::take_page_profile());
            let timer = Instant::now();
            let _ = h.root.redraw();
            let scene_ms = milliseconds(timer.elapsed());
            parley_restored += crate::ui::editor::take_ui_restore_hits();
            assert_open_note(&h, path, letter);
            switches.push(switched_ms);
            scenes.push(scene_ms);
            h.app.on_core_events(&mut h.root);
        }
        if trace_output.is_some() {
            aq_trace::mark(match kib {
                64 => "akula: matrix 64 end",
                512 => "akula: matrix 512 end",
                4096 => "akula: matrix 4096 end",
                _ => unreachable!(),
            });
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

        let p95_stage = |field: fn(&crate::app::tabs::open::TabSwitchStages) -> Duration| {
            let samples: Vec<f64> = stages.iter().map(|sample| milliseconds(field(sample))).collect();
            percentiles(&samples, [0.95])[0]
        };
        let leave_p95 = p95_stage(|s| s.leave);
        let read_p95 = p95_stage(|s| s.file_read);
        let page_p95 = p95_stage(|s| s.page_build);
        let tree_p95 = p95_stage(|s| s.tree);
        let tab_p95 = p95_stage(|s| s.tab);
        let version_p95 = p95_stage(|s| s.version);
        let strip_p95 = p95_stage(|s| s.strip);
        let panel_p95 = p95_stage(|s| s.panel);
        let cache_hits = stages.iter().filter(|stage| stage.cache_hit).count();
        let p95_editor = |field: fn(&crate::ui::note::page::EditorPageProfile) -> Duration| {
            let samples: Vec<f64> = editor_profiles.iter().map(|profile| milliseconds(field(profile))).collect();
            percentiles(&samples, [0.95])[0]
        };
        let constructor_p95 = p95_editor(|p| p.constructor);
        let rescan_p95 = p95_editor(|p| p.document_rescan);
        let initial_window_p95 = p95_editor(|p| p.document_window);
        let markdown_p95 = p95_editor(|p| p.markdown);
        let page_total_p95 = p95_editor(|p| p.page_total);
        let install_p95 = p95_editor(|p| p.install);
        let slot_pod_p95 = p95_editor(|p| p.slot_pod);
        let slot_remove_p95 = p95_editor(|p| p.slot_remove);
        let slot_notify_p95 = p95_editor(|p| p.slot_notify);
        let slot_framework_p95 = p95_editor(|p| p.slot_framework);
        let save_error_refresh_p95 = p95_editor(|p| p.save_error_refresh);
        let [switch_p50, switch_p95, switch_max] = percentiles(&switches, [0.5, 0.95, 1.0]);
        let [_, scene_p95, scene_max] = percentiles(&scenes, [0.5, 0.95, 1.0]);
        let [_, dispatch_p95, dispatch_max] = percentiles(&dispatches, [0.5, 0.95, 1.0]);
        eprintln!(
            "NOTE_LATENCY kib={kib} cycles={cycles} cold_open_ms={cold_open_ms:.3} first_scene_ms={first_scene_ms:.3} \
             switch_p50_ms={switch_p50:.3} switch_p95_ms={switch_p95:.3} switch_max_ms={switch_max:.3} \
             scene_p95_ms={scene_p95:.3} scene_max_ms={scene_max:.3} \
             dispatch_p95_ms={dispatch_p95:.3} dispatch_max_ms={dispatch_max:.3} \
             external_visible_ms={external_visible_ms:.3} cache_hits={cache_hits} parley_restored={parley_restored} stale_overwrites=0"
        );
        eprintln!(
            "TAB_STAGE kib={kib} cycles={cycles} leave_p95_ms={leave_p95:.3} read_p95_ms={read_p95:.3} \
             page_p95_ms={page_p95:.3} tree_p95_ms={tree_p95:.3} tab_p95_ms={tab_p95:.3} \
             version_p95_ms={version_p95:.3} strip_p95_ms={strip_p95:.3} panel_p95_ms={panel_p95:.3}"
        );
        eprintln!(
            "EDITOR_PAGE kib={kib} cycles={cycles} constructor_p95_ms={constructor_p95:.3} \
             rescan_p95_ms={rescan_p95:.3} initial_window_p95_ms={initial_window_p95:.3} \
             markdown_p95_ms={markdown_p95:.3} page_total_p95_ms={page_total_p95:.3} \
             install_p95_ms={install_p95:.3}"
        );
        eprintln!(
            "EDITOR_MOUNT kib={kib} cycles={cycles} slot_pod_p95_ms={slot_pod_p95:.3} \
             slot_remove_p95_ms={slot_remove_p95:.3} slot_notify_p95_ms={slot_notify_p95:.3} \
             framework_p95_ms={slot_framework_p95:.3} save_error_refresh_p95_ms={save_error_refresh_p95:.3}"
        );
        // Cleanup is deliberately excluded from the measured operation times.
        // Shutdown joins background search/index workers before TempDir removal.
        h.app.shutdown();
    }
    if let Some(path) = trace_output {
        aq_trace::write(std::path::Path::new(&path)).expect("failed to write Masonry rewrite trace");
    }
}
