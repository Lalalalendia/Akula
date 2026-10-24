use super::*;
use std::path::Path;

fn click_file(h: &mut Harness, path: &Path) {
    h.app.tree.paths_changed(&mut h.root, &[path.to_path_buf()]);
    h.settle();
    let name = path.file_name().unwrap().to_str().unwrap();
    let row = h.app.tree.row_id(name).expect("изображение должно быть видно в дереве");
    let at = h.root.get_widget(row).unwrap().ctx().bounding_box().center();
    h.click(at.x, at.y);
}

#[test]
fn png_jpeg_and_webp_open_from_tree_as_readonly_document_pages() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let note = vault.join("a.md");
    assert!(h.app.open(&mut h.root, note.clone()));
    let active = h.app.tabs.active_id();
    let initial_text = h.body_text();

    for (name, format) in [
        ("Фотография.PnG", image::ImageFormat::Png),
        ("снимок.JPEG", image::ImageFormat::Jpeg),
        ("рисунок.webp", image::ImageFormat::WebP),
    ] {
        let path = vault.join(name);
        if format == image::ImageFormat::Jpeg {
            image::RgbImage::from_pixel(24, 12, image::Rgb([20, 80, 190]))
                .save_with_format(&path, format).unwrap();
        } else {
            image::RgbaImage::from_pixel(24, 12, image::Rgba([20, 80, 190, 255]))
                .save_with_format(&path, format).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        click_file(&mut h, &path);
        assert!(h.app.popup.is_none(), "фото не должно перекрывать README: {name}");
        assert_eq!(h.app.tabs.active().path.as_deref(), Some(path.as_path()));
        assert!(h.app.note.is_none(), "фотография не открывается в редакторе");
        assert!(h.app.image_tab_widget().is_some());
        assert_eq!(std::fs::read(&path).unwrap(), before, "просмотр изменил исходник");
        assert!(h.app.open(&mut h.root, note.clone()));
        assert_eq!(h.app.tabs.active_id(), active);
        assert_eq!(h.body_text(), initial_text);
    }
    let photo = vault.join("Фотография.PnG");
    assert!(h.app.open_tree_file(&mut h.root, photo.clone(), true));
    assert_eq!(h.app.tabs.tabs().len(), 2);
    assert_eq!(h.app.tabs.tabs()[0].path.as_deref(), Some(note.as_path()));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(photo.as_path()));
}

#[test]
fn corrupt_oversized_and_unsupported_files_do_not_open_or_modify_the_note() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let note = vault.join("a.md");
    assert!(h.app.open(&mut h.root, note.clone()));
    let active = h.app.tabs.active_id();
    let original_text = h.body_text();

    let corrupt = vault.join("повреждено.png");
    std::fs::write(&corrupt, b"not-an-image").unwrap();
    click_file(&mut h, &corrupt);
    assert!(h.app.popup.is_none(), "повреждённое изображение открылось");

    let large = vault.join("слишком-большое.jpg");
    std::fs::File::create(&large).unwrap().set_len(33 * 1024 * 1024).unwrap();
    click_file(&mut h, &large);
    assert!(h.app.popup.is_none(), "превышен лимит 32 МиБ");

    // Tiny compressed PNG claiming huge dimensions must be rejected.
    let bomb = vault.join("небольшой-файл-огромное-изображение.png");
    let mut header = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut header, 65_000, 65_000);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        // An IDAT chunk lets the dimension check execute before decoding.
        writer.write_chunk(png::chunk::IDAT, &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]).unwrap();
        writer.finish().unwrap();
    }
    assert!(header.len() < 1024, "test must remain a tiny compressed input");
    let dimensions = image::ImageReader::new(std::io::Cursor::new(&header))
        .with_guessed_format().unwrap().into_dimensions().unwrap();
    assert_eq!(dimensions, (65_000, 65_000));
    std::fs::write(&bomb, &header).unwrap();
    click_file(&mut h, &bomb);
    assert!(h.app.popup.is_none(), "huge native dimensions must be rejected");
    assert_eq!(std::fs::read(&bomb).unwrap(), header, "rejected source must not change");

    let unsupported = vault.join("вектор.svg");
    std::fs::write(&unsupported, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    click_file(&mut h, &unsupported);
    assert!(h.app.popup.is_none(), "SVG пока не поддерживается просмотрщиком");

    assert_eq!(h.app.tabs.active_id(), active);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(note.as_path()));
    assert_eq!(h.body_text(), original_text);
    assert_eq!(std::fs::read(&corrupt).unwrap(), b"not-an-image");
}

#[test]
fn tree_image_preview_refuses_files_outside_the_open_vault() {
    let mut h = Harness::new();
    let outside = tempfile::tempdir().unwrap();
    let picture = outside.path().join("private.png");
    image::RgbaImage::new(8, 8).save(&picture).unwrap();

    let active = h.app.tabs.active_id();
    assert!(!h.app.open_tree_file(&mut h.root, picture, false));
    assert_eq!(h.app.tabs.active_id(), active);
    assert!(h.app.popup.is_none(), "нельзя смотреть за пределами рабочей папки");

    #[cfg(unix)]
    {
        let link = h.vault.path().join("внешняя.png");
        std::os::unix::fs::symlink(outside.path().join("private.png"), &link).unwrap();
        assert!(!h.app.open_tree_file(&mut h.root, link, false));
        assert!(h.app.popup.is_none(), "ссылка не должна обходить рабочую папку");
    }
}
