use crate::search::paths::{is_hidden_name, is_markdown};
use super::attachments::MEDIA_EXTENSIONS;
use super::error::FileCommandError;
use super::models::{FileItem, FileItemType};
use std::fs;
use std::path::Path;

// Structured formats are listed like attachments until native dedicated views exist.
// Never treat them as Markdown notes or send their content to the text editor.
const DOCUMENT_EXTENSIONS: &[&str] = &["pdf", "epub", "mobi", "azw3", "fb2", "txt", "canvas", "base", "docx"];

fn is_attachment(extension: &str) -> bool {
    MEDIA_EXTENSIONS.iter().chain(DOCUMENT_EXTENSIONS).any(|known| known.eq_ignore_ascii_case(extension))
}

pub fn read_directory_impl(path: &Path) -> Result<Vec<FileItem>, FileCommandError> {
    let mut items = Vec::new();
    for entry in fs::read_dir(path)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Ok(file_type) = entry.file_type() else { continue };
        // Only internal dot names are hidden; user .hidden.md and .заметки stay visible.
        if file_type.is_symlink() || name.as_encoded_bytes().first() == Some(&b'.') && is_hidden_name(&name) {
            continue;
        }
        let folder = file_type.is_dir();
        let markdown = if folder {
            false
        } else {
            if !file_type.is_file() { continue; }
            let Some(extension) = Path::new(&name).extension().and_then(|ext| ext.to_str()) else { continue };
            if !extension.eq_ignore_ascii_case("md") && !is_attachment(extension) { continue; }
            extension.eq_ignore_ascii_case("md")
        };
        // Do not allocate a path, a display name or an ID for rejected files.
        let path = entry.path();
        let id = path.to_string_lossy().into_owned();
        let name = if markdown {
            path.file_stem().map(|v| v.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string_lossy().into_owned())
        } else {
            name.to_string_lossy().into_owned()
        };
        items.push(FileItem {
            id,
            name,
            item_type: if folder { FileItemType::Folder } else { FileItemType::File },
        });
    }
    items.sort_by_cached_key(|item| {
        (item.item_type == FileItemType::File, item.name.to_lowercase())
    });
    Ok(items)
}

pub fn existing_files_impl(paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|value| {
            let path = Path::new(value);
            path.is_file() && is_markdown(path)
        })
        .collect()
}
