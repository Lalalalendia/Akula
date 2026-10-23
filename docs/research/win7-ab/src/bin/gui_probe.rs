//! Create and close a real winit 0.31.0-beta.3 window; never run on CI's host.
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{WindowAttributes, WindowId};

struct Probe;

impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.create_window(WindowAttributes::default())
            .expect("could not create test window");
        event_loop.exit();
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn main() {
    let event_loop = EventLoop::new().expect("could not create event loop");
    event_loop.run_app(&mut Probe).expect("event loop failed");
}
