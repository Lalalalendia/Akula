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
