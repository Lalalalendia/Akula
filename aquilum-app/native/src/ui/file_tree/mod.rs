mod actions;
mod build;
mod drag;
mod ghost;
#[cfg(all(windows, feature = "p2p-inbox-preview"))]
mod imported;
pub mod model;
mod parts;
mod pinned;
mod persistence;
mod pins;
mod row;
mod scan;
mod scroll_ops;
mod store;
#[cfg(test)]
mod testing;

use std::path::{Path, PathBuf};

use masonry::app::RenderRoot;
use masonry::core::WidgetId;
use masonry::widgets::Flex;

use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::scroll::ScrollArea;
use crate::ui::tokens::size;

use drag::Drag;
use model::TreeModel;
pub use model::{Press, Sort};
use parts::Toolbar;
use pinned::Pinned;
use store::ExpandedStore;

pub enum TreeAction {
    ContextMenu { targets: Vec<PathBuf>, clicked: PathBuf, folder: bool, at: masonry::kurbo::Point },
    Open { path: PathBuf, new_tab: bool },
    Rename { from: PathBuf, to: PathBuf },
    Move { sources: Vec<PathBuf>, target: PathBuf },
    CreateNote,
    NewNote { folder: PathBuf },
    NewFolder { folder: PathBuf },
    SortMenu { button: WidgetId },
}

pub struct FileTree {
    model: TreeModel,
    drag: Option<Drag>,
    create_button: Option<WidgetId>,
    row_ids: Vec<WidgetId>,
    scroll: Option<Handle<ScrollArea<Flex>>>,
    pinned: Option<Handle<Pinned>>,
    counting: scan::worker::CountScan,
    #[cfg(all(windows, feature = "p2p-inbox-preview"))]
    imported: imported::ImportedScan,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    rename: Option<Handle<TextEditor>>,
    store: ExpandedStore,
    first: usize,
    offset: f64,
    toolbar: Option<Toolbar>,
}

const SPAN: usize = 160;
const MARGIN: usize = 40;

fn step() -> f64 {
    size::SIDEBAR_ITEM_PADDING * 2.0 + size::SIDEBAR_ICON_SIZE + size::SPACE_2
}

impl FileTree {
    pub fn new(root: Option<PathBuf>, data_dir: &Path, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Self {
        let store = ExpandedStore::load(data_dir);
        let expanded = root.as_deref().map(|r| store.get(r)).unwrap_or_default();
        let sort = root.as_deref().map(|r| store.sort(r)).unwrap_or_default();
        let colors = root.as_deref().map(|r| store.colors(r)).unwrap_or_default();
        let mut model = TreeModel::with_saved_sort(root, expanded, sort);
        if !colors.is_empty() { model.set_colors(colors); }
        let mut tree = FileTree {
            model,
            drag: None,
            create_button: None,
            row_ids: Vec::new(),
            scroll: None,
            pinned: None,
            counting: scan::worker::CountScan::default(),
            #[cfg(all(windows, feature = "p2p-inbox-preview"))]
            imported: imported::ImportedScan::default(),
            wake: std::sync::Arc::clone(&wake),
            rename: None,
            store,
            first: 0,
            offset: 0.0,
            toolbar: None,
        };
        tree.count_notes(wake);
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        tree.request_imported();
        tree
    }

    pub fn root(&self) -> Option<&Path> {
        self.model.root()
    }

    pub fn reveal(&mut self, root: &mut RenderRoot, path: &Path) {
        self.model.expand_to(path);
        self.save_expanded();
        let index = self.model.index_of(path);
        self.show_row(root, index);
    }

    pub fn set_active(&mut self, root: &mut RenderRoot, path: Option<PathBuf>) {
        self.model.set_active(path);
        let index = self.model.rows().iter().position(|r| r.active);
        self.show_row(root, index);
    }

    pub fn clear_selection(&mut self, root: &mut RenderRoot) {
        self.model.clear_selection();
        self.refresh(root);
    }

    pub fn paths_changed(&mut self, root: &mut RenderRoot, paths: &[PathBuf]) {
        self.model.refresh_paths(paths);
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        if paths.iter().any(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
        }) {
            self.request_imported();
        }
        self.refresh(root);
    }

}
