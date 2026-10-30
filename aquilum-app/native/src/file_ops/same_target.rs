//! Suppress normalization-only filename changes in an unchanged parent.
//! Different parent spellings always require File Gate admission.
use std::path::Path;
use aquilum_core::search::paths::same_path;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn is_noop_rename(from: &Path, to: &Path) -> bool {
    if from.as_os_str() == to.as_os_str() {
        return true;
    }
    // same_path NFC-normalizes parent components and folds their case on
    // Windows/macOS. Two such paths may still be different real directories,
    // especially on Linux or on Windows with case-sensitive folders. Never
    // silently swallow a parent change as a filename-only no-op.
    if from.parent().map(Path::as_os_str) != to.parent().map(Path::as_os_str) {
        return false;
    }
    if !same_path(from, to) {
        return false;
    }
    // Never equate two OS paths through to_string_lossy(): distinct invalid
    // byte sequences can otherwise collapse to the same replacement character.
    if from.to_str().is_none() || to.to_str().is_none() {
        return false;
    }
    let (Some(a), Some(b)) = (
        from.file_name().and_then(|name| name.to_str()),
        to.file_name().and_then(|name| name.to_str()),
    ) else {
        return false;
    };
    // NFC preserves case. same_path also folds case on Windows/macOS, so
    // comparing lowercased raw spellings would wrongly hide an intentional
    // case-only rename combined with a Unicode normalization difference.
    a.nfc().eq(b.nfc())
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


    #[test]
    fn normalized_parent_aliases_remain_real_moves_for_file_gate() {
        let left = Path::new("vault/Мои\u{306}/Note.md");
        let right = Path::new("vault/Мой/Note.md");
        assert!(same_path(left, right));
        assert!(!is_noop_rename(left, right));
        assert!(!is_noop_rename(right, left));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn case_change_with_different_unicode_normalization_is_not_noop() {
        let original = Path::new("vault/É.md");
        let changed = Path::new("vault/e\u{301}.md");
        assert!(same_path(original, changed));
        assert!(!is_noop_rename(original, changed));
        assert!(!is_noop_rename(changed, original));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_parent_aliases_are_not_assumed_identical() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;

        let left = PathBuf::from(OsString::from_vec(b"vault/\xff/Note.md".to_vec()));
        let right = PathBuf::from(OsString::from_vec(b"vault/\xfe/Note.md".to_vec()));
        assert!(!is_noop_rename(&left, &right));
    }

    #[cfg(windows)]
    #[test]
    fn windows_parent_aliases_require_gate_admission() {
        let source = Path::new(r"D:\Книги\Без названия 3.md");
        let parent_alias = Path::new("d:/Книги/Без названия 3.md");
        // Search identity ignores drive case and separator spelling. That
        // alone does not authorize treating a relocation as a UI no-op.
        assert!(same_path(source, parent_alias));
        assert!(!is_noop_rename(source, parent_alias));
        assert!(!is_noop_rename(
            source,
            Path::new(r"D:\Книги\БЕЗ НАЗВАНИЯ 3.md"),
        ));
        assert!(!is_noop_rename(
            Path::new(r"D:\Книги\Folder\Note.md"),
            Path::new(r"D:\Книги\folder\Note.md"),
        ));
    }
}
