//! Isolated, dependency-free 2D Breakout experiment for Akula.
 //! No OS, filesystem, networking, or Akula Core access.
 //!
 //! This is a laboratory baseline, not a general-purpose game engine.

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FIXED_HZ: u32 = 120;
pub const DT: f32 = 1.0 / FIXED_HZ as f32;

const COLS: usize = 10;
const ROWS: usize = 4;
const BRICK_COUNT: usize = COLS * ROWS;
const BRICK_W: f32 = 52.0;
const BRICK_H: f32 = 14.0;
const BRICK_GAP: f32 = 6.0;
const BRICK_LEFT: f32 = 33.0;
const BRICK_TOP: f32 = 56.0;
const PADDLE_Y: f32 = 328.0;
const PADDLE_W: f32 = 84.0;
const PADDLE_H: f32 = 10.0;
const PADDLE_SPEED: f32 = 340.0;
const BALL_R: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    /// Rising-edge action: start or restart. Clear after one simulation step.
    pub start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Playing,
    Won,
    Lost,
}

#[derive(Clone, Debug)]
pub struct World {
    pub phase: Phase,
    pub paddle_x: f32,
    pub ball_x: f32,
    pub ball_y: f32,
    pub ball_vx: f32,
    pub ball_vy: f32,
    pub lives: u8,
    pub score: u32,
    pub ticks: u64,
    bricks: [bool; BRICK_COUNT],
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            phase: Phase::Ready,
            paddle_x: WIDTH as f32 * 0.5,
            ball_x: WIDTH as f32 * 0.5,
            ball_y: PADDLE_Y - BALL_R - 2.0,
            ball_vx: 95.0,
            ball_vy: -220.0,
            lives: 3,
            score: 0,
            ticks: 0,
            bricks: [true; BRICK_COUNT],
        }
    }

    pub fn bricks_left(&self) -> usize {
        self.bricks.iter().filter(|&&alive| alive).count()
    }

    pub fn step(&mut self, input: Input) {
        self.ticks = self.ticks.saturating_add(1);
        if input.start && self.phase != Phase::Playing {
            *self = World::new();
            self.phase = Phase::Playing;
        }
        let direction = i32::from(input.right) - i32::from(input.left);
        self.paddle_x = (self.paddle_x + direction as f32 * PADDLE_SPEED * DT)
            .clamp(PADDLE_W * 0.5, WIDTH as f32 - PADDLE_W * 0.5);

        if self.phase != Phase::Playing {
            if self.phase == Phase::Ready {
                self.ball_x = self.paddle_x;
                self.ball_y = PADDLE_Y - BALL_R - 2.0;
            }
            return;
        }

        let (previous_x, previous_y) = (self.ball_x, self.ball_y);
        self.ball_x += self.ball_vx * DT;
        self.ball_y += self.ball_vy * DT;

        if self.ball_x - BALL_R < 0.0 {
            self.ball_x = BALL_R;
            self.ball_vx = self.ball_vx.abs();
        } else if self.ball_x + BALL_R > WIDTH as f32 {
            self.ball_x = WIDTH as f32 - BALL_R;
            self.ball_vx = -self.ball_vx.abs();
        }
        if self.ball_y - BALL_R < 28.0 {
            self.ball_y = 28.0 + BALL_R;
            self.ball_vy = self.ball_vy.abs();
        }

        if self.ball_vy > 0.0
            && previous_y + BALL_R <= PADDLE_Y
            && self.ball_y + BALL_R >= PADDLE_Y
            && self.ball_x + BALL_R >= self.paddle_x - PADDLE_W * 0.5
            && self.ball_x - BALL_R <= self.paddle_x + PADDLE_W * 0.5
        {
            self.ball_y = PADDLE_Y - BALL_R;
            let offset = ((self.ball_x - self.paddle_x) / (PADDLE_W * 0.5))
                .clamp(-1.0, 1.0);
            self.ball_vx = offset * 210.0;
            self.ball_vy = -(260.0_f32.powi(2) - self.ball_vx.powi(2)).sqrt();
        }

        for index in 0..BRICK_COUNT {
            if !self.bricks[index] {
                continue;
            }
            let (x, y, w, h) = brick_rect(index);
            let nearest_x = self.ball_x.clamp(x, x + w);
            let nearest_y = self.ball_y.clamp(y, y + h);
            let dx = self.ball_x - nearest_x;
            let dy = self.ball_y - nearest_y;
            if dx * dx + dy * dy > BALL_R * BALL_R {
                continue;
            }

            self.bricks[index] = false;
            self.score += 10;
            if previous_x + BALL_R <= x {
                self.ball_x = x - BALL_R;
                self.ball_vx = -self.ball_vx.abs();
            } else if previous_x - BALL_R >= x + w {
                self.ball_x = x + w + BALL_R;
                self.ball_vx = self.ball_vx.abs();
            } else if previous_y < y {
                self.ball_y = y - BALL_R;
                self.ball_vy = -self.ball_vy.abs();
            } else {
                self.ball_y = y + h + BALL_R;
                self.ball_vy = self.ball_vy.abs();
            }
            if self.bricks_left() == 0 {
                self.phase = Phase::Won;
            }
            break; // One hit per fixed tick, with repeatable ordering.
        }

        if self.ball_y - BALL_R > HEIGHT as f32 && self.phase == Phase::Playing {
            self.lives = self.lives.saturating_sub(1);
            if self.lives == 0 {
                self.phase = Phase::Lost;
            } else {
                self.phase = Phase::Ready;
                self.ball_x = self.paddle_x;
                self.ball_y = PADDLE_Y - BALL_R - 2.0;
                self.ball_vx = 95.0;
                self.ball_vy = -220.0;
            }
        }
    }

    /// A reproducibility witness for scripted input sequences, not a file format.
    pub fn fingerprint(&self) -> u64 {
        fn mix(state: u64, word: u64) -> u64 {
            (state ^ word).wrapping_mul(0x1000_0000_01b3)
        }
        let mut state = 0xcbf2_9ce4_8422_2325;
        for word in [
            self.phase as u64,
            self.paddle_x.to_bits() as u64,
            self.ball_x.to_bits() as u64,
            self.ball_y.to_bits() as u64,
            self.ball_vx.to_bits() as u64,
            self.ball_vy.to_bits() as u64,
            self.lives as u64,
            self.score as u64,
            self.ticks,
        ] {
            state = mix(state, word);
        }
        for &alive in &self.bricks {
            state = mix(state, u64::from(alive));
        }
        state
    }
}

