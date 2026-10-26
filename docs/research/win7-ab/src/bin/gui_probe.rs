//! Create and close a real winit 0.31.0-beta.3 window; do not execute on CI.
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{WindowAttributes, WindowId};

struct Probe;

impl ApplicationHandler for Probe {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop
            .create_window(WindowAttributes::default())
            .expect("could not create test window");
        event_loop.exit();
    }

    fn window_event(
        &mut self,
        _: &dyn ActiveEventLoop,
        _: WindowId,
        _: WindowEvent,
    ) {}
}

fn main() {
    let event_loop = EventLoop::new().expect("could not create event loop");
    event_loop.run_app(Probe).expect("event loop failed");
}
