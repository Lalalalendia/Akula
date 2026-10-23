use std::rc::Rc;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;
use super::{Area, Canvas, Target};
use super::{dxgi, soft};

enum Backend {
    Modern(dxgi::Output),
    Software(soft::Output),
}

/// One executable on Windows 7/10/11. Prefer the existing DXGI path, falling
/// back to the existing GDI softbuffer output if DXGI 1.2 is unavailable.
pub struct Output {
    backend: Backend,
}

impl Output {
    pub fn new(display: OwnedDisplayHandle, window: Rc<dyn Window>) -> Result<Self, String> {
        match dxgi::Output::new(display.clone(), window.clone()) {
            Ok(modern) => Ok(Self { backend: Backend::Modern(modern) }),
            Err(err) => {
                report!("DXGI unavailable, using software output: {err}");
                let soft = soft::Output::new(display, window)?;
                Ok(Self { backend: Backend::Software(soft) })
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        match &mut self.backend {
            Backend::Modern(output) => output.resize(width, height),
            Backend::Software(output) => output.resize(width, height),
        }
    }

    pub fn trim(&mut self) {
        match &mut self.backend {
            Backend::Modern(output) => output.trim(),
            Backend::Software(output) => output.trim(),
        }
    }

    pub fn set_background(&mut self, bgra: u32) {
        match &mut self.backend {
            Backend::Modern(output) => output.set_background(bgra),
            Backend::Software(output) => output.set_background(bgra),
        }
    }

    pub fn fresh(&self) -> bool {
        match &self.backend {
            Backend::Modern(output) => output.fresh(),
            Backend::Software(output) => output.fresh(),
        }
    }

    pub fn present(&mut self, dirty: &[Area]) {
        match &mut self.backend {
            Backend::Modern(output) => output.present(dirty),
            Backend::Software(output) => output.present(dirty),
        }
    }
}

impl Canvas for Output {
    fn with_target(&mut self, paint: &mut dyn FnMut(Target<'_>)) {
        match &mut self.backend {
            Backend::Modern(output) => output.with_target(paint),
            Backend::Software(output) => output.with_target(paint),
        }
    }
}
