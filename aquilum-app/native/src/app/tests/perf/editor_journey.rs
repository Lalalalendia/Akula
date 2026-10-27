//! Opt-in real native editor journey: open, scroll, type, persist, raster.
//! Headless Masonry and CPU framebuffer only: no Winit present or monitor FPS.
//! No production branches, test-only runtime switches, or timing thresholds.

use super::*;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

fn source(lines: usize) -> String {
    let mut text = String::with_capacity(lines * 120);
    for i in 0..lines {
        if i % 80 == 0 {
            writeln!(&mut text, "## Раздел {i}: работа с большой заметкой").unwrap();
        } else {
            writeln!(
                &mut text,
                "Строка {i:06}: русский текст и UTF-8, **жирный**, *курсив*, [[Akula]]; \
                 длинная фраза для переноса текста в окне редактора."
            )
            .unwrap();
        }
    }
    text
}

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

fn p(values: &[f64]) -> [f64; 2] {
    assert!(values.len() >= 20, "insufficient user events for p95");
    percentiles(values, [0.5, 0.95])
}

fn run(lines: usize) {
    let mut h = Harness::new();
    let text = source(lines);
    let size = text.len();
    let note = h.write_note("large-editor-journey.md", &text);

    let start = Instant::now();
    assert!(h.app.open(&mut h.root, note.clone()), "note did not open");
    let open_ms = ms(start.elapsed());
    assert_eq!(h.app.text.byte_len(), size, "opening truncated source");

    let mut renderer = Renderer::new(2);
    let start = Instant::now();
    let (layers, _) = h.root.redraw();
    renderer
        .draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), true)
        .expect("initial editor CPU frame");
    let first_frame_ms = ms(start.elapsed());

    // A scrollbar jump loads a different part of a virtualized document.
    // Exclude the warm frame from the steady-state scroll distribution.
    let scroll = h.app.note_ids.as_ref().expect("editor is open").scroll.clone();
    let start = Instant::now();
    scroll.edit(&mut h.root, |mut area| {
        crate::ui::scroll::ScrollArea::set_fraction(&mut area, 0.5);
    });
    h.deliver();
    let (layers, _) = h.root.redraw();
    renderer
        .draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), true)
        .expect("jump CPU frame");
    let jump_ms = ms(start.elapsed());

    let initial_offset = super::super::super::support::note_offset(&h);
    let mut scroll_total = Vec::with_capacity(32);
    let mut scroll_input = Vec::with_capacity(32);
    let mut scroll_scene = Vec::with_capacity(32);
    let mut scroll_raster = Vec::with_capacity(32);
    for i in 0..32 {
        let started = Instant::now();
        let dy = if i < 24 { -34.0 } else { 27.0 };
        h.scroll((f64::from(W) * 0.6, f64::from(H) * 0.5), 1.0, (0.0, dy));
        h.anim(Duration::from_millis(16));
        let after_input = Instant::now();
        let (layers, _) = h.root.redraw();
        let after_scene = Instant::now();
        renderer
            .draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), false)
            .expect("scroll CPU frame");
        scroll_input.push(ms(after_input - started));
        scroll_scene.push(ms(after_scene - after_input));
        scroll_raster.push(ms(after_scene.elapsed()));
        scroll_total.push(ms(started.elapsed()));
    }
    let moved = (super::super::super::support::note_offset(&h) - initial_offset).abs();
    assert!(moved > 4.0, "scroll did not move the editor: {moved}");

    // Real editor pointer focus and character-key path, not a synthetic
    // TextAction without a corresponding Rope mutation.
    h.click(f64::from(W) * 0.6, f64::from(H) * 0.5);
    h.focus_editor();
    let before = h.app.text.byte_len();
    let mut typing_total = Vec::with_capacity(24);
    let mut typing_event = Vec::with_capacity(24);
    let mut typing_raster = Vec::with_capacity(24);
    for _ in 0..24 {
        let started = Instant::now();
        h.key(Key::Character("ы".into()), Modifiers::empty());
        let after_event = Instant::now();
        // Harness::key already delivers actions and redraws internally.
        // A final redraw precedes the CPU raster, unlike a host present.
        let (layers, _) = h.root.redraw();
        let after_scene = Instant::now();
        renderer
            .draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), false)
            .expect("typing CPU frame");
        typing_event.push(ms(after_event - started));
        typing_raster.push(ms(after_scene.elapsed()));
        typing_total.push(ms(started.elapsed()));
    }
    assert_eq!(h.app.text.byte_len(), before + 48, "24 UTF-8 key events did not insert 24 characters");

    let start = Instant::now();
    h.save_now();
    crate::disk::wait();
    let saved = std::fs::read(&note).expect("read saved note");
    let current = h.app.text.byte_slice(..h.app.text.byte_len()).to_string();
    assert_eq!(saved, current.as_bytes(), "file gate lost or corrupted edited bytes");
    let save_ms = ms(start.elapsed());

    let [scroll_p50, scroll_p95] = p(&scroll_total);
    let [input_p50, input_p95] = p(&scroll_input);
    let [scene_p50, scene_p95] = p(&scroll_scene);
    let [raster_p50, raster_p95] = p(&scroll_raster);
    let [typing_p50, typing_p95] = p(&typing_total);
    let [key_p50, key_p95] = p(&typing_event);
    let [type_raster_p50, type_raster_p95] = p(&typing_raster);

    eprintln!(
        "EDITOR_JOURNEY lines={lines} bytes={size} open_ms={open_ms:.3} \
         first_frame_ms={first_frame_ms:.3} jump_ms={jump_ms:.3} \
         scroll_p50_ms={scroll_p50:.3} scroll_p95_ms={scroll_p95:.3} \
         scroll_input_p50_ms={input_p50:.3} scroll_input_p95_ms={input_p95:.3} \
         scroll_scene_p50_ms={scene_p50:.3} scroll_scene_p95_ms={scene_p95:.3} \
         scroll_raster_p50_ms={raster_p50:.3} scroll_raster_p95_ms={raster_p95:.3} \
         typing_p50_ms={typing_p50:.3} typing_p95_ms={typing_p95:.3} \
         key_p50_ms={key_p50:.3} key_p95_ms={key_p95:.3} \
         typing_raster_p50_ms={type_raster_p50:.3} typing_raster_p95_ms={type_raster_p95:.3} \
         save_ms={save_ms:.3} scrolled=true persisted=true"
    );
    h.app.shutdown();
}

#[test]
#[ignore = "opt-in 4k/30k/100k native editor UI-to-CPU-frame diagnostic"]
fn benchmark_native_large_editor_journey() {
    for lines in [4_000, 30_000, 100_000] {
        run(lines);
    }
}
