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

/// The one atomic publication sequence for both normal note files and
/// HistoryService version bytes. CAS must be checked after staging and before
/// commit; a failed CAS must never publish the temporary file.
fn write_atomic_bytes_impl(
    path: &Path,
    bytes: &[u8],
    expected_hash: Option<&str>,
    return_hash: bool,
) -> Result<Option<String>, FileCommandError> {
    #[cfg(feature = "save-stage-probe")]
    let profile_start = std::time::Instant::now();
    // Some callers (primary note saves) need the new BLAKE3 digest. History
    // version persistence previously calculated one only to discard it.
    let hash = return_hash.then(|| hash_bytes(bytes));
    #[cfg(feature = "save-stage-probe")]
    let profile_hash = profile_start.elapsed();
    let mut file = AtomicWriteFile::open(path)?;
    #[cfg(test)]
    atomic_kill_tests::atomic_write_kill_probe(path, "after-temp-open");
    #[cfg(feature = "save-stage-probe")]
    let profile_open = profile_start.elapsed();
    #[cfg(test)]
    if std::env::var("AKULA_ATOMIC_KILL_STAGE").as_deref() == Ok("after-first-half") {
        let split = (bytes.len() / 2).max(1).min(bytes.len().saturating_sub(1));
        file.write_all(&bytes[..split])?;
        atomic_kill_tests::atomic_write_kill_probe(path, "after-first-half");
        file.write_all(&bytes[split..])?;
    } else {
        file.write_all(bytes)?;
    }
    #[cfg(not(test))]
    file.write_all(bytes)?;
    #[cfg(test)]
    atomic_kill_tests::atomic_write_kill_probe(path, "after-temp-write-before-cas");
    #[cfg(feature = "save-stage-probe")]
    let profile_write = profile_start.elapsed();

    if let Some(expected_hash) = expected_hash {
        check_expected_hash(path, expected_hash)?;
    }
    #[cfg(test)]
    atomic_kill_tests::atomic_write_kill_probe(path, "after-cas-before-commit");
    #[cfg(feature = "save-stage-probe")]
    let profile_cas = profile_start.elapsed();

    file.commit()?;
    #[cfg(test)]
    atomic_kill_tests::atomic_write_kill_probe(path, "after-commit");
    #[cfg(feature = "save-stage-probe")]
    {
        let profile_commit = profile_start.elapsed();
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
        eprintln!(
            "AKULA_SAVE_ATOMIC cas={} bytes={} hash_ms={:.4} open_ms={:.4} write_ms={:.4} cas_ms={:.4} commit_ms={:.4} total_ms={:.4} hash_used={}",
            u8::from(expected_hash.is_some()),
            bytes.len(),
            ms(profile_hash),
            ms(profile_open - profile_hash),
            ms(profile_write - profile_open),
            ms(profile_cas - profile_write),
            ms(profile_commit - profile_cas),
            ms(profile_commit),
            u8::from(return_hash),
        );
    }
    Ok(hash)
}
pub fn write_file_atomic_impl(
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    // This is still the only primary-document writer: its digest and
    // raw-byte expected-hash CAS semantics are unchanged.
    let hash = write_atomic_bytes_impl(path, content.as_bytes(), expected_hash, true)?
        .expect("hash is always computed for the primary document");
    Ok(FileWriteResult { hash })
}

/// HistoryService's version writer needs durable bytes but never consumes
/// FileWriteResult.hash. Reuse the exact same atomic open/write/commit sequence
/// without recalculating a digest that was not checked or stored. Only the
/// history store should call this, under its existing custody/lock lifetime.
pub(crate) fn write_history_snapshot_atomic_impl(
    path: &Path,
    content: &str,
) -> Result<(), FileCommandError> {
    write_atomic_bytes_impl(path, content.as_bytes(), None, false).map(|_| ())
}

