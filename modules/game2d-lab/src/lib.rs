//! Isolated no-dependency, no-I/O game model and ARGB8888 software raster lab.
//! This is NOT wired to Akula Base or an installed app.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input { pub left: bool, pub right: bool }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ball { pub x: f32, pub y: f32, pub vx: f32, pub vy: f32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Playing, Won, Lost }
#[derive(Clone, Debug)]
pub struct Game {
    pub ball: Ball,
    pub paddle_x: f32,
    pub bricks: [bool; 24],
    pub lives: u8,
    pub score: u32,
    pub phase: Phase,
    pub ticks: u64,
}
pub const W: usize = 320;
pub const H: usize = 180;
const PADDLE_W: f32 = 52.0;
const PADDLE_Y: f32 = 163.0;
const R: f32 = 4.0;
pub const DT: f32 = 1.0 / 120.0;
impl Default for Game {
    fn default() -> Self {
        Self {
            ball: Ball { x: 160.0, y: 125.0, vx: 86.0, vy: -103.0 },
            paddle_x: 160.0,
            bricks: [true; 24],
            lives: 3, score: 0, phase: Phase::Playing, ticks: 0,
        }
    }
}
impl Game {
    /// Fixed timestep; caller owns scheduling and input capture.
    pub fn step(&mut self, input: Input) {
        if self.phase != Phase::Playing { return; }
        self.ticks += 1;
        let d = (input.right as i8 - input.left as i8) as f32;
        self.paddle_x = (self.paddle_x + d * 180.0 * DT)
            .clamp(PADDLE_W / 2.0, W as f32 - PADDLE_W / 2.0);
        let previous = self.ball;
        self.ball.x += self.ball.vx * DT;
        self.ball.y += self.ball.vy * DT;
        if self.ball.x < R { self.ball.x = R; self.ball.vx = self.ball.vx.abs(); }
        if self.ball.x > W as f32 - R { self.ball.x = W as f32 - R; self.ball.vx = -self.ball.vx.abs(); }
        if self.ball.y < R { self.ball.y = R; self.ball.vy = self.ball.vy.abs(); }
        let p_left = self.paddle_x - PADDLE_W / 2.0;
        let p_right = self.paddle_x + PADDLE_W / 2.0;
        if previous.y + R <= PADDLE_Y && self.ball.y + R >= PADDLE_Y
            && self.ball.vy > 0.0 && self.ball.x + R >= p_left && self.ball.x - R <= p_right {
            self.ball.y = PADDLE_Y - R;
            self.ball.vy = -self.ball.vy.abs();
            self.ball.vx = ((self.ball.x - self.paddle_x) / (PADDLE_W / 2.0) * 130.0).clamp(-130.0,130.0);
        }
        for (idx, alive) in self.bricks.iter_mut().enumerate() {
            if !*alive { continue; }
            let (x, y) = brick_origin(idx);
            let closest_x = self.ball.x.clamp(x as f32, (x + 35) as f32);
            let closest_y = self.ball.y.clamp(y as f32, (y + 12) as f32);
            let dx = self.ball.x - closest_x;
            let dy = self.ball.y - closest_y;
            if dx * dx + dy * dy <= R * R {
                *alive = false; self.score += 10;
                if previous.y + R <= y as f32 || previous.y - R >= (y+12) as f32 { self.ball.vy = -self.ball.vy; }
                else { self.ball.vx = -self.ball.vx; }
                break;
            }
        }
        if self.bricks.iter().all(|alive| !*alive) { self.phase = Phase::Won; }
        if self.ball.y - R > H as f32 {
            self.lives -= 1;
            if self.lives == 0 { self.phase = Phase::Lost; }
            else { self.ball = Ball { x: self.paddle_x, y: 125.0, vx: 86.0, vy: -103.0 }; }
        }
    }
    pub fn restart(&mut self) { *self = Self::default(); }
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        let mut byte = |v: u8| { h = (h ^ u64::from(v)).wrapping_mul(0x100000001b3); };
        for value in [self.ball.x, self.ball.y, self.ball.vx, self.ball.vy, self.paddle_x] {
            for b in value.to_bits().to_le_bytes() { byte(b); }
        }
        for b in self.bricks { byte(b as u8); }
        for b in self.score.to_le_bytes().into_iter().chain(self.ticks.to_le_bytes()) { byte(b); }
        byte(self.lives);
        byte(match self.phase { Phase::Playing => 0, Phase::Won => 1, Phase::Lost => 2 });
        h
    }
}
fn brick_origin(idx: usize) -> (usize,usize) { (12 + (idx % 8) * 39, 16 + (idx / 8) * 21) }
fn rect(buf: &mut [u32], stride: usize, x:i32, y:i32, w:i32, h:i32, color:u32) {
    let height = buf.len() / stride;
    let x0 = x.max(0).min(stride as i32) as usize;
    let y0 = y.max(0).min(height as i32) as usize;
    let x1 = x.saturating_add(w).max(0).min(stride as i32) as usize;
    let y1 = y.saturating_add(h).max(0).min(height as i32) as usize;
    for row in y0..y1 { buf[row*stride+x0..row*stride+x1].fill(color); }
}
/// Draw into Akula-compatible opaque 0xAARRGGBB u32 pixels.
/// The caller may pass an output stride larger than the logical width.
pub fn draw(game:&Game, pixels:&mut [u32], stride:usize, height:usize) -> Result<(), &'static str> {
    if stride < W || height < H || pixels.len() < stride.checked_mul(height).ok_or("overflow")? {
        return Err("buffer too small");
    }
    for row in 0..H { pixels[row*stride..row*stride+W].fill(0xff131c2c); }
    for (i,active) in game.bricks.iter().enumerate() {
        if *active { let (x,y)=brick_origin(i); rect(pixels,stride,x as i32,y as i32,35,12,
          [0xff45b6d9,0xff5fd6ab,0xffe5ad56][i/8]); }
    }
    rect(pixels,stride,(game.paddle_x-PADDLE_W/2.0) as i32,PADDLE_Y as i32,
        PADDLE_W as i32,7,0xff7aa8ff);
    let cx = game.ball.x as i32; let cy=game.ball.y as i32;
    for dy in -4..=4 { for dx in -4..=4 {
        if dx*dx + dy*dy <= 16 { rect(pixels,stride,cx+dx,cy+dy,1,1,0xfffaf7ee); }
    }}
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn initial_scene_and_paddle_bounds() {
        let mut g=Game::default();
        assert_eq!(g.bricks.iter().filter(|b|**b).count(),24);
        for _ in 0..800 {g.step(Input{left:true,right:false});}
        assert_eq!(g.paddle_x,PADDLE_W/2.0);
    }
    #[test] fn deterministic_replay() {
        let replay = | | {
            let mut g=Game::default();
            for n in 0..2400 { g.step(Input { left:n%29<6,right:n%29>=14 && n%29<21 }); }
            g.state_hash()
        };
        assert_eq!(replay(),replay());
    }
    #[test] fn collision_destroys_brick() {
        let mut g=Game::default();
        g.ball=Ball{x:20.0,y:45.0,vx:0.0,vy:-120.0};
        for _ in 0..80 { g.step(Input::default()); if g.score>0 {break;} }
        assert_eq!(g.score,10);
        assert!(!g.bricks[8]);
    }
    #[test] fn life_loss_and_restart() {
        let mut g=Game::default(); g.lives=1;
        g.ball=Ball{x:40.0,y:182.0,vx:0.0,vy:120.0};
        g.step(Input::default());
        assert_eq!(g.phase,Phase::Lost);
        g.restart(); assert_eq!(g.phase,Phase::Playing); assert_eq!(g.score,0);
    }
    #[test] fn wins_if_last_brick_breaks() {
        let mut g=Game::default(); g.bricks.fill(false); g.bricks[0]=true;
        g.ball=Ball{x:20.0,y:11.0,vx:0.0,vy:100.0};
        for _ in 0..35 {g.step(Input::default()); if g.phase==Phase::Won {break;}}
        assert_eq!(g.phase,Phase::Won);
    }
    #[test] fn pixel_clipping_and_stride() {
        let mut g=Game::default(); g.ball.x=-3.0;g.ball.y=-4.0;
        let stride=W+16;let mut pix=vec![0xfedcba98;stride*H];
        draw(&g,&mut pix,stride,H).unwrap();
        assert_eq!(pix[W],0xfedcba98);
        assert_eq!(pix[0],0xff131c2c);
        assert!(draw(&g,&mut pix[..10],stride,H).is_err());
    }
}
