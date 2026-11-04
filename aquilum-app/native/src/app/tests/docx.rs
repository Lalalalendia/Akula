use super::*;

#[test]
fn simple_docx_opens_in_a_read_only_scrollable_tab_without_rewriting_source() {
    let mut h = Harness::new();
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
        <w:p><w:r><w:t>Простой документ</w:t></w:r></w:p>
        <w:p><w:r><w:t>Вторая строка</w:t></w:r></w:p>
    </w:body></w:document>"#;
    let path = h.vault.path().join("пример.DOCX");
    let bytes = crate::docx::test_archive(xml);
    std::fs::write(&path, &bytes).unwrap();
    h.app.tree.paths_changed(&mut h.root, std::slice::from_ref(&path));
    assert!(h.app.tree.names().iter().any(|name| name == "пример.DOCX"));
    assert!(h.app.open(&mut h.root, path.clone()));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(path.as_path()));
    assert!(h.app.note.is_none(), "DOCX cannot acquire a note write path");
    assert!(h.app.note_ids.is_none(), "DOCX cannot acquire an editor");
    let _ = h.root.redraw();
    assert!(h.app.open(&mut h.root, h.vault.path().join("a.md")));
    assert!(h.app.open(&mut h.root, path.clone()));
    assert_eq!(crate::docx::read(h.vault.path(), &path).unwrap(),
        "Простой документ\nВторая строка");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn corrupt_docx_shows_an_error_page_without_crashing_or_writing() {
    let mut h = Harness::new();
    let path = h.vault.path().join("bad.docx");
    std::fs::write(&path, b"not a Word file").unwrap();
    assert!(h.app.open(&mut h.root, path.clone()));
    assert!(h.app.note.is_none());
    assert!(h.app.open_error.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"not a Word file");
}
