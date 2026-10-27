//! Bound bytes returned from an already opened MCP Vault note HANDLE/FD.
use std::fs::File;
use std::io::Read;

pub(super) const MAX_NOTE_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn read(mut file: File) -> Result<Vec<u8>, String> {
    read_limit(&mut file, MAX_NOTE_BYTES)
}

fn read_limit(file: &mut File, limit: u64) -> Result<Vec<u8>, String> {
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() {
        return Err("MCP note must be a regular file".to_owned());
    }
    if before.len() > limit {
        return Err(format!("MCP note exceeds {limit} byte limit"));
    }
    // The limit is enforced on the live handle, not only by metadata: a
    // concurrent process may append after the initial size preflight.
    let mut bytes = Vec::new();
    (&mut *file).take(limit.saturating_add(1)).read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("MCP note exceeds {limit} byte limit"));
    }
    let after = file.metadata().map_err(|error| error.to_string())?;
    if !after.is_file() || before.len() != after.len()
        || after.len() != bytes.len() as u64
        || before.modified().ok() != after.modified().ok()
    {
        return Err("MCP note changed during pinned read".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{read, read_limit, MAX_NOTE_BYTES};
    use std::fs::{self, File};

    #[test]
    fn accepts_exact_limit_and_denies_one_byte_above() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Заметка.md");
        fs::write(&path, b"123456789").unwrap();
        assert_eq!(read_limit(&mut File::open(&path).unwrap(), 9).unwrap(), b"123456789");
        assert!(read_limit(&mut File::open(&path).unwrap(), 8).is_err());
        assert!(read_limit(&mut File::open(&path).unwrap(), 0).is_err());
        assert_eq!(read(File::open(&path).unwrap()).unwrap(), b"123456789");
    }

    #[test]
    fn rejects_sparse_oversize_file_from_metadata_without_loading_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.md");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_NOTE_BYTES + 1).unwrap();
        drop(file);
        assert!(read(File::open(&path).unwrap()).is_err());
    }

    #[test]
    fn zero_length_regular_note_is_allowed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.md");
        fs::write(&path, b"").unwrap();
        assert_eq!(read(File::open(&path).unwrap()).unwrap(), Vec::<u8>::new());
    }
}
