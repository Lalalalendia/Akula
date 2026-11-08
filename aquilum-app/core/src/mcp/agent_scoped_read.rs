//! AKAI-AUTH-01b1a: descriptor-relative, grant-checked read candidate.
//!
//! This code is INERT: no HTTP, stdio, GUI or MCP tool routes invoke it.
//! Crucially there is no production constructor for HostPinnedReadRoot until
//! an independently protected installer-owned custody witness can mint one.
//! A client string, bare VaultId, pathname or matching UUID is NOT authority.
//!
//! A bounded read is pinned to an OS directory handle and revalidates both the
//! credential and durable grant before bytes leave this function. It is NOT
//! yet atomic with sending the HTTP reply, and cannot promise post-response
//! revocation fencing. That is a separate security gate.

#![allow(dead_code)]

use super::agent_principals::AgentCredentialStore;
use super::agent_vault_grants::{AgentVaultGrantStore, HostVaultRegistration};
use crate::plugins::VaultId;
use crate::sync::{PinnedReadOnlyRoot, SyncRelativePath};
use crate::sync::filegate_lock_probe::PinnedProbeRoot;
#[cfg(test)]
use std::path::Path;

const MAX_MCP_NOTE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopedReadError {
    Denied,
}

pub(crate) struct CheckedNote {
    pub(crate) path: String,
    pub(crate) content: String,
    pub(crate) revision: String,
}

/// An independently enrolled vault and an open root handle. The registration
/// receipt must originate from a *trusted* host enrollment. A caller-controlled
/// path or dynamically generated digest cannot create a trusted instance.
pub(crate) struct HostPinnedReadRoot {
    receipt: HostVaultRegistration,
    identity: (u64, u64),
    probe: PinnedProbeRoot,
    reader: PinnedReadOnlyRoot,
}

impl HostPinnedReadRoot {
    fn validate_root(&self) -> Result<(), ScopedReadError> {
        self.probe.recheck_root().map_err(|_| ScopedReadError::Denied)?;
        if root_identity(&self.reader)? != self.identity
            || self.probe.identity() != self.identity
        {
            return Err(ScopedReadError::Denied);
        }
        Ok(())
    }

    /// Read a *single exact canonical relative Markdown path*. No fuzzy-title
    /// resolution, batch partial results, implicit workspace selection, search
    /// or notes outside this pinned, trusted host registration.
    ///
    /// This is not an HTTP authorization/response fence; no network route
    /// calls it until the trust witness and final-send guard exist.
    pub(crate) fn read_checked(
        &self,
        credentials: &AgentCredentialStore,
        grants: &AgentVaultGrantStore,
        bearer: &str,
        requested_vault: VaultId,
        requested_note: &str,
    ) -> Result<CheckedNote, ScopedReadError> {
        self.read_with_post_read_probe(
            credentials, grants, bearer, requested_vault, requested_note, || {},
        )
    }

    fn read_with_post_read_probe(
        &self,
        credentials: &AgentCredentialStore,
        grants: &AgentVaultGrantStore,
        bearer: &str,
        requested_vault: VaultId,
        requested_note: &str,
        before_final_check: impl FnOnce(),
    ) -> Result<CheckedNote, ScopedReadError> {
        // Do not reveal path/existence or read filesystem bytes for a wrong
        // vault, stale credential or missing grant.
        if requested_vault != self.receipt.vault_id() {
            return Err(ScopedReadError::Denied);
        }
        let first = credentials.authenticate(bearer)
            .map_err(|_| ScopedReadError::Denied)?;
        self.validate_root()?;
        let lease = grants.authorize_notes_read(first, self.receipt)
            .map_err(|_| ScopedReadError::Denied)?;

        let relative = strict_markdown_path(requested_note)?;
        let snapshot = self.reader.read_bounded_single_link(&relative, MAX_MCP_NOTE_BYTES)
            .map_err(|_| ScopedReadError::Denied)?;

        // The test hook lets us deterministically introduce a revoke/rename
        // in the critical interval. No hook is offered to real HTTP clients.
        before_final_check();

        self.validate_root()?;
        let fresh = credentials.authenticate(bearer)
            .map_err(|_| ScopedReadError::Denied)?;
        if fresh != first {
            return Err(ScopedReadError::Denied);
        }
        grants.revalidate_read_lease(&lease, fresh, self.receipt)
            .map_err(|_| ScopedReadError::Denied)?;
        let content = String::from_utf8(snapshot.bytes)
            .map_err(|_| ScopedReadError::Denied)?;
        Ok(CheckedNote {
            path: relative.as_str().to_owned(),
            content,
            revision: snapshot.content_hash,
        })
    }

