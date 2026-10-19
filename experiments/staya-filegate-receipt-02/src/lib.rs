//! STAYA FILEGATE-RECEIPT-02: real Core File Gate create + filesystem sync
//! + independent process readback + Ed25519 research receipt.
//!
//! NON-PRODUCTION. This experiment uses deterministic signer seeds, a known
//! temporary workspace, friendly single-user path handling and one OS host.
//! It does NOT attest a remote disk, a machine-wide lock, a real Akula grant,
//! power-loss persistence or host-install custody. The production Writer is
//! not replaced: the only user-vault mutation goes through gate::create.

pub mod peer_scope;

use aquilum_core::{Core, CoreEvent};
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use lesha_replication::{ProtectedSubject, ReceiptId, ReplicaSubject, VerifiedReplicaReceipt};
use lesha_types::{ChunkId, ClusterId, NodeGeneration, NodeId, NodeRef};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TEXT: &str = "STAYA-FILEGATE-RECEIPT-02: verified local replica\n";
const MAGIC: &[u8; 8] = b"STAYAR02";
const DOMAIN: &[u8] = b"akula/staya/filegate-receipt-02/v1";
const VAULT_SCOPE: [u8; 16] = [31; 16];
const RELATIVE: &str = "notes/replica.md";
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const FIELDS_LEN: usize = 8 + 16 + 32 + 8 + 8 + 16 + 32 + 32 + 16 + 32;
const WIRE_LEN: usize = FIELDS_LEN + 64;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReceiptError {
    MissingFile,
    MissingReceipt,
    InvalidWire,
    BadSignature,
    WrongIdentity,
    WrongGeneration,
    WrongSubject,
    CorruptBytes,
    UnsafeFile,
    Io,
    FileGateDenied,
}
pub type R<T> = Result<T, ReceiptError>;

pub fn node(tag: u8) -> NodeRef {
    NodeRef {
        cluster_id: ClusterId([1; 16]),
        node_id: NodeId([tag; 32]),
        generation: NodeGeneration(1),
    }
}

pub fn signer(tag: u8) -> SigningKey {
    // Test vector ONLY. Never derive private keys from node IDs in production.
    SigningKey::from_bytes(&[tag; 32])
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&Sha256::digest(bytes));
    hash
}

pub fn target() -> ProtectedSubject {
    let digest = sha256(TEXT.as_bytes());
    // SHA-256 LesHa ChunkId is intentionally distinct from Akula BLAKE3.
    ProtectedSubject {
        cluster_id: ClusterId([1; 16]),
        subject: ReplicaSubject::Chunk(ChunkId(digest)),
        subject_hash: digest,
    }
}

pub fn root(base: &Path, tag: u8) -> PathBuf {
    base.join(format!("node-{tag}")).join("vault")
}
pub fn appdata(base: &Path, tag: u8) -> PathBuf {
    base.join(format!("node-{tag}")).join("appdata")
}
pub fn file_path(base: &Path, tag: u8) -> PathBuf {
    root(base, tag).join(RELATIVE)
}
pub fn receipt_path(base: &Path, tag: u8) -> PathBuf {
    appdata(base, tag).join("receipts").join("replica.receipt")
}

fn io<T>(value: std::io::Result<T>) -> R<T> {
    value.map_err(|_| ReceiptError::Io)
}

fn sync_parent(path: &Path) -> R<()> {
    #[cfg(unix)]
    {
        let directory = path.parent().ok_or(ReceiptError::Io)?;
        io(File::open(directory))?.sync_all().map_err(|_| ReceiptError::Io)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        // No false durable-parent claim on unsupported OS.
        return Err(ReceiptError::FileGateDenied);
    }
    Ok(())
}

