//! A checked, pinned file handle for duplicate-source reads.
//! This narrows (but does not eliminate) path races with external writers.
use super::{fingerprint, EntryKind, Fingerprint};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

pub(super) fn read_checked(path: &Path, expected: &Fingerprint) -> io::Result<File> {
    if expected.kind != EntryKind::File || fingerprint(&fs::symlink_metadata(path)?)? != *expected {
        return Err(io::Error::other("source changed before opening"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the final entry itself: do not follow an NTFS symlink/reparse point.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        // Existing writers or handles allowing replacement block this copy.
        // Keep the source object stable while its bytes are read.
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        options.share_mode(FILE_SHARE_READ);
    }
    let handle = options.open(path)?;
    // Validate the *opened object*, not only the pathname tested earlier.
    if fingerprint(&handle.metadata()?)? != *expected {
        return Err(io::Error::other("source changed or became linked while opening"));
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn checked_open_denies_external_write_until_handle_closes() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("source.md");
        fs::write(&source, "original").unwrap();
        let expected = fingerprint(&fs::symlink_metadata(&source).unwrap()).unwrap();
        let guarded = read_checked(&source, &expected).unwrap();
        assert!(
            OpenOptions::new().write(true).open(&source).is_err(),
            "Windows must reject concurrent writers while a source handle is pinned",
        );
        drop(guarded);
        assert!(
            OpenOptions::new().write(true).open(&source).is_ok(),
            "writing must be possible after the checked handle closes",
        );
    }

    #[test]
    fn checked_open_rejects_source_replaced_with_different_size() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("source.md");
        fs::write(&source, "original").unwrap();
        let expected = fingerprint(&fs::symlink_metadata(&source).unwrap()).unwrap();
        fs::write(&source, "replaced with a different size").unwrap();
        assert!(read_checked(&source, &expected).is_err());
    }
}
