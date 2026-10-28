//! Explicit opt-in native window experiment. This is not part of installed Akula.
//! cargo run --manifest-path modules/native-2d-window-lab/Cargo.toml -- --game tetris
// Avoid Windows Terminal obscuring the actual game window in desktop CI.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

macro_rules! report {
    ($($arg:tt)*) => { eprintln!($($arg)*); };
}

// Include the exact Akula Canvas/Output implementation, without editing Base.
#[path = "../../../aquilum-app/native/src/output/mod.rs"]
mod output;
// Standalone fixed-tick game rules, without bringing them into Akula Base.
#[path = "../../native-2d-lab/src/lib.rs"]
mod game2d;
#[path = "../../native-tetris-lab/src/lib.rs"]
mod tetris;

use output::{Area, Canvas};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::event_loop::run_on_demand::EventLoopExtRunOnDemand;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

const FRAME: Duration = Duration::from_nanos(16_666_667);
const TICK: Duration = Duration::from_nanos(8_333_333);
const MAX_STEPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode { Breakout, Tetris }

fn p95(samples: &[f64]) -> f64 {
    if samples.is_empty() { return 0.0; }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() - 1) as f64 * 0.95).ceil() as usize]
}

struct Lab {
    window: Option<Rc<dyn Window>>,
    output: Option<output::Output>,
    mode: Mode,
    world: game2d::World,
    tetris_world: tetris::Game,
    controls: game2d::Input,
    pending_start: bool,
    auto_frames: Option<u64>,
    completed: Rc<Cell<bool>>,
    failed: Rc<Cell<bool>>,
    focused: bool,
    occluded: bool,
    last_tick: Instant,
    next_frame: Instant,
    accumulator: Duration,
    started: Instant,
    presented: u64,
    dropped_ticks: u64,
    simulation: Vec<f64>,
    raster: Vec<f64>,
    present: Vec<f64>,
}

impl Lab {
    fn new(mode: Mode, auto_frames: Option<u64>, completed: Rc<Cell<bool>>, failed: Rc<Cell<bool>>) -> Self {
        let now = Instant::now();
        Self {
            window: None,
            output: None,
            mode,
            world: game2d::World::new(),
            tetris_world: tetris::Game::new(2026),
            controls: game2d::Input::default(),
            pending_start: auto_frames.is_some(),
            auto_frames,
            completed,
            failed,
            focused: true,
            occluded: false,
            last_tick: now,
            next_frame: now,
            accumulator: Duration::ZERO,
            started: now,
            presented: 0,
            dropped_ticks: 0,
            simulation: Vec::new(),
            raster: Vec::new(),
            present: Vec::new(),
        }
    }

    fn report_and_exit(&mut self, events: &dyn ActiveEventLoop) {
        let seconds = self.started.elapsed().as_secs_f64().max(0.001);
        let (ticks, score, phase, hash) = match self.mode {
            Mode::Breakout => (self.world.ticks, self.world.score, format!("{:?}", self.world.phase), self.world.fingerprint()),
            Mode::Tetris => (self.tetris_world.ticks, self.tetris_world.score, format!("{:?}", self.tetris_world.phase), self.tetris_world.fingerprint()),
        };
        println!(
            "AKULA_GAME2D_WINDOW game={:?} presented={} ticks={} score={} phase={} hash={:016x} \
             actual_fps={:.2} sim_p95_ms={:.4} raster_p95_ms={:.4} \
             present_p95_ms={:.4} dropped_ticks={} output_present={} source=AkulaOutput",
            self.mode,
            self.presented, ticks, score, phase, hash,
            self.presented as f64 / seconds,
            p95(&self.simulation),
            p95(&self.raster),
            p95(&self.present),
            self.dropped_ticks,
            if self.failed.get() { "FAIL" } else { "PASS" },
        );
        if let Some(goal) = self.auto_frames {
            self.completed.set(!self.failed.get() && self.presented >= goal);
        }
        events.exit();
    }