    // Test-only root binding. Production must NOT have an equivalent that
    // accepts a naked expected physical identity from an MCP or UI argument.
    // The future installer-witness adapter must obtain this from independently
    // protected custody, without self-certifying from the same candidate root.
    #[cfg(test)]
    pub(crate) fn from_test_host_claim(
        root_path: &Path,
        receipt: HostVaultRegistration,
        expected_root_identity: (u64, u64),
    ) -> Result<Self, ScopedReadError> {
        let probe = PinnedProbeRoot::open(root_path)
            .map_err(|_| ScopedReadError::Denied)?;
        let reader = PinnedReadOnlyRoot::open(root_path)
            .map_err(|_| ScopedReadError::Denied)?;
        let root = Self { receipt, identity: expected_root_identity, probe, reader };
        root.validate_root()?;
        Ok(root)
    }
}

fn root_identity(reader: &PinnedReadOnlyRoot) -> Result<(u64, u64), ScopedReadError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = reader.root_metadata().map_err(|_| ScopedReadError::Denied)?;
        return Ok((metadata.dev(), metadata.ino()));
    }
    #[cfg(windows)]
    {
        let (volume, file) = reader.root_win_identity()
            .map_err(|_| ScopedReadError::Denied)?;
        return Ok((u64::from(volume), file));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = reader;
        Err(ScopedReadError::Denied)
    }
}

