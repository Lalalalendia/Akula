use super::*;

impl Host {
    /// Win32 WM_PAINT must restore pixels uncovered by a different window.
    pub(super) fn expose_redraw(&mut self) {
        #[cfg(target_os = "windows")]
        { self.full_next = true; }
        self.redraw();
        // RedrawRequested is an OS expose, not necessarily a scene mutation.
        // A Win7 GDI surface must re-present retained pixels even if the
        // renderer reports only partial damage after another window moves.
        #[cfg(target_os = "windows")]
        if let Some(output) = self.output.as_mut() {
            output.restore_exposed_pixels();
        }
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
