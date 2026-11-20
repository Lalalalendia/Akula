//! AK-CALL-04: pure CPU, default-OFF screen-damage/recovery laboratory.
//!
//! This is NOT a video codec, Windows capture backend, media transport, OS
//! permission, encryption, or claimed bandwidth benchmark. The input frames
//! are synthetic or provided locally by the caller. BLAKE3 below is an
//! UNKEYED corruption checksum, not authentication. The tile cost metric
//! counts RAW bytes, not H.264/AV1 encoded size or actual QUIC throughput.
//! Never accept this as permission to capture/share a screen.

#![deny(clippy::all)]

pub const PIXEL_BYTES: usize = 4; // BGRA8
pub const TILE_SIZE: usize = 64;
pub const MAX_WIDTH: usize = 4096;
pub const MAX_HEIGHT: usize = 2160;
pub const MAX_MOVES: usize = 64;
pub const MAX_TILES: usize = 4096;
const DIGEST_DOMAIN: &str = "Akula/AK-CALL-04/screen-frame-bgra8/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DamageError {
    InvalidDimensions,
    InvalidPixels,
    InvalidRect,
    OverlappingMoves,
    InvalidTile,
    TooManyMoves,
    TooManyTiles,
    MissingBase,
    WrongBase,
    StaleFrame,
    CorruptFrame,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    id: u64,
    width: usize,
    height: usize,
    bgra: Vec<u8>,
}

impl Frame {
    pub fn new(id: u64, width: usize, height: usize, bgra: Vec<u8>)
        -> Result<Self, DamageError>
    {
        let needed = frame_len(width, height)?;
        if id == 0 {
            return Err(DamageError::StaleFrame);
        }
        if bgra.len() != needed {
            return Err(DamageError::InvalidPixels);
        }
        Ok(Self { id, width, height, bgra })
    }

    pub fn id(&self) -> u64 { self.id }
    pub fn dimensions(&self) -> (usize, usize) { (self.width, self.height) }
    pub fn pixels(&self) -> &[u8] { &self.bgra }

    pub fn digest(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new_derive_key(DIGEST_DOMAIN);
        h.update(&(self.width as u32).to_be_bytes());
        h.update(&(self.height as u32).to_be_bytes());
        h.update(&self.bgra);
        *h.finalize().as_bytes()
    }
}

fn frame_len(width: usize, height: usize) -> Result<usize, DamageError> {
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(DamageError::InvalidDimensions);
    }
    width.checked_mul(height).and_then(|n| n.checked_mul(PIXEL_BYTES))
        .ok_or(DamageError::InvalidDimensions)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoveRect {
    pub source: Rect,
    pub dest_x: usize,
    pub dest_y: usize,
}