    fn failed(&mut self, events: &dyn ActiveEventLoop, error: impl std::fmt::Display) {
        eprintln!("AKULA_GAME2D_WINDOW_ERROR: {error}");
        self.failed.set(true);
        self.report_and_exit(events);
    }

    fn advance(&mut self, now: Instant) {
        let delta = now.saturating_duration_since(self.last_tick).min(Duration::from_millis(100));
        self.last_tick = now;
        if !self.focused && self.auto_frames.is_none() { self.accumulator = Duration::ZERO; return; }
        let start = Instant::now();
        self.accumulator += delta;
        let mut count = 0;
        while self.accumulator >= TICK && count < MAX_STEPS {
            match self.mode {
                Mode::Breakout => {
                    let mut input = self.controls;
                    input.start = std::mem::take(&mut self.pending_start);
                    self.world.step(input);
                }
                Mode::Tetris => self.tetris_world.tick(),
            }
            self.accumulator -= TICK;
            count += 1;
        }
        if self.accumulator >= TICK {
            self.dropped_ticks += (self.accumulator.as_nanos() / TICK.as_nanos()) as u64;
            self.accumulator = Duration::ZERO;
        }
        if self.simulation.len() < 60_000 {
            self.simulation.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    fn paint(&mut self, events: &dyn ActiveEventLoop) {
        let Some(window) = self.window.clone() else { return };
        let size = window.surface_size();
        if size.width == 0 || size.height == 0 || self.occluded { return; }
        let Some(output) = self.output.as_mut() else { return };
        if let Err(error) = output.resize(size.width, size.height) {
            self.failed(events, format!("resize: {error}"));
            return;
        }
        if size.width > u32::from(u16::MAX) || size.height > u32::from(u16::MAX) {
            self.failed(events, "unsupported canvas dimensions");
            return;
        }
        let mut success = None;
        let begin = Instant::now();
        output.with_target(&mut |target| {
            success = Some(match self.mode {
                Mode::Breakout => game2d::raster(
                    &self.world, target.pixels, usize::from(target.width), size.width, size.height
                ).is_ok(),
                Mode::Tetris => tetris::raster(
                    &self.tetris_world, target.pixels, usize::from(target.width), size.width, size.height
                ).is_ok(),
            });
        });
        if success != Some(true) {
            self.failed(events, format!("canvas raster/map failed: {success:?}"));
            return;
        }
        if self.raster.len() < 60_000 {
            self.raster.push(begin.elapsed().as_secs_f64() * 1000.0);
        }
        let before = Instant::now();
        output.present(&[Area { x: 0, y: 0, w: size.width, h: size.height }]);
        if output.fresh() {
            self.failed(events, "Akula Output::present did not complete");
            return;
        }
        if self.present.len() < 60_000 {
            self.present.push(before.elapsed().as_secs_f64() * 1000.0);
        }
        self.presented += 1;
        if self.auto_frames.is_some_and(|goal| self.presented >= goal) {
            self.report_and_exit(events);
        }
    }
}

impl ApplicationHandler for Lab {
    fn can_create_surfaces(&mut self, events: &dyn ActiveEventLoop) {
        let title = match self.mode {
            Mode::Breakout => "Akula 2D: Breakout (Arrows/A-D, Space, Esc)",
            Mode::Tetris => "Akula 2D: Falling Blocks (Arrows, Z/X, Space, Enter, Esc)",
        };
        let attributes = WindowAttributes::default()
            .with_title(title)
            .with_surface_size(PhysicalSize::new(960, 540));
        let window: Rc<dyn Window> = match events.create_window(attributes) {
            Ok(w) => Rc::from(w),
            Err(e) => { self.failed(events, format!("window: {e}")); return; }
        };
        let output = match output::Output::new(events.owned_display_handle(), window.clone()) {
            Ok(output) => output,
            Err(e) => { self.failed(events, format!("output: {e}")); return; }
        };
        let now = Instant::now();
        self.last_tick = now;
        self.next_frame = now;
        self.started = now;
        self.output = Some(output);
        self.window = Some(window.clone());
        window.request_redraw();
    }

    fn proxy_wake_up(&mut self, _events: &dyn ActiveEventLoop) {}

    fn about_to_wait(&mut self, events: &dyn ActiveEventLoop) {
        if self.window.is_none() { return; }
        let now = Instant::now();
        self.advance(now);
        if now >= self.next_frame {
            if !self.occluded {
                if let Some(window) = &self.window { window.request_redraw(); }
            }
            self.next_frame = now + FRAME;
        }
        events.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }

    fn window_event(&mut self, events: &dyn ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.report_and_exit(events),
            WindowEvent::RedrawRequested => self.paint(events),
            WindowEvent::SurfaceResized(_) => {
                if let Some(window) = &self.window { window.request_redraw(); }
            }
            WindowEvent::Occluded(state) => {
                self.occluded = state;
                self.last_tick = Instant::now();
            }
            WindowEvent::Focused(state) => {
                self.focused = state;
                self.last_tick = Instant::now();
                if !state && self.auto_frames.is_none() {
                    self.controls = game2d::Input::default();
                    self.pending_start = false;
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    if code == KeyCode::Escape && down {
                        self.report_and_exit(events);
                        return;
                    }
                    match self.mode {
                        Mode::Breakout => match code {
                            KeyCode::ArrowLeft | KeyCode::KeyA => self.controls.left = down,
                            KeyCode::ArrowRight | KeyCode::KeyD => self.controls.right = down,
                            KeyCode::Space | KeyCode::Enter if down && !event.repeat =>
                                self.pending_start = true,
                            _ => {}
                        },
                        Mode::Tetris if down => {
                            let action = match code {
                                KeyCode::ArrowLeft | KeyCode::KeyA => Some(tetris::Action::Left),
                                KeyCode::ArrowRight | KeyCode::KeyD => Some(tetris::Action::Right),
                                KeyCode::ArrowDown | KeyCode::KeyS => Some(tetris::Action::SoftDrop),
                                KeyCode::ArrowUp | KeyCode::KeyW | KeyCode::KeyX =>
                                    Some(tetris::Action::RotateClockwise),
                                KeyCode::KeyZ => Some(tetris::Action::RotateCounterClockwise),
                                KeyCode::Space if !event.repeat => Some(tetris::Action::HardDrop),
                                KeyCode::Enter if !event.repeat => Some(tetris::Action::Restart),
                                _ => None,
                            };
                            if let Some(action) = action { self.tetris_world.press(action); }
                        }
                        Mode::Tetris => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_args() -> Result<(Mode, Option<u64>), String> {
    let mut args = std::env::args().skip(1);
    let mut mode = Mode::Breakout;
    let mut frames = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--game" => {
                mode = match args.next().as_deref() {
                    Some("breakout") => Mode::Breakout,
                    Some("tetris") => Mode::Tetris,
                    _ => return Err("--game must be breakout or tetris".into()),
                };
            }
            "--frames" => {
                let value = args.next().ok_or("missing --frames value")?;
                let n = value.parse::<u64>().map_err(|_| "bad --frames value")?;
                if !(1..=1200).contains(&n) || frames.is_some() {
                    return Err("--frames must be in 1..=1200 and specified once".into());
                }
                frames = Some(n);
            }
            _ => return Err("use: --game breakout|tetris [--frames 240]".into()),
        }
    }
    Ok((mode, frames))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mode, frames) = parse_args()?;
    let completed = Rc::new(Cell::new(false));
    let failed = Rc::new(Cell::new(false));
    let mut event_loop = EventLoop::new()?;
    event_loop.run_app_on_demand(Lab::new(mode, frames, completed.clone(), failed.clone()))?;
    if failed.get() || frames.is_some() && !completed.get() {
        return Err("Akula 2D native window witness incomplete or failed".into());
    }
    Ok(())
}
