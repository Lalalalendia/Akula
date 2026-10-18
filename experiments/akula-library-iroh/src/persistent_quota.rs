//! Durable accounting for *payload* upload quota in the isolated Akula Library lab.
//! This deliberately does NOT measure IP/QUIC overhead or implement daily resets.
//! A corrupt, deleted, truncated, or inaccessible journal must fail closed.
//! Local owners can tamper with their own files; this is not an anti-owner DRM scheme.

use crate::{sha, QuotaState};
use anyhow::{bail, ensure, Context, Result};
use iroh::EndpointId;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAGIC: &[u8; 4] = b"AKQ1";
const HEADER_LEN: usize = 4 + 32 + 8 + 8 + 32;
const RECORD_LEN: usize = 8 + 32 + 8 + 32;
/// Safety valve: cap the append-only journal at approximately 5 MiB.
/// Compaction with crash-safe checkpoints is future work.
pub const MAX_RECORDS: usize = 65_536;

fn secure_regular(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    ensure!(meta.file_type().is_file(), "not a regular state file: {}", path.display());
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        ensure!(meta.permissions().mode() & 0o077 == 0, "quota state file is not private");
    }
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<bool> {
    let new = match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(meta.file_type().is_dir(), "quota state must be a real directory");
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                ensure!(meta.permissions().mode() & 0o077 == 0, "quota directory must be mode 0700");
            }
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)] {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(path)?;
            }
            #[cfg(not(unix))]
            fs::create_dir(path)?;
            true
        }
        Err(e) => return Err(e.into()),
    };
    Ok(new)
}

fn open_private_new(path: &Path, append: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create_new(true).append(append);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    Ok(opts.open(path)?)
}

fn open_private_existing(path: &Path, append: bool) -> Result<File> {
    secure_regular(path)?;
    Ok(OpenOptions::new().read(true).write(true).append(append).open(path)?)
}

fn durable_directory_entry(dir: &Path) -> Result<()> {
    // Persist creation of lock/journal names as well as their content on Unix.
    #[cfg(unix)] {
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))] { let _ = dir; }
    Ok(())
}

fn header(owner: EndpointId, global: u64, per_peer: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN);
    buf.extend(MAGIC);
    buf.extend(owner.as_bytes());
    buf.extend(global.to_be_bytes());
    buf.extend(per_peer.to_be_bytes());
    let digest = sha(&buf);
    buf.extend(digest);
    buf
}

fn record_hash(prev: [u8; 32], data: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(32 + data.len());
    input.extend(prev);
    input.extend(data);
    sha(&input)
}

/// Owns an exclusive OS file lock until dropped.
/// All reservations are written and fsync'ed before bytes are returned to the server.
pub struct PersistentQuota {
    _lock: File,
    journal: File,
    state: QuotaState,
    digest: [u8; 32],
    records: usize,
    poisoned: bool,
}

impl PersistentQuota {
    /// `dir` is an isolated state directory that must not be shared by different identities.
    /// A missing journal in an *existing* directory is an error, never a fresh zero balance.
    pub fn open(dir: &Path, owner: EndpointId, global: u64, per_peer: u64) -> Result<Self> {
        let fresh = ensure_private_dir(dir)?;
        let lock_path = dir.join("quota.lock");
        let journal_path = dir.join("quota.journal");

        let lock = if fresh {
            open_private_new(&lock_path, false)?
        } else {
            open_private_existing(&lock_path, false)?
        };
        lock.try_lock().context("quota state locked by another sender")?;

        let mut journal = if fresh {
            let mut f = open_private_new(&journal_path, true)?;
            f.write_all(&header(owner, global, per_peer))?;
            f.sync_all()?;
            durable_directory_entry(dir)?;
            f
        } else {
            open_private_existing(&journal_path, true)?
        };

        let len = journal.metadata()?.len();
        ensure!(len >= HEADER_LEN as u64, "quota journal missing/truncated header");
        ensure!(len <= (HEADER_LEN + MAX_RECORDS * RECORD_LEN) as u64, "quota journal exceeds size limit");
        ensure!((len as usize - HEADER_LEN) % RECORD_LEN == 0, "truncated quota journal record");

        let mut bytes = Vec::with_capacity(len as usize);
        journal.seek(SeekFrom::Start(0))?;
        journal.read_to_end(&mut bytes)?;
        ensure!(bytes.len() == len as usize, "quota journal changed while loading");
        let expected_header = header(owner, global, per_peer);
        ensure!(bytes[..HEADER_LEN] == expected_header, "quota journal owner/limits/integrity mismatch");
        let mut previous: [u8; 32] = expected_header[HEADER_LEN-32..].try_into()?;
        let mut state = QuotaState::new(global, per_peer);
        let mut sequence = 0u64;
        for rec in bytes[HEADER_LEN..].chunks_exact(RECORD_LEN) {
            sequence += 1;
            ensure!(u64::from_be_bytes(rec[..8].try_into()?) == sequence, "quota journal sequence mismatch");
            let peer_bytes: [u8; 32] = rec[8..40].try_into()?;
            let peer = EndpointId::from_bytes(&peer_bytes)
                .context("invalid stored quota peer identity")?;
            let amount = u64::from_be_bytes(rec[40..48].try_into()?);
            ensure!(amount <= usize::MAX as u64, "stored quota amount overflows machine");
            let checksum = record_hash(previous, &rec[..48]);
            ensure!(rec[48..80] == checksum[..], "quota journal record hash mismatch");
            ensure!(state.reserve(peer, amount as usize), "quota journal exceeds saved quota");
            previous = checksum;
        }
        Ok(Self { _lock: lock, journal, state, digest: previous, records: sequence as usize, poisoned: false })
    }

