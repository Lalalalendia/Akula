//! Native application path for a PDF stored inside the vault.
//! No external script, Python or shell is required by the Akula user.
use super::*;
use masonry::kurbo::Point;
use std::fs;

#[test]
fn pdf_tree_menu_opens_trust_confirmation_without_modifying_user_content() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let existing = vault.join("a.md");
    assert!(h.app.open(&mut h.root, existing.clone()));
    let original = fs::read(&existing).unwrap();
    let pdf = vault.join("contract.PDF");
    let bytes = b"%PDF-1.4\nuser-trusted fixture data";
    fs::write(&pdf, bytes).unwrap();

    h.app.open_file_menu(
        &mut h.root,
        vec![pdf.clone()],
        pdf.clone(),
        false,
        Point::ORIGIN,
        masonry::kurbo::Size::new(1000.0, 600.0),
        None,
    );
    let layer = match &h.app.popup {
        Some(Popup::FileMenu { layer, .. }) => *layer,
        _ => panic!("Expected the real native PDF context menu"),
    };
    h.send(layer, crate::ui::menu::MenuChoice::Item(3));
    let dialog = match &h.app.popup {
        Some(Popup::PdfImport { dialog, source }) => {
            assert_eq!(*source, pdf);
            dialog.layer
        }
        _ => panic!("Expected consent dialog rather than auto-running an untrusted PDF"),
    };
    h.send(dialog, crate::ui::dialog::DialogAnswer::Cancel);
    assert!(h.app.popup.is_none());
    assert_eq!(fs::read(&existing).unwrap(), original);
    assert_eq!(fs::read(&pdf).unwrap(), bytes);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(existing.as_path()));
}

#[test]
fn selected_external_pdf_copies_via_existing_core_file_gate() {
    let h = Harness::new();
    let external = crate::test_dir::tempdir();
    let pdf = external.path().join("shared.pdf");
    let data = b"%PDF-1.4\nThis is a test PDF for File Gate";
    fs::write(&pdf, data).unwrap();
    let imported = crate::app::pdf_preview::admit_pdf_source(
        &h.app.workspace.core, h.vault.path(), &pdf,
    ).expect("the user's file should be imported with File Gate");
    assert!(imported.starts_with(h.vault.path()));
    assert_eq!(fs::read(&imported).unwrap(), data);
    assert!(fs::read(&pdf).unwrap() == data);
}

#[test]
fn converter_error_is_a_real_visible_native_dialog() {
    let mut h = Harness::new();
    h.app.show_pdf_notice(&mut h.root, "Нет текстового слоя PDF");
    let dialog = match &h.app.popup {
        Some(Popup::PdfNotice { dialog }) => dialog.layer,
        _ => panic!("No error dialog was shown to the user"),
    };
    h.send(dialog, crate::ui::dialog::DialogAnswer::Cancel);
    assert!(h.app.popup.is_none());
}


