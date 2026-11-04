use super::*;
use masonry::imaging::{BlurredRoundedRect, PaintSink};
use masonry::kurbo::{Affine, Rect};
use masonry::peniko::Color;
use vello_cpu::{Pixmap, RenderContext, Resources};

fn shadow(rect: Rect, std_dev: f64, transform: Affine) -> BlurredRoundedRect {
    BlurredRoundedRect {
        transform,
        rect,
        radius: 6.0,
        std_dev,
        color: Color::from_rgb8(200, 40, 80),
        composite: Default::default(),
    }
}

fn pixels(draws: &[BlurredRoundedRect], candidate: bool) -> Vec<u8> {
    let settings = crate::render::paint::settings(0);
    if candidate {
        let mut painter = Painter::new(96, 96, settings);
        painter.begin(0, 0, 96, 96);
        for &draw in draws {
            painter.blurred_rounded_rect(draw);
        }
        painter.finish().expect("all valid shadows accepted");
        let mut image = Pixmap::new(96, 96);
        painter.ctx.render(&mut image, &mut painter.resources);
        image.data_as_u8_slice().to_vec()
    } else {
        let mut ctx = RenderContext::new_with(96, 96, settings);
        for draw in draws {
            super::super::shadow_guard::validate(draw).expect("baseline shadow valid");
            ctx.set_transform(draw.transform);
            ctx.set_paint(draw.color.multiply_alpha(draw.composite.alpha));
            ctx.set_blend_mode(draw.composite.blend);
            ctx.fill_blurred_rounded_rect(&draw.rect, draw.radius as f32, draw.std_dev as f32, false);
        }
        ctx.flush();
        let mut image = Pixmap::new(96, 96);
        ctx.render(&mut image, &mut Resources::new());
        image.data_as_u8_slice().to_vec()
    }
}

#[test]
fn conservative_cull_preserves_pixels_at_blur_and_viewport_boundaries() {
    let settings = crate::render::paint::settings(0);
    let painter = Painter::new(96, 96, settings);
    let far_right = shadow(Rect::new(350.0, 12.0, 390.0, 45.0), 4.0, Affine::IDENTITY);
    let far_left = shadow(Rect::new(-350.0, 12.0, -300.0, 45.0), 4.0, Affine::IDENTITY);
    let grazing = shadow(Rect::new(96.0, 25.0, 124.0, 45.0), 4.0, Affine::IDENTITY);
    let rotated = shadow(Rect::new(120.0, 40.0, 145.0, 66.0), 2.0, Affine::rotate(-0.28));
    let fractional = shadow(Rect::new(-11.0, 62.0, 9.0, 89.0), 7.0, Affine::translate((0.3, 0.7)));
    let large_blur = shadow(Rect::new(162.0, 30.0, 190.0, 65.0), 32.0, Affine::IDENTITY);
    assert!(outside(&painter, &far_right));
    assert!(outside(&painter, &far_left));
    assert!(!outside(&painter, &grazing));
    assert!(!outside(&painter, &large_blur), "large halo may reach the frame");
    for batch in [
        vec![far_right],
        vec![far_left],
        vec![grazing],
        vec![rotated],
        vec![fractional],
        vec![large_blur],
        vec![far_right, grazing, far_left, fractional, rotated, large_blur],
    ] {
        assert_eq!(pixels(&batch, true), pixels(&batch, false), "shadow pixel parity: {batch:?}");
    }
}

#[test]
fn filtered_groups_are_not_culled_and_invalid_offscreen_shadows_still_error() {
    let mut painter = Painter::new(96, 96, crate::render::paint::settings(0));
    let far = shadow(Rect::new(350.0, 10.0, 380.0, 40.0), 2.0, Affine::IDENTITY);
    assert!(outside(&painter, &far));
    painter.groups.push(1);
    assert!(!outside(&painter, &far), "filtered group needs full intermediate scene");
    painter.groups.clear();

    let bad = BlurredRoundedRect { std_dev: f64::NAN, ..far };
    painter.begin(0, 0, 96, 96);
    painter.blurred_rounded_rect(bad);
    assert!(painter.finish().is_err(), "culling must not bypass validation");
}

#[test]
#[ignore = "diagnostic Release shadow-cull ABBA; no application FPS claim"]
fn benchmark_shadow_offscreen_cull_abba() {
    use std::time::Instant;
    const CYCLES: usize = 8; // 16 paired samples per implementation per scene.
    let offscreen: Vec<_> = (0..256).map(|i| {
        shadow(Rect::new(250.0 + f64::from(i), 10.0, 280.0 + f64::from(i), 35.0), 3.0, Affine::IDENTITY)
    }).collect();
    let boundary: Vec<_> = (0..64).map(|i| {
        shadow(Rect::new(92.0 + f64::from(i % 5), 10.0, 122.0 + f64::from(i % 5), 35.0), 3.0, Affine::IDENTITY)
    }).collect();
    let visible: Vec<_> = (0..64).map(|i| {
        let x = 12.0 + f64::from(i % 8) * 4.0;
        shadow(Rect::new(x, 12.0, x + 20.0, 32.0), 3.0, Affine::IDENTITY)
    }).collect();
    let mixed: Vec<_> = offscreen.iter().take(64).chain(boundary.iter()).copied().collect();

    for (name, draws) in [
        ("offscreen", offscreen.as_slice()),
        ("boundary", boundary.as_slice()),
        ("visible", visible.as_slice()),
        ("mixed", mixed.as_slice()),
    ] {
        let reference = pixels(draws, false); // Independent direct-Vello pixel oracle.
        let mut legacy = Vec::with_capacity(CYCLES * 2);
        let mut cull = Vec::with_capacity(CYCLES * 2);
        for cycle in 0..CYCLES {
            for (phase, candidate) in [false, true, true, false].into_iter().enumerate() {
                let began = Instant::now();
                // Both A and B use the *same* validated Painter path; the only
                // switch is whether shadow_cull::outside() can reject a draw.
                let actual = with_test_culling(candidate, || pixels(draws, true));
                let elapsed_us = began.elapsed().as_micros();
                assert_eq!(actual, reference, "ABBA pixel parity: {name}");
                let samples = if candidate { &mut cull } else { &mut legacy };
                samples.push(elapsed_us);
                println!(
                    "SHADOW_CULL_ABBA scene={name} route={} cycle={} phase={} draws={} elapsed_us={} parity=PASS",
                    if candidate { "cull" } else { "legacy" },
                    cycle + 1, phase + 1, draws.len(), elapsed_us,
                );
            }
        }
        legacy.sort_unstable();
        cull.sort_unstable();
        let mid = CYCLES;
        let legacy_us = (legacy[mid - 1] + legacy[mid]) / 2;
        let cull_us = (cull[mid - 1] + cull[mid]) / 2;
        println!(
            "SHADOW_CULL_MEDIAN scene={name} samples_per_route={} legacy_us={legacy_us} cull_us={cull_us} pixel_parity=PASS",
            CYCLES * 2,
        );
    }
}
