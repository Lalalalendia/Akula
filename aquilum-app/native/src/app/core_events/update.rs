use super::super::*;

impl App {
    pub(in crate::app) fn on_update(&mut self, root: &mut RenderRoot) {
        use crate::updater::Status;
        let status = self.updater.status();
        if status != self.update_shown {
            self.update_shown = status.clone();
            let splash = match &status {
                Status::Downloading { done, total, .. } => Some((false, total.filter(|t| *t > 0).map(|t| *done as f64 * 100.0 / t as f64))),
                Status::Ready(_) | Status::Installing => Some((true, None)),
                _ => None,
            };
            match (splash, self.update_layer) {
                (Some((installing, percent)), Some(layer)) => {
                    root.edit_widget(layer, |mut w| crate::ui::update::UpdateSplash::set(&mut w.downcast(), installing, percent));
                }
                (Some((installing, percent)), None) => {
                    let widget = crate::ui::update::UpdateSplash::new(installing, percent);
                    self.update_layer = Some(widget.id());
                    root.add_layer(widget, Point::ORIGIN);
                }
                (None, Some(layer)) => {
                    root.remove_layer(layer);
                    self.update_layer = None;
                }
                (None, None) => {}
            }
            if let Some(Popup::Settings { dialog, .. }) = &self.popup
                && dialog.section == crate::ui::settings::schema::Section::System
            {
                self.refresh_settings(root);
            }
        }
        if let Some(payload) = self.updater.take_ready(&self.workspace.data_dir) {
            let _ = root.redraw();
            if let Err(error) = crate::updater::apply(&payload, || self.shutdown()) {
                report!("обновление не установлено: {error}");
                self.updater.fail(error);
            }
        }
    }
}