/// Tests the app's production PDF->File Gate->Tantivy->native search result
/// against the installed, SHA-pinned Shiva executable on disposable Windows.
#[test]
#[ignore = "Windows installed-beta acceptance with pinned Shiva sidecar"]
fn installed_shiva_pdf_is_searchable_and_opens_from_native_result() {
    use std::path::PathBuf;
    use std::time::Duration;
    let sidecar = PathBuf::from(
        std::env::var_os("AKULA_PDF_INSTALLED_SIDECAR")
            .expect("installed sidecar path required"),
    );
    let fixture = PathBuf::from(
        std::env::var_os("AKULA_PDF_FIXTURE_PATH")
            .expect("synthetic PDF fixture required"),
    );
    assert!(sidecar.is_file() && fixture.is_file());
    let mut h = Harness::new();
    let original = fs::read(&fixture).unwrap();
    let source = h.vault.path().join("CI trusted PDF.pdf");
    fs::write(&source, &original).unwrap();
    let vault = h.vault.path().to_path_buf();
    let note = crate::app::pdf_preview::import_trusted_with_converter(
        &h.app.workspace.core, &vault, &source, &sidecar,
    ).expect("Akula must create an indexed PDF preview from installed Shiva");
    let contents = fs::read_to_string(&note).unwrap();
    assert!(contents.contains("AKULANATIVEPDFNEEDLE"), "source text not indexed");
    assert!(contents.contains("Needs review"), "not allowed to mark PDF complete");
    assert_eq!(fs::read(&source).unwrap(), original, "user source was modified");

    let duplicate = crate::app::pdf_preview::import_trusted_with_converter(
        &h.app.workspace.core, &vault, &source, &sidecar,
    ).expect("repeated import must be idempotent");
    assert_eq!(duplicate, note);
    assert_eq!(fs::read_to_string(&note).unwrap(), contents);

    // The same native App search popup and result click as the real user.
    h.app.workspace.core.search.prepare(&vault.to_string_lossy())
        .expect("search service must initialize");
    assert!(h.app.open(&mut h.root, note.clone()), "preview must open as a native tab");
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(note.as_path()));
    // Do not leave the PDF preview active: otherwise Enter could be a no-op
    // and this test would falsely pass without opening the search result.
    let sentinel = vault.join("a.md");
    assert!(h.app.open(&mut h.root, sentinel.clone()),
        "a different existing note must open before exercising result selection");
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(sentinel.as_path()));
    h.app.open_search(&mut h.root);
    let field = match &h.app.popup {
        Some(Popup::Search(state)) => state.field_id(),
        _ => panic!("expected native search popup"),
    };
    h.send(
        field,
        crate::ui::editor::TextAction::Changed("AKULANATIVEPDFNEEDLE".to_owned()),
    );
    let mut result = None;
    for _ in 0..600 {
        h.app.on_core_events(&mut h.root);
        h.app.on_search_results(&mut h.root);
        if let Some(Popup::Search(state)) = &h.app.popup {
            if state.result_count() > 0 {
                result = Some(state.selected_path());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let found_path = result.expect("native search did not return the PDF-derived note");
    // File Gate may use extended-length Windows paths while Tantivy stores
    // ordinary drive paths. Compare their resolved file identities.
    let expected = fs::canonicalize(&note)
        .expect("the File Gate-created PDF preview must still exist");
    assert_eq!(
        fs::canonicalize(&found_path)
            .expect("the search result must resolve to an existing file"),
        expected,
        "native search returned a different file than the imported PDF preview",
    );
    h.app.search_key(
        &mut h.root,
        &Key::Named(NamedKey::Enter),
        Modifiers::empty(),
    ).expect("native search Enter key must be recognized");
    let opened = h.app.tabs.active().path
        .as_ref().expect("native search Enter must open a tab");
    assert_eq!(
        fs::canonicalize(opened)
            .expect("opened PDF preview tab must resolve to an existing file"),
        expected,
        "native Enter opened a file other than the PDF preview search result",
    );
    assert!(!matches!(h.app.popup, Some(Popup::Search(_))),
        "opening the search result must close the search modal");
    println!("AKULA_PDF_NATIVE_SEARCH_E2E=PASS;installed_shiva=1;gate=1;tantivy=1;ui=1;source_unchanged=1");
}

#[test]
fn pdf_tree_secondary_click_reaches_native_preview_menu() {
    let mut h = Harness::new();
    let pdf = h.vault.path().join("right-click.PDF");
    fs::write(&pdf, b"%PDF-1.4\ntrusted PDF fixture").unwrap();
    h.app.tree.paths_changed(&mut h.root, std::slice::from_ref(&pdf));
    h.settle();
    let row = h.app.tree.row_id("right-click.PDF")
        .expect("PDF must be visible as a file-tree row");
    let at = h.root.get_widget(row).unwrap().ctx().bounding_box().center();
    let state = PointerState {
        position: PhysicalPosition::new(at.x, at.y),
        count: 1,
        scale_factor: 1.0,
        ..PointerState::default()
    };
    h.root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
        pointer: MOUSE,
        button: Some(PointerButton::Secondary),
        state,
    }));
    h.deliver();
    let layer = match &h.app.popup {
        Some(Popup::FileMenu { layer, clicked, .. }) => {
            assert_eq!(clicked, &pdf);
            *layer
        }
        _ => panic!("PDF secondary click did not reach the native file context menu"),
    };
    h.send(layer, crate::ui::menu::MenuChoice::Item(3));
    assert!(matches!(h.app.popup, Some(Popup::PdfImport { .. })),
        "PDF context menu must offer opt-in native text extraction");
}
