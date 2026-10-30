//! A rename to a differently spelled representation of the SAME filename
//! must not enter File Gate as a move. Preserve intentional case-only renames.
use std::path::Path;
use aquilum_core::search::paths::same_path;

pub(crate) fn is_noop_rename(from: &Path, to: &Path) -> bool {
    if from == to {
        return true;
    }
    if !same_path(from, to) {
        return false;
    }
    let (Some(a), Some(b)) = (from.file_name(), to.file_name()) else {
        return false;
    };
    if a == b {
        return true; // Parent path spelling differs, but filename is unchanged.
    }
    // same_path uses NFC and, on Windows, case folding. Only a spelling /
    // normalization-only change is a no-op: case-only renames still run.
    a.to_string_lossy().to_lowercase() != b.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_filename_and_unicode_normalization_do_not_relocate_a_note() {
        let original = Path::new("vault/Мои\u{306} заметки.md");
        let composed = Path::new("vault/Мой заметки.md");
        assert_ne!(original, composed);
        assert!(same_path(original, composed));
        assert!(is_noop_rename(original, composed));
        assert!(is_noop_rename(original, original));
    }

    #[test]
    fn a_real_new_name_or_another_folder_is_not_an_alias_noop() {
        assert!(!is_noop_rename(Path::new("vault/a.md"), Path::new("vault/b.md")));
        assert!(!is_noop_rename(Path::new("vault/a.md"), Path::new("elsewhere/a.md")));
        assert!(!is_noop_rename(Path::new("vault/Note.md"), Path::new("vault/note.md")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_case_and_slash_alias_of_the_same_name_is_noop() {
        assert!(is_noop_rename(
            Path::new(r"D:\Книги\Без названия 3.md"),
            Path::new("d:/Книги/Без названия 3.md"),
        ));
        assert!(!is_noop_rename(
            Path::new(r"D:\Книги\Без названия 3.md"),
            Path::new(r"D:\Книги\БЕЗ НАЗВАНИЯ 3.md"),
        ));
    }
}
