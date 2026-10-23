use super::preview_dimensions_allowed;

#[test]
fn native_pixel_budget_is_separate_from_compressed_byte_budget() {
    assert!(preview_dimensions_allowed((6_000, 4_000)));
    assert!(preview_dimensions_allowed((1, 24_000_000)));
    assert!(!preview_dimensions_allowed((6_001, 4_000)));
    assert!(!preview_dimensions_allowed((65_000, 65_000)));
    assert!(!preview_dimensions_allowed((0, 1)));
    assert!(!preview_dimensions_allowed((1, 0)));
}

#[test]
fn standalone_image_tab_is_bounded_to_its_vault() {
    let root = tempfile::tempdir().unwrap();
    let vault = root.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let inside = vault.join("verification-desktop.png");
    let outside = root.path().join("outside.png");
    let fixture = image::RgbImage::new(1, 1);
    fixture.save(&inside).unwrap();
    fixture.save(&outside).unwrap();
    assert!(super::local_image_preview(&vault, &inside).is_some());
    assert!(super::local_image_preview(&vault, &outside).is_none());
}

#[test]
fn standalone_image_extensions_have_normal_tab_identity() {
    for filename in ["verification-desktop.png", "cover.jpg", "scan.webp"] {
        assert_eq!(crate::images::kind(filename), Some(crate::images::MediaKind::Image));
    }
    let mut tabs = crate::tabs::Tabs::default();
    tabs.open(std::path::PathBuf::from("README.md"));
    tabs.open_new(std::path::PathBuf::from("verification-desktop.png"));
    assert_eq!(tabs.tabs().len(), 2);
    assert_eq!(tabs.active().title().as_deref(), Some("verification-desktop"));
}
