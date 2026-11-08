//! Legacy Windows text files must be viewable without changing their bytes.
//! Editable notes still require valid UTF-8; transcoding silently on save
//! would destroy the original encoding and is deliberately not permitted.
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use aquilum_core::files::error::FileCommandError;
use masonry::core::NewWidget;
use masonry::properties::types::CrossAxisAlignment;
use masonry::widgets::Flex;

use super::*;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::{Font, Label, Leading};

const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

// Legacy preview must not display arbitrary binary control bytes as text.
// Preserve the whitespace that occurs in ordinary Windows TXT documents.
fn binary_controls(text: &str) -> bool {
    text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n' | '\u{000c}'))
}

fn utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("Неполный символ UTF-16 в TXT".into());
    }
    let words = bytes.chunks_exact(2).map(|pair| {
        if little_endian { u16::from_le_bytes([pair[0], pair[1]]) }
        else { u16::from_be_bytes([pair[0], pair[1]]) }
    }).collect::<Vec<_>>();
    let text = String::from_utf16(&words)
        .map_err(|_| "Повреждённая последовательность UTF-16".to_owned())?;
    if binary_controls(&text) {
        return Err("TXT содержит управляющие символы UTF-16: возможно, это двоичный файл".into());
    }
    Ok(text)
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetACP() -> u32;
    fn MultiByteToWideChar(page: u32, flags: u32, source: *const u8, size: i32, target: *mut u16, capacity: i32) -> i32;
}

#[cfg(target_os = "windows")]
fn ansi(bytes: &[u8]) -> Result<(String, String), String> {
    // GetACP and MultiByteToWideChar exist on Windows 7. The user's active
    // ANSI page may be 1251, 1252, 932, etc.; do not assume it is Cyrillic.
    let page = unsafe { GetACP() };
    let size = i32::try_from(bytes.len()).map_err(|_| "Слишком большой ANSI TXT".to_owned())?;
    let count = unsafe { MultiByteToWideChar(page, 0x00000008, bytes.as_ptr(), size, std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(format!("ANSI (кодовая страница Windows {page}) не декодирован"));
    }
    let mut decoded = vec![0u16; count as usize];
    let written = unsafe { MultiByteToWideChar(page, 0x00000008, bytes.as_ptr(), size, decoded.as_mut_ptr(), count) };
    if written != count {
        return Err("TXT изменился при декодировании ANSI".to_owned());
    }
    let text = String::from_utf16(&decoded).map_err(|_| "Недопустимые символы ANSI".to_owned())?;
    if binary_controls(&text) {
        return Err("TXT содержит управляющие символы ANSI: возможно, это двоичный файл".into());
    }
    Ok((text, format!("Windows ANSI ({page})")))
}

fn decode(bytes: &[u8]) -> Result<(String, String), String> {
    if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return utf16(body, true).map(|text| (text, "UTF-16 LE".to_owned()));
    }
    if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return utf16(body, false).map(|text| (text, "UTF-16 BE".to_owned()));
    }
    // An explicit UTF-8 BOM is not a license to reinterpret malformed UTF-8
    // as the system ANSI code page: that would silently misrepresent bytes.
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err("Повреждённый UTF-8 TXT с BOM: ANSI-просмотр запрещён".into());
    }
    // A BOM-less UTF-16 or arbitrary binary file is ambiguous. Never feed
    // embedded NULs to a legacy ANSI decoder and display arbitrary garbage.
    if bytes.contains(&0) {
        return Err("TXT содержит нулевые байты без BOM UTF-16; кодировка неизвестна".into());
    }
    #[cfg(target_os = "windows")]
    { ansi(bytes) }
    #[cfg(not(target_os = "windows"))]
    { Err("Кодировка TXT неизвестна: файл не является UTF-8/UTF-16".into()) }
}

fn read_legacy(vault: &Path, path: &Path) -> Result<(String, String), String> {
    let canonical_vault = fs::canonicalize(vault).map_err(|error| error.to_string())?;
    let canonical_path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical_path.starts_with(&canonical_vault) {
        return Err("TXT находится за пределами хранилища".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("TXT не является обычным файлом".into());
    }
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err("Старый TXT больше 2 МиБ; просмотр без преобразования ограничен".into());
    }
    let mut bytes = Vec::new();
    File::open(path).map_err(|error| error.to_string())?
        .take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err("TXT вырос во время чтения, превышен лимит 2 МиБ".into());
    }
    decode(&bytes)
}

