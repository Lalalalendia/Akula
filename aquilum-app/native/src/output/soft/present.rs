use super::Output;
use super::super::Area;
use std::num::NonZeroU32;

impl Output {
    pub fn present(&mut self, dirty: &[Area]) {
        let mut buffer = match self.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(err) => {
                report!("software output buffer: {err}");
                self.fresh = true;
                return;
            }
        };
        if buffer.len() != self.pixels.len() {
            report!(
                "software output buffer size mismatch: surface={} retained={}",
                buffer.len(),
                self.pixels.len()
            );
            self.fresh = true;
            return;
        }
        buffer.copy_from_slice(&self.pixels);

        // On Windows GDI the uncovered area after another window moves away is
        // not necessarily part of the application's own damage rectangles.
        // Present the complete retained frame, even if dirty is empty.
        #[cfg(target_os = "windows")]
        let result = if dirty.is_empty() || self.fresh {
            // WM_PAINT may expose retained pixels without changing the scene.
            buffer.present()
        } else {
            // Keep partial updates for regular scene changes.
            let mut areas = dirty.to_vec();
            super::super::merge(&mut areas);
            let rects: Vec<softbuffer::Rect> = areas
                .iter()
                .filter_map(|a| Some(softbuffer::Rect {
                    x: a.x,
                    y: a.y,
                    width: NonZeroU32::new(a.w)?,
                    height: NonZeroU32::new(a.h)?,
                }))
                .collect();
            buffer.present_with_damage(&rects)
        };
        #[cfg(not(target_os = "windows"))]
        let result = {
            let mut areas = dirty.to_vec();
            super::super::merge(&mut areas);
            let rects: Vec<softbuffer::Rect> = areas
                .iter()
                .filter_map(|a| Some(softbuffer::Rect {
                    x: a.x,
                    y: a.y,
                    width: NonZeroU32::new(a.w)?,
                    height: NonZeroU32::new(a.h)?,
                }))
                .collect();
            buffer.present_with_damage(&rects)
        };
        match result {
            Ok(()) => self.fresh = false,
            Err(err) => {
                report!("software output present: {err}");
                self.fresh = true;
            }
        }
    }
}