/// A bounded prepublication serializer for future File Gate content sources.
///
/// This is NOT an alternate authorization path. Only an existing Core File
/// Gate operation holding the applicable custody/OS lock and history lifetime
/// may call it. No UI or sync caller is wired to it by this change.
///
/// The source must be bound in advance to its expected byte length and BLAKE3
/// digest (for example a stable, cloned Rope). Reject truncated, extended or
/// changed streams before committing the temporary atomic-write file.
/// Compare the existing file's *raw* bytes with expected_disk_hash only after
/// the temporary content is complete, just as write_file_atomic_impl does.
///
/// The entire source is never allocated here; only a 256 KiB scratch buffer.
#[allow(dead_code)] // Private until a separately reviewed File Gate integration.
fn write_file_atomic_buffered_impl(
    path: &Path,
    mut source: impl Read,
    proposed_len: usize,
    proposed_hash: &str,
    expected_disk_hash: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    const WRITE_BATCH: usize = 256 * 1024;
    let mut file = AtomicWriteFile::open(path)?;
    let mut buffer = vec![0u8; WRITE_BATCH];
    let mut hasher = blake3::Hasher::new();
    let mut written = 0usize;
    let mut filled = 0usize;

    loop {
        // Read may legally return tiny pieces (including one Rope chunk).
        // Coalesce them before hashing and writing: per-chunk BLAKE3 updates
        // were significantly slower than bounded batches in Akula's A/B.
        let count = match source.read(&mut buffer[filled..]) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            other => other?,
        };
        if count == 0 {
            break;
        }
        written = written.checked_add(count).ok_or_else(|| FileCommandError::Task {
            message: "bounded atomic write source length overflow".to_owned(),
        })?;
        if written > proposed_len {
            return Err(FileCommandError::Task {
                message: "bounded atomic write source exceeded its announced length".to_owned(),
            });
        }
        filled += count;
        if filled == WRITE_BATCH {
            hasher.update(&buffer);
            file.write_all(&buffer)?;
            filled = 0;
        }
    }
    if filled > 0 {
        hasher.update(&buffer[..filled]);
        file.write_all(&buffer[..filled])?;
    }

    let hash = hasher.finalize().to_hex().to_string();
    if written != proposed_len || hash != proposed_hash {
        // AtomicWriteFile is dropped without commit: the existing document
        // remains untouched even if the source ended early or mutated.
        return Err(FileCommandError::Task {
            message: "bounded atomic write source did not match its announced bytes".to_owned(),
        });
    }
    if let Some(expected_disk_hash) = expected_disk_hash {
        check_expected_hash(path, expected_disk_hash)?;
    }
    file.commit()?;
    Ok(FileWriteResult { hash })
}

#[cfg(test)]
mod buffered_write_tests {
    use super::{hash_bytes, write_file_atomic_buffered_impl, write_file_atomic_impl};
    use crate::files::error::FileCommandError;
    use std::fs;
    use std::io::{self, Cursor, Read};

