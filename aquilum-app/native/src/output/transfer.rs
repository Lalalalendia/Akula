//! Developer-only WAC-02 copy experiment: replace overlapping DXGI copy
//! rectangles with a disjoint cover of exactly the same pixels.
//! No scroll/PRESENT behavior changes and no change to the installed build.

use super::Area;

/// Pixel work submitted to CopySubresourceRegion, counting overlaps each time.
/// The caller multiplies by four for BGRA8 bytes; this is NOT GPU completion.
pub(super) fn copied_pixels(areas: &[Area]) -> u64 {
    areas.iter().map(|a| u64::from(a.w) * u64::from(a.h)).sum()
}

fn subtract(area: Area, covered: Area, remaining: &mut Vec<Area>) {
    let Some(hit) = area.intersect(covered) else {
        remaining.push(area);
        return;
    };
    let (right, bottom) = (area.x + area.w, area.y + area.h);
    let (hit_right, hit_bottom) = (hit.x + hit.w, hit.y + hit.h);
    if area.y < hit.y {
        remaining.push(Area { x: area.x, y: area.y, w: area.w, h: hit.y - area.y });
    }
    if hit_bottom < bottom {
        remaining.push(Area { x: area.x, y: hit_bottom, w: area.w, h: bottom - hit_bottom });
    }
    if area.x < hit.x {
        remaining.push(Area { x: area.x, y: hit.y, w: hit.x - area.x, h: hit.h });
    }
    if hit_right < right {
        remaining.push(Area { x: hit_right, y: hit.y, w: right - hit_right, h: hit.h });
    }
}

/// Preserve the exact union of the input rectangles while eliminating overlap.
/// More regions/calls may be required, so do not enable in normal or installed
/// builds until an actual Windows profile shows a benefit.
pub(super) fn disjoint_copy_plan(original: &[Area]) -> Vec<Area> {
    let mut disjoint = Vec::new();
    for &area in original {
        if area.w == 0 || area.h == 0 {
            continue;
        }
        let mut fragments = vec![area];
        for &covered in &disjoint {
            let mut next = Vec::new();
            for piece in fragments {
                subtract(piece, covered, &mut next);
            }
            fragments = next;
            if fragments.is_empty() {
                break;
            }
        }
        disjoint.extend(fragments);
    }
    disjoint
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn cells(areas: &[Area]) -> HashSet<(u32, u32)> {
        areas.iter().flat_map(|a| {
            (a.x..a.x + a.w).flat_map(move |x| (a.y..a.y + a.h).map(move |y| (x, y)))
        }).collect()
    }

    fn verify(original: &[Area]) {
        let candidate = disjoint_copy_plan(original);
        let expected = cells(original);
        assert_eq!(cells(&candidate), expected, "copy coverage changed");
        assert_eq!(copied_pixels(&candidate), expected.len() as u64, "duplicate pixels remain");
        assert!(copied_pixels(&candidate) <= copied_pixels(original));
    }

    #[test]
    fn overlapping_l_shape_reduces_submitted_pixels() {
        let areas = [Area { x: 0, y: 0, w: 4, h: 2 }, Area { x: 2, y: 1, w: 4, h: 3 }];
        verify(&areas);
        assert_eq!(copied_pixels(&areas), 20);
        assert_eq!(copied_pixels(&disjoint_copy_plan(&areas)), 18);
    }

    #[test]
    fn three_buffer_stale_and_contained_rectangles_preserve_pixels() {
        verify(&[
            Area { x: 0, y: 0, w: 12, h: 8 },
            Area { x: 1, y: 1, w: 10, h: 5 },
            Area { x: 4, y: 3, w: 9, h: 7 },
            Area { x: 0, y: 2, w: 0, h: 8 },
        ]);
    }

    #[test]
    fn deterministic_small_grid_coverage_and_disjointness() {
        let mut seed = 0x5A17_u32;
        for _ in 0..256 {
            let mut areas = Vec::new();
            for _ in 0..8 {
                let mut next = || {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    seed >> 24
                };
                areas.push(Area {
                    x: next() % 12,
                    y: next() % 12,
                    w: next() % 6,
                    h: next() % 6,
                });
            }
            verify(&areas);
        }
    }
}