fn strict_markdown_path(path: &str) -> Result<SyncRelativePath, ScopedReadError> {
    if path.is_empty() || path.len() > 1024 || !path.ends_with(".md") {
        return Err(ScopedReadError::Denied);
    }
    let relative = SyncRelativePath::parse(path).map_err(|_| ScopedReadError::Denied)?;
    if relative.as_str() != path
        || relative.as_str().split('/').any(|part| part.starts_with('.'))
    {
        return Err(ScopedReadError::Denied);
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{Capability, GrantScope};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        path_x: PathBuf,
        path_y: PathBuf,
        credential_path: PathBuf,
        grant_path: PathBuf,
        root_x: HostPinnedReadRoot,
        root_y: HostPinnedReadRoot,
        credentials: AgentCredentialStore,
        grants: AgentVaultGrantStore,
        key_a: super::super::agent_principals::OneTimeAgentBearer,
        key_b: super::super::agent_principals::OneTimeAgentBearer,
        credential_a: super::super::agent_principals::AgentCredentialId,
        principal_a: super::super::agent_principals::AgentPrincipalId,
        x_epoch: u64,
        _dir: TempDir,
    }

    fn enrolled(root: &Path, marker: u8) -> HostPinnedReadRoot {
        let receipt = HostVaultRegistration::from_host_custody(
            VaultId::new_for_host(), [marker; 32],
        ).unwrap();
        let probe = PinnedProbeRoot::open(root).unwrap();
        let claim = probe.identity();
        drop(probe);
        HostPinnedReadRoot::from_test_host_claim(root, receipt, claim).unwrap()
    }

    fn setup() -> Fixture {
        let dir = tempdir().unwrap();
        let path_x = dir.path().join("vault-X");
        let path_y = dir.path().join("vault-Y");
        fs::create_dir_all(path_x.join("sub")).unwrap();
        fs::create_dir_all(path_y.join("sub")).unwrap();
        fs::write(path_x.join("sub/note.md"), "Only A may read X").unwrap();
        fs::write(path_y.join("sub/note.md"), "Only B may read Y").unwrap();
        fs::write(path_y.join("hidden.md"), "foreign secret").unwrap();
        let root_x = enrolled(&path_x, 11);
        let root_y = enrolled(&path_y, 22);
        let credential_path = dir.path().join("credentials.sqlite3");
        let grant_path = dir.path().join("grants.sqlite3");
        let mut credentials = AgentCredentialStore::open(&credential_path).unwrap();
        let mut a = credentials.issue_principal("Agent A", None).unwrap();
        let mut b = credentials.issue_principal("Agent B", None).unwrap();
        let key_a = a.take_bearer_once().unwrap();
        let key_b = b.take_bearer_once().unwrap();
        let credential_a = a.credential;
        let principal_a = a.principal;
        let mut grants = AgentVaultGrantStore::provision_new(&grant_path).unwrap();
        grants.register_host_vault(root_x.receipt).unwrap();
        grants.register_host_vault(root_y.receipt).unwrap();
        let x_epoch = grants.grant_notes_read(
            a.principal, root_x.receipt, Capability::NotesRead,
            GrantScope::WholeVault, 0, None,
        ).unwrap();
        grants.grant_notes_read(
            b.principal, root_y.receipt, Capability::NotesRead,
            GrantScope::WholeVault, 0, None,
        ).unwrap();
        Fixture {
            path_x, path_y, credential_path, grant_path,
            root_x, root_y, credentials, grants, key_a, key_b,
            credential_a, principal_a, x_epoch, _dir: dir,
        }
    }

    #[test]
    fn two_agents_two_opened_roots_never_cross_read() {
        let f = setup();
        let x = f.root_x.read_checked(
            &f.credentials, &f.grants, f.key_a.as_str(),
            f.root_x.receipt.vault_id(), "sub/note.md",
        ).unwrap();
        assert_eq!(x.content, "Only A may read X");
        assert_eq!(x.path, "sub/note.md");
        assert_eq!(x.revision.len(), 64);
        let y = f.root_y.read_checked(
            &f.credentials, &f.grants, f.key_b.as_str(),
            f.root_y.receipt.vault_id(), "sub/note.md",
        ).unwrap();
        assert_eq!(y.content, "Only B may read Y");

        for (root, token, vault) in [
            (&f.root_x, f.key_b.as_str(), f.root_x.receipt.vault_id()),
            (&f.root_y, f.key_a.as_str(), f.root_y.receipt.vault_id()),
            (&f.root_x, f.key_a.as_str(), f.root_y.receipt.vault_id()),
        ] {
            assert!(matches!(
                root.read_checked(&f.credentials, &f.grants, token, vault, "sub/note.md"),
                Err(ScopedReadError::Denied),
            ));
        }
        // A raw path into a foreign root is never an alternative identity.
        let wrong = f.path_y.join("hidden.md").to_string_lossy().to_string();
        assert!(matches!(
            f.root_x.read_checked(
                &f.credentials, &f.grants, f.key_a.as_str(),
                f.root_x.receipt.vault_id(), &wrong,
            ),
            Err(ScopedReadError::Denied),
        ));
    }

    #[test]
    fn accepts_only_exact_nonhidden_portable_markdown_paths() {
        let f = setup();
        for path in [
            "../hidden.md", "sub/../../hidden.md", "./sub/note.md",
            "sub//note.md", "sub\\note.md", ".trash/item.md",
            ".git/config.md", "notes/../sub/note.md", "sub/note.txt",
            "", "C:/secret.md", "/etc/private.md",
        ] {
            assert!(matches!(
                f.root_x.read_checked(
                    &f.credentials, &f.grants, f.key_a.as_str(),
                    f.root_x.receipt.vault_id(), path,
                ),
                Err(ScopedReadError::Denied),
            ), "must reject {path}");
        }
    }

    #[test]
    fn revoke_between_open_and_return_discards_already_read_bytes() {
        let f = setup();
        let mut concurrent = AgentVaultGrantStore::open_existing(&f.grant_path).unwrap();
        let denied = f.root_x.read_with_post_read_probe(
            &f.credentials, &f.grants, f.key_a.as_str(),
            f.root_x.receipt.vault_id(), "sub/note.md",
            || {
                concurrent.revoke_notes_read(
                    f.principal_a, f.root_x.receipt, f.x_epoch,
                ).unwrap();
            },
        );
        assert!(matches!(denied, Err(ScopedReadError::Denied)));
    }

    #[test]
    fn revoked_credential_cannot_reuse_an_old_grant() {
        let f = setup();
        let mut other = AgentCredentialStore::open(&f.credential_path).unwrap();
        let denied = f.root_x.read_with_post_read_probe(
            &f.credentials, &f.grants, f.key_a.as_str(),
            f.root_x.receipt.vault_id(), "sub/note.md",
            || { other.revoke(f.credential_a).unwrap(); },
        );
        assert!(matches!(denied, Err(ScopedReadError::Denied)));
    }

    #[test]
    fn renamed_registered_root_does_not_read_replacement_path() {
        let f = setup();
        let moved = f.path_x.with_extension("moved");
        let denied = f.root_x.read_with_post_read_probe(
            &f.credentials, &f.grants, f.key_a.as_str(),
            f.root_x.receipt.vault_id(), "sub/note.md",
            || {
                fs::rename(&f.path_x, &moved).unwrap();
                fs::create_dir(&f.path_x).unwrap();
                fs::create_dir(f.path_x.join("sub")).unwrap();
                fs::write(f.path_x.join("sub/note.md"), "substituted").unwrap();
            },
        );
        assert!(matches!(denied, Err(ScopedReadError::Denied)));
    }

    #[test]
    fn hardlinked_file_from_other_vault_is_not_readable() {
        let f = setup();
        let external = f.path_y.join("hidden.md");
        let imported_alias = f.path_x.join("sub/hardlink.md");
        fs::hard_link(&external, &imported_alias).unwrap();
        assert!(matches!(
            f.root_x.read_checked(
                &f.credentials, &f.grants, f.key_a.as_str(),
                f.root_x.receipt.vault_id(), "sub/hardlink.md",
            ),
            Err(ScopedReadError::Denied),
        ));
        assert_eq!(fs::read_to_string(&external).unwrap(), "foreign secret");
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_relative_read_refuses_symlink_escape() {
        use std::os::unix::fs::symlink;
        let f = setup();
        let outsider = f.path_y.join("hidden.md");
        symlink(&outsider, f.path_x.join("sub/escape.md")).unwrap();
        assert!(matches!(
            f.root_x.read_checked(
                &f.credentials, &f.grants, f.key_a.as_str(),
                f.root_x.receipt.vault_id(), "sub/escape.md",
            ),
            Err(ScopedReadError::Denied),
        ));
        symlink(&f.path_y, f.path_x.join("external")).unwrap();
        assert!(matches!(
            f.root_x.read_checked(
                &f.credentials, &f.grants, f.key_a.as_str(),
                f.root_x.receipt.vault_id(), "external/hidden.md",
            ),
            Err(ScopedReadError::Denied),
        ));
    }

    #[test]
    fn stale_root_claim_never_gets_reader() {
        let f = setup();
        let wrong = (f.root_x.identity.0, f.root_x.identity.1.wrapping_add(1));
        assert!(matches!(
            HostPinnedReadRoot::from_test_host_claim(
                &f.path_x, f.root_x.receipt, wrong,
            ),
            Err(ScopedReadError::Denied),
        ));
    }
}