    pub fn global_used(&self) -> u64 { self.state.used() }
    pub fn used_for(&self, peer: &EndpointId) -> u64 { self.state.used_for(peer) }

    /// A quota rejection is `Ok(false)`; a storage problem is `Err`, and poisons this instance.
    pub fn reserve(&mut self, peer: EndpointId, bytes: usize) -> Result<bool> {
        ensure!(!self.poisoned, "quota journal is poisoned; restart is required");
        ensure!(self.records < MAX_RECORDS, "quota journal full; refuse more uploads");
        let n = bytes as u64;
        let Some(new_global) = self.state.used().checked_add(n) else { return Ok(false); };
        let Some(new_peer) = self.state.used_for(&peer).checked_add(n) else { return Ok(false); };
        if new_global > self.state.global_limit || new_peer > self.state.per_peer_limit {
            return Ok(false);
        }
        let number = (self.records as u64).checked_add(1).context("quota sequence overflow")?;
        let mut rec = Vec::with_capacity(RECORD_LEN);
        rec.extend(number.to_be_bytes());
        rec.extend(peer.as_bytes());
        rec.extend(n.to_be_bytes());
        let digest = record_hash(self.digest, &rec);
        rec.extend(digest);
        if let Err(e) = self.journal.write_all(&rec).and_then(|_| self.journal.sync_all()) {
            self.poisoned = true; // Never serve another chunk after a failed durable write.
            return Err(e).context("could not persist quota reservation; uploads stopped");
        }
        ensure!(self.state.reserve(peer, bytes), "quota state changed while persisting");
        self.records += 1;
        self.digest = digest;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_state() -> std::path::PathBuf {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("akula-quota-{}-{suffix}", std::process::id()));
        assert!(!p.exists());
        p
    }

    #[test]
    fn cannot_reset_by_restart_or_switching_peers() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        let a = SecretKey::generate().public();
        let b = SecretKey::generate().public();
        let mut q = PersistentQuota::open(&dir, owner, 10, 6)?;
        assert!(q.reserve(a, 6)?);
        assert!(q.reserve(b, 4)?);
        assert!(!q.reserve(a, 1)?);
        drop(q);
        let mut restored = PersistentQuota::open(&dir, owner, 10, 6)?;
        assert_eq!(restored.global_used(), 10);
        assert_eq!(restored.used_for(&a), 6);
        assert_eq!(restored.used_for(&b), 4);
        assert!(!restored.reserve(a, 1)?);
        assert!(!restored.reserve(b, 1)?);
        drop(restored);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn second_sender_cannot_open_same_ledger() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        let first = PersistentQuota::open(&dir, owner, 100, 100)?;
        assert!(PersistentQuota::open(&dir, owner, 100, 100).is_err());
        drop(first);
        let second = PersistentQuota::open(&dir, owner, 100, 100)?;
        drop(second);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn corrupt_or_truncated_journal_is_not_reset() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        let peer = SecretKey::generate().public();
        let mut q = PersistentQuota::open(&dir, owner, 100, 100)?;
        assert!(q.reserve(peer, 10)?);
        drop(q);
        let path = dir.join("quota.journal");
        let good = fs::read(&path)?;
        let mut corrupt = good.clone();
        *corrupt.last_mut().unwrap() ^= 0x7f;
        fs::write(&path, corrupt)?;
        assert!(PersistentQuota::open(&dir, owner, 100, 100).is_err());
        fs::write(&path, &good[..good.len()-1])?;
        assert!(PersistentQuota::open(&dir, owner, 100, 100).is_err());
        fs::write(&path, good)?;
        let restored = PersistentQuota::open(&dir, owner, 100, 100)?;
        assert_eq!(restored.global_used(), 10);
        drop(restored);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn absent_journal_in_existing_dir_cannot_start_fresh() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        drop(PersistentQuota::open(&dir, owner, 100, 100)?);
        fs::remove_file(dir.join("quota.journal"))?;
        assert!(PersistentQuota::open(&dir, owner, 100, 100).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn wrong_owner_or_limits_are_refused() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        drop(PersistentQuota::open(&dir, owner, 100, 50)?);
        assert!(PersistentQuota::open(&dir, SecretKey::generate().public(), 100, 50).is_err());
        assert!(PersistentQuota::open(&dir, owner, 101, 50).is_err());
        assert!(PersistentQuota::open(&dir, owner, 100, 51).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn failed_storage_state_refuses_next_reservation() -> Result<()> {
        let dir = temp_state();
        let owner = SecretKey::generate().public();
        let peer = SecretKey::generate().public();
        let mut quota = PersistentQuota::open(&dir, owner, 100, 100)?;
        quota.poisoned = true; // Model a failed fsync/write path.
        assert!(quota.reserve(peer, 10).is_err());
        assert_eq!(quota.global_used(), 0);
        drop(quota);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn rejects_symlinks_and_world_readable_state() -> Result<()> {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = temp_state();
        fs::create_dir(&root)?;
        let owner = SecretKey::generate().public();
        let p = root.join("quota");
        drop(PersistentQuota::open(&p, owner, 100, 100)?);
        let link = root.join("alias");
        symlink(&p, &link)?;
        assert!(PersistentQuota::open(&link, owner, 100, 100).is_err());
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755))?;
        assert!(PersistentQuota::open(&p, owner, 100, 100).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
