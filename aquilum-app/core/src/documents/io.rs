use super::conflict::{copy_content, copy_stem, ConflictReport, LocalMinute};
use super::resolve::{SyncPoint, SyncPointLookup};
use super::session::{DiskError, DiskSnapshot, SessionIo};
use super::store::DocumentStore;
use crate::files::document::{hash_bytes, read_file_hash_impl, read_file_snapshot_impl};
use crate::files::error::FileCommandError;
use crate::files::gate;
use crate::history::Source;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::app_core::Core;

const MAX_COPY_ATTEMPTS: u32 = 10_000;

pub struct HubIo<'a> {
    pub core: &'a Core,
    pub path: &'a Path,
    pub key: &'a str,
    pub store: &'a Mutex<Option<DocumentStore>>,
    pub offset_seconds: i64,
    pub source: Source,
}

fn disk_error(path: &Path, error: FileCommandError) -> DiskError {
    match error {
        FileCommandError::Conflict { .. } => DiskError::Conflict,
        _ if !path.exists() => DiskError::Missing,
        FileCommandError::InvalidUtf8 { message } => DiskError::Other { code: "invalid_utf8", message },
        error => DiskError::Other { code: "io", message: error.to_string() },
    }
}

fn now_nanos() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as i64)
}

fn copy_path(note: &Path, label: &str, attempt: u32) -> PathBuf {
    let name = if attempt == 0 { format!("{label}.md") } else { format!("{label} {}.md", attempt + 1) };
    note.with_file_name(name)
}

/// A protective copy is a precondition for a destructive session transition.
///
/// Returning an error matters: a log message is not evidence that the conflict
/// copy exists. Empty content needs no backup; whitespace-only content does.
pub fn preserve_copy(core: &Core, path: &Path, body: &str, report: ConflictReport, offset_seconds: i64)
    -> Result<Option<PathBuf>, DiskError>
{
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let moment = LocalMinute::at(now_nanos(), offset_seconds);
    let note = path.to_string_lossy();
    let Some(content) = copy_content(&note, &stem, &report, &moment, body) else {
        return Ok(None);
    };
    let label = copy_stem(&stem, &moment);
    for attempt in 0..MAX_COPY_ATTEMPTS {
        let target = copy_path(path, &label, attempt);
        match gate::create(core, &target, &content, Source::Me, None) {
            Ok(_) => return Ok(Some(target)),
            Err(FileCommandError::AlreadyExists { .. }) => continue,
            Err(error) => {
                return Err(DiskError::Other {
                    code: "preservation_failed",
                    message: format!("could not preserve conflict copy at {}: {error}", target.display()),
                });
            }
        }
    }
    Err(DiskError::Other {
        code: "preservation_failed",
        message: format!("no free filename for conflict copy {label}"),
    })
}

impl HubIo<'_> {
    fn with_store<T>(&self, operation: impl FnOnce(&DocumentStore) -> T) -> Option<T> {
        let guard = self.store.lock().ok()?;
        guard.as_ref().map(operation)
    }
}

impl SessionIo for HubIo<'_> {
    fn read(&self) -> Result<DiskSnapshot, DiskError> {
        let snapshot = read_file_snapshot_impl(self.path).map_err(|error| disk_error(self.path, error))?;
        Ok(DiskSnapshot { content: snapshot.content, hash: snapshot.hash, text_hash: snapshot.text_hash })
    }

    fn hash(&self) -> Result<String, DiskError> {
        read_file_hash_impl(self.path).map_err(|error| disk_error(self.path, error))
    }

    fn write(&self, content: &str, expected_hash: &str) -> Result<String, DiskError> {
        gate::write(self.core, self.path, content, Some(expected_hash), self.source, None)
            .map(|result| result.hash)
            .map_err(|error| disk_error(self.path, error))
    }

    fn text_hash(&self, text: &str) -> String {
        hash_bytes(text.as_bytes())
    }

    fn preserve(&self, body: &str, report: ConflictReport) -> Result<(), DiskError> {
        preserve_copy(self.core, self.path, body, report, self.offset_seconds).map(|_| ())
    }

    fn append(&self, update: &[u8]) -> Result<(), DiskError> {
        match self.with_store(|store| store.append(self.key, update)) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(DiskError::Other {
                code: "append_failed",
                message: format!("document update not persisted: {error}"),
            }),
            None => Err(DiskError::Other {
                code: "append_failed",
                message: "document store unavailable; refusing unjournaled edit".into(),
            }),
        }
    }

    fn sync_point(&self) -> SyncPointLookup {
        self.with_store(|store| store.sync_point(self.key)).unwrap_or(SyncPointLookup::Unavailable)
    }

    fn remember(&self, point: SyncPoint) -> Result<(), DiskError> {
        match self.with_store(|store| store.remember_sync_point(self.key, &point)) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(DiskError::Other {
                code: "remember_failed",
                message: format!("sync point not persisted in document store: {error}"),
            }),
            None => Err(DiskError::Other {
                code: "remember_failed",
                message: "document store unavailable; sync point cannot be confirmed".into(),
            }),
        }
    }
}
