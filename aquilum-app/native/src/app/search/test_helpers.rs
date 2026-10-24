use super::*;

impl SearchState {
    pub fn field_id(&self) -> WidgetId {
        self.field.id()
    }

    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    pub fn selected_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.results[self.selected].path)
    }

    pub fn selected_snippet(&self) -> &str {
        &self.results[self.selected].snippet
    }

    #[cfg(feature = "memory-probe")]
    pub fn generation(&self) -> u64 {
        self.worker.current()
    }
}

