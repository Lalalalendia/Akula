use super::*;
mod tick;
#[cfg(feature = "notion-offline-preview")]
mod notion;

impl App {
    pub fn title(&self) -> String {
        let product = if cfg!(any(target_os = "windows", target_os = "linux")) { "Akula" } else { "Aquilum" };
        match self.note.as_ref().map(|n| n.path.as_path()).or_else(|| self.tabs.active().path.as_deref()) {
            Some(path) => format!("{} — {product}", path.file_stem().unwrap_or_default().to_string_lossy()),
            None => product.to_owned(),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.note.as_ref().and_then(Note::deadline), self.view_due, self.hold].into_iter().flatten().min()
    }

    pub fn scale(&self) -> f64 {
        self.interface.scale()
    }

    pub fn zoom(&mut self, up: bool) -> bool {
        let step = if up { SCALE_STEP } else { -SCALE_STEP };
        self.interface.set_scale(self.interface.scale() + step)
    }

    pub fn shutdown(&mut self) {
        self.flush_view();
        match &mut self.note {
            Some(note) => note.flush(&self.workspace.core, &self.text),
            None => crate::disk::wait(),
        }
        self.workspace.core.shutdown();
    }

    pub fn hold_due(&mut self, now: Instant) -> bool {
        self.hold.is_some_and(|until| until <= now) && self.hold.take().is_some()
    }

    pub fn holding(&mut self) -> bool {
        let loading = self.embeds.as_ref().is_some_and(embeds::Embeds::busy);
        if !loading || self.hold.is_some_and(|until| until <= Instant::now()) {
            self.hold = None;
        }
        self.hold.is_some()
    }

    pub fn editor_id(&self) -> Option<WidgetId> {
        self.note_ids.as_ref().map(|ids| ids.body.id())
    }

    pub fn restore(&mut self, root: &mut RenderRoot) -> bool {
        self.show_active(root);
        self.note.is_some()
    }

    pub fn forget_session(&mut self) {
        self.session = None;
        self.tabs = Tabs::default();
    }
}