fn brick_rect(index: usize) -> (f32, f32, f32, f32) {
    let x = BRICK_LEFT + (index % COLS) as f32 * (BRICK_W + BRICK_GAP);
    let y = BRICK_TOP + (index / COLS) as f32 * (BRICK_H + BRICK_GAP);
    (x, y, BRICK_W, BRICK_H)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    InvalidDimensions,
    InsufficientBuffer,
}

/// Writes *opaque* 0xAARRGGBB pixels into the existing Akula BGRA canvas.
/// The stride may exceed the visible width (notably with D3D11 row pitch).
/// Does not allocate, retain the pixel buffer, or access application files.
pub fn raster(
    world: &World,
    pixels: &mut [u32],
    stride: usize,
    width: u32,
    height: u32,
) -> Result<(), FrameError> {
    let w = usize::try_from(width).map_err(|_| FrameError::InvalidDimensions)?;
    let h = usize::try_from(height).map_err(|_| FrameError::InvalidDimensions)?;
    if w == 0 || h == 0 || stride < w {
        return Err(FrameError::InvalidDimensions);
    }
    let need = stride
        .checked_mul(h - 1)
        .and_then(|n| n.checked_add(w))
        .ok_or(FrameError::InvalidDimensions)?;
    if pixels.len() < need {
        return Err(FrameError::InsufficientBuffer);
    }
    let scale = (width as f32 / WIDTH as f32).min(height as f32 / HEIGHT as f32);
    let offset_x = (width as f32 - WIDTH as f32 * scale) * 0.5;
    let offset_y = (height as f32 - HEIGHT as f32 * scale) * 0.5;
    let mut paint = Paint {
        pixels,
        stride,
        width: w,
        height: h,
        scale,
        offset_x,
        offset_y,
    };
    paint.clear(0xff0d_1522);
    paint.rect(0.0, 0.0, WIDTH as f32, HEIGHT as f32, 0xff17_2335);
    paint.rect(0.0, 27.0, WIDTH as f32, 1.0, 0xff40_536e);
    let top = format!("AKULA 2D   SCORE {:03}   LIVES {}", world.score, world.lives);
    paint.text(17.0, 9.0, &top, 0xffda_e8ff, 2.0);

    for index in 0..BRICK_COUNT {
        if world.bricks[index] {
            let (x, y, w, h) = brick_rect(index);
            let color = match index / COLS {
                0 => 0xff53_bbd8,
                1 => 0xff5d_cda4,
                2 => 0xffe2_b460,
                _ => 0xffe681_75,
            };
            paint.rect(x, y, w, h, color);
        }
    }
    paint.rect(
        world.paddle_x - PADDLE_W * 0.5,
        PADDLE_Y,
        PADDLE_W,
        PADDLE_H,
        0xff83_a8ff,
    );
    paint.circle(world.ball_x, world.ball_y, BALL_R, 0xfff5_f8ff);
    match world.phase {
        Phase::Ready => paint.text(212.0, 210.0, "SPACE TO START", 0xffea_f3ff, 3.0),
        Phase::Won => paint.text(206.0, 210.0, "YOU WIN SPACE RETRY", 0xff65_dba8, 2.4),
        Phase::Lost => paint.text(182.0, 210.0, "GAME OVER SPACE RETRY", 0xfff0_8277, 2.4),
        Phase::Playing => {}
    }
    Ok(())
}

