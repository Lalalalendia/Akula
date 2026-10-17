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
            let elapsed = started.elapsed();
            let (layers, _) = h.root.redraw();
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
                let tab_ms = elapsed.as_secs_f64() * 1000.0;
                if enabled {
                    new_page.push(page_ms);
                    new_tab.push(tab_ms);
                } else {
                    old_page.push(page_ms);
                    old_tab.push(tab_ms);
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

    eprintln!(
        "PARLEY_TAB_ABBA note_bytes={} cases={} restored={} parity={} \
         baseline_page_p50_ms={old_p50:.3} candidate_page_p50_ms={new_p50:.3} \
         baseline_page_p95_ms={old_p95:.3} candidate_page_p95_ms={new_p95:.3} \
         baseline_switch_p50_ms={old_tab_p50:.3} candidate_switch_p50_ms={new_tab_p50:.3} \
         baseline_switch_p95_ms={old_tab_p95:.3} candidate_switch_p95_ms={new_tab_p95:.3}",
        original.len(), old_page.len(), true_hits, parity
    );

    // The same-length/file timestamp is not trusted: an external file change
    // must invalidate both the Rope cache and the offered Parley snapshot.
    h.app.parley_trial_enabled = true;
    let _ = select(&mut h, &second);
    let changed = original.replacen("Абзац", "Сцена", 1);
    assert_eq!(original.len(), changed.len(), "must not depend on byte length");
    std::fs::write(&first, changed).unwrap();
    let stages = select(&mut h, &first);
    let _ = h.root.redraw();
    let hits = crate::ui::editor::take_trial_restore_count();
    assert!(!stages.cache_hit, "BLAKE3 must reject external change");
    assert_eq!(hits, 0, "stale external bytes must not restore Parley layout");
    eprintln!("PARLEY_TAB_FRESHNESS stale_hash_rejected=true restored=0");
}
