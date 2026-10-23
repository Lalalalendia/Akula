//! Stable non-root service identity. The private key never enters settings,
//! logs, SSH arguments or the user Vault. No auto-import from untrusted files.

use super::NodeV1Error;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const KEY_FILE: &str = "node-identity.pk8";
const DOMAIN: &[u8] = b"Akula/NodeV1/local-health-proof/v1\0";

pub struct NodeIdentity {
    key: Ed25519KeyPair,
    public: [u8; 32],
}
impl NodeIdentity {
    /// Only an independently approved installer enrollment may create a key.
    pub fn open_or_create(state: &Path) -> Result<Self, NodeV1Error> {
        Self::load(state, true, false)
    }

    /// Only a non-root, one-shot installer enrollment can create a *new*
    /// identity. Existing keys, including previously valid ones, are HOLD.
    pub fn enroll_first(state: &Path) -> Result<Self, NodeV1Error> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Err(NodeV1Error::Hold("Node enrollment refuses root"));
        }
        Self::load(state, true, true)
    }

    /// A running Node must refuse missing or revoked identity, never rotate.
    pub fn open_provisioned(state: &Path) -> Result<Self, NodeV1Error> {
        Self::load(state, false, false)
    }

    fn load(state: &Path, allow_create: bool, require_absent: bool)
        -> Result<Self, NodeV1Error>
    {
        let stat = fs::symlink_metadata(state)?;
        if !stat.file_type().is_dir()
            || stat.uid() != rustix::process::geteuid().as_raw()
            || stat.permissions().mode() & 0o7777 != 0o700
        {
            return Err(NodeV1Error::Hold("unsafe Node StateDirectory"));
        }
        let dir = File::from(open(
            state, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(std::io::Error::from)?);
        let physical = dir.metadata()?;
        if physical.dev() != stat.dev() || physical.ino() != stat.ino() {
            return Err(NodeV1Error::Hold("Node StateDirectory changed"));
        }
        let mut key_file = match openat(
            &dir, KEY_FILE, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ) {
            Ok(fd) if !require_absent => File::from(fd),
            Ok(_) => return Err(NodeV1Error::Hold("Node identity already enrolled")),
            Err(rustix::io::Errno::NOENT) if allow_create => {
                let bytes = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                    .map_err(|_| NodeV1Error::Hold("Node key generation failed"))?;
                let mut fresh = File::from(openat(
                    &dir, KEY_FILE, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL
                        | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                ).map_err(std::io::Error::from)?);
                fresh.write_all(bytes.as_ref())?;
                fresh.sync_all()?;
                dir.sync_all()?;
                drop(fresh);
                File::from(openat(
                    &dir, KEY_FILE, OFlags::RDONLY | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC, Mode::empty(),
                ).map_err(std::io::Error::from)?)
            }
            Err(rustix::io::Errno::NOENT) => {
                return Err(NodeV1Error::Hold("Node identity not enrolled"));
            }
            Err(error) => return Err(NodeV1Error::Io(error.into())),
        };
        let meta = key_file.metadata()?;
        if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.nlink() != 1 || meta.permissions().mode() & 0o7777 != 0o600
            || meta.len() > 256
        {
            return Err(NodeV1Error::Hold("unsafe Node identity file"));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        key_file.take(257).read_to_end(&mut bytes)?;
        if bytes.len() > 256 {
            return Err(NodeV1Error::Hold("oversize Node identity"));
        }
        let key = Ed25519KeyPair::from_pkcs8(&bytes)
            .map_err(|_| NodeV1Error::Hold("corrupt Node identity"))?;
        let public: [u8; 32] = key.public_key().as_ref().try_into()
            .map_err(|_| NodeV1Error::Hold("invalid Node key length"))?;
        if public == [0; 32] {
            return Err(NodeV1Error::Hold("invalid Node public key"));
        }
        Ok(Self { key, public })
    }

    pub fn public_key(&self) -> [u8; 32] { self.public }

    /// Only internal callers can sign a domain-separated local receipt.
    pub(super) fn sign_receipt_claim(&self, claim: &[u8]) -> [u8; 64] {
        let signature = self.key.sign(claim);
        let mut out = [0; 64];
        out.copy_from_slice(signature.as_ref());
        out
    }

    pub fn sign_health(&self, nonce: [u8; 32])
        -> Result<[u8; 64], NodeV1Error>
    {
        if nonce == [0; 32] {
            return Err(NodeV1Error::Hold("empty health nonce"));
        }
        let mut claim = Vec::with_capacity(DOMAIN.len() + 64);
        claim.extend_from_slice(DOMAIN);
        claim.extend_from_slice(&self.public);
        claim.extend_from_slice(&nonce);
        self.key.sign(&claim).as_ref().try_into()
            .map_err(|_| NodeV1Error::Hold("invalid Node signature"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_node_key_survives_reopen_and_rejects_wrong_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("private-state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(NodeIdentity::open_provisioned(&state).is_err());
        let first = NodeIdentity::enroll_first(&state).unwrap();
        assert!(NodeIdentity::enroll_first(&state).is_err());
        let key = first.public_key();
        let signature = first.sign_health([9; 32]).unwrap();
        use ring::signature::{UnparsedPublicKey, ED25519};
        let mut claim = Vec::from(DOMAIN);
        claim.extend_from_slice(&key);
        claim.extend_from_slice(&[9; 32]);
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&claim, &signature).unwrap();
        drop(first);
        assert_eq!(NodeIdentity::open_provisioned(&state).unwrap().public_key(), key);
        assert_eq!(NodeIdentity::open_or_create(&state).unwrap().public_key(), key);
        let key_file = state.join(KEY_FILE);
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(NodeIdentity::open_or_create(&state).is_err());
        assert!(NodeIdentity::open_or_create(&state.join("missing")).is_err());
    }
}
