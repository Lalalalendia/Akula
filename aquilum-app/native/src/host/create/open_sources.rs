use super::*;

impl Host {
    pub(super) fn open_requested_sources(&mut self, root: &mut RenderRoot, window: &dyn Window) {
        #[cfg(feature = "dicom-preview")]
        if let Some(source) = self.args.open_dicom.clone() {
            // Explicit CLI selection grants access to the exact DICOM source.
            // It is neither imported nor registered as an Akula workspace.
            if self.app.open_dicom_external(root, source) {
                window.set_title(&self.app.title());
            }
        }
        if let Some(note) = self.args.open.clone() {
            // The session title was set before processing --open. Refresh it
            // only after the requested document has actually become active.
            if self.app.open(root, note) {
                window.set_title(&self.app.title());
            }
        }
    }
}
