//! Dormant file-lock identity diagnostic. It does not grant write authority.
//! Root identity and the pre-existing lock filename are checked using pinned
//! OS handles. Registration is verified separately by RegisteredReadAdmission.
//! No normal File Gate mutation calls this; bootstrap, lock-object custody,
//! hostile namespace swaps, and durable bilateral transactions remain P0.

use std::fs::{File, TryLockError};
#[cfg(unix)]
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use rustix::fs::{open, openat, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(crate) const LOCK_NAME: &str = ".akula-filegate-oslock-probe";

fn refused(reason: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason.to_owned())
}

fn converted(error: impl std::fmt::Debug) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, format!("{error:?}"))
}

/// Identity of an opened OS object. A text pathname or timestamp is never
/// used as proof that it is the same object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectIdentity {
    volume: u64,
    file: u64,
}

fn descriptor_identity(file: &File, regular: bool) -> io::Result<ObjectIdentity> {
    let meta = file.metadata()?;
    if (regular && !meta.is_file()) || (!regular && !meta.is_dir()) {
        return Err(refused("OS object kind does not match expected root or lock"));
    }

    #[cfg(unix)]
    {
        if meta.ino() == 0 || (regular && meta.nlink() != 1) {
            return Err(refused("missing inode or multiply linked lock file"));
        }
        return Ok(ObjectIdentity { volume: meta.dev(), file: meta.ino() });
    }
    #[cfg(windows)]
    {
        let (volume, id, links) = super::pinned_windows::file_identity_with_links(file)
            .map_err(converted)?;
        if id == 0 || (regular && links != 1) {
            return Err(refused("missing file ID or multiply linked lock file"));
        }
        return Ok(ObjectIdentity { volume: u64::from(volume), file: id });
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        Err(refused("OS lock identity probe unsupported on this platform"))
    }
}

fn open_root(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        let fd = open(path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(converted)?;
        return Ok(File::from(fd));
    }
    #[cfg(windows)]
    {
        return super::pinned_windows::open_root(path).map_err(converted);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(refused("unsupported OS"))
    }
}

fn open_existing_leaf(root: &File) -> io::Result<File> {
    #[cfg(unix)]
    {
        let fd = openat(root, LOCK_NAME,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ).map_err(converted)?;
        return Ok(File::from(fd));
    }
    #[cfg(windows)]
    {
        return super::pinned_windows::open_existing_probe_lock_child(root, LOCK_NAME)
            .map_err(converted);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(refused("unsupported OS"))
    }
}

pub(crate) struct PinnedProbeRoot {
    handle: File,
    selected_path: PathBuf,
    id: ObjectIdentity,
}

pub(crate) struct HeldProbeLock {
    // Keep root pinned while the lock remains held; never unlink lock files.
    _root: File,
    _file: File,
    id: ObjectIdentity,
}

impl HeldProbeLock {
    /// Diagnostic only. An observed lock ID is NOT transfer/write permission.
    pub(crate) fn identity(&self) -> (u64, u64) {
        (self.id.volume, self.id.file)
    }

    /// Metadata is read from the pinned kernel handle, not a mutable path.
    pub(crate) fn opened_file_metadata(&self) -> io::Result<std::fs::Metadata> {
        self._file.metadata()
    }

    /// Owner/DACL read is through the LIVE locked file handle.
    /// This is a negative forensic check, never a write permit.
    #[cfg(windows)]
    pub(crate) fn audit_windows_acl(&self) -> io::Result<()> {
        super::filegate_windows_acl::audit_owner_dacl(&self._file).map_err(converted)
    }

    /// Negative forensic check ONLY. An exclusive OS lock protects an opened
    /// file object, not its directory entry. Another process with sufficient
    /// filesystem rights can rename the held object and replace its name;
    /// the replacement then has a *different*, independently lockable ID.
    ///
    /// Detect an already-performed swap while this handle is still held.
    /// A result is negative forensic evidence, NOT a production write grant:
    /// another actor may swap the directory entry immediately afterward.
    /// The method is also used by the cooperative lifetime test harness.
    pub(crate) fn recheck_held_name(&self, root: &PinnedProbeRoot) -> io::Result<()> {
        root.recheck_root()?;
        if descriptor_identity(&self._root, false)? != root.id {
            return Err(refused("held lock belongs to a different pinned root"));
        }
        if descriptor_identity(&self._file, true)? != self.id {
            return Err(refused("held OS lock handle identity changed"));
        }
        let named = open_existing_leaf(&root.handle)?;
        if descriptor_identity(&named, true)? != self.id {
            return Err(refused("held lock pathname now points to another OS object"));
        }
        root.recheck_root()?;
        Ok(())
    }
}

