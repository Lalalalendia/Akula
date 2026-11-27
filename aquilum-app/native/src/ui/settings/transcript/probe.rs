use super::*;

impl Editor {
    pub(crate) fn buttons_for_test(&self, button: Button) -> Option<WidgetId> {
        self.buttons
            .iter()
            .find_map(|(&id, &action)| (action == button).then_some(id))
    }

    pub(crate) fn page_for_test(&self) -> Option<&edits::Page> {
        self.page.as_ref()
    }

    pub(crate) fn input_for_test(&self) -> Option<Handle<TextEditor>> {
        self.input
    }

    #[cfg(all(target_os = "linux", feature = "memory-probe"))]
    pub(crate) fn playback_for_test(&self) -> (Option<u64>, f64) {
        let playback = self.playback.borrow();
        (
            playback.frame().map(|frame| frame.image.data.id()),
            playback.position(),
        )
    }
}
