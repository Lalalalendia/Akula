mod actions;
mod commands;
mod book; mod canvas;
mod core_events; #[cfg(feature = "dropbox-readonly-preview")] mod dropbox_preview;
mod embeds;
mod file_menu;
mod find;
mod editor_requests;
mod graph_page;
mod history;
mod home;
mod init;
mod lifecycle;
mod media;
mod pdf_preview;
mod note_page;
mod panel;
#[cfg(feature = "memory-probe")] mod probe;
mod reader;
mod root_ui;
mod search;
mod settings;
mod tabs;
mod templates;
mod transfer;
mod widths;
mod workspaces;
#[cfg(test)] mod tests;
use std::collections::HashMap;
#[cfg(feature = "dicom-preview")] use std::collections::HashSet;
use crate::ui::editor::TextEditor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;
use aquilum_core::CoreEvent;
use aquilum_core::mcp::bridge::{Disposition, Navigation};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, NewWidget, Widget, WidgetId};
use masonry::kurbo::{Point, Size};
use crate::ui::editor::TextAction;
use crate::i18n::{self, t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::file_tree::{FileTree, TreeAction};
use crate::ui::menu::{Menu, MenuChoice};
use crate::ui::theme::{self, UiFont};
use crate::ui::settings::SettingsDialog;
use crate::ui::handle::Handle;
use crate::ui::titlebar::TitlebarAction;
use crate::ui::widgets::{Pressed, Slot};
use crate::ui::{self, Chrome, ROOT_TAG};
use crate::file_ops;
use crate::interface::{Interface, SCALE_STEP};
use crate::links::Listing;
use crate::ui::backlinks::{Body, HistoryBody, VersionView};
use crate::ui::titlebar::WindowButton;
use crate::{note::Note, session::Session};
use crate::tabs::Tabs;
use crate::workspace::Workspace;
use uuid::Uuid;
enum Popup {
    FileMenu { layer: WidgetId, targets: Vec<PathBuf>, clicked: PathBuf, tab: Option<Uuid> },
    Delete { dialog: DialogIds, targets: Vec<PathBuf> },
    Settings { dialog: SettingsDialog, menu: Option<settings::OpenMenu> },
    VersionMenu { layer: WidgetId, index: usize, commands: Vec<history::VersionCommand> },
    TableMenu { layer: WidgetId, commands: Vec<crate::ui::editor::TableCommand> },
    EditorMenu { layer: WidgetId, commands: Vec<editor_requests::EditorCommand> },
    NoteActionsMenu { layer: WidgetId },
    SortMenu { layer: WidgetId, button: WidgetId },
    TabsMenu { layer: WidgetId, button: WidgetId },
    Viewer { layer: WidgetId },
    PdfImport { dialog: DialogIds, source: PathBuf },
    PdfNotice { dialog: DialogIds },
    Reader(Box<reader::ReaderState>),
    Search(search::SearchState),
    Templates(templates::TemplatesState),
    Workspaces(workspaces::WorkspacesState),
    Transfers(crate::ui::transfer::TransferDialog),
}
#[derive(Default)]
pub struct Outcome {
    pub title: bool,
    pub rescale: bool,
    pub window: Option<WindowCommand>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowCommand {
    Drag,
    ToggleMaximize,
    Minimize,
    Close,
}
const MENU_RENAME: usize = 0;
const MENU_DUPLICATE: usize = 1;
const MENU_DELETE: usize = 2;
const MENU_CLOSE: usize = 3;
const MENU_COLOR: usize = 3;
const MENU_MARGIN: f64 = 8.0;
const HOLD: std::time::Duration = std::time::Duration::from_millis(500);
pub struct App {
    workspace: Workspace,
    #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview: dropbox_preview::DropboxPreview,
    tree: FileTree,
    #[cfg(feature = "dicom-preview")] dicom_external_sources: HashSet<PathBuf>,
    chrome: Option<Chrome>,
    interface: Interface,
    popup: Option<Popup>,
    typing: Option<(crate::ui::settings::schema::Setter, String, usize)>,
    server_probe: settings::server_state::ServerProbeState,
    graph: graph_page::GraphState, canvas: canvas::CanvasState,
    transfer: transfer::TransferState,
    suggest_layer: Option<(WidgetId, WidgetId)>,
    find: Option<crate::ui::find::FindBar>,
    note: Option<Note>,
    text: crop::Rope,
    // At most two clean, hash-verified Rope snapshots from inactive tabs.
    inactive_notes: Vec<tabs::open::CachedNote>,
    #[cfg(test)]
    pub(crate) parley_trial_enabled: bool,
    #[cfg(test)]
    parley_capture: Option<crate::ui::editor::LayoutTrialSnapshot>,
    #[cfg(test)]
    parley_offer: Option<crate::ui::editor::LayoutTrialSnapshot>,
    system_dark: bool,
    tabs: Tabs,
    renaming_tab: Option<Uuid>,
    rename_field: Option<Handle<TextEditor>>,
    session: Option<Session>,
    maximized: bool,
    listing: Option<Listing>,
    panel_key: Option<(PathBuf, crate::links::Mode, crate::links::Method)>,
    panel_generation: u64,
    panel_tx: Sender<Listing>,
    panel_rx: Receiver<Listing>,
    panel_body: Option<Body>,
    retention_done: Arc<std::sync::atomic::AtomicBool>,
    yandex_probe: crate::yandex_probe_session::ProbeSession,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    s3_pair: crate::s3_pair_session::PairSession,
    controls: Option<(WidgetId, Handle<WindowButton>)>,
    window: Size,
    resizing: Option<(crate::interface::Side, f64, f64)>,
    history: Option<history::HistoryState>,
    history_body: Option<HistoryBody>,
    opened_versions: HashMap<Uuid, crate::history::Version>,
    version_view: Option<VersionView>,
    version_notice: Option<String>,
    search_tx: Sender<search::Found>,
    search_rx: Receiver<search::Found>,
    vault_error: Option<String>,
    note_ids: Option<crate::ui::note::NoteIds>,
    hidden: usize,
    view: Option<(usize, f64)>,
    view_due: Option<Instant>,
    restore: Option<(usize, f64)>,
    embeds: Option<embeds::Embeds>,
    navigation: crate::navigation::Navigation,
    renaming_to: Option<PathBuf>,
    hero_shown: Option<Vec<String>>,
    picked_tx: Sender<book::Picked>,
    failed_renames: (Sender<(PathBuf, PathBuf)>, Receiver<(PathBuf, PathBuf)>),
    picked_rx: Receiver<book::Picked>,
    transfer_tx: Sender<Vec<PathBuf>>,
    transfer_rx: Receiver<Vec<PathBuf>>,
    home: Option<PathBuf>,
    home_tx: Sender<(u64, Option<PathBuf>)>,
    home_rx: Receiver<(u64, Option<PathBuf>)>,
    home_generation: u64,
    shown: Option<(bool, bool)>,
    titles_tx: Sender<(WidgetId, String, String)>,
    titles_rx: Receiver<(WidgetId, String, String)>,
    hold: Option<Instant>,
    updater: crate::updater::Updater,
    update_shown: crate::updater::Status,
    update_layer: Option<WidgetId>,
    pub open_error: Option<(PathBuf, String)>,
    pub open_error_ids: Option<crate::ui::note::OpenErrorIds>,
    pub save_error_ids: Option<crate::ui::note::SaveErrorIds>,
    pub pending_reveal: Option<usize>,
}
