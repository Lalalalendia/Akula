use std::fs::File;
use std::io::Read;
use std::path::Path;

use aquilum_core::files::document::normalize_line_endings_in_place as normalize;
use aquilum_core::files::error::FileCommandError;

const BLOCK: usize = 8 << 20;

const HASH_BUFFER: usize = 64 << 10;

/// Hash on-disk bytes without decoding Markdown or building a second Rope.
/// The BLAKE3 digest intentionally covers raw CRLF bytes, like `read()`.
pub fn hash_raw(path: &Path) -> Result<String, FileCommandError> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; HASH_BUFFER];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub struct Loaded {
    pub text: crop::Rope,
    pub hash: String,
}

fn fill(file: &mut File, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    let start = buf.len();
    buf.resize(start + BLOCK, 0);
    let mut got = 0;
    while got < BLOCK {
        match file.read(&mut buf[start + got..])? {
            0 => break,
            n => got += n,
        }
    }
    buf.truncate(start + got);
    Ok(got)
}

pub fn read(path: &Path) -> Result<Loaded, FileCommandError> {
    let mut file = File::open(path)?;
    let (mut hasher, mut builder, mut buf) = (blake3::Hasher::new(), crop::RopeBuilder::new(), Vec::with_capacity(BLOCK + 8));
    loop {
        let start = buf.len();
        let eof = fill(&mut file, &mut buf)? < BLOCK;
        hasher.update_rayon(&buf[start..]);
        let held = usize::from(!eof && buf.last() == Some(&b'\r'));
        let len = buf.len() - held;
        let kept = normalize(&mut buf[..len]);
        buf.copy_within(len.., kept);
        buf.truncate(kept + held);
        let invalid = |error: std::str::Utf8Error| FileCommandError::InvalidUtf8 { message: error.to_string() };
        let valid = match std::str::from_utf8(&buf[..kept]) {
            Ok(text) => {
                builder.append(text);
                kept
            }
            Err(error) if error.error_len().is_none() && !eof => {
                builder.append(std::str::from_utf8(&buf[..error.valid_up_to()]).map_err(invalid)?);
                error.valid_up_to()
            }
            Err(error) => return Err(invalid(error)),
        };
        buf.drain(..valid);
        if eof {
            break;
        }
    }
    Ok(Loaded { text: builder.build(), hash: hasher.finalize().to_hex().to_string() })
}

#[cfg(test)]
mod tests {
    use super::{hash_raw, read};

    #[test]
    fn a_file_reads_into_the_same_text_and_hash_as_before() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("a.md");
        let body = "строка\r\n".repeat(700_000) + "конец\rхвост";
        std::fs::write(&path, &body).unwrap();
        let loaded = read(&path).unwrap();
        let snapshot = aquilum_core::files::document::read_file_snapshot_impl(&path).unwrap();
        assert_eq!(loaded.text.to_string(), snapshot.content);
        assert_eq!(loaded.hash, snapshot.hash);
    }

    #[test]
    fn streamed_raw_hash_matches_full_loader_across_chunks_and_crlf() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("raw.md");
        let body = "строка 🌐\r\n".repeat(40_000) + "финал\rхвост";
        std::fs::write(&path, &body).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(hash_raw(&path).unwrap(), loaded.hash);
        assert_eq!(hash_raw(&path).unwrap(), aquilum_core::files::document::hash_bytes(body.as_bytes()));
        assert!(loaded.text.to_string().contains("строка 🌐\n"));
    }
}