impl MoveRect {
    fn destination(self) -> Rect {
        Rect {
            x: self.dest_x, y: self.dest_y,
            width: self.source.width, height: self.source.height,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TilePatch {
    pub x: usize,
    pub y: usize,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameUpdate {
    /// No pixel bytes and NO frame-ID advancement. A future delta still
    /// references the last actually transmitted and applied baseline.
    Unchanged,
    Keyframe(Frame),
    Delta {
        base_id: u64,
        base_digest: [u8; 32],
        next_id: u64,
        next_digest: [u8; 32],
        moves: Vec<MoveRect>,
        tiles: Vec<TilePatch>,
    },
}

impl FrameUpdate {
    /// Uncompressed frame-or-tile byte count + move metadata estimate. This
    /// is NOT transport traffic, codec bitrate or any performance result.
    pub fn raw_payload_bytes(&self) -> usize {
        match self {
            Self::Unchanged => 0,
            Self::Keyframe(frame) => frame.bgra.len(),
            Self::Delta { moves, tiles, .. } => {
                moves.len() * 24 + tiles.iter().map(|tile| tile.bgra.len()).sum::<usize>()
            }
        }
    }
}

fn within(rect: Rect, width: usize, height: usize) -> bool {
    rect.width > 0 && rect.height > 0
        && rect.x.checked_add(rect.width).is_some_and(|x| x <= width)
        && rect.y.checked_add(rect.height).is_some_and(|y| y <= height)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width
        && a.y < b.y + b.height && b.y < a.y + a.height
}

fn validate_moves(moves: &[MoveRect], width: usize, height: usize)
    -> Result<(), DamageError>
{
    if moves.len() > MAX_MOVES {
        return Err(DamageError::TooManyMoves);
    }
    for (i, item) in moves.iter().enumerate() {
        if !within(item.source, width, height)
            || !within(item.destination(), width, height)
        {
            return Err(DamageError::InvalidRect);
        }
        for earlier in &moves[..i] {
            if overlaps(earlier.destination(), item.destination()) {
                return Err(DamageError::OverlappingMoves);
            }
        }
    }
    Ok(())
}

/// Move sources always read from the SAME immutable pre-move base, even when
/// rectangles overlap their own destination (e.g. scrolling a window).
fn apply_moves(base: &[u8], next: &mut [u8], width: usize, moves: &[MoveRect]) {
    for item in moves {
        for row in 0..item.source.height {
            let start = ((item.source.y + row) * width + item.source.x) * PIXEL_BYTES;
            let dest = ((item.dest_y + row) * width + item.dest_x) * PIXEL_BYTES;
            let bytes = item.source.width * PIXEL_BYTES;
            next[dest..dest + bytes].copy_from_slice(&base[start..start + bytes]);
        }
    }
}

/// Constructs an exact lossless candidate by inspecting locally available
/// BGRA frames. O(width*height) CPU comparison is deliberate in this lab;
/// production capture should start from OS dirty/move metadata and benchmark
/// GPU copies/ROI against hardware H.264/AV1 at the same content and quality.
pub fn plan_update(base: Option<&Frame>, target: &Frame, moves: &[MoveRect])
    -> Result<FrameUpdate, DamageError>
{
    frame_len(target.width, target.height)?;
    if base.is_none() || base.is_some_and(|b| b.dimensions() != target.dimensions()) {
        // Resize, first frame or incompatible baseline forces a new keyframe.
        return Ok(FrameUpdate::Keyframe(target.clone()));
    }
    let base = base.ok_or(DamageError::MissingBase)?;
    if target.id <= base.id {
        return Err(DamageError::StaleFrame);
    }
    if base.bgra == target.bgra {
        return Ok(FrameUpdate::Unchanged);
    }
    validate_moves(moves, target.width, target.height)?;
    let mut predicted = base.bgra.clone();
    apply_moves(&base.bgra, &mut predicted, target.width, moves);

    let mut tiles = Vec::new();
    for y in (0..target.height).step_by(TILE_SIZE) {
        for x in (0..target.width).step_by(TILE_SIZE) {
            let tw = TILE_SIZE.min(target.width - x);
            let th = TILE_SIZE.min(target.height - y);
            let differs = (0..th).any(|row| {
                let start = ((y + row) * target.width + x) * PIXEL_BYTES;
                let end = start + tw * PIXEL_BYTES;
                predicted[start..end] != target.bgra[start..end]
            });
            if differs {
                if tiles.len() >= MAX_TILES {
                    return Ok(FrameUpdate::Keyframe(target.clone()));
                }
                let mut bgra = Vec::with_capacity(tw * th * PIXEL_BYTES);
                for row in 0..th {
                    let start = ((y + row) * target.width + x) * PIXEL_BYTES;
                    bgra.extend_from_slice(&target.bgra[start..start + tw * PIXEL_BYTES]);
                }
                tiles.push(TilePatch { x, y, bgra });
            }
        }
    }
    let raw_cost = moves.len() * 24
        + tiles.iter().map(|tile| tile.bgra.len()).sum::<usize>();
    // A conservative RAW-area heuristic only. Do not claim this is better
    // than inter-frame video: that must be measured with real encoders.
    if raw_cost >= target.bgra.len() * 3 / 4 {
        return Ok(FrameUpdate::Keyframe(target.clone()));
    }
    Ok(FrameUpdate::Delta {
        base_id: base.id,
        base_digest: base.digest(),
        next_id: target.id,
        next_digest: target.digest(),
        moves: moves.to_vec(),
        tiles,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyResult {
    Unchanged,
    Applied,
}

/// Receives only already-trusted local simulated FrameUpdate objects. There
/// is no network frame parser, authentication or capture authorization here.
#[derive(Default)]
pub struct ScreenReceiver {
    current: Option<Frame>,
}

impl ScreenReceiver {
    pub fn new() -> Self { Self::default() }
    pub fn frame(&self) -> Option<&Frame> { self.current.as_ref() }

    /// All operations stage in a *separate* bounded Vec. A corrupt/old/lost
    /// update leaves the existing display and baseline untouched.
    pub fn apply(&mut self, update: &FrameUpdate) -> Result<ApplyResult, DamageError> {
        match update {
            FrameUpdate::Unchanged => {
                if self.current.is_none() {
                    return Err(DamageError::MissingBase);
                }
                Ok(ApplyResult::Unchanged)
            }
            FrameUpdate::Keyframe(frame) => {
                frame_len(frame.width, frame.height)?;
                if frame.bgra.len() != frame_len(frame.width, frame.height)? {
                    return Err(DamageError::InvalidPixels);
                }
                if frame.id == 0 || self.current.as_ref().is_some_and(|b| frame.id <= b.id) {
                    return Err(DamageError::StaleFrame);
                }
                self.current = Some(frame.clone());
                Ok(ApplyResult::Applied)
            }
            FrameUpdate::Delta {
                base_id, base_digest, next_id, next_digest, moves, tiles
            } => {
                let base = self.current.as_ref().ok_or(DamageError::MissingBase)?;
                if *base_id != base.id || *base_digest != base.digest() {
                    return Err(DamageError::WrongBase);
                }
                if *next_id <= base.id {
                    return Err(DamageError::StaleFrame);
                }
                validate_moves(moves, base.width, base.height)?;
                if tiles.len() > MAX_TILES {
                    return Err(DamageError::TooManyTiles);
                }
                let mut output = base.bgra.clone();
                apply_moves(&base.bgra, &mut output, base.width, moves);
                let mut previous = None;
                for tile in tiles {
                    let key = (tile.y, tile.x);
                    if previous.is_some_and(|p| p >= key)
                        || tile.x % TILE_SIZE != 0 || tile.y % TILE_SIZE != 0
                        || tile.x >= base.width || tile.y >= base.height
                    {
                        return Err(DamageError::InvalidTile);
                    }
                    previous = Some(key);
                    let width = TILE_SIZE.min(base.width - tile.x);
                    let height = TILE_SIZE.min(base.height - tile.y);
                    if tile.bgra.len() != width * height * PIXEL_BYTES {
                        return Err(DamageError::InvalidTile);
                    }
                    for row in 0..height {
                        let dest = ((tile.y + row) * base.width + tile.x) * PIXEL_BYTES;
                        let source = row * width * PIXEL_BYTES;
                        let count = width * PIXEL_BYTES;
                        output[dest..dest + count]
                            .copy_from_slice(&tile.bgra[source..source + count]);
                    }
                }
                let result = Frame::new(*next_id, base.width, base.height, output)?;
                if result.digest() != *next_digest {
                    return Err(DamageError::CorruptFrame);
                }
                self.current = Some(result);
                Ok(ApplyResult::Applied)
            }
        }
    }
}

/// A separate, bounded cursor-position event does not resend video pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorPosition {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
}

pub fn plan_cursor(
    previous: Option<CursorPosition>,
    next: CursorPosition,
    frame_width: usize,
    frame_height: usize,
) -> Result<Option<CursorPosition>, DamageError> {
    frame_len(frame_width, frame_height)?;
    if next.x >= frame_width || next.y >= frame_height {
        return Err(DamageError::InvalidRect);
    }
    Ok((previous != Some(next)).then_some(next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(id: u64, w: usize, h: usize, color: u8) -> Frame {
        Frame::new(id, w, h, vec![color; w * h * PIXEL_BYTES]).unwrap()
    }

    fn paint(frame: &Frame, id: u64, x: usize, y: usize, color: u8) -> Frame {
        let mut pixels = frame.bgra.clone();
        let offset = (y * frame.width + x) * PIXEL_BYTES;
        pixels[offset..offset + PIXEL_BYTES].copy_from_slice(&[color; PIXEL_BYTES]);
        Frame::new(id, frame.width, frame.height, pixels).unwrap()
    }

    fn receiver(base: &Frame) -> ScreenReceiver {
        let mut r = ScreenReceiver::new();
        assert_eq!(r.apply(&FrameUpdate::Keyframe(base.clone())).unwrap(),
            ApplyResult::Applied);
        r
    }

    #[test]
    fn first_frame_must_be_keyframe_and_reconstruct_exact_pixels() {
        let frame = solid(1, 128, 128, 19);
        let update = plan_update(None, &frame, &[]).unwrap();
        assert_eq!(update.raw_payload_bytes(), 128 * 128 * PIXEL_BYTES);
        let mut r = ScreenReceiver::new();
        assert_eq!(r.apply(&FrameUpdate::Unchanged), Err(DamageError::MissingBase));
        assert_eq!(r.apply(&update), Ok(ApplyResult::Applied));
        assert_eq!(r.frame().unwrap(), &frame);
    }

    #[test]
    fn single_pixel_sends_one_tile_and_preserves_lossless_frame() {
        let base = solid(1, 256, 256, 0);
        let changed = paint(&base, 2, 19, 33, 255);
        let update = plan_update(Some(&base), &changed, &[]).unwrap();
        let FrameUpdate::Delta { tiles, moves, .. } = &update else { panic!("expected delta") };
        assert_eq!(tiles.len(), 1);
        assert!(moves.is_empty());
        assert_eq!(update.raw_payload_bytes(), TILE_SIZE * TILE_SIZE * PIXEL_BYTES);
        let mut r = receiver(&base);
        r.apply(&update).unwrap();
        assert_eq!(r.frame().unwrap(), &changed);
    }

    #[test]
    fn identical_frame_sends_zero_pixels_and_cursor_is_independent() {
        let base = solid(1, 128, 128, 1);
        let identical = solid(2, 128, 128, 1);
        let update = plan_update(Some(&base), &identical, &[]).unwrap();
        assert_eq!(update, FrameUpdate::Unchanged);
        assert_eq!(update.raw_payload_bytes(), 0);
        let mut r = receiver(&base);
        assert_eq!(r.apply(&update), Ok(ApplyResult::Unchanged));
        assert_eq!(r.frame().unwrap().id(), 1);
        let a = CursorPosition { x: 1, y: 2, visible: true };
        let b = CursorPosition { x: 5, y: 7, visible: true };
        assert_eq!(plan_cursor(None, a, 128, 128).unwrap(), Some(a));
        assert_eq!(plan_cursor(Some(a), a, 128, 128).unwrap(), None);
        assert_eq!(plan_cursor(Some(a), b, 128, 128).unwrap(), Some(b));
        assert_eq!(plan_cursor(Some(a),
            CursorPosition { x: 128, ..a }, 128, 128), Err(DamageError::InvalidRect));
    }

    #[test]
    fn pixels_on_tile_boundary_update_two_tiles() {
        let base = solid(1, 256, 256, 0);
        let changed = paint(&paint(&base, 2, 63, 2, 17), 3, 64, 2, 18);
        let update = plan_update(Some(&base), &changed, &[]).unwrap();
        let FrameUpdate::Delta { tiles, .. } = &update else { panic!("expected delta") };
        assert_eq!(tiles.len(), 2);
        let mut r = receiver(&base);
        r.apply(&update).unwrap();
        assert_eq!(r.frame().unwrap(), &changed);
    }

    #[test]
    fn overlapping_scroll_move_reads_immutable_pre_move_pixels() {
        let mut raw = vec![0; 128 * 128 * PIXEL_BYTES];
        for row in 0..128 {
            raw[row * 128 * PIXEL_BYTES..(row + 1) * 128 * PIXEL_BYTES]
                .fill(row as u8);
        }
        let base = Frame::new(1, 128, 128, raw.clone()).unwrap();
        let moved = MoveRect {
            source: Rect { x: 0, y: 0, width: 128, height: 64 },
            dest_x: 0, dest_y: 32,
        };
        let mut after = raw.clone();
        apply_moves(&raw, &mut after, 128, &[moved]);
        let target = Frame::new(2, 128, 128, after).unwrap();
        let update = plan_update(Some(&base), &target, &[moved]).unwrap();
        let FrameUpdate::Delta { moves, tiles, .. } = &update else { panic!("expected delta") };
        assert_eq!(moves.len(), 1);
        assert!(tiles.is_empty());
        assert_eq!(update.raw_payload_bytes(), 24);
        let mut r = receiver(&base);
        r.apply(&update).unwrap();
        assert_eq!(r.frame().unwrap(), &target);
    }

    #[test]
    fn invalid_move_bounds_and_duplicate_destination_are_rejected() {
        let base = solid(1, 128, 128, 0);
        let changed = paint(&base, 2, 3, 4, 1);
        let invalid = MoveRect {
            source: Rect { x: 125, y: 1, width: 8, height: 8 },
            dest_x: 0, dest_y: 0,
        };
        assert_eq!(plan_update(Some(&base), &changed, &[invalid]),
            Err(DamageError::InvalidRect));
        let first = MoveRect {
            source: Rect { x: 0, y: 0, width: 16, height: 16 },
            dest_x: 10, dest_y: 10,
        };
        let second = MoveRect { dest_x: 15, dest_y: 15, ..first };
        assert_eq!(plan_update(Some(&base), &changed, &[first, second]),
            Err(DamageError::OverlappingMoves));
    }

    #[test]
    fn missing_base_rejects_delta_and_keyframe_recovers() {
        let base = solid(1, 256, 256, 0);
        let target = paint(&base, 2, 1, 1, 22);
        let update = plan_update(Some(&base), &target, &[]).unwrap();
        let mut r = ScreenReceiver::new();
        assert_eq!(r.apply(&update), Err(DamageError::MissingBase));
        assert!(r.frame().is_none());
        let recovery = plan_update(None, &target, &[]).unwrap();
        assert_eq!(r.apply(&recovery), Ok(ApplyResult::Applied));
        assert_eq!(r.frame().unwrap(), &target);
    }

    #[test]
    fn dropped_intermediate_delta_needs_new_keyframe() {
        let base = solid(1, 256, 256, 0);
        let next = paint(&base, 2, 1, 1, 22);
        let after = paint(&next, 3, 2, 2, 33);
        let skipped = plan_update(Some(&next), &after, &[]).unwrap();
        let mut r = receiver(&base);
        assert_eq!(r.apply(&skipped), Err(DamageError::WrongBase));
        assert_eq!(r.frame().unwrap(), &base);
        let recovery = FrameUpdate::Keyframe(after.clone());
        assert_eq!(r.apply(&recovery), Ok(ApplyResult::Applied));
        assert_eq!(r.frame().unwrap(), &after);
    }

    #[test]
    fn tampered_patch_and_digest_cannot_publish_partial_frame() {
        let base = solid(1, 256, 256, 0);
        let target = paint(&base, 2, 19, 33, 255);
        let mut update = plan_update(Some(&base), &target, &[]).unwrap();
        let FrameUpdate::Delta { tiles, .. } = &mut update else { panic!("expected delta") };
        tiles[0].bgra[0] ^= 1;
        let mut r = receiver(&base);
        assert_eq!(r.apply(&update), Err(DamageError::CorruptFrame));
        assert_eq!(r.frame().unwrap(), &base);
    }

    #[test]
    fn invalid_tile_length_duplicate_and_alignment_fail_closed() {
        let base = solid(1, 256, 256, 0);
        let target = paint(&base, 2, 3, 4, 5);
        let mut patch = plan_update(Some(&base), &target, &[]).unwrap();
        let FrameUpdate::Delta { tiles, .. } = &mut patch else { panic!("expected delta") };
        tiles[0].bgra.pop();
        let mut r = receiver(&base);
        assert_eq!(r.apply(&patch), Err(DamageError::InvalidTile));
        assert_eq!(r.frame().unwrap(), &base);
        let mut patch = plan_update(Some(&base), &target, &[]).unwrap();
        let FrameUpdate::Delta { tiles, .. } = &mut patch else { panic!("expected delta") };
        tiles[0].x = 1;
        assert_eq!(r.apply(&patch), Err(DamageError::InvalidTile));
        let mut patch = plan_update(Some(&base), &target, &[]).unwrap();
        let FrameUpdate::Delta { tiles, .. } = &mut patch else { panic!("expected delta") };
        tiles.push(tiles[0].clone());
        assert_eq!(r.apply(&patch), Err(DamageError::InvalidTile));
        assert_eq!(r.frame().unwrap(), &base);
    }

    #[test]
    fn keyframe_replay_never_rolls_back_display() {
        let base = solid(2, 128, 128, 5);
        let old = solid(1, 128, 128, 3);
        let mut r = receiver(&base);
        assert_eq!(r.apply(&FrameUpdate::Keyframe(old)), Err(DamageError::StaleFrame));
        assert_eq!(r.apply(&FrameUpdate::Keyframe(base.clone())),
            Err(DamageError::StaleFrame));
        assert_eq!(r.frame().unwrap(), &base);
    }

    #[test]
    fn resizing_screen_forces_full_keyframe() {
        let base = solid(1, 128, 128, 0);
        let target = solid(2, 192, 128, 6);
        let patch = plan_update(Some(&base), &target, &[]).unwrap();
        assert!(matches!(patch, FrameUpdate::Keyframe(_)));
        let mut r = receiver(&base);
        r.apply(&patch).unwrap();
        assert_eq!(r.frame().unwrap(), &target);
    }

    #[test]
    fn high_churn_uses_full_keyframe_not_worse_raw_tiles() {
        let base = solid(1, 256, 256, 0);
        let target = solid(2, 256, 256, 1);
        let patch = plan_update(Some(&base), &target, &[]).unwrap();
        assert!(matches!(patch, FrameUpdate::Keyframe(_)));
        assert_eq!(patch.raw_payload_bytes(), 256 * 256 * PIXEL_BYTES);
    }

    #[test]
    fn sparse_1080p_raw_tiles_smaller_but_not_a_codec_comparison() {
        let base = solid(1, 1920, 1080, 0);
        let target = paint(&base, 2, 501, 401, 44);
        let patch = plan_update(Some(&base), &target, &[]).unwrap();
        let raw = patch.raw_payload_bytes();
        assert_eq!(raw, TILE_SIZE * TILE_SIZE * PIXEL_BYTES);
        assert!(raw < base.pixels().len() / 100);
        // This says nothing about H.264/AV1 output or actual QUIC bandwidth.
        let mut r = receiver(&base);
        r.apply(&patch).unwrap();
        assert_eq!(r.frame().unwrap(), &target);
    }
}
