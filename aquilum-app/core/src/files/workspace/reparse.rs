use std::path::Path;

/// Treat every Windows reparse point as opaque: junctions can redirect a
/// directory scan outside its vault even if FileType::is_symlink is false.
#[cfg(windows)]
pub(super) fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
pub(super) fn is_reparse(_path: &Path) -> bool {
    false
}