/// Open the same descriptor for bounded readback and check pre/post metadata.
/// Safe against ordinary accidental corruption, NOT hostile TOCTOU races.
fn read_bounded_file(path: &Path) -> R<Vec<u8>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            return Err(ReceiptError::MissingFile),
        Err(_) => return Err(ReceiptError::Io),
    };
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES {
        return Err(ReceiptError::UnsafeFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(ReceiptError::UnsafeFile);
        }
    }
    let mut handle = io(File::open(path))?;
    let before = handle.metadata().map_err(|_| ReceiptError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != meta.dev() || before.ino() != meta.ino() || before.nlink() != 1 {
            return Err(ReceiptError::UnsafeFile);
        }
    }
    let mut data = Vec::new();
    io((&mut handle).take(MAX_BYTES + 1).read_to_end(&mut data))?;
    let after = handle.metadata().map_err(|_| ReceiptError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(ReceiptError::UnsafeFile);
        }
    }
    if data.len() as u64 != before.len()
        || data.len() as u64 != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(ReceiptError::UnsafeFile);
    }
    Ok(data)
}

fn verify_expected_bytes(path: &Path) -> R<[u8; 32]> {
    let data = read_bounded_file(path)?;
    if sha256(&data) != target().subject_hash {
        return Err(ReceiptError::CorruptBytes);
    }
    let got_blake3 = *blake3::hash(&data).as_bytes();
    if got_blake3 != *blake3::hash(TEXT.as_bytes()).as_bytes() {
        return Err(ReceiptError::CorruptBytes);
    }
    Ok(got_blake3)
}

/// Write an actual test note ONLY through the existing Core File Gate.
/// Then additionally sync the containing directory and read back bytes.
/// No replicated/durable receipt is created by this function.
pub fn filegate_publish(base: &Path, tag: u8) -> R<()> {
    let vault = root(base, tag);
    let app = appdata(base, tag);
    io(fs::create_dir_all(vault.join("notes")))?;
    io(fs::create_dir_all(&app))?;
    let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().ok_or(ReceiptError::Io)?, 1)
        .map_err(|_| ReceiptError::FileGateDenied)?;

    let filename = file_path(base, tag);
    let written = gate::create(&core, &filename, TEXT, Source::Me, None)
        .map_err(|_| ReceiptError::FileGateDenied)?;
    let expected_b3 = blake3::hash(TEXT.as_bytes()).to_hex().to_string();
    if written.hash != expected_b3 {
        return Err(ReceiptError::CorruptBytes);
    }
    // The Core Gate syncs new file data. This additional parent sync and
    // readback is part of the scoped experiment, not a second writer.
    io(File::open(&filename))?.sync_all().map_err(|_| ReceiptError::Io)?;
    sync_parent(&filename)?;
    verify_expected_bytes(&filename)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedReceipt {
    pub node: NodeRef,
    pub storage_generation: u64,
    pub receipt_id: ReceiptId,
    pub sha256: [u8; 32],
    pub blake3: [u8; 32],
    signature: [u8; 64],
}

impl SignedReceipt {
    fn fields(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FIELDS_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.node.cluster_id.0);
        out.extend_from_slice(&self.node.node_id.0);
        out.extend_from_slice(&self.node.generation.0.to_be_bytes());
        out.extend_from_slice(&self.storage_generation.to_be_bytes());
        out.extend_from_slice(&self.receipt_id.0);
        out.extend_from_slice(&self.sha256);
        out.extend_from_slice(&self.blake3);
        out.extend_from_slice(&VAULT_SCOPE);
        out.extend_from_slice(blake3::hash(RELATIVE.as_bytes()).as_bytes());
        debug_assert_eq!(out.len(), FIELDS_LEN);
        out
    }
    fn signing_material(&self) -> Vec<u8> {
        let mut out = DOMAIN.to_vec();
        out.extend_from_slice(&self.fields());
        out
    }
    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = self.fields();
        out.extend_from_slice(&self.signature);
        out
    }
    pub fn from_wire(bytes: &[u8]) -> R<Self> {
        if bytes.len() != WIRE_LEN || &bytes[..8] != MAGIC {
            return Err(ReceiptError::InvalidWire);
        }
        fn take<const N: usize>(wire: &[u8], cursor: &mut usize) -> [u8; N] {
            let mut out = [0u8; N];
            out.copy_from_slice(&wire[*cursor..*cursor+N]);
            *cursor += N;
            out
        }
        let mut at = 8;
        let cluster_id = ClusterId(take::<16>(bytes, &mut at));
        let node_id = NodeId(take::<32>(bytes, &mut at));
        let generation = NodeGeneration(u64::from_be_bytes(take::<8>(bytes, &mut at)));
        let storage_generation = u64::from_be_bytes(take::<8>(bytes, &mut at));
        let receipt_id = ReceiptId(take::<16>(bytes, &mut at));
        let sha256 = take::<32>(bytes, &mut at);
        let blake3 = take::<32>(bytes, &mut at);
        if take::<16>(bytes, &mut at) != VAULT_SCOPE
            || take::<32>(bytes, &mut at) != *blake3::hash(RELATIVE.as_bytes()).as_bytes()
        {
            return Err(ReceiptError::WrongIdentity);
        }
        let signature = take::<64>(bytes, &mut at);
        debug_assert_eq!(at, WIRE_LEN);
        Ok(Self {
            node: NodeRef { cluster_id, node_id, generation },
            storage_generation, receipt_id, sha256, blake3, signature,
        })
    }
}

