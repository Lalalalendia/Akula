use super::*;

impl Editor {
    pub(super) fn capture_draft(&mut self, root: &RenderRoot) {
        if let (Some((_, draft)), Some(input)) = (
            self.edit.as_mut(),
            self.input.and_then(|input| input.get(root)),
        ) {
            *draft = input.inner().text();
        }
    }

    pub(super) fn play(&mut self, button: Button) {
        let Some(path) = self.media.as_ref().and_then(Transcript::current_path) else {
            self.media = None;
            *self.playback.borrow_mut() = Playback::default();
            self.error = Some(label("transcriptMediaChanged"));
            return;
        };
        let mut preview = self.playback.borrow_mut();
        if !preview.open(path) {
            self.error = Some(label("transcriptNoPlayer"));
            return;
        }
        match button {
            Button::Jump(ms) => {
                preview.seek(ms);
                if let Some(state) = crate::video::state(path).filter(|state| state.duration > 0.0)
                {
                    crate::video::seek(path, ms as f64 / 1000.0 / state.duration);
                }
            }
            Button::Toggle => preview.toggle(),
            _ => {}
        }
    }
}