struct Paint<'a> {
    pixels: &'a mut [u32],
    stride: usize,
    width: usize,
    height: usize,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Paint<'_> {
    fn clear(&mut self, color: u32) {
        for y in 0..self.height {
            let row = &mut self.pixels[y * self.stride..y * self.stride + self.width];
            row.fill(color);
        }
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let x0 = ((self.offset_x + x * self.scale).floor() as i32)
            .clamp(0, self.width as i32) as usize;
        let y0 = ((self.offset_y + y * self.scale).floor() as i32)
            .clamp(0, self.height as i32) as usize;
        let x1 = ((self.offset_x + (x + w) * self.scale).ceil() as i32)
            .clamp(0, self.width as i32) as usize;
        let y1 = ((self.offset_y + (y + h) * self.scale).ceil() as i32)
            .clamp(0, self.height as i32) as usize;
        for row in y0..y1 {
            self.pixels[row * self.stride + x0..row * self.stride + x1].fill(color);
        }
    }

    fn circle(&mut self, x: f32, y: f32, radius: f32, color: u32) {
        let cx = self.offset_x + x * self.scale;
        let cy = self.offset_y + y * self.scale;
        let radius = radius * self.scale;
        let x0 = ((cx - radius).floor() as i32).clamp(0, self.width as i32) as usize;
        let x1 = ((cx + radius).ceil() as i32).clamp(0, self.width as i32) as usize;
        let y0 = ((cy - radius).floor() as i32).clamp(0, self.height as i32) as usize;
        let y1 = ((cy + radius).ceil() as i32).clamp(0, self.height as i32) as usize;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let dx = xx as f32 + 0.5 - cx;
                let dy = yy as f32 + 0.5 - cy;
                if dx * dx + dy * dy <= radius * radius {
                    self.pixels[yy * self.stride + xx] = color;
                }
            }
        }
    }

    fn text(&mut self, x: f32, y: f32, text: &str, color: u32, size: f32) {
        for (n, character) in text.bytes().enumerate() {
            let glyph = glyph(character);
            for (row, &bits) in glyph.iter().enumerate() {
                for column in 0..5 {
                    if bits & (0b1_0000 >> column) != 0 {
                        self.rect(
                            x + (n * 6 + column) as f32 * size,
                            y + row as f32 * size,
                            size,
                            size,
                            color,
                        );
                    }
                }
            }
        }
    }
}

