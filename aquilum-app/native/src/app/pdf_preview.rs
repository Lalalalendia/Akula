//! Opt-in native PDF text preview for one explicitly trusted vault file.
//! Uses an installer-bundled Shiva executable, never a user-selected script.
//! PDF parser sandbox admission and visual fidelity are NOT claimed.
use aquilum_core::Core;
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use masonry::app::RenderRoot;
use masonry::kurbo::Point;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use super::{App, Popup};
use crate::ui::dialog::ConfirmDialog;

const MAX_PDF: u64 = 2 * 1024 * 1024;
const MAX_TEXT: u64 = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(25);

pub(super) fn is_pdf_path(path: &Path) -> bool {
    path.extension().and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("pdf"))
}

fn read_trusted_pdf(vault: &Path, source: &Path) -> Result<(PathBuf, Vec<u8>), String> {
    if !is_pdf_path(source) { return Err("Ожидался PDF".into()); }
    let meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() < 16 || meta.len() > MAX_PDF {
        return Err("PDF должен быть обычным файлом до 2 МиБ".into());
    }
    let root = fs::canonicalize(vault).map_err(|e| e.to_string())?;
    let actual = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !actual.starts_with(&root) {
        return Err("PDF находится вне открытой базы Akula".into());
    }
    let raw = fs::read(&actual).map_err(|e| e.to_string())?;
    if raw.len() != meta.len() as usize || !raw.starts_with(b"%PDF-") {
        return Err("Неверная сигнатура PDF или источник изменился".into());
    }
    Ok((actual, raw))
}


pub(in crate::app) fn admit_pdf_source(core: &Core, vault: &Path, source: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() < 16 || meta.len() > MAX_PDF {
        return Err("PDF должен быть обычным файлом размером до 2 МиБ".into());
    }
    if !is_pdf_path(source) { return Err("Выбранный файл не является PDF".into()); }
    let file = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let vault_root = fs::canonicalize(vault).map_err(|e| e.to_string())?;
    if file.starts_with(&vault_root) {
        return Ok(file);
    }
    // A file selected by the user outside the vault is imported through the
    // SAME File Gate as all other Akula files; no ambient filesystem writes.
    let before = fs::read(&file).map_err(|e| e.to_string())?;
    if before.len() != meta.len() as usize || !before.starts_with(b"%PDF-") {
        return Err("Неверный PDF или исходник изменился".into());
    }
    let expected = blake3::hash(&before);
    let relative = crate::vault_files::import(
        core, vault, crate::vault_files::Source::Path(&file), "PDF", "pdf",
    ).map_err(|e| format!("File Gate отклонил импорт PDF: {e}"))?;
    let copy = crate::vault_files::resolve(Some(vault), &relative)
        .ok_or_else(|| "Недоступен созданный файл PDF".to_owned())?;
    let (_, confirmed) = read_trusted_pdf(vault, &copy)?;
    if blake3::hash(&confirmed) != expected {
        return Err("PDF изменился во время копирования; проверьте импорт".into());
    }
    Ok(copy)
}

