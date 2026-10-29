use std::path::Path;

/// Only opt-in P2P may observe content changes of deterministic QA Vault copies.
/// This is an invalidation event, not indexing or File Gate authorization.
#[cfg(feature = "p2p-inbox-lab")]
pub(super) fn is_qa_import_copy(path: &Path) -> bool {
    path.file_name()
        .and_then(|file| file.to_str())
        .and_then(|file| file.strip_prefix("received-"))
        .and_then(|file| file.strip_suffix(".bin"))
        .is_some_and(|identity| uuid::Uuid::parse_str(identity).is_ok())
}

#[cfg(not(feature = "p2p-inbox-lab"))]
pub(super) fn is_qa_import_copy(_path: &Path) -> bool {
    false
}

