use super::error::FileCommandError;
use super::models::{FileSnapshot, FileWriteResult};
use atomic_write_file::AtomicWriteFile;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

const HASH_CHUNK: usize = 64 << 10;

/// Hash file bytes incrementally; do not allocate a full-file Vec merely to
/// check a revision or verify a conflict before an atomic write.
fn hash_reader(mut reader: impl Read) -> Result<String, FileCommandError> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; HASH_CHUNK];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn read_hash_if_exists(path: &Path) -> Result<Option<String>, FileCommandError> {
    match fs::File::open(path) {
        Ok(file) => hash_reader(file).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn normalize_line_endings_in_place(bytes: &mut [u8]) -> usize {
    let Some(mut read) = memchr::memchr(b'\r', bytes) else {
        return bytes.len();
    };
    let mut write = read;
    while read < bytes.len() {
        bytes[write] = b'\n';
        write += 1;
        read += 1 + usize::from(bytes.get(read + 1) == Some(&b'\n'));
        let next = memchr::memchr(b'\r', &bytes[read..]).map_or(bytes.len(), |at| read + at);
        bytes.copy_within(read..next, write);
        write += next - read;
        read = next;
    }
    write
}

pub fn text_from_bytes(mut bytes: Vec<u8>) -> std::io::Result<String> {
    let kept = normalize_line_endings_in_place(&mut bytes);
    bytes.truncate(kept);
    String::from_utf8(bytes).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))
}

pub fn normalize_line_endings(content: String) -> String {
    text_from_bytes(content.into_bytes()).expect("line endings are ASCII, so the text stays UTF-8")
}

pub fn read_text(path: &Path) -> std::io::Result<String> {
    text_from_bytes(fs::read(path)?)
}

fn snapshot_of(bytes: Vec<u8>) -> Result<FileSnapshot, FileCommandError> {
    // Without CR, text_from_bytes validates UTF-8 but never modifies bytes.
    // The raw-file hash is therefore also the editor's normalized text hash.
    let has_cr = memchr::memchr(b'\r', &bytes).is_some();
    let hash = hash_bytes(&bytes);
    let content = text_from_bytes(bytes).map_err(|error| FileCommandError::InvalidUtf8 {
        message: error.to_string(),
    })?;
    let text_hash = if has_cr { hash_bytes(content.as_bytes()) } else { hash.clone() };

    Ok(FileSnapshot {
        content,
        hash,
        text_hash,
    })
}

pub fn read_file_snapshot_impl(path: &Path) -> Result<FileSnapshot, FileCommandError> {
    snapshot_of(fs::read(path)?)
}

pub fn read_file_hash_impl(path: &Path) -> Result<String, FileCommandError> {
    hash_reader(fs::File::open(path)?)
}

pub fn read_file_stat_impl(
    path: &Path,
) -> Result<super::models::FileStat, FileCommandError> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(FileCommandError::Io {
            message: format!("not a file: {}", path.display()),
        });
    }
    Ok(super::models::FileStat {
        byte_length: meta.len(),
    })
}

fn check_expected_hash(path: &Path, expected_hash: &str) -> Result<(), FileCommandError> {
    let actual_hash = read_hash_if_exists(path)?;
    if actual_hash.as_deref() != Some(expected_hash) {
        return Err(FileCommandError::Conflict {
            expected_hash: expected_hash.to_owned(),
            actual_hash,
        });
    }
    Ok(())
}

pub fn write_file_atomic_impl(
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    let bytes = content.as_bytes();
    let hash = hash_bytes(bytes);
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(bytes)?;

    if let Some(expected_hash) = expected_hash {
        check_expected_hash(path, expected_hash)?;
    }

    file.commit()?;
    Ok(FileWriteResult { hash })
}

pub fn ensure_directory_impl(path: &Path) -> Result<(), FileCommandError> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod snapshot_hash_reuse_tests {
    use super::{hash_bytes, snapshot_of, text_from_bytes, FileSnapshot};
    use std::hint::black_box;
    use std::time::Instant;

    // Exact former snapshot implementation: keep as an in-process baseline
    // instead of comparing two different runners, CPUs or compiler revisions.
    fn legacy(mut bytes: Vec<u8>) -> FileSnapshot {
        let hash = hash_bytes(&bytes);
        let content = text_from_bytes(std::mem::take(&mut bytes)).unwrap();
        let text_hash = hash_bytes(content.as_bytes());
        FileSnapshot { content, hash, text_hash }
    }

    #[test]
    fn lf_hash_reuse_matches_previous_raw_and_normalized_hashes() {
        let cases = [
            String::new(),
            "Простая строка без переноса".to_owned(),
            "Русский текст 😀\nEnglish content\n".repeat(100),
            "Смешанные переносы\r\nстрока\rдругая\nконец".to_owned(),
            "Одиночный CR\rвместо LF".to_owned(),
            "\r\r\n\r\n\r".to_owned(),
        ];
        for input in cases {
            let baseline = legacy(input.as_bytes().to_vec());
            let fast = snapshot_of(input.as_bytes().to_vec()).unwrap();
            assert_eq!(fast.content, baseline.content);
            assert_eq!(fast.hash, baseline.hash);
            assert_eq!(fast.text_hash, baseline.text_hash);
        }
    }

    #[test]
    #[ignore = "diagnostic paired release benchmark; run with --ignored --nocapture"]
    fn benchmark_snapshot_hash_reuse() {
        fn sample(bytes: &[u8], old: bool) -> f64 {
            let start = Instant::now();
            for _ in 0..30 {
                let owned = black_box(bytes.to_vec());
                let output = if old { legacy(owned) } else { snapshot_of(owned).unwrap() };
                black_box((output.hash, output.text_hash, output.content.len()));
            }
            start.elapsed().as_secs_f64() * 1_000.0 / 30.0
        }

        let lf = "Строка текста 😀 with Unicode and ASCII\n".repeat(20_000);
        let crlf = "Строка текста 😀 with Unicode and ASCII\r\n".repeat(20_000);
        for (name, bytes) in [("lf", lf.as_bytes()), ("crlf", crlf.as_bytes())] {
            let mut old = Vec::new();
            let mut new = Vec::new();
            for _ in 0..5 {
                old.push(sample(bytes, true));
                new.push(sample(bytes, false));
                new.push(sample(bytes, false));
                old.push(sample(bytes, true));
            }
            old.sort_by(f64::total_cmp);
            new.sort_by(f64::total_cmp);
            let baseline_ms = (old[4] + old[5]) / 2.0;
            let candidate_ms = (new[4] + new[5]) / 2.0;
            println!(
                "BENCH-SNAPSHOT case={name} baseline_ms={baseline_ms:.4} candidate_ms={candidate_ms:.4} speedup={:.3}x",
                baseline_ms / candidate_ms
            );
        }
    }
}
