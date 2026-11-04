//! Level-0 DOCX view: plain text only, never a writer or a Word layout engine.
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_XML: usize = 4 * 1024 * 1024;
const MAX_TEXT: usize = 512 * 1024;
const MAX_PARAGRAPHS: usize = 10_000;

const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORD_NS: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

pub(crate) fn is_docx_path(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
}

/// Reject links, oversized files and paths outside the active vault before opening a tab.
pub(crate) fn admit(vault: &Path, path: &Path) -> Result<PathBuf, String> {
    if !is_docx_path(path) {
        return Err("Ожидался файл DOCX".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !meta.file_type().is_file() {
        return Err("DOCX должен быть обычным файлом, а не ссылкой".into());
    }
    if meta.len() > MAX_FILE {
        return Err("Для простого просмотра DOCX должен быть не больше 8 МиБ".into());
    }
    let root = fs::canonicalize(vault).map_err(|error| error.to_string())?;
    let source = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !source.starts_with(&root) {
        return Err("DOCX находится вне открытой базы Akula".into());
    }
    Ok(source)
}

fn word(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == name
        && matches!(node.tag_name().namespace(), Some(WORD_NS | STRICT_WORD_NS))
}

fn plain_text(xml: &str) -> Result<String, String> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|error| format!("Не удалось разобрать текст DOCX: {error}"))?;
    let root = doc.root_element();
    if !word(root, "document") {
        return Err("DOCX не содержит документа Word".into());
    }
    let body = root.children().find(|child| word(*child, "body"))
        .ok_or_else(|| "DOCX не содержит основного текста".to_owned())?;
    if body.descendants().any(|node| {
        ["tbl", "drawing", "pict", "object"].iter().any(|tag| word(node, tag))
    }) {
        return Err("В этом DOCX есть таблицы или изображения. Пока поддерживается только обычный текст".into());
    }

    let mut text = String::new();
    let mut paragraphs = 0usize;
    for paragraph in body.descendants().filter(|node| word(*node, "p")) {
        if paragraphs == MAX_PARAGRAPHS {
            return Err("В DOCX слишком много абзацев для простого просмотра".into());
        }
        if paragraphs != 0 {
            text.push('\n');
        }
        paragraphs += 1;
        for node in paragraph.descendants() {
            if word(node, "t") {
                text.push_str(node.text().unwrap_or_default());
            } else if word(node, "tab") {
                text.push('\t');
            } else if word(node, "br") || word(node, "cr") {
                text.push('\n');
            }
            if text.len() > MAX_TEXT {
                return Err("Текст DOCX слишком большой для простого просмотра".into());
            }
        }
    }
    Ok(text)
}

/// The result is a derived, read-only string. The source DOCX is never rewritten.
pub(crate) fn read(vault: &Path, path: &Path) -> Result<String, String> {
    let source = admit(vault, path)?;
    let before = fs::metadata(&source).map_err(|error| error.to_string())?.len();
    let bytes = fs::read(&source).map_err(|error| error.to_string())?;
    if bytes.len() as u64 != before || before > MAX_FILE {
        return Err("DOCX изменился во время чтения".into());
    }
    let archive = crate::reader::zip::Zip::new(bytes)
        .ok_or_else(|| "DOCX повреждён: неверный ZIP-архив".to_owned())?;
    let data = archive.read_limited("word/document.xml", MAX_XML)
        .ok_or_else(|| "DOCX повреждён или содержит слишком большой документ Word".to_owned())?;
    let xml = std::str::from_utf8(&data)
        .map_err(|_| "DOCX содержит XML в неподдерживаемой кодировке".to_owned())?;
    plain_text(xml)
}

#[cfg(test)]
pub(crate) fn test_archive(xml: &str) -> Vec<u8> {
    // A minimal valid Open Packaging Conventions DOCX, using stored ZIP entries.
    fn u16le(out: &mut Vec<u8>, value: u16) { out.extend_from_slice(&value.to_le_bytes()); }
    fn u32le(out: &mut Vec<u8>, value: u32) { out.extend_from_slice(&value.to_le_bytes()); }
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
            }
        }
        !crc
    }
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
    let parts = [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", xml.as_bytes()),
    ];
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in parts {
        let name = name.as_bytes();
        let at = out.len() as u32;
        let size = data.len() as u32;
        let crc = crc32(data);
        u32le(&mut out, 0x0403_4b50);
        u16le(&mut out, 20);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, name.len() as u16);
        u16le(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        u32le(&mut directory, 0x0201_4b50);
        u16le(&mut directory, 20);
        u16le(&mut directory, 20);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u32le(&mut directory, crc);
        u32le(&mut directory, size);
        u32le(&mut directory, size);
        u16le(&mut directory, name.len() as u16);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u32le(&mut directory, 0);
        u32le(&mut directory, at);
        directory.extend_from_slice(name);
    }
    let offset = out.len() as u32;
    let size = directory.len() as u32;
    out.extend_from_slice(&directory);
    u32le(&mut out, 0x0605_4b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, 3);
    u16le(&mut out, 3);
    u32le(&mut out, size);
    u32le(&mut out, offset);
    u16le(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_russian_paragraphs_split_runs_and_soft_breaks() {
        let xml = format!(
            r#"<w:document xmlns:w="{WORD_NS}"><w:body>
            <w:p><w:r><w:t xml:space="preserve">Первая </w:t></w:r><w:r><w:t>строка</w:t></w:r></w:p>
            <w:p><w:r><w:t>Вторая &amp; третья</w:t><w:tab/><w:t>часть</w:t></w:r></w:p>
            <w:p/>
            <w:p><w:r><w:t>Конец</w:t><w:br/><w:t>строки</w:t></w:r></w:p>
            </w:body></w:document>"#
        );
        let vault = crate::test_dir::tempdir();
        let file = vault.path().join("Текст.DOCX");
        let original = test_archive(&xml);
        fs::write(&file, &original).unwrap();
        assert_eq!(read(vault.path(), &file).unwrap(),
            "Первая строка\nВторая & третья\tчасть\n\nКонец\nстроки");
        assert_eq!(fs::read(&file).unwrap(), original);
    }

    #[test]
    fn rejects_non_word_xml_and_unsupported_tables() {
        assert!(plain_text("<root/>").is_err());
        let table = format!(r#"<w:document xmlns:w="{WORD_NS}"><w:body>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>таблица</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            </w:body></w:document>"#);
        assert!(plain_text(&table).unwrap_err().contains("таблицы"));
    }

    #[test]
    fn rejects_large_xml_and_damaged_archives_without_writing() {
        let vault = crate::test_dir::tempdir();
        let damaged = vault.path().join("bad.docx");
        fs::write(&damaged, b"not a zip").unwrap();
        assert!(read(vault.path(), &damaged).is_err());
        assert_eq!(fs::read(&damaged).unwrap(), b"not a zip");

        let huge = vault.path().join("large.docx");
        fs::File::create(&huge).unwrap().set_len(MAX_FILE + 1).unwrap();
        assert!(admit(vault.path(), &huge).is_err());
    }

    #[test]
    fn refuses_outside_paths_and_symlinks() {
        let vault = crate::test_dir::tempdir();
        let outside = crate::test_dir::tempdir();
        let file = outside.path().join("outside.docx");
        fs::write(&file, test_archive(&format!(
            r#"<w:document xmlns:w="{WORD_NS}"><w:body><w:p/></w:body></w:document>"#
        ))).unwrap();
        assert!(admit(vault.path(), &file).is_err());
        #[cfg(unix)]
        {
            let link = vault.path().join("link.docx");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(admit(vault.path(), &link).is_err());
        }
    }
}