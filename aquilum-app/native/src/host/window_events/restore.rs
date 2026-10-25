use super::*;

impl Host {
    /// Win32 WM_PAINT must restore pixels uncovered by a different window.
    pub(super) fn expose_redraw(&mut self) {
        #[cfg(target_os = "windows")]
        { self.full_next = true; }
        self.redraw();
    }

    pub(super) fn expose_focus(&mut self, focused: bool) {
        self.text_event(TextEvent::WindowFocusChange(focused));
        #[cfg(target_os = "windows")]
        if focused {
            self.full_next = true;
            self.request_redraw();
        }
    }
}
