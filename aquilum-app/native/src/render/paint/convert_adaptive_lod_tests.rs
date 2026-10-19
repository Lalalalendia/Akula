//! IMG-PERF-06: developer-only real-pixel cache experiments, no installed path.
use super::*;
use std::time::Instant;
use vello_cpu::peniko::ImageAlphaType;
use vello_cpu::PixelMetadata;

const W: u16 = 256;
const H: u16 = 192;
const L1_BYTES: usize = 128 * 96 * 4;
const L2_BYTES: usize = 64 * 48 * 4;
const TIGHT_BUDGET: usize = 12 * L2_BYTES;

fn source(seed: u64) -> ImageSource {
    let mut rng = (seed as u32) ^ 0x39bd_714c;
    let mut raw = Vec::with_capacity(usize::from(W) * usize::from(H) * 4);
    for _ in 0..usize::from(W) * usize::from(H) {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let alpha = (rng >> 24) as u8;
        raw.extend_from_slice(&[
            (rng as u8).min(alpha),
            ((rng >> 8) as u8).min(alpha),
            ((rng >> 16) as u8).min(alpha),
            alpha,
        ]);
    }
    ImageSource::Pixmap(Arc::new(Pixmap::from_parts(
        raw, W, H, PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, true),
    )))
}

fn pixmap(image: ImageSource) -> Arc<Pixmap> {
    match image {
        ImageSource::Pixmap(pixmap) => pixmap,
        _ => panic!("expected premultiplied Pixmap"),
    }
}

fn used_bytes(images: &Images) -> usize {
    images.mips.values()
        .map(|(p, _)| usize::from(p.width()) * usize::from(p.height()) * 4)
        .sum()
}

fn request(images: &mut Images, id: u64, original: &ImageSource, level: u8,
           budget: usize, direct: bool) -> Option<Arc<Pixmap>> {
    images.mip_source_with_strategy(id, original, level, budget, true, direct).map(pixmap)
}

#[test]
fn selective_l2_matches_eager_pixels_and_lazily_adds_l1() {
    let input = source(101);
    let mut eager = Images::default();
    let mut selective = Images::default();
    let eager_l2 = request(&mut eager, 101, &input, 2, TIGHT_BUDGET, false).unwrap();
    let lazy_l2 = request(&mut selective, 101, &input, 2, TIGHT_BUDGET, true).unwrap();
    assert_eq!(eager_l2.data_as_u8_slice(), lazy_l2.data_as_u8_slice());
    assert_eq!(eager_l2.may_have_transparency(), lazy_l2.may_have_transparency());
    assert_eq!(used_bytes(&selective), L2_BYTES);
    assert_eq!(used_bytes(&eager), L1_BYTES + L2_BYTES);
    assert!(!selective.mips.contains_key(&(101, 1)));
    assert!(selective.mips.contains_key(&(101, 2)));

    let reused = request(&mut selective, 101, &input, 2, TIGHT_BUDGET, true).unwrap();
    assert!(Arc::ptr_eq(&lazy_l2, &reused), "warm L2 must not regenerate");

    let eager_l1 = request(&mut eager, 101, &input, 1, TIGHT_BUDGET, false).unwrap();
    let late_l1 = request(&mut selective, 101, &input, 1, TIGHT_BUDGET, true).unwrap();
    assert_eq!(eager_l1.data_as_u8_slice(), late_l1.data_as_u8_slice());
    assert_eq!(eager_l1.may_have_transparency(), late_l1.may_have_transparency());
    assert_eq!(used_bytes(&selective), L1_BYTES + L2_BYTES);
    let l2_again = request(&mut selective, 101, &input, 2, TIGHT_BUDGET, true).unwrap();
    assert!(Arc::ptr_eq(&lazy_l2, &l2_again), "lazy L1 must not replace cached L2");
}

