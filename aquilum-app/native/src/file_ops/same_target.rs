//! A rename to a differently spelled representation of the SAME filename
//! must not enter File Gate as a move. Preserve intentional case-only renames.
use std::path::Path;
use aquilum_core::search::paths::same_path;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn is_noop_rename(from: &Path, to: &Path) -> bool {
    if from == to {
        return true;
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
