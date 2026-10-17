//! Experimental **coverage-preserving** coalescing for adjacent damage regions.
//!
//! Never expand a bounding box over undamaged pixels: a merger is allowed only
//! for full-edge neighbours, and only when *all* input regions are disjoint.
//! Reject overlapping damage entirely because repeated SrcOver on those pixels
//! may have observable consequences. Used exclusively by test-only A/B paths.

type Region = (u16, u16, u16, u16);

fn overlaps(a: Region, b: Region) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    u32::from(ax) < u32::from(bx) + u32::from(bw)
        && u32::from(bx) < u32::from(ax) + u32::from(aw)
        && u32::from(ay) < u32::from(by) + u32::from(bh)
        && u32::from(by) < u32::from(ay) + u32::from(ah)
}

fn join(a: Region, b: Region) -> Option<Region> {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    if ay == by && ah == bh {
        if u32::from(ax) + u32::from(aw) == u32::from(bx) {
            return Some((ax, ay, u16::try_from(u32::from(aw) + u32::from(bw)).ok()?, ah));
        }
        if u32::from(bx) + u32::from(bw) == u32::from(ax) {
            return Some((bx, ay, u16::try_from(u32::from(aw) + u32::from(bw)).ok()?, ah));
        }
    }
    if ax == bx && aw == bw {
        if u32::from(ay) + u32::from(ah) == u32::from(by) {
            return Some((ax, ay, aw, u16::try_from(u32::from(ah) + u32::from(bh)).ok()?));
        }
        if u32::from(by) + u32::from(bh) == u32::from(ay) {
            return Some((ax, by, aw, u16::try_from(u32::from(ah) + u32::from(bh)).ok()?));
        }
    }
    None
}

/// Merge exact-edge neighbours until no additional merge is possible.
///
/// The union of written pixels is **identical** to the input union. On any
/// overlap or malformed zero-sized region, return the original regions so
/// scene alpha blending cannot be changed by removing a repeated draw.
pub(super) fn coalesce_adjacent(regions: &[Region]) -> Vec<Region> {
    if regions.iter().any(|&(_, _, w, h)| w == 0 || h == 0) {
        return regions.to_vec();
    }
    for i in 0..regions.len() {
        for j in (i + 1)..regions.len() {
            if overlaps(regions[i], regions[j]) {
                return regions.to_vec();
            }
        }
    }

    let mut out = regions.to_vec();
    loop {
        let mut merged = false;
        'pairs: for i in 0..out.len() {
            for j in (i + 1)..out.len() {
                if let Some(union) = join(out[i], out[j]) {
                    out[i] = union;
                    out.remove(j);
                    merged = true;
                    break 'pairs;
                }
            }
        }
        if !merged {
            break;
        }
    }
    debug_assert_eq!(
        regions.iter().map(|&(_, _, w, h)| u64::from(w) * u64::from(h)).sum::<u64>(),
        out.iter().map(|&(_, _, w, h)| u64::from(w) * u64::from(h)).sum::<u64>(),
        "coalescing must not change scheduled pixel area"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::{Region, coalesce_adjacent};

    fn covered(regions: &[Region], width: usize, height: usize) -> Vec<bool> {
        let mut bitmap = vec![false; width * height];
        for &(x, y, w, h) in regions {
            for yy in usize::from(y)..usize::from(y) + usize::from(h) {
                for xx in usize::from(x)..usize::from(x) + usize::from(w) {
                    bitmap[yy * width + xx] = true;
                }
            }
        }
        bitmap
    }

    #[test]
    fn merges_two_four_eight_and_sixteen_bands_without_extra_pixels() {
        for count in [2, 4, 8, 16] {
            let strips: Vec<Region> = (0..count)
                .map(|i| (0, i * 5, 100, 5))
                .collect();
            let result = coalesce_adjacent(&strips);
            assert_eq!(result, vec![(0, 0, 100, count * 5)]);
            assert_eq!(covered(&strips, 100, 80), covered(&result, 100, 80));
        }
    }

    #[test]
    fn merges_horizontal_neighbours_and_unordered_bands() {
        let input = [(6, 2, 3, 5), (0, 2, 3, 5), (3, 2, 3, 5)];
        assert_eq!(coalesce_adjacent(&input), vec![(0, 2, 9, 5)]);
        assert_eq!(covered(&input, 12, 8), covered(&coalesce_adjacent(&input), 12, 8));
    }

    #[test]
    fn refuses_gaps_and_l_shaped_bounding_box_expansion() {
        let input = [(0, 0, 5, 5), (0, 6, 5, 5), (5, 0, 2, 2)];
        let result = coalesce_adjacent(&input);
        assert_eq!(covered(&input, 8, 12), covered(&result, 8, 12));
        assert!(!result.iter().any(|&(_, _, w, h)| w == 7 && h == 11));
    }

    #[test]
    fn refuses_any_overlap_even_when_other_rectangles_could_merge() {
        let input = [(0, 0, 10, 4), (0, 4, 10, 4), (2, 5, 2, 2)];
        assert_eq!(coalesce_adjacent(&input), input);
    }

    #[test]
    fn refuses_zero_sized_input_and_overflowing_u16_size() {
        let bad = [(0, 0, 3, 3), (0, 3, 0, 2)];
        assert_eq!(coalesce_adjacent(&bad), bad);
        let oversized = [(0, 0, 65000, 1), (65000, 0, 1000, 1)];
        assert_eq!(coalesce_adjacent(&oversized), oversized);
    }

    #[test]
    fn empty_and_singleton_preserve_identity() {
        assert!(coalesce_adjacent(&[]).is_empty());
        assert_eq!(coalesce_adjacent(&[(14, 10, 7, 9)]), vec![(14, 10, 7, 9)]);
    }
}