impl App {
    pub(super) fn show_txt_or_open_error(&mut self, root: &mut RenderRoot, path: &Path, error: FileCommandError) {
        let reason = if is_plain_text(path) && matches!(&error, FileCommandError::InvalidUtf8 { .. }) {
            match self.show_legacy_txt(root, path) {
                Ok(()) => return,
                Err(legacy) => format!("{error}; {legacy}"),
            }
        } else {
            error.to_string()
        };
        report!("заметка {} не прочитана: {reason}", path.display());
        self.open_error = Some((path.to_path_buf(), reason.clone()));
        self.show_open_error(root, path, &reason);
    }

    fn show_legacy_txt(&mut self, root: &mut RenderRoot, path: &Path) -> Result<(), String> {
        let vault = self.tree.root().ok_or_else(|| "Хранилище Akula не выбрано".to_owned())?;
        let (body, encoding) = read_legacy(vault, path)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let heading = NewWidget::new(crate::ui::widgets::Caption::new(&format!(
            "{name} — {encoding}, только чтение (исходный файл не изменён)"
        )));
        let body = NewWidget::new(Label::new(&body, |theme| theme.text_primary)
            .font(Font::Editor).leading(Leading::Editor).selectable());
        let column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(heading).with_fixed(body);
        let page = NewWidget::new(ScrollArea::new(NewWidget::new(column)));
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
        self.text = crop::Rope::new();
        if let Some(chrome) = &self.chrome {
            chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
        }
        report!("TXT {} открыт только для чтения ({encoding})", path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_bom_is_decoded_without_losing_unicode() {
        let original = "Привет Windows 7 🐈\r\nстрока";
        let mut little = vec![0xff, 0xfe];
        let mut big = vec![0xfe, 0xff];
        for word in original.encode_utf16() {
            little.extend_from_slice(&word.to_le_bytes());
            big.extend_from_slice(&word.to_be_bytes());
        }
        assert_eq!(decode(&little).unwrap().0, original);
        assert_eq!(decode(&big).unwrap().0, original);
    }

    #[test]
    fn reject_truncated_utf16_and_binary_without_bom() {
        assert!(decode(&[0xff, 0xfe, 0x10]).is_err());
        assert!(decode(&[0xff, 0xfe, 0x00, 0xd8]).is_err());
        assert!(decode(&[0x41, 0, 0x42, 0]).is_err());
    }

    #[test]
    fn nul_containing_utf16_and_corrupt_utf8_bom_fail_closed() {
        assert!(decode(&[0xff, 0xfe, 0, 0]).is_err());
        assert!(decode(&[0xfe, 0xff, 0, 0]).is_err());
        assert!(decode(&[0xff, 0xfe, 0x41, 0, 0, 0]).is_err());
        assert!(decode(&[0xef, 0xbb, 0xbf, 0xff]).is_err());
    }

    #[test]
    fn utf16_binary_controls_are_rejected_but_text_whitespace_survives() {
        assert!(decode(&[0xff, 0xfe, 0x01, 0x00]).is_err());
        assert!(decode(&[0xfe, 0xff, 0x00, 0x1b]).is_err());
        let original = "строка\r\nс табуляцией\tи формой\u{000c}";
        let mut bytes = vec![0xff, 0xfe];
        for word in original.encode_utf16() {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(decode(&bytes).unwrap().0, original);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn ansi_binary_controls_fail_closed_without_rejecting_lines_and_tabs() {
        assert!(decode(b"header\x01payload").is_err());
        assert!(decode(b"header\x1b[0m").is_err());
        assert_eq!(decode(b"line\r\ncolumn\tvalue").unwrap().0, "line\r\ncolumn\tvalue");
    }

    #[test]
    fn preview_does_not_write_or_follow_out_of_vault_paths() {
        let vault = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let path = vault.path().join("старый.TXT");
        let bytes = vec![0xff, 0xfe, 0x1f, 0x04, 0x40, 0x04];
        fs::write(&path, &bytes).unwrap();
        let (text, kind) = read_legacy(vault.path(), &path).unwrap();
        assert_eq!(text, "Пр");
        assert_eq!(kind, "UTF-16 LE");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(read_legacy(external.path(), &path).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows7_active_ansi_codepage_can_decode_a_legacy_txt() {
        let (decoded, kind) = decode(b"ASCII\r\n").unwrap();
        assert_eq!(decoded, "ASCII\r\n");
        assert!(kind.starts_with("Windows ANSI ("));
        if unsafe { GetACP() } == 1251 {
            assert_eq!(decode(&[0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2]).unwrap().0, "Привет");
        }
    }
}