/// The signed receipt is stored in *isolated appdata*, not the user's vault.
/// No replacement of an existing receipt: ambiguous attempts remain visible.
pub fn sign_and_persist(base: &Path, tag: u8) -> R<()> {
    let filename = file_path(base, tag);
    verify_expected_bytes(&filename)?;
    let mut claim = SignedReceipt {
        node: node(tag),
        storage_generation: 1,
        receipt_id: ReceiptId([tag; 16]),
        sha256: target().subject_hash,
        blake3: *blake3::hash(TEXT.as_bytes()).as_bytes(),
        signature: [0; 64],
    };
    claim.signature = signer(tag).sign(&claim.signing_material()).to_bytes();
    let path = receipt_path(base, tag);
    io(fs::create_dir_all(path.parent().ok_or(ReceiptError::Io)?))?;
    let mut receipt = io(OpenOptions::new().write(true).create_new(true).open(&path))?;
    io(receipt.write_all(&claim.to_wire()))?;
    io(receipt.sync_all())?;
    sync_parent(&path)?;
    Ok(())
}

/// Independent reopen/readback. The caller chooses the trusted node and key:
/// the recorded message NEVER enrolls itself. This fixture has no remote
/// attestation, installer-owned host lock, power-loss test or persisted ACL.
pub fn admit_after_restart(base: &Path, tag: u8) -> R<VerifiedReplicaReceipt> {
    let bytes = match fs::read(receipt_path(base, tag)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            return Err(ReceiptError::MissingReceipt),
        Err(_) => return Err(ReceiptError::Io),
    };
    let claimed = SignedReceipt::from_wire(&bytes)?;
    let trusted = node(tag);
    if claimed.node != trusted || claimed.receipt_id != ReceiptId([tag; 16]) {
        return Err(ReceiptError::WrongIdentity);
    }
    if claimed.storage_generation != 1 || claimed.node.generation != NodeGeneration(1) {
        return Err(ReceiptError::WrongGeneration);
    }
    let vk: VerifyingKey = signer(tag).verifying_key();
    vk.verify_strict(
        &claimed.signing_material(),
        &Signature::from_bytes(&claimed.signature),
    ).map_err(|_| ReceiptError::BadSignature)?;
    if claimed.sha256 != target().subject_hash
        || claimed.blake3 != *blake3::hash(TEXT.as_bytes()).as_bytes()
    {
        return Err(ReceiptError::WrongSubject);
    }
    verify_expected_bytes(&file_path(base, tag))?;
    // The trust-boundary constructor remains PRIVATE TO THIS EXPERIMENT.
    // Real remote proofs cannot inspect a remote filesystem this way.
    Ok(VerifiedReplicaReceipt::new_prevalidated(
        claimed.receipt_id,
        target().cluster_id,
        trusted,
        target().subject,
        target().subject_hash,
        claimed.storage_generation,
    ))
}