#[test]
fn selective_l2_rejects_failed_reservation_without_destroying_existing_bytes() {
    let input = source(202);
    let mut images = Images::default();
    assert!(request(&mut images, 202, &input, 2, L2_BYTES - 1, true).is_none());
    assert!(images.mips.is_empty(), "failed cold reservation must be atomic");
    let prior_l2 = request(&mut images, 202, &input, 2, L2_BYTES, true).unwrap();
    assert_eq!(used_bytes(&images), L2_BYTES);
    assert!(request(&mut images, 202, &input, 1, L2_BYTES, true).is_none());
    assert_eq!(used_bytes(&images), L2_BYTES, "failed late L1 must not evict L2");
    assert!(Arc::ptr_eq(
        &prior_l2,
        &request(&mut images, 202, &input, 2, L2_BYTES, true).unwrap(),
    ));
}

#[test]
fn selective_cache_handles_one_ten_and_hundred_distinct_sources() {
    for count in [1usize, 10, 100] {
        let mut eager = Images::default();
        let mut selective = Images::default();
        for n in 0..count {
            let id = n as u64 + 1000;
            let input = source(id);
            eager.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
            selective.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
            let expected = request(&mut eager, id, &input, 2, TIGHT_BUDGET, false).unwrap();
            let actual = request(&mut selective, id, &input, 2, TIGHT_BUDGET, true).unwrap();
            assert_eq!(actual.data_as_u8_slice(), expected.data_as_u8_slice(),
                "source {id} output mismatch");
            assert!(used_bytes(&eager) <= TIGHT_BUDGET);
            assert!(used_bytes(&selective) <= TIGHT_BUDGET);
        }
        let selective_l2_count = selective.mips.keys()
            .filter(|(_, level)| *level == 2).count();
        let eager_l2_count = eager.mips.keys()
            .filter(|(_, level)| *level == 2).count();
        assert_eq!(selective_l2_count, count.min(12),
            "selective must retain the most recent 12 L2 levels");
        assert_eq!(eager_l2_count, count.min(2),
            "eager must retain at most two full pyramids under this budget");
        // Zoom revisit: must retain correct final-level bytes without budget escape.
        for n in (count.saturating_sub(10)..count).rev() {
            let id = n as u64 + 1000;
            let input = source(id);
            selective.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
            let actual = request(&mut selective, id, &input, 2, TIGHT_BUDGET, true).unwrap();
            let ImageSource::Pixmap(ref base) = input else { unreachable!() };
            let reference = mipmap::half(base).and_then(|l1| mipmap::half(&l1)).unwrap();
            assert_eq!(actual.data_as_u8_slice(), reference.data_as_u8_slice());
            assert!(used_bytes(&selective) <= TIGHT_BUDGET);
        }
        let last = count as u64 - 1 + 1000;
        let input = source(last);
        let late_l1 = request(&mut selective, last, &input, 1, TIGHT_BUDGET, true).unwrap();
        let ImageSource::Pixmap(ref base) = input else { unreachable!() };
        assert_eq!(late_l1.data_as_u8_slice(),
            mipmap::half(base).unwrap().data_as_u8_slice());
        assert!(selective.mips.contains_key(&(last, 2)),
            "eviction must not drop the requesting source's L2");
        assert!(used_bytes(&selective) <= TIGHT_BUDGET);
    }
}

#[test]
#[ignore = "non-gating release cache economics; not user FPS"]
fn adaptive_lod_multi_image_microbench_diagnostic() {
    for count in [1usize, 10, 100] {
        let fixtures: Vec<_> = (0..count)
            .map(|n| (n as u64 + 1000, source(n as u64 + 1000)))
            .collect();
        for direct in [false, true] {
            let mut images = Images::default();
            let mut timing = [0.0_f64; 2];
            for elapsed in &mut timing {
                let start = Instant::now();
                for (id, src) in &fixtures {
                    images.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
                    assert!(request(&mut images, *id, src, 2, TIGHT_BUDGET, direct).is_some(),
                        "unable to reserve final L2");
                }
                *elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(used_bytes(&images) <= TIGHT_BUDGET);
            }
            println!(
                "AKULA_IMG_PERF_06 sources={count} direct={direct} cold_ms={:.4} revisit_ms={:.4} retained_levels={} resident_bytes={} limit_bytes={TIGHT_BUDGET}",
                timing[0], timing[1], images.mips.len(), used_bytes(&images),
            );
        }
    }
}
