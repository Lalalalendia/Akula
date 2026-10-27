//! Bounded read of an untrusted book file before any text or ZIP parsing.
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

// Applies to EPUB, MOBI/AZW/PRC and FB2; on-disk bytes, not inflated data.
pub(super) const MAX_BOOK_INPUT_BYTES: usize = 512 * 1024 * 1024;
const CHUNK_BYTES: usize = 32 * 1024;

pub(super) fn read(path: &Path) -> Option<Vec<u8>> {
    read_path_with_limit(path, MAX_BOOK_INPUT_BYTES)
}

fn read_path_with_limit(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    // Metadata is only a fast rejection: enforce the cap again while reading
    // the open file handle, in case another process grows the file.
    if file.metadata().ok()?.len() > limit as u64 {
        return None;
    }
    read_stream(file, limit)
}

fn read_stream(mut source: impl Read, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; CHUNK_BYTES];
    loop {
        let count = match source.read(&mut chunk) {
            Ok(0) => return Some(bytes),
            Ok(count) => count,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        if count > limit.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.try_reserve(count).ok()?;
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::{read, read_path_with_limit, read_stream, CHUNK_BYTES, MAX_BOOK_INPUT_BYTES};
    use std::io::{Cursor, Read};

    #[test]
    fn exact_limit_is_accepted_but_the_next_byte_is_rejected() {
        assert_eq!(read_stream(Cursor::new(b"abcdef"), 6), Some(b"abcdef".to_vec()));
        assert_eq!(read_stream(Cursor::new(b"abcdef"), 5), None);
        assert_eq!(read_stream(Cursor::new(b""), 0), Some(Vec::new()));
        assert_eq!(read_stream(Cursor::new(b"x"), 0), None);
    }

    #[test]
    fn repeated_input_ends_at_budget_without_unbounded_growth() {
        // std::io::repeat() has no EOF; this test would hang for unbounded reads.
        assert!(read_stream(std::io::repeat(b'X'), CHUNK_BYTES + 3).is_none());
        assert_eq!(read_stream(Cursor::new(vec![b'Y'; CHUNK_BYTES + 3]), CHUNK_BYTES + 3)
            .unwrap().len(), CHUNK_BYTES + 3);
    }

    #[test]
    fn rejects_oversized_file_by_metadata_before_reading() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("oversized.epub");
        std::fs::write(&path, b"123456789").unwrap();
        assert!(read_stream(Cursor::new(b"123456789"), 9).is_some());
        assert_eq!(read_path_with_limit(&path, 8), None);
        assert_eq!(read_path_with_limit(&path, 9), Some(b"123456789".to_vec()));
        assert!(MAX_BOOK_INPUT_BYTES > 9);
        assert!(read(&dir.path().join("does-not-exist.epub")).is_none());
    }

    struct InterruptedOnce<R>(bool, R);

    impl<R: Read> Read for InterruptedOnce<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
            }
            self.1.read(buf)
        }
    }

    #[test]
    fn interrupted_io_is_retried_without_treating_it_as_eof() {
        let stream = InterruptedOnce(false, Cursor::new(b"book"));
        assert_eq!(read_stream(stream, 4), Some(b"book".to_vec()));
    }
}