    struct TinyReads<'a> {
        input: &'a [u8],
        at: usize,
        max: usize,
    }

    impl Read for TinyReads<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.max).min(self.input.len() - self.at);
            buffer[..count].copy_from_slice(&self.input[self.at..self.at + count]);
            self.at += count;
            Ok(count)
        }
    }

    struct BrokenReader {
        first: bool,
    }

    impl Read for BrokenReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.first {
                self.first = false;
                // Complete one buffered batch so a real temporary file write
                // happens before the next read fails.
                buffer.fill(b'b');
                Ok(buffer.len())
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "injected source read fault"))
            }
        }
    }

    struct InterruptedOnce<'a> {
        source: Cursor<&'a [u8]>,
        first: bool,
    }

    impl Read for InterruptedOnce<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.first {
                self.first = false;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "injected interrupt"));
            }
            self.source.read(buffer)
        }
    }

    #[test]
    fn buffered_exact_utf8_binary_and_empty_match_existing_atomic_writer() {
        let temp = tempfile::tempdir().unwrap();
        let large = "Русский 漢字 😀\r\nnew line\0".repeat(40_000);
        let cases = [
            Vec::new(),
            b"ASCII bytes".to_vec(),
            large.into_bytes(),
            vec![0, 255, 7, 128, 13, 10, 0],
        ];
        for (i, bytes) in cases.into_iter().enumerate() {
            let path = temp.path().join(format!("stream-{i}.md"));
            let legacy = temp.path().join(format!("legacy-{i}.md"));
            fs::write(&path, "previous").unwrap();
            fs::write(&legacy, "previous").unwrap();
            let initial_hash = hash_bytes(b"previous");
            let proposed_hash = hash_bytes(&bytes);

            let saved = write_file_atomic_buffered_impl(
                &path, Cursor::new(&bytes), bytes.len(), &proposed_hash, Some(&initial_hash),
            ).unwrap();
            assert_eq!(saved.hash, proposed_hash);
            assert_eq!(fs::read(&path).unwrap(), bytes);

            // Raw byte writer parity for the UTF-8 cases; binary parity is
            // already guaranteed by the hash + exact-byte assertion above.
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let baseline = write_file_atomic_impl(&legacy, text, Some(&initial_hash)).unwrap();
                assert_eq!(baseline.hash, saved.hash);
                assert_eq!(fs::read(&legacy).unwrap(), fs::read(&path).unwrap());
            }
        }
    }

    #[test]
    fn buffered_small_reads_can_split_multibyte_utf8_without_changing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("utf8.md");
        let body = "😀漢字\na\r\nРусский текст".repeat(500);
        let raw = body.as_bytes();
        let hash = hash_bytes(raw);
        for max in [1, 2, 3, 7, 127] {
            let reader = TinyReads { input: raw, at: 0, max };
            let saved = write_file_atomic_buffered_impl(
                &path, reader, raw.len(), &hash, None,
            ).unwrap();
            assert_eq!(saved.hash, hash, "short-read size {max}");
            assert_eq!(fs::read(&path).unwrap(), raw, "short-read size {max}");
        }
    }

    #[test]
    fn buffered_refuses_short_extra_and_mutated_source_before_commit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("keep.md");
        fs::write(&path, b"unchanged").unwrap();
        let bytes = b"the proposed bytes";
        let hash = hash_bytes(bytes);
        for (announced_len, announced_hash) in [
            (bytes.len() + 1, hash.as_str()),
            (bytes.len() - 1, hash.as_str()),
            (bytes.len(), "0"),
        ] {
            let error = write_file_atomic_buffered_impl(
                &path, Cursor::new(bytes), announced_len, announced_hash, None,
            ).unwrap_err();
            assert!(matches!(error, FileCommandError::Task { .. }));
            assert_eq!(fs::read(&path).unwrap(), b"unchanged");
        }
    }

    #[test]
    fn buffered_failed_source_keeps_original_and_does_not_create_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("keep.md");
        fs::write(&path, b"original").unwrap();
        let err = write_file_atomic_buffered_impl(
            &path, BrokenReader { first: true },
            2 * 256 * 1024, &hash_bytes(&vec![b'b'; 2 * 256 * 1024]), None,
        ).unwrap_err();
        assert!(matches!(err, FileCommandError::Io { .. }));
        assert_eq!(fs::read(&path).unwrap(), b"original");

        let missing = temp.path().join("not-created.md");
        let err = write_file_atomic_buffered_impl(
            &missing, Cursor::new(b"short"), 6, &hash_bytes(b"short"), None,
        ).unwrap_err();
        assert!(matches!(err, FileCommandError::Task { .. }));
        assert!(!missing.exists());
    }

    #[test]
    fn buffered_stale_raw_disk_hash_keeps_foreign_edit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("conflict.md");
        fs::write(&path, b"foreign\r\nedit").unwrap();
        let proposed = b"local edit";
        let err = write_file_atomic_buffered_impl(
            &path, Cursor::new(proposed), proposed.len(),
            &hash_bytes(proposed), Some(&hash_bytes(b"previous")),
        ).unwrap_err();
        assert!(matches!(err, FileCommandError::Conflict { .. }));
        assert_eq!(fs::read(&path).unwrap(), b"foreign\r\nedit");
    }

    #[test]
    fn buffered_primary_remains_old_while_reader_is_paused_before_commit() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        struct PausedReader {
            stage: u8,
            ready: mpsc::Sender<()>,
            resume: mpsc::Receiver<()>,
        }

        impl Read for PausedReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = match self.stage {
                    0 => {
                        // Force the first complete 256-KiB batch to be staged
                        // in the atomic temporary before we pause.
                        buffer.fill(b'x');
                        buffer.len()
                    }
                    1 => {
                        self.ready.send(()).unwrap();
                        self.resume.recv_timeout(Duration::from_secs(10))
                            .expect("stalled source was not resumed");
                        let tail = b"second";
                        buffer[..tail.len()].copy_from_slice(tail);
                        tail.len()
                    }
                    _ => return Ok(0),
                };
                self.stage += 1;
                Ok(count)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ongoing.md");
        fs::write(&path, b"old revision").unwrap();
        let old_hash = hash_bytes(b"old revision");
        let (ready_tx, ready_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let mut payload = vec![b'x'; 256 * 1024];
        payload.extend_from_slice(b"second");
        let announced_hash = hash_bytes(&payload);
        let announced_len = payload.len();
        let worker_path = path.clone();
        let worker = thread::spawn(move || {
            write_file_atomic_buffered_impl(
                &worker_path,
                PausedReader { stage: 0, ready: ready_tx, resume: resume_rx },
                announced_len,
                &announced_hash,
                Some(&old_hash),
            )
        });

        ready_rx.recv_timeout(Duration::from_secs(10))
            .expect("writer never staged its first part");
        assert_eq!(fs::read(&path).unwrap(), b"old revision",
            "a partial temporary write must never be visible at the original path");
        resume_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap().unwrap().hash, hash_bytes(&payload));
        assert_eq!(fs::read(&path).unwrap(), payload);
    }

    #[test]
    fn buffered_interrupted_reader_is_retried_and_publishes_identical_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("interrupt.md");
        let bytes = b"UTF8 \xE2\x82\xAC bytes";
        let source = InterruptedOnce { source: Cursor::new(bytes.as_slice()), first: true };
        let saved = write_file_atomic_buffered_impl(
            &path, source, bytes.len(), &hash_bytes(bytes), None,
        ).unwrap();
        assert_eq!(saved.hash, hash_bytes(bytes));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
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

#[cfg(test)]
mod history_unhashed_atomic_tests;
#[cfg(test)]
mod atomic_kill_tests;
