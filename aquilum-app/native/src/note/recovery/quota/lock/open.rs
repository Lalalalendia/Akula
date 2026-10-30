//! Shared validated persistent lock-file open for disk and UI paths.
use std::{fs::{File, OpenOptions}, io, path::Path};

use super::{FILENAME, validate};
use super::super::super::invalid;

pub(super) fn validated(bucket: &Path) -> io::Result<File> {
    let root = bucket.parent().ok_or_else(|| invalid("missing recovery root"))?;
    let path = root.join(FILENAME);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate(&path)?;
            OpenOptions::new().read(true).write(true).open(&path)?
        }
        Err(error) => return Err(error),
    };
    validate(&path)?;
    #[cfg(target_os = "windows")]
    super::super::super::acl::restrict(&path)?;
    Ok(file)
}