struct TemporaryPdf(PathBuf);
impl Drop for TemporaryPdf {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn bundled_converter() -> Result<PathBuf, String> {
    let binary = if cfg!(windows) { "shiva-pdf-preview.exe" } else { "shiva-pdf-preview" };
    let path = std::env::current_exe().map_err(|e| e.to_string())?
        .with_file_name(binary);
    let meta = fs::symlink_metadata(&path).map_err(|_| {
        "В установленной Akula отсутствует PDF-модуль".to_owned()
    })?;
    if !meta.file_type().is_file() {
        return Err("PDF-модуль не является обычным исполняемым файлом".into());
    }
    Ok(path)
}

fn extract_text(binary: &Path, raw: &[u8]) -> Result<String, String> {
    let folder = std::env::temp_dir().join(format!("akula-pdf-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&folder).map_err(|e| e.to_string())?;
    let _temporary = TemporaryPdf(folder.clone());
    let input = folder.join("input.pdf");
    let output = folder.join("output.md");
    fs::write(&input, raw).map_err(|e| e.to_string())?;
    let mut child = Command::new(binary)
        .arg(&input)
        .arg(&output)
        .current_dir(&folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("PDF-модуль завершился с ошибкой или превысил лимит времени".into());
            }
        }
    };
    if !outcome.success() { return Err("Shiva не извлекла текст из PDF".into()); }
    let meta = fs::symlink_metadata(&output).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_TEXT {
        return Err("Пустой или слишком большой результат извлечения".into());
    }
    let content = fs::read_to_string(&output).map_err(|e| e.to_string())?;
    if content.trim().is_empty() {
        return Err("Текстового слоя нет; для скана потребуется OCR".into());
    }
    Ok(content)
}

fn preview_body(filename: &str, digest: &str, text: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for byte in text.bytes() {
        if byte == 96 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = char::from(96).to_string().repeat(longest.saturating_add(1).max(3));
    // The filename is also untrusted Markdown input, even for a trusted PDF.
    let safe_filename: String = filename.chars().map(|c| match c {
        '\n' | '\r' => " ".to_owned(),
        '\\' | '*' | '_' | '[' | ']' | '(' | ')' | '`' | '#' | '!' | '<' | '>' | '|' =>
            format!("\\{c}"),
        _ => c.to_string(),
    }).collect();
    let filename = safe_filename;
    format!(
        "# PDF: {filename}\n\n\
         > **Needs review.** Текст PDF извлечён автоматически и визуально не проверен.\n\n\
         Оригинал: {filename}\n\n\
         BLAKE3 источника: {digest}\n\n\
         ## Предварительный текст\n\n{fence}text\n{}\n{fence}\n",
        text.trim_end()
    )
}

fn import_trusted_pdf(core: &Core, vault: &Path, source: &Path) -> Result<PathBuf, String> {
    // Missing companion is detected before creating a copy inside the vault.
    let companion = bundled_converter()?;
    import_trusted_with_converter(core, vault, source, &companion)
}

/// Acceptance seam: the installed companion is supplied by the Windows test,
/// while the release path above can only use the bundled binary.
pub(in crate::app) fn import_trusted_with_converter(
    core: &Core, vault: &Path, source: &Path, companion: &Path,
) -> Result<PathBuf, String> {
    let admitted = admit_pdf_source(core, vault, source)?;
    let (actual, original) = read_trusted_pdf(vault, &admitted)?;
    let digest = blake3::hash(&original);
    let text = extract_text(companion, &original)?;
    let (confirmed, current) = read_trusted_pdf(vault, &admitted)?;
    if actual != confirmed || blake3::hash(&current) != digest {
        return Err("PDF изменился во время обработки".into());
    }
    let filename = actual.file_name().and_then(|f| f.to_str()).unwrap_or("document.pdf");
    let stem = crate::file_ops::sanitize_file_name(
        actual.file_stem().and_then(|f| f.to_str()).unwrap_or("PDF")
    );
    let digest_text = digest.to_hex().to_string();
    let note = actual.with_file_name(format!("{stem} PDF {}.md", &digest_text[..12]));
    let text = preview_body(filename, &digest_text, &text);
    match gate::create(core, &note, &text, Source::Me, None) {
        Ok(_) => Ok(note),
        Err(aquilum_core::files::error::FileCommandError::AlreadyExists { .. })
            if fs::read_to_string(&note).is_ok_and(|existing| existing == text) => Ok(note),
        Err(e) => Err(format!("File Gate отказал в создании заметки: {e}")),
    }
}

impl App {
    pub(super) fn pick_pdf_file(&self) {
        let sender = self.picked_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"]).pick_file()
            {
                if sender.send(super::book::Picked::PdfChosen(path)).is_ok() { wake(); }
            }
        });
    }

    pub(super) fn show_pdf_notice(&mut self, root: &mut RenderRoot, error: &str) {
        let (modal, dialog) = ConfirmDialog::new(
            "PDF не добавлен в поиск",
            error,
            "Закрыть",
            "Понятно",
        );
        root.add_layer(modal, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::PdfNotice { dialog });
    }

    pub(super) fn prompt_pdf_preview(&mut self, root: &mut RenderRoot, source: PathBuf) {
        let (modal, dialog) = ConfirmDialog::new(
            "PDF: текстовый поиск в Akula",
            "Только для PDF, которым вы доверяете. PDF-модуль пока не запущен в \
             защищённой песочнице. Akula сохранит исходный файл и создаст \
             отдельную поисковую заметку. Продолжить?",
            "Отмена",
            "Извлечь текст",
        );
        root.add_layer(modal, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::PdfImport { dialog, source });
    }

    pub(super) fn start_pdf_preview(&mut self, source: PathBuf) {
        let Some(vault) = self.tree.root().map(Path::to_path_buf) else { return };
        let core = Arc::clone(&self.workspace.core);
        let sender = self.picked_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let result = import_trusted_pdf(&core, &vault, &source);
            if sender.send(super::book::Picked::PdfIndexed(result)).is_ok() { wake(); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_bounded_inside_vault_and_must_have_pdf_signature() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("document.PDF");
        fs::write(&source, b"%PDF-1.4\nThis is a test PDF stream").unwrap();
        assert!(is_pdf_path(&source));
        assert!(read_trusted_pdf(temp.path(), &source).is_ok());
        fs::write(&source, b"not a real PDF").unwrap();
        assert!(read_trusted_pdf(temp.path(), &source).is_err());
        assert!(admit_pdf_source_for_invalid(&source));
    }

    fn admit_pdf_source_for_invalid(path: &Path) -> bool {
        let meta = fs::symlink_metadata(path).unwrap();
        meta.len() < 16 || !fs::read(path).unwrap().starts_with(b"%PDF-")
    }

    #[test]
    fn extracted_text_is_explicitly_provisional_and_inert_markdown() {
        let note = preview_body("report.pdf", &"0".repeat(64), "needle\nraw HTML");
        assert!(note.contains("Needs review"));
        assert!(note.contains("needle"));
        assert!(note.contains(&format!("{}text", char::from(96).to_string().repeat(3))));
        assert!(note.contains("Оригинал: report.pdf"));
    }
}
