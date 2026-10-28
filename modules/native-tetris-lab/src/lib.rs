//! Small falling-block (Tetris-style) control game for the Akula 2D experiment.
//! No engine dependency, OS calls, asset loading, file access, or unsafe code.
//! Rule set is deliberately minimal, NOT full modern Guideline/SRS Tetris.

pub const COLS: usize = 10;
pub const ROWS: usize = 20;
pub const LOGICAL_WIDTH: u32 = 640;
pub const LOGICAL_HEIGHT: u32 = 360;
pub const FIXED_HZ: u32 = 120;
pub const EMPTY: u8 = 0;

const SHAPES: [[(i8, i8); 4]; 7] = [
    [(0, 1), (1, 1), (2, 1), (3, 1)], // I
    [(0, 0), (1, 0), (0, 1), (1, 1)], // O
    [(1, 0), (0, 1), (1, 1), (2, 1)], // T
    [(1, 0), (2, 0), (0, 1), (1, 1)], // S
    [(0, 0), (1, 0), (1, 1), (2, 1)], // Z
    [(0, 0), (0, 1), (1, 1), (2, 1)], // J
    [(2, 0), (0, 1), (1, 1), (2, 1)], // L
];
const COLORS: [u32; 8] = [
    0xff182438, 0xff40c9e7, 0xfff0d15f, 0xffab80e8,
    0xff66d5a1, 0xffeb7479, 0xff729bea, 0xffefa15b,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Playing,
    Over,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Left,
    Right,
    RotateClockwise,
    RotateCounterClockwise,
    SoftDrop,
    HardDrop,
    Restart,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Piece {
    pub kind: u8, // 0..=6: I O T S Z J L
    pub rotation: u8,
    pub x: i8,
    pub y: i8,
}

impl Piece {
    pub fn cells(self) -> [(i8, i8); 4] {
        let mut points = SHAPES[usize::from(self.kind)];
        if self.kind == 1 { // O always has the same shape
            return points.map(|(x, y)| (self.x + x, self.y + y));
        }
        let side: i8 = if self.kind == 0 { 4 } else { 3 };
        for _ in 0..self.rotation % 4 {
            for point in &mut points {
                *point = (side - 1 - point.1, point.0);
            }
        }
        points.map(|(x, y)| (self.x + x, self.y + y))
    }
}

#[derive(Clone, Debug)]
pub struct Game {
    pub board: [[u8; COLS]; ROWS],
    pub active: Piece,
    pub next: u8,
    pub phase: Phase,
    pub score: u32,
    pub lines: u32,
    pub ticks: u64,
    seed: u64,
    random_state: u64,
    bag: [u8; 7],
    bag_index: usize,
}

impl Default for Game {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Game {
    pub fn new(seed: u64) -> Self {
        let mut game = Self {
            board: [[0; COLS]; ROWS],
            active: Piece { kind: 0, rotation: 0, x: 3, y: -1 },
            next: 0,
            phase: Phase::Playing,
            score: 0,
            lines: 0,
            ticks: 0,
            seed,
            random_state: seed | 1,
            bag: [0, 1, 2, 3, 4, 5, 6],
            bag_index: 7,
        };
        game.active.kind = game.draw_bag();
        game.next = game.draw_bag();
        game
    }

    fn random(&mut self) -> u64 {
        let mut n = self.random_state;
        n ^= n << 13;
        n ^= n >> 7;
        n ^= n << 17;
        self.random_state = n;
        n
    }

    fn draw_bag(&mut self) -> u8 {
        if self.bag_index == 7 {
            self.bag = [0, 1, 2, 3, 4, 5, 6];
            for i in (1..7).rev() {
                let j = (self.random() % (i as u64 + 1)) as usize;
                self.bag.swap(i, j);
            }
            self.bag_index = 0;
        }
        let kind = self.bag[self.bag_index];
        self.bag_index += 1;
        kind
    }

    pub fn can_place(&self, piece: Piece) -> bool {
        piece.cells().iter().all(|&(x, y)| {
            x >= 0 && x < COLS as i8 && y < ROWS as i8 &&
                (y < 0 || self.board[y as usize][x as usize] == EMPTY)
        })
    }

    fn try_shift(&mut self, dx: i8, dy: i8) -> bool {
        let mut candidate = self.active;
        candidate.x += dx;
        candidate.y += dy;
        if !self.can_place(candidate) { return false; }
        self.active = candidate;
        true
    }

    fn rotate(&mut self, direction: i8) -> bool {
        if self.active.kind == 1 { return true; }
        let mut candidate = self.active;
        candidate.rotation = (i16::from(candidate.rotation) + i16::from(direction))
            .rem_euclid(4) as u8;
        // Deliberately simple kicks; NOT SRS (no T-spin semantics).
        for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (-2, 0), (2, 0), (0, -1)] {
            candidate.x = self.active.x + dx;
            candidate.y = self.active.y + dy;
            if self.can_place(candidate) {
                self.active = candidate;
                return true;
            }
        }
        false
    }

    fn spawn(&mut self) {
        self.active = Piece { kind: self.next, rotation: 0, x: 3, y: -1 };
        self.next = self.draw_bag();
        if !self.can_place(self.active) { self.phase = Phase::Over; }
    }

    fn clear_rows(&mut self) -> u32 {
        let mut compact = [[0u8; COLS]; ROWS];
        let mut write = ROWS;
        let mut cleared = 0u32;
        for read in (0..ROWS).rev() {
            if self.board[read].iter().all(|&v| v != 0) {
                cleared += 1;
            } else {
                write -= 1;
                compact[write] = self.board[read];
            }
        }
        self.board = compact;
        cleared
    }

    fn lock(&mut self) {
        let mut above_top = false;
        for (x, y) in self.active.cells() {
            if y < 0 {
                above_top = true;
            } else {
                self.board[y as usize][x as usize] = self.active.kind + 1;
            }
        }
        if above_top {
            self.phase = Phase::Over;
            return;
        }
        let cleared = self.clear_rows();
        let level = self.lines / 10 + 1;
        self.score = self.score.saturating_add(
            [0, 40, 100, 300, 1200][cleared.min(4) as usize] * level
        );
        self.lines += cleared;
        self.spawn();
    }

    fn down(&mut self) {
        if !self.try_shift(0, 1) { self.lock(); }
    }

    /// Advances fixed simulation time by one 1/120-second tick.
    pub fn tick(&mut self) {
        if self.phase == Phase::Over { return; }
        self.ticks += 1;
        let gravity_ticks = 60u64.saturating_sub(u64::from(self.lines / 10) * 4).max(6);
        if self.ticks.is_multiple_of(gravity_ticks) { self.down(); }
    }

    /// Discrete input action; keyboard autorepeat is managed by the caller.
    pub fn press(&mut self, action: Action) {
        if action == Action::Restart {
            *self = Self::new(self.seed);
            return;
        }
        if self.phase == Phase::Over { return; }
        match action {
            Action::Left => { self.try_shift(-1, 0); }
            Action::Right => { self.try_shift(1, 0); }
            Action::RotateClockwise => { self.rotate(1); }
            Action::RotateCounterClockwise => { self.rotate(-1); }
            Action::SoftDrop => {
                if self.try_shift(0, 1) {
                    self.score = self.score.saturating_add(1);
                } else {
                    self.lock();
                }
            }
            Action::HardDrop => {
                let mut dropped = 0u32;
                while self.try_shift(0, 1) { dropped += 1; }
                self.score = self.score.saturating_add(dropped.saturating_mul(2));
                self.lock();
            }
            Action::Restart => unreachable!(),
        }
    }

    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        let mut write = |byte: u8| {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        };
        for row in self.board { for cell in row { write(cell); } }
        for value in [self.active.kind, self.active.rotation,
            self.active.x as u8, self.active.y as u8, self.next,
            self.phase as u8, self.bag_index as u8] {
            write(value);
        }
        for byte in self.lines.to_le_bytes()
            .into_iter().chain(self.score.to_le_bytes())
            .chain(self.ticks.to_le_bytes())
            .chain(self.random_state.to_le_bytes()) {
            write(byte);
        }
        for byte in self.bag { write(byte); }
        hash
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderError { Dimensions, BufferTooSmall }

const CELL: i32 = 16;
const ORIGIN_X: i32 = 208;
const ORIGIN_Y: i32 = 20;

struct Painter<'a> {
    pixels: &'a mut [u32],
    stride: usize,
    w: usize,
    h: usize,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Painter<'_> {
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        if w <= 0 || h <= 0 { return; }
        let clip_x = |n: i32| {
            (n as f32 * self.scale + self.offset_x)
                .floor().clamp(0.0, self.w as f32) as usize
        };
        let clip_y = |n: i32| {
            (n as f32 * self.scale + self.offset_y)
                .floor().clamp(0.0, self.h as f32) as usize
        };
        let (left, right) = (clip_x(x), clip_x(x + w));
        let (top, bottom) = (clip_y(y), clip_y(y + h));
        for yy in top..bottom {
            self.pixels[yy * self.stride + left..yy * self.stride + right].fill(color);
        }
    }

    fn cell(&mut self, x: i32, y: i32, color: u32, ghost: bool) {
        self.rect(x, y, CELL, CELL, 0xff0c1626);
        if ghost {
            self.rect(x + 2, y + 2, CELL - 4, CELL - 4, color);
            self.rect(x + 4, y + 4, CELL - 8, CELL - 8, 0xff182438);
        } else {
            self.rect(x + 1, y + 1, CELL - 2, CELL - 2, color);
            self.rect(x + 2, y + 2, CELL - 4, 2, 0xffd8e7f5);
        }
    }
}

