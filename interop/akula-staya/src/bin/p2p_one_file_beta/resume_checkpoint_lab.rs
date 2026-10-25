//! Non-shipping checkpoint lab for a QUIC photo receiver. This is a fixed,
//! peer-bound, content-bound **test format**; it is neither a signed W3 notice
//! nor a power-loss-durable or security-hardened Windows file-store contract.
//! The sender must still check the authenticated QUIC peer and exact prefix.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"AKCPT001";
const WIRE_BYTES: usize = 8 + 32 + 32 + 32 + 8 + 8 + 32;
const IO_BLOCK: usize = 64 * 1024;
const MAX_LAB_FILE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Identity {
    pub sender: [u8; 32],
    pub receiver: [u8; 32],
    pub full_hash: [u8; 32],
    pub total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Checkpoint {
    pub identity: Identity,
    pub offset: u64,
    pub prefix_hash: [u8; 32],
}

impl Checkpoint {
    fn wire(self) -> [u8; WIRE_BYTES] {
        let mut bytes = [0; WIRE_BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..40].copy_from_slice(&self.identity.sender);
        bytes[40..72].copy_from_slice(&self.identity.receiver);
        bytes[72..104].copy_from_slice(&self.identity.full_hash);
        bytes[104..112].copy_from_slice(&self.identity.total.to_be_bytes());
        bytes[112..120].copy_from_slice(&self.offset.to_be_bytes());
        bytes[120..152].copy_from_slice(&self.prefix_hash);
        bytes
    }

    fn decode(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() != WIRE_BYTES || &raw[..8] != MAGIC {
            return Err("invalid checkpoint version or length");
        }
        let sender = raw[8..40].try_into().expect("fixed length");
        let receiver = raw[40..72].try_into().expect("fixed length");
        let full_hash = raw[72..104].try_into().expect("fixed length");
        let total = u64::from_be_bytes(raw[104..112].try_into().expect("fixed length"));
        let offset = u64::from_be_bytes(raw[112..120].try_into().expect("fixed length"));
        let prefix_hash = raw[120..152].try_into().expect("fixed length");
        Ok(Self {
            identity: Identity {
                sender,
                receiver,
                full_hash,
                total,
            },
            offset,
            prefix_hash,
        })
    }
}

fn check(identity: Identity, offset: u64) -> Result<(), &'static str> {
    if identity.sender == [0; 32]
        || identity.receiver == [0; 32]
        || identity.sender == identity.receiver
        || identity.full_hash == [0; 32]
        || identity.total == 0
        || identity.total > MAX_LAB_FILE_BYTES
        || offset == 0
        || offset >= identity.total
        || offset % IO_BLOCK as u64 != 0
    {
        return Err("invalid checkpoint identity, size or alignment");
    }
    Ok(())
}

// This is a lab caller-owned private directory. Production requires OS-pinned
// root handles, ACL checks, metadata atomic replacement and directory fsync.
pub(super) fn persist(
    incoming_prefix: &Path,
    stage_path: &Path,
    checkpoint_path: &Path,
    identity: Identity,
    offset: u64,
) -> Result<Checkpoint, String> {
    check(identity, offset).map_err(str::to_owned)?;
    if fs::metadata(incoming_prefix)
        .map_err(|e| format!("incoming prefix metadata: {e}"))?
        .len()
        != offset
    {
        return Err("incoming prefix has wrong length".into());
    }
    let mut input =
        File::open(incoming_prefix).map_err(|e| format!("open partial input: {e}"))?;
    let mut stage = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage_path)
        .map_err(|e| format!("create private stage: {e}"))?;
    let mut remaining = offset;
    let mut hasher = blake3::Hasher::new();
    let mut block = [0u8; IO_BLOCK];
    while remaining > 0 {
        let request = usize::try_from(remaining.min(IO_BLOCK as u64)).unwrap();
        let count = input
            .read(&mut block[..request])
            .map_err(|e| format!("read incoming bytes: {e}"))?;
        if count == 0 {
            return Err("truncated incoming bytes".into());
        }
        stage
            .write_all(&block[..count])
            .map_err(|e| format!("write private stage: {e}"))?;
        hasher.update(&block[..count]);
        remaining -= count as u64;
    }
    stage
        .sync_all()
        .map_err(|e| format!("fsync private stage: {e}"))?;
    drop(stage);
    let checkpoint = Checkpoint {
        identity,
        offset,
        prefix_hash: *hasher.finalize().as_bytes(),
    };
    let mut meta = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(checkpoint_path)
        .map_err(|e| format!("create checkpoint sidecar: {e}"))?;
    meta.write_all(&checkpoint.wire())
        .map_err(|e| format!("write checkpoint sidecar: {e}"))?;
    meta.sync_all()
        .map_err(|e| format!("fsync checkpoint sidecar: {e}"))?;
    Ok(checkpoint)
}

