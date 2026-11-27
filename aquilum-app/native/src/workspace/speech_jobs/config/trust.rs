use std::{fs::File, io::Read, path::Path};

pub(super) fn digest(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("expected a regular local file".into());
    }
    let mut reader = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = blake3::Hasher::new();
    let mut block = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut block).map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(hash.finalize().to_hex().to_string());
        }
        hash.update(&block[..count]);
    }
}