impl PinnedProbeRoot {
    pub(crate) fn open(selected_path: &Path) -> io::Result<Self> {
        // A direct symlink/reparse at the chosen root is never silently
        // canonicalized into a different workspace.
        let handle = open_root(selected_path)?;
        let id = descriptor_identity(&handle, false)?;
        let candidate = Self {
            handle,
            selected_path: selected_path.to_path_buf(),
            id,
        };
        candidate.recheck_root()?;
        Ok(candidate)
    }

    pub(crate) fn identity(&self) -> (u64, u64) {
        (self.id.volume, self.id.file)
    }

    pub(crate) fn opened_root_metadata(&self) -> io::Result<std::fs::Metadata> {
        self.handle.metadata()
    }

    /// ACL is read from the PINNED external custody directory handle.
    #[cfg(windows)]
    pub(crate) fn audit_windows_acl(&self) -> io::Result<()> {
        super::filegate_windows_acl::audit_owner_dacl(&self.handle).map_err(converted)
    }

    pub(crate) fn recheck_root(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let named = fs::symlink_metadata(&self.selected_path)?;
            if !named.is_dir() || named.file_type().is_symlink()
                || named.ino() != self.id.file || named.dev() != self.id.volume
            {
                return Err(refused("pinned workspace root replaced or aliased"));
            }
        }
        #[cfg(windows)]
        {
            // The NT opener refuses a final reparse point. Compare a FRESH
            // opened handle with the already pinned root, not path spelling.
            let named = open_root(&self.selected_path)?;
            if descriptor_identity(&named, false)? != self.id {
                return Err(refused("registered workspace root was replaced"));
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            return Err(refused("unsupported OS"));
        }
        Ok(())
    }

    /// The lock FILE MUST ALREADY EXIST (created by the test fixture). This
    /// function does no creation, unlink, rename, data write or journal write.
    /// It is not yet a root/custody-attested production File Gate admission.
    pub(crate) fn try_lock(&self) -> io::Result<Option<HeldProbeLock>> {
        self.recheck_root()?;
        let file = open_existing_leaf(&self.handle)?;
        let original = descriptor_identity(&file, true)?;
        // A second root-relative open detects a swapped name before locking.
        let reread = open_existing_leaf(&self.handle)?;
        if descriptor_identity(&reread, true)? != original {
            return Err(refused("lock object identity changed before acquisition"));
        }
        drop(reread);

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(error),
        }
        // Verify again WHILE holding the exclusive OS lock. This narrows the
        // namespace race but does not prevent a hostile post-check unlink.
        self.recheck_root()?;
        let reread = open_existing_leaf(&self.handle)?;
        if descriptor_identity(&reread, true)? != original {
            return Err(refused("lock pathname no longer names the held file"));
        }
        Ok(Some(HeldProbeLock {
            _root: self.handle.try_clone()?,
            _file: file,
            id: original,
        }))
    }
}

#[cfg(test)]
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(test)]
const TEST_BOOTSTRAP_MAGIC: &[u8] = b"AKULA_FILEGATE_LOCK_V1";
#[cfg(test)]
const TEST_BOOTSTRAP_RECORD_LEN: usize = TEST_BOOTSTRAP_MAGIC.len() + 16 + 32;

/// A test fixture's observed OS identity and self-checking generation.
/// This is neither a trusted VaultId nor persistent authorization.
#[cfg(test)]
pub(crate) struct TestLockBootstrapEvidence {
    pub(crate) created: bool,
    pub(crate) generation: uuid::Uuid,
    pub(crate) identity: (u64, u64),
    /// Unix fsync() on parent was attempted and succeeded; this is not a
    /// blanket power-loss promise on network/virtual/noncompliant filesystems.
    pub(crate) parent_sync_completed: bool,
}

#[cfg(test)]
fn exclusive_test_leaf(root: &PinnedProbeRoot) -> io::Result<Option<File>> {
    #[cfg(unix)]
    {
        let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL
            | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        return match openat(
            &root.handle,
            LOCK_NAME,
            flags,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => Ok(Some(File::from(fd))),
            Err(error) if error == rustix::io::Errno::EXIST => Ok(None),
            Err(error) => Err(converted(error)),
        };
    }
    #[cfg(windows)]
    {
        // Windows needs a separately opened parent with ADD_FILE authority.
        // Verify it names the same physical root we previously pinned.
        let parent = super::pinned_windows::open_root_for_create(&root.selected_path)
            .map_err(converted)?;
        if descriptor_identity(&parent, false)? != root.id {
            return Err(refused("lock bootstrap parent no longer matches pinned root"));
        }
        return match super::pinned_windows::create_probe_lock_child(&parent, LOCK_NAME) {
            Ok(file) => Ok(Some(file)),
            Err(super::VaultPortError::DestinationExists) => Ok(None),
            Err(error) => Err(converted(error)),
        };
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(refused("lock bootstrap experiment unavailable on this OS"))
    }
}