fn glyph(c: u8) -> [u8; 7] {
    match c {
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'G' => [14, 17, 16, 23, 17, 17, 14],
        b'I' => [31, 4, 4, 4, 4, 4, 31],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'N' => [17, 25, 21, 19, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'P' => [30, 17, 17, 30, 16, 16, 16],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'U' => [17, 17, 17, 17, 17, 17, 14],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'W' => [17, 17, 17, 21, 21, 21, 10],
        b'Y' => [17, 17, 10, 4, 4, 4, 4],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 30, 1, 1, 17, 14],
        b'6' => [6, 8, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 2, 12],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_paddle_clamp() {
        let mut w = World::new();
        assert_eq!(w.bricks_left(), BRICK_COUNT);
        w.step(Input { start: true, ..Input::default() });
        assert_eq!(w.phase, Phase::Playing);
        for _ in 0..1000 {
            w.step(Input { left: true, ..Input::default() });
        }
        assert_eq!(w.paddle_x, PADDLE_W * 0.5);
        for _ in 0..1000 {
            w.step(Input { right: true, ..Input::default() });
        }
        assert_eq!(w.paddle_x, WIDTH as f32 - PADDLE_W * 0.5);
    }

    #[test]
    fn brick_collision_removes_exactly_one_brick() {
        let mut w = World::new();
        w.phase = Phase::Playing;
        let (x, y, width, _) = brick_rect(0);
        w.ball_x = x + width * 0.5;
        w.ball_y = y + BRICK_H + BALL_R + 0.5;
        w.ball_vx = 0.0;
        w.ball_vy = -200.0;
        w.step(Input::default());
        assert_eq!(w.bricks_left(), BRICK_COUNT - 1);
        assert_eq!(w.score, 10);
        assert!(w.ball_vy > 0.0);
    }

    #[test]
    fn lost_ball_reduces_lives_and_stops_at_zero() {
        let mut w = World::new();
        w.phase = Phase::Playing;
        w.lives = 1;
        w.ball_y = HEIGHT as f32 + BALL_R + 3.0;
        w.ball_vy = 100.0;
        w.step(Input::default());
        assert_eq!(w.lives, 0);
        assert_eq!(w.phase, Phase::Lost);
        w.step(Input { start: true, ..Input::default() });
        assert_eq!(w.phase, Phase::Playing);
        assert_eq!(w.lives, 3);
    }

    #[test]
    fn input_sequence_is_reproducible() {
        let mut a = World::new();
        let mut b = World::new();
        for tick in 0..4000 {
            let input = Input {
                start: tick == 0 || tick == 2100,
                left: tick % 190 < 80,
                right: tick % 190 >= 80,
            };
            a.step(input);
            b.step(input);
            assert_eq!(a.fingerprint(), b.fingerprint(), "tick {tick}");
        }
    }

    #[test]
    fn raster_clips_and_keeps_pitch_padding() {
        let w = World::new();
        let mut pixels = vec![0xdead_beef; 12 * 10];
        raster(&w, &mut pixels, 12, 10, 10).unwrap();
        assert_ne!(pixels[0], 0xdead_beef);
        for y in 0..10 {
            assert_eq!(pixels[y * 12 + 10], 0xdead_beef);
            assert_eq!(pixels[y * 12 + 11], 0xdead_beef);
        }
    }

    #[test]
    fn malformed_canvas_cannot_write_out_of_bounds() {
        let mut pixels = vec![0; 20];
        assert_eq!(
            raster(&World::new(), &mut pixels, 2, 10, 2),
            Err(FrameError::InvalidDimensions)
        );
        assert_eq!(
            raster(&World::new(), &mut pixels, 20, 20, 2),
            Err(FrameError::InsufficientBuffer)
        );
    }

    #[test]
    fn wall_bounce_is_bounded() {
        let mut w = World::new();
        w.phase = Phase::Playing;
        w.ball_x = BALL_R - 0.1;
        w.ball_y = 200.0;
        w.ball_vx = -220.0;
        w.ball_vy = 0.0;
        w.step(Input::default());
        assert_eq!(w.ball_x, BALL_R);
        assert!(w.ball_vx > 0.0);
    }
}
