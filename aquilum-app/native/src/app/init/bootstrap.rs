use super::super::*;
impl App {
    pub fn new(workspace: Workspace, initial_text: String) -> Self {
        let tree = FileTree::new(workspace.root.clone(), &workspace.data_dir, Arc::clone(&workspace.wake));
        let interface = Interface::load(&workspace.data_dir);
        let navigation = crate::navigation::Navigation::load(&workspace.data_dir, workspace.root.as_deref());
        let (session, tabs) = match workspace.root.as_deref().and_then(|root| Session::open(&workspace.core, root)) {
            Some((session, tabs)) => (Some(session), tabs),
            None => (None, Tabs::default()),
        };
        let (panel_tx, panel_rx) = channel();
        let (search_tx, search_rx) = channel();
        let (picked_tx, picked_rx) = channel();
        let (transfer_tx, transfer_rx) = channel();
        let (home_tx, home_rx) = channel();
        let (titles_tx, titles_rx) = channel();
        let workspace_wake = Arc::clone(&workspace.wake);
        let auto_update = workspace.core.settings.get_config().updates.auto;
        let mut yandex_probe = crate::yandex_probe_session::ProbeSession::new(Arc::clone(&workspace_wake));
        yandex_probe.set_bucket(&workspace.core.settings.get_config().s3_onboarding.bucket);
        let mut app = App {
            workspace,
            #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview: dropbox_preview::DropboxPreview::new(),
            tree,
            #[cfg(feature = "dicom-preview")] dicom_external_sources: HashSet::new(),
            chrome: None,
            interface,
            popup: None,
            typing: None,
            server_probe: settings::server_state::ServerProbeState::default(),
            graph: graph_page::GraphState::default(), canvas: canvas::CanvasState::default(),
            transfer: transfer::TransferState::default(),
            suggest_layer: None,
            find: None,
            note: None,
            recovery: note_recovery::RecoveryUi::default(),
            text: crop::Rope::from(initial_text),
            inactive_notes: Vec::new(),
            #[cfg(test)] parley_trial_enabled: false,
            #[cfg(test)]
            parley_capture: None,
            #[cfg(test)]
            parley_offer: None,
            system_dark: false,
            tabs,
            renaming_tab: None,
            rename_field: None,
            session,
            maximized: false,
            listing: None,
            panel_key: None,
            panel_generation: 0,
            panel_tx,
            panel_rx,
            panel_body: None,
            retention_done: Arc::default(),
            yandex_probe,
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            s3_pair: crate::s3_pair_session::PairSession::new(Arc::clone(&workspace_wake)),
            controls: None,
            window: Size::ZERO,
            resizing: None,
            history: None,
            history_body: None,
            opened_versions: HashMap::new(),
            version_view: None,
            version_notice: None,
            search_tx,
            search_rx,
            vault_error: None,
            note_ids: None,
            hidden: 0,
            view: None,
            view_due: None,
            restore: None,
            embeds: None,
            navigation,
            renaming_to: None,
            hero_shown: None,
            picked_tx,
            failed_renames: channel(),
            picked_rx,
            transfer_tx,
            transfer_rx,
            home: None,
            home_tx,
            home_rx,
            home_generation: 0,
            shown: None,
            titles_tx,
            titles_rx,
            hold: None,
            updater: crate::updater::Updater::new(Arc::clone(&workspace_wake)),
            update_shown: crate::updater::Status::Idle,
            update_layer: None,
            open_error: None,
            open_error_ids: None,
            save_error_ids: None,
            pending_reveal: None,
        };
        crate::cover::set_waker(Arc::clone(&app.workspace.wake));
        app.start_index();
        app.workspace.core.apply_mcp_settings();
        if auto_update {
            app.updater.install();
        }
        let remote = app.workspace.data_dir.join("remote");
        std::thread::spawn(move || embeds::prune_remote(&remote));
        app.refresh_home();
        app.cleanup_retention();
        app
    }
}