/// After a process exits, never trust the sidecar's offset without an
/// independent cold read of the stage and an expected authenticated identity.
pub(super) fn recover(
    stage_path: &Path,
    checkpoint_path: &Path,
    expected: Identity,
) -> Result<Checkpoint, String> {
    let data = fs::read(checkpoint_path).map_err(|e| format!("read checkpoint sidecar: {e}"))?;
    let checkpoint = Checkpoint::decode(&data).map_err(str::to_owned)?;
    if checkpoint.identity != expected {
        return Err("checkpoint belongs to another sender, receiver or root".into());
    }
    check(checkpoint.identity, checkpoint.offset).map_err(str::to_owned)?;
    let mut file = File::open(stage_path).map_err(|e| format!("open checkpoint stage: {e}"))?;
    if file
        .metadata()
        .map_err(|e| format!("stage metadata: {e}"))?
        .len()
        != checkpoint.offset
    {
        return Err("checkpoint stage length mismatch".into());
    }
    let mut remaining = checkpoint.offset;
    let mut hasher = blake3::Hasher::new();
    let mut block = [0u8; IO_BLOCK];
    while remaining > 0 {
        let request = usize::try_from(remaining.min(IO_BLOCK as u64)).unwrap();
        let count = file
            .read(&mut block[..request])
            .map_err(|e| format!("checkpoint cold read: {e}"))?;
        if count == 0 {
            return Err("checkpoint stage truncated".into());
        }
        hasher.update(&block[..count]);
        remaining -= count as u64;
    }
    if hasher.finalize().as_bytes() != &checkpoint.prefix_hash {
        return Err("checkpoint prefix BLAKE3 mismatch".into());
    }
    Ok(checkpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_or_reassigned_checkpoint_never_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let incoming = dir.path().join("untrusted.part");
        let stage = dir.path().join("private.stage");
        let meta = dir.path().join("private.checkpoint");
        fs::write(&incoming, vec![0x5b; IO_BLOCK * 2]).unwrap();
        let expected = Identity {
            sender: [1; 32],
            receiver: [2; 32],
            full_hash: [3; 32],
            total: 5 * 1024 * 1024,
        };
        let checkpoint = persist(&incoming, &stage, &meta, expected, 2 * IO_BLOCK as u64).unwrap();
        assert_eq!(recover(&stage, &meta, expected).unwrap(), checkpoint);
        assert!(recover(&stage, &meta, Identity { receiver: [4; 32], ..expected }).is_err());
        assert!(recover(&stage, &meta, Identity { sender: [5; 32], ..expected }).is_err());
        assert!(recover(&stage, &meta, Identity { full_hash: [6; 32], ..expected }).is_err());
        let mut changed = fs::read(&meta).unwrap();
        changed[12] ^= 1;
        fs::write(&meta, &changed).unwrap();
        assert!(recover(&stage, &meta, expected).is_err());
        changed[12] ^= 1;
        fs::write(&meta, &changed).unwrap();
        let mut changed_stage = OpenOptions::new().append(true).open(&stage).unwrap();
        changed_stage.write_all(b"x").unwrap();
        changed_stage.sync_all().unwrap();
        assert!(recover(&stage, &meta, expected).is_err());
        drop(changed_stage);
        let mut fixed = fs::read(&stage).unwrap();
        fixed.truncate(2 * IO_BLOCK);
        fixed[0] ^= 1;
        fs::write(&stage, &fixed).unwrap();
        assert!(recover(&stage, &meta, expected).is_err());
        assert!(Checkpoint::decode(&checkpoint.wire()[..WIRE_BYTES - 1]).is_err());
    }
}