#[cfg(test)]
fn encode_test_bootstrap_record(generation: uuid::Uuid) -> Vec<u8> {
    let mut record = Vec::with_capacity(TEST_BOOTSTRAP_RECORD_LEN);
    record.extend_from_slice(TEST_BOOTSTRAP_MAGIC);
    record.extend_from_slice(generation.as_bytes());
    let digest = blake3::hash(&record);
    record.extend_from_slice(digest.as_bytes());
    record
}

#[cfg(test)]
fn read_test_bootstrap_record(file: &mut File) -> io::Result<uuid::Uuid> {
    // A stale, truncated or future-version file is never rewritten in place.
    // This checksum detects accidental damage, not malicious forgery.
    if file.metadata()?.len() != TEST_BOOTSTRAP_RECORD_LEN as u64 {
        return Err(refused("incomplete or unknown-version lock bootstrap record"));
    }
    let mut bytes = vec![0u8; TEST_BOOTSTRAP_RECORD_LEN];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut bytes)?;
    if !bytes.starts_with(TEST_BOOTSTRAP_MAGIC) {
        return Err(refused("unexpected lock bootstrap signature"));
    }
    let prefix = TEST_BOOTSTRAP_MAGIC.len() + 16;
    if bytes[prefix..] != blake3::hash(&bytes[..prefix]).as_bytes()[..] {
        return Err(refused("lock bootstrap checksum mismatch"));
    }
    uuid::Uuid::from_slice(&bytes[TEST_BOOTSTRAP_MAGIC.len()..prefix])
        .map_err(|_| refused("invalid shared lock generation UUID"))
}

#[cfg(test)]
impl PinnedProbeRoot {
    /// Controlled disposable-vault experiment ONLY. No production writer,
    /// Core File Gate or HistoryService calls this method. The committed
    /// creation record cannot yet be considered durable on Windows or on
    /// arbitrary filesystems, and its filename remains externally mutable.
    pub(crate) fn bootstrap_exclusive_for_test(
        &self,
    ) -> io::Result<TestLockBootstrapEvidence> {
        self.recheck_root()?;
        let created_file = exclusive_test_leaf(self)?;
        let created = created_file.is_some();
        let mut created_id = None;
        let mut created_generation = None;
        #[cfg(unix)]
        let mut parent_sync_completed = false;
        #[cfg(not(unix))]
        let parent_sync_completed = false;
        if let Some(mut file) = created_file {
            let id = descriptor_identity(&file, true)?;
            let generation = uuid::Uuid::new_v4();
            let bytes = encode_test_bootstrap_record(generation);
            file.write_all(&bytes)?;
            file.sync_all()?;
            created_id = Some(id);
            created_generation = Some(generation);
            // Directory fsync is not uniformly supported (notably on some
            // macOS and virtual filesystems). Record an unsupported result
            // explicitly; never claim parent durability on such a filesystem.
            #[cfg(unix)]
            match self.handle.sync_all() {
                Ok(()) => parent_sync_completed = true,
                Err(error) if matches!(
                    error.kind(),
                    ErrorKind::InvalidInput | ErrorKind::Unsupported
                ) => {}
                Err(error) => return Err(error),
            }
        }
        self.recheck_root()?;
        // Open the named object again relative to the original pinned root:
        // never allow an exclusive-created but subsequently swapped name to
        // become a positive observation.
        let mut reopened = open_existing_leaf(&self.handle)?;
        let id = descriptor_identity(&reopened, true)?;
        if created_id.is_some_and(|original| original != id) {
            return Err(refused("newly created lock object has been replaced"));
        }
        let generation = read_test_bootstrap_record(&mut reopened)?;
        if created_generation.is_some_and(|original| original != generation) {
            return Err(refused("newly created lock generation has been substituted"));
        }
        self.recheck_root()?;
        let reread = open_existing_leaf(&self.handle)?;
        if descriptor_identity(&reread, true)? != id {
            return Err(refused("lock leaf changed during bootstrap observation"));
        }
        Ok(TestLockBootstrapEvidence {
            created,
            generation,
            identity: (id.volume, id.file),
            parent_sync_completed,
        })
    }
}