/// Renders a simple board and preview directly into Akula-compatible opaque
/// 0xAARRGGBB pixels, honoring padded DXGI row pitches.
pub fn raster(
    game: &Game,
    pixels: &mut [u32],
    stride: usize,
    width: u32,
    height: u32,
) -> Result<(), RenderError> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || stride < w {
        return Err(RenderError::Dimensions);
    }
    let len = stride.checked_mul(h - 1).and_then(|n| n.checked_add(w))
        .ok_or(RenderError::Dimensions)?;
    if pixels.len() < len { return Err(RenderError::BufferTooSmall); }
    for row in 0..h { pixels[row * stride..row * stride + w].fill(0xff0d1522); }

    let scale = (width as f32 / LOGICAL_WIDTH as f32)
        .min(height as f32 / LOGICAL_HEIGHT as f32);
    let mut paint = Painter {
        pixels, stride, w, h, scale,
        offset_x: (width as f32 - LOGICAL_WIDTH as f32 * scale) * 0.5,
        offset_y: (height as f32 - LOGICAL_HEIGHT as f32 * scale) * 0.5,
    };
    paint.rect(ORIGIN_X - 3, ORIGIN_Y - 3, 10 * CELL + 6, 20 * CELL + 6, 0xff4b6585);
    paint.rect(ORIGIN_X, ORIGIN_Y, 10 * CELL, 20 * CELL, COLORS[0]);

    for y in 0..ROWS {
        for x in 0..COLS {
            let cell = game.board[y][x];
            let xx = ORIGIN_X + x as i32 * CELL;
            let yy = ORIGIN_Y + y as i32 * CELL;
            if cell == 0 {
                paint.rect(xx + 1, yy + 1, CELL - 2, CELL - 2, 0xff1b2a40);
            } else {
                paint.cell(xx, yy, COLORS[cell as usize], false);
            }
        }
    }
    if game.phase == Phase::Playing {
        let mut ghost = game.active;
        while {
            let mut next = ghost;
            next.y += 1;
            if game.can_place(next) { ghost = next; true } else { false }
        } {}
        for (x, y) in ghost.cells() {
            if y >= 0 {
                paint.cell(ORIGIN_X + i32::from(x) * CELL,
                    ORIGIN_Y + i32::from(y) * CELL,
                    0xff40566d, true);
            }
        }
        for (x, y) in game.active.cells() {
            if y >= 0 {
                paint.cell(ORIGIN_X + i32::from(x) * CELL,
                    ORIGIN_Y + i32::from(y) * CELL,
                    COLORS[usize::from(game.active.kind + 1)], false);
            }
        }
    }
    // Preview of the next piece.
    paint.rect(398, 62, 90, 90, 0xff24344b);
    let preview = Piece { kind: game.next, rotation: 0, x: 0, y: 0 };
    for (x, y) in preview.cells() {
        paint.cell(404 + i32::from(x) * CELL,
            70 + i32::from(y) * CELL,
            COLORS[usize::from(game.next + 1)], false);
    }
    // Level progress and lives are intentionally not part of this version.
    let filled = (game.lines % 10) as i32;
    paint.rect(400, 190, 80, 8, 0xff30445b);
    paint.rect(400, 190, filled * 8, 8, 0xff40c9e7);
    if game.phase == Phase::Over {
        paint.rect(ORIGIN_X + 20, ORIGIN_Y + 145, 120, 30, 0xffc74d58);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seven_shapes_use_four_unique_cells() {
        for kind in 0..7 {
            for rotation in 0..4 {
                let cells = Piece {kind, rotation, x: 3, y: 0}.cells();
                let mut sorted = cells;
                sorted.sort();
                assert!(sorted.windows(2).all(|pair| pair[0] != pair[1]));
            }
        }
    }

    #[test]
    fn each_bag_has_seven_different_pieces() {
        let mut game = Game::new(45);
        let mut first = vec![game.active.kind, game.next];
        first.extend((0..5).map(|_| game.draw_bag()));
        first.sort();
        assert_eq!(first, vec![0, 1, 2, 3, 4, 5, 6]);
        for _ in 0..2 {
            let mut bag = (0..7).map(|_| game.draw_bag()).collect::<Vec<_>>();
            bag.sort();
            assert_eq!(bag, vec![0, 1, 2, 3, 4, 5, 6]);
        }
    }

    #[test]
    fn sideways_movement_respects_walls() {
        let mut g = Game::new(13);
        for _ in 0..30 { g.press(Action::Left); }
        assert!(g.can_place(g.active));
        let x = g.active.x;
        g.press(Action::Left);
        assert_eq!(g.active.x, x);
    }

    #[test]
    fn hard_drop_locks_and_spawns() {
        let mut g = Game::new(1);
        g.press(Action::HardDrop);
        assert!(g.board.iter().flatten().any(|&cell| cell != 0));
        assert_eq!(g.phase, Phase::Playing);
    }

    #[test]
    fn clears_full_rows_and_moves_remaining_down() {
        let mut g = Game::new(1);
        g.board[ROWS - 1] = [1; COLS];
        g.board[ROWS - 2][4] = 2;
        assert_eq!(g.clear_rows(), 1);
        assert_eq!(g.board[ROWS - 1][4], 2);
        assert!(g.board[ROWS - 2].iter().all(|v| *v == 0));
    }

    #[test]
    fn blocked_spawn_ends_game() {
        let mut g = Game::new(5);
        g.board[0] = [1; COLS];
        g.spawn();
        assert_eq!(g.phase, Phase::Over);
        g.press(Action::Restart);
        assert_eq!(g.phase, Phase::Playing);
        assert_eq!(g.board[0], [0; COLS]);
    }

    #[test]
    fn repeatable_recording_and_restart() {
        fn run() -> u64 {
            let mut g = Game::new(101);
            for i in 0..3000 {
                if i % 11 == 0 { g.press(Action::RotateClockwise); }
                if i % 17 == 0 { g.press(Action::Left); }
                if i % 23 == 0 { g.press(Action::Right); }
                if i % 71 == 0 { g.press(Action::HardDrop); }
                g.tick();
            }
            let hash = g.fingerprint();
            g.press(Action::Restart);
            assert_eq!(g.fingerprint(), Game::new(101).fingerprint());
            hash
        }
        assert_eq!(run(), run());
    }

    #[test]
    fn raster_keeps_pitch_padding_and_rejects_small_buffer() {
        let g = Game::default();
        let mut pixels = vec![0xdead_beef; 14 * 10];
        raster(&g, &mut pixels, 14, 10, 10).unwrap();
        assert_ne!(pixels[0], 0xdead_beef);
        for row in 0..10 { assert_eq!(pixels[row * 14 + 10], 0xdead_beef); }
        assert_eq!(raster(&g, &mut pixels[0..10], 14, 10, 10),
            Err(RenderError::BufferTooSmall));
        assert_eq!(raster(&g, &mut pixels, 3, 10, 10),
            Err(RenderError::Dimensions));
    }
}
