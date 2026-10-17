//! Opt-in actual tab-switch A/B for the bounded Parley snapshot from PR #116.
//! This uses real Masonry page replacement and the normal BLAKE3-verified Rope
//! tab cache, not the synthetic Document-only benchmark.

use super::*;
use std::path::Path;
use std::time::Instant;

fn select(h: &mut Harness, path: &Path) -> crate::app::tabs::open::TabSwitchStages {
    let id = h
        .app
        .tabs
        .tabs()
        .iter()
        .find(|tab| tab.path.as_deref() == Some(path))
        .expect("tab exists")
        .id;
    h.app.tabs.select(id);
    h.app.show_active(&mut h.root);
    crate::app::tabs::open::take_tab_switch_stages()
}

#[test]
#[ignore = "diagnostic real tab switching only, no production cache enabled"]
fn benchmark_parley_real_tabs_abba() {
    let mut h = Harness::new();
    let original = dense_body(24_000);
    assert!(original.len() > 2_000_000 && original.len() < 8_000_000);
    let first = h.write_note("layout-A.md", &original);
    let second = h.write_note("layout-B.md", &dense_body(80));

    h.app.open(&mut h.root, first.clone());
    h.app.open_in(&mut h.root, second.clone(), true);
    let _ = h.root.redraw();
    let _ = crate::ui::editor::take_trial_restore_count();

    let mut old_page = Vec::new();
    let mut new_page = Vec::new();
    let mut old_tab = Vec::new();
    let mut new_tab = Vec::new();
    let mut old_scene = Vec::new();
    let mut new_scene = Vec::new();
    let mut true_hits = 0usize;
    let mut baseline_pixels: Option<Vec<u32>> = None;
    let mut parity = 0usize;

    // Warm each mode, then ABBA with the same tabs, files, fonts and UI
    // process. Ignore the first iteration of each mode while caches settle.
    for enabled in [false, true, true, false] {
        h.app.parley_trial_enabled = enabled;
        for iteration in 0..8 {
            let _ = select(&mut h, &second);
            let _ = crate::ui::editor::take_trial_restore_count();
            let started = Instant::now();
            let stages = select(&mut h, &first);
            let scene_started = Instant::now();
            let (layers, _) = h.root.redraw();
            let scene_elapsed = scene_started.elapsed();
            let elapsed = started.elapsed();
            let hits = crate::ui::editor::take_trial_restore_count();

            if enabled && iteration > 0 {
                true_hits += hits;
            } else if !enabled {
                assert_eq!(hits, 0, "disabled test-only cache restored layouts");
            }

            // Compare full real UI output in stable active tab identity.
            // Capturing exact bytes stays outside measured switch stages.
            if iteration % 4 == 1 {
                let full = render_full(&layers, W as u16, H as u16, 1.0);
                if let Some(original) = &baseline_pixels {
                    let changed = differing_pixels(original, full.frame_pixels());
                    assert_eq!(changed, 0, "Parley trial changed full UI pixels: {changed}");
                } else {
                    baseline_pixels = Some(full.frame_pixels().to_vec());
                }
                parity += 1;
            }

            if iteration > 0 {
                let page_ms = stages.page_build.as_secs_f64() * 1000.0;
                // Full switch+first scene is the user-relevant UI-thread
                // operation for this experiment. Page construction alone
                // excludes Parley shaping, and a pre-redraw stopwatch does too.
                let tab_ms = elapsed.as_secs_f64() * 1000.0;
                let scene_ms = scene_elapsed.as_secs_f64() * 1000.0;
                if enabled {
                    new_page.push(page_ms);
                    new_tab.push(tab_ms);
                    new_scene.push(scene_ms);
                } else {
                    old_page.push(page_ms);
                    old_tab.push(tab_ms);
                    old_scene.push(scene_ms);
                }
            }
        }
    }

    assert!(true_hits > 0, "real tab switches never restored any Parley paragraph layouts");
    assert!(parity >= 8 && baseline_pixels.is_some());
    assert_eq!(old_page.len(), new_page.len(), "ABBA cases must be balanced");

    let [old_p50, old_p95] = percentiles(&old_page, [0.5, 0.95]);
    let [new_p50, new_p95] = percentiles(&new_page, [0.5, 0.95]);
    let [old_tab_p50, old_tab_p95] = percentiles(&old_tab, [0.5, 0.95]);
    let [new_tab_p50, new_tab_p95] = percentiles(&new_tab, [0.5, 0.95]);
    let [old_scene_p50, old_scene_p95] = percentiles(&old_scene, [0.5, 0.95]);
    let [new_scene_p50, new_scene_p95] = percentiles(&new_scene, [0.5, 0.95]);

    eprintln!(
        "PARLEY_TAB_ABBA note_bytes={} cases={} restored={} parity={} \
         baseline_page_p50_ms={old_p50:.3} candidate_page_p50_ms={new_p50:.3} \
         baseline_page_p95_ms={old_p95:.3} candidate_page_p95_ms={new_p95:.3} \
         baseline_switch_p50_ms={old_tab_p50:.3} candidate_switch_p50_ms={new_tab_p50:.3} \
         baseline_switch_p95_ms={old_tab_p95:.3} candidate_switch_p95_ms={new_tab_p95:.3} \
         baseline_first_scene_p50_ms={old_scene_p50:.3} candidate_first_scene_p50_ms={new_scene_p50:.3} \
         baseline_first_scene_p95_ms={old_scene_p95:.3} candidate_first_scene_p95_ms={new_scene_p95:.3}",
        original.len(), old_page.len(), true_hits, parity
    );

    // The same-length/file timestamp is not trusted: an external file change
    // must invalidate both the Rope cache and the offered Parley snapshot.
    h.app.parley_trial_enabled = true;
    let _ = select(&mut h, &second);
    let changed = original.replacen("Абзац", "Сцена", 1);
    assert_eq!(original.len(), changed.len(), "must not depend on byte length");
    let original_mtime = std::fs::metadata(&first).unwrap().modified().unwrap();
    std::fs::write(&first, changed).unwrap();
    std::fs::File::open(&first).unwrap().set_modified(original_mtime).unwrap();
    let stages = select(&mut h, &first);
    let _ = h.root.redraw();
    let hits = crate::ui::editor::take_trial_restore_count();
    assert!(!stages.cache_hit, "BLAKE3 must reject external change");
    assert_eq!(hits, 0, "stale external bytes must not restore Parley layout");
    eprintln!("PARLEY_TAB_FRESHNESS stale_hash_rejected=true restored=0");
}

