//! Bound retained Vello alpha/luminance mask payloads; the frame Pixmap is temporary.
//! Mask::new_* stores one byte per pixel, not four. Limit only cache retention,
//! never drop an effect from the current draw when its mask is too large.

use std::collections::VecDeque;
use super::CachedMask;

/// At most 128 MiB of retained one-byte-per-pixel mask payloads per Painter.
/// Other compositor allocations have separate budgets.
pub(super) const MAX_MASK_CACHE_BYTES: u64 = 128 * 1024 * 1024;

fn mask_bytes(width: u16, height: u16) -> u64 {
    u64::from(width) * u64::from(height)
}

pub(super) fn prepare(
    cache: &mut VecDeque<CachedMask>,
    width: u16,
    height: u16,
    count_limit: usize,
) -> bool {
    trim_before_render(cache, mask_bytes(width, height), MAX_MASK_CACHE_BYTES, count_limit)
}

fn trim_before_render(
    cache: &mut VecDeque<CachedMask>,
    incoming: u64,
    byte_limit: u64,
    count_limit: usize,
) -> bool {
    if incoming > byte_limit || count_limit == 0 {
        cache.clear();
        return false;
    }
    let mut retained = cache.iter().map(|entry| {
        mask_bytes(entry.mask.width(), entry.mask.height())
    }).sum::<u64>();
    while cache.len() >= count_limit || retained.saturating_add(incoming) > byte_limit {
        let Some(removed) = cache.pop_front() else { break };
        retained = retained.saturating_sub(mask_bytes(removed.mask.width(), removed.mask.height()));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::imaging::{MaskMode, record::Scene};
    use masonry::kurbo::Affine;
    use vello_cpu::Mask;

    fn entry(id: u16, width: u16, height: u16) -> CachedMask {
        CachedMask {
            scene: Scene::new(),
            mode: MaskMode::Alpha,
            transform: Affine::translate((f64::from(id), 0.0)),
            mask: Mask::from_parts(vec![255; usize::from(width) * usize::from(height)], width, height),
        }
    }

    #[test]
    fn mask_cache_budget_lru_evicts_oldest_before_new_mask_allocation() {
        let mut cache = VecDeque::from([entry(1, 16, 16), entry(2, 16, 16)]);
        assert!(trim_before_render(&mut cache, 256, 512, 8));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].transform, Affine::translate((2.0, 0.0)));
        cache.push_back(entry(3, 16, 16));
        assert_eq!(cache.len(), 2);
        assert!(trim_before_render(&mut cache, 256, 1024, 2));
        assert_eq!(cache.len(), 1, "existing count limit must still hold");
    }

    #[test]
    fn mask_cache_budget_uncacheable_mask_is_rendered_but_not_retained() {
        let mut cache = VecDeque::from([entry(1, 16, 16), entry(2, 16, 16)]);
        assert!(!trim_before_render(&mut cache, 1024, 512, 8));
        assert!(cache.is_empty());
        assert!(!trim_before_render(&mut cache, 1, 128, 0));
        assert!(cache.is_empty());
    }

    #[test]
    fn mask_cache_budget_admits_4k_8k_and_limits_combined_retention() {
        let px4k = mask_bytes(3840, 2160);
        let px8k = mask_bytes(7680, 4320);
        assert!(px4k < MAX_MASK_CACHE_BYTES);
        assert!(px8k < MAX_MASK_CACHE_BYTES);
        assert!(4 * px8k <= MAX_MASK_CACHE_BYTES);
        assert!(5 * px8k > MAX_MASK_CACHE_BYTES);
        assert!(8 * px4k < MAX_MASK_CACHE_BYTES);
    }
}
