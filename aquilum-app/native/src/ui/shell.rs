mod chrome;
pub use chrome::Chrome;
use masonry::core::{NewWidget, Widget};
use masonry::properties::types::CrossAxisAlignment;
use masonry::widgets::{Flex, IndexedStack, SizedBox};

use super::handle::Handle;
use super::header::titlebar;
use super::new_tab::new_tab_view;
use super::rail::rail;
use super::sidebar::sidebar;
use super::widgets::{Collapse, Slot};
use super::{backlinks, style, workspaces};

pub struct RootParts<'a> {
    pub workspace_name: &'a str,
    pub note: NewWidget<dyn Widget>,
    pub sidebar_open: bool,
    pub sidebar_from: bool,
    pub sidebar_width: f64,
    pub panel_width: f64,
    pub tabs: Vec<NewWidget<dyn Widget>>,
    pub active_tab: usize,
    pub empty_tab: bool,
    pub vault_open: bool,
    pub vault_error: Option<&'a str>,
    pub panel_open: bool,
    pub panel_from: bool,
    pub panel_mode: crate::links::Mode,
    pub has_analysis: bool,
    pub focus_mode: bool,
    pub home: bool,
    pub graph: Option<NewWidget<dyn Widget>>,
    pub graph_active: bool,
}

pub fn root(tree: NewWidget<impl Widget>, parts: RootParts<'_>) -> (NewWidget<impl Widget>, Chrome) {
    let RootParts { workspace_name, note, sidebar_open, sidebar_from, sidebar_width, panel_width, tabs, active_tab, empty_tab, vault_open, vault_error, panel_open, panel_from, panel_mode, has_analysis, focus_mode, home, graph, graph_active } = parts;

    let rail = rail(sidebar_open, home, focus_mode);
    let sidebar = sidebar(tree, workspace_name, sidebar_width, sidebar_open && !focus_mode, sidebar_from);

    let note_page = Slot::new(note);
    let note_handle = Handle::of(&note_page);
    let titlebar = titlebar(tabs, active_tab, panel_open, panel_from, focus_mode);
    let focus_hidden = vec![titlebar.toggle_shown];
    let (panel, panel_handle, panel_parts) = backlinks::panel(panel_mode, has_analysis, panel_width, panel_open, panel_from);
    let (empty, create_note, open_file, close_tab, dicom_new_tab_folder, dicom_new_tab_file) = new_tab_view();
    let version_page = Slot::new(NewWidget::new(Flex::column()).erased());
    let version_handle = Handle::of(&version_page);
    let (no_vault, open_vault, dicom_no_vault_folder, dicom_no_vault_file) = workspaces::no_vault(vault_error);
    let active = if !vault_open {
        3
    } else if graph_active {
        4
    } else {
        usize::from(empty_tab)
    };
    let graph_page = Slot::new(graph.unwrap_or_else(|| NewWidget::new(SizedBox::empty()).erased()));
    let graph_handle = Handle::of(&graph_page);
    let pages = IndexedStack::new().with(note_page).with(empty).with(version_page).with(no_vault).with(graph_page).with_active_child(active);
    let pages = NewWidget::new(pages);
    let pages_handle = Handle::of(&pages);
    let main = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(titlebar.widget)
        .with(pages, 1.0);

    let window = Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(rail.widget)
        .with_fixed(sidebar.widget)
        .with(NewWidget::new(main).with_class(style::CANVAS), 1.0)
        .with_fixed(panel);
    let chrome = Chrome {
        toggle_sidebar: rail.toggle_sidebar,
        settings: sidebar.settings,
        new_tab: titlebar.new_tab,
        tab_list: titlebar.tab_list,
        tab_strip: titlebar.strip,
        panel_toggle: titlebar.toggle,
        controls_inset: titlebar.inset,
        panel: panel_handle,
        panel_parts,
        pages: pages_handle,
        version_page: version_handle,
        create_note,
        open_file,
        close_tab,
        workspaces: sidebar.workspaces,
        open_vault,
        dicom_no_vault_folder,
        dicom_no_vault_file,
        dicom_new_tab_folder,
        dicom_new_tab_file,
        sidebar: sidebar.handle,
        sidebar_sash: sidebar.sash,
        note_page: note_handle,
        rail: rail.handle,
        rail_settings: titlebar.settings,
        search_button: rail.search_button,
        home_slot: rail.home_slot,
        home_button: rail.home_button,
        graph_page: graph_handle,
        graph_button: rail.graph_button,
        transfer_button: rail.transfer_button,
        focus_hidden,
        focus_shown: titlebar.settings_shown,
    };
    (NewWidget::new(window), chrome)
}