#[test]
#[ignore = "opt-in native pixel parity at two DPI scales and two editor wrap widths"]
fn benchmark_parley_real_tabs_multiscale_fidelity() {
    let mut scenarios = 0usize;
    // Both the actual Masonry root and the software renderer receive the
    // physical DPI-scaled viewport. Scaling only the PNG would not test the
    // editor's text layout and scrollbar transitions.
    for scale in [1.0, 1.75] {
        let mut h = Harness::with(None, scale);
        let first = h.write_note("parley-scale-A.md", &dense_body(4_000));
        let second = h.write_note("parley-scale-B.md", &dense_body(80));
        assert!(h.app.open_in(&mut h.root, first.clone(), true));
        assert!(h.app.open_in(&mut h.root, second.clone(), true));
        let _ = h.root.redraw();

        for width in [1000u32, 860] {
            let height = 600u32;
            let physical_width = (f64::from(width) * scale).round() as u32;
            let physical_height = (f64::from(height) * scale).round() as u32;
            h.root.handle_window_event(masonry::core::WindowEvent::Resize(
                PhysicalSize::new(physical_width, physical_height),
            ));
            h.app.window_resized(
                &mut h.root,
                Size::new(f64::from(width), f64::from(height)),
            );
            let _ = h.root.redraw();

            let mut baseline: Option<Vec<u32>> = None;
            let mut accepted = 0usize;
            // Do not compare merely offered snapshots. A real redraw must
            // apply Parley glyphs and produce identical entire UI pixels.
            for enabled in [false, true, true, false] {
                h.app.parley_trial_enabled = enabled;
                let _ = select(&mut h, &second);
                let _ = h.root.redraw();
                let _ = crate::ui::editor::take_trial_restore_count();
                let stages = select(&mut h, &first);
                let (layers, _) = h.root.redraw();
                let hits = crate::ui::editor::take_trial_restore_count();
                assert!(stages.cache_hit, "clean Rope must hit at scale {scale}, width {width}");
                if enabled {
                    assert!(hits > 0, "offered but no Parley was restored at scale {scale}, width {width}");
                    accepted += hits;
                } else {
                    assert_eq!(hits, 0, "baseline reused Parley at scale {scale}, width {width}");
                }
                let rendered = render_full(
                    &layers,
                    physical_width as u16,
                    physical_height as u16,
                    scale,
                );
                let pixels = rendered.frame_pixels();
                if let Some(original) = &baseline {
                    assert_eq!(
                        differing_pixels(original, pixels),
                        0,
                        "cached UI differs at DPI {scale}, logical width {width}",
                    );
                } else {
                    baseline = Some(pixels.to_vec());
                }
            }
            assert!(accepted > 0);
            scenarios += 1;
            eprintln!(
                "PARLEY_TAB_MULTISCALE scale={scale} width={width} actual_reused={accepted} differing_pixels=0"
            );
        }
        h.app.shutdown();
    }
    assert_eq!(scenarios, 4, "both DPI factors and both widths must be checked");
}
