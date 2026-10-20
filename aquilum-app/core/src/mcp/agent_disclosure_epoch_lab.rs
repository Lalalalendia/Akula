//! AKAI-AUTH-01d0: test-only SQLite disclosure epoch falsifiers.
//!
//! This module is cfg(test) ONLY. No live HTTP writer, privileged broker,
//! enrolled root, persistent lease or production permission is introduced.
//!
//! The candidate uses BEGIN IMMEDIATE on BOTH independent host-private
//! SQLite stores before reauthentication and before returning bytes to a
//! transport. A trusted issuer committing through either database cannot
//! acknowledge revoke while the transport write is inside both transactions.
//! This is a *cooperative* epoch-ordering experiment, not a trusted OS
//! broker or a guarantee of client delivery.
//!
//! WARNING: a same-user process replacing SQLite/WAL/SHM, OS death, a writer
//! that bypasses these stores, or revocation across another host remain
//! explicitly UNSOLVED. A partial socket write is irrecoverable.

use super::agent_principals::{AgentCredentialStore, AuthenticatedAgent};
use super::agent_vault_grants::{
    AgentReadLease, AgentVaultGrantStore, HostVaultRegistration,
};
use rusqlite::{Connection, TransactionBehavior};
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DisclosureOutcome {
    Sent,
    Denied,
    PartialDisclosureHold,
}

struct CountingWriter<'a, W: Write> {
    inner: &'a mut W,
    accepted: usize,
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.accepted = self.accepted.saturating_add(n);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// TEST-ONLY proof that revocation ACK cannot outrun transport write when
/// ALL trustworthy issuers modify the same two SQLite databases. The caller
/// must already have a checked response candidate. No physical root is
/// revalidated here, and no production network path invokes this method.
///
/// Strict order: credential DB writer epoch, then vault grant DB writer
/// epoch. An independent real broker would need to enforce the same order.
/// Never expose the two transactions or a writer outside the closure.
fn disclose_cooperative<W: Write>(
    credential_epoch_db: &mut Connection,
    grant_epoch_db: &mut Connection,
    credentials: &AgentCredentialStore,
    grants: &AgentVaultGrantStore,
    bearer: &str,
    original: AuthenticatedAgent,
    lease: AgentReadLease,
    registration: HostVaultRegistration,
    out: &mut W,
    emit: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> DisclosureOutcome {
    let Ok(_credential_tx) = credential_epoch_db
        .transaction_with_behavior(TransactionBehavior::Immediate)
    else {
        return DisclosureOutcome::Denied;
    };
    let Ok(_grant_tx) = grant_epoch_db
        .transaction_with_behavior(TransactionBehavior::Immediate)
    else {
        return DisclosureOutcome::Denied;
    };

    // If either revoke ACKed before these locks, its durable epoch/secret
    // state must be observed; older candidates cannot be sent.
    let Ok(fresh) = credentials.authenticate(bearer) else {
        return DisclosureOutcome::Denied;
    };
    if fresh != original
        || grants.revalidate_read_lease(&lease, fresh, registration).is_err()
    {
        return DisclosureOutcome::Denied;
    }

    let mut counted = CountingWriter { inner: out, accepted: 0 };
    let result = emit(&mut counted).and_then(|()| counted.flush());
    // Even an I/O error with zero *reported* bytes might have reached the
    // socket before an interrupted write returned Err. Buffered flush failures
    // are ambiguous too. Do NOT interpret either as provably zero disclosure.
    match result {
        Ok(()) => DisclosureOutcome::Sent,
        Err(_) => DisclosureOutcome::PartialDisclosureHold,
    }
    // Both transactions roll back only after emit AND flush returned.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::agent_principals::{
        AgentCredentialId, AgentPrincipalId, OneTimeAgentBearer,
    };
    use crate::plugins::{Capability, GrantScope, VaultId};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    struct Fixture {
        credential_db: PathBuf,
        grant_db: PathBuf,
        credentials: AgentCredentialStore,
        grants: AgentVaultGrantStore,
        token: OneTimeAgentBearer,
        credential_id: AgentCredentialId,
        principal: AgentPrincipalId,
        agent: AuthenticatedAgent,
        registration: HostVaultRegistration,
        lease: AgentReadLease,
        epoch: u64,
        _dir: TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let credential_db = dir.path().join("principals.sqlite3");
        let grant_db = dir.path().join("grants.sqlite3");
        let mut credentials = AgentCredentialStore::open(&credential_db).unwrap();
        let mut issued = credentials.issue_principal("agent-A", None).unwrap();
        let token = issued.take_bearer_once().unwrap();
        let agent = credentials.authenticate(token.as_str()).unwrap();
        let registration =
            HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [31; 32])
                .unwrap();
        let mut grants = AgentVaultGrantStore::provision_new(&grant_db).unwrap();
        grants.register_host_vault(registration).unwrap();
        let epoch = grants.grant_notes_read(
            issued.principal, registration, Capability::NotesRead,
            GrantScope::WholeVault, 0, None,
        ).unwrap();
        let lease = grants.authorize_notes_read(agent, registration).unwrap();
        Fixture {
            credential_db, grant_db, credentials, grants, token,
            credential_id: issued.credential, principal: issued.principal,
            agent, registration, lease, epoch, _dir: dir,
        }
    }

    fn fence_connections(f: &Fixture) -> (Connection, Connection) {
        let first = Connection::open(&f.credential_db).unwrap();
        let second = Connection::open(&f.grant_db).unwrap();
        first.busy_timeout(Duration::from_secs(3)).unwrap();
        second.busy_timeout(Duration::from_secs(3)).unwrap();
        (first, second)
    }

    #[test]
    fn two_independent_revoke_acks_cannot_precede_complete_emit() {
        let f = fixture();
        // Independent connections also model independent OS processes under
        // the same cooperative SQLite/WAL storage contract.
        let mut credential_revoke = AgentCredentialStore::open(&f.credential_db).unwrap();
        let mut grant_revoke = AgentVaultGrantStore::open_existing(&f.grant_db).unwrap();
        let (mut guard_cred, mut guard_grant) = fence_connections(&f);

        let rendezvous = Arc::new(Barrier::new(3));
        let (starting, began) = mpsc::channel::<&'static str>();
        let (acks, received) = mpsc::channel::<&'static str>();
        let cred_start = Arc::clone(&rendezvous);
        let cred_began = starting.clone();
        let cred_acks = acks.clone();
        let credential_id = f.credential_id;
        let credential_worker = thread::spawn(move || {
            cred_start.wait();
            cred_began.send("credential").unwrap();
            credential_revoke.revoke(credential_id).unwrap();
            cred_acks.send("credential").unwrap();
        });
        let grant_start = Arc::clone(&rendezvous);
        let grant_began = starting;
        let grant_acks = acks;
        let principal = f.principal;
        let receipt = f.registration;
        let version = f.epoch;
        let grant_worker = thread::spawn(move || {
            grant_start.wait();
            grant_began.send("grant").unwrap();
            grant_revoke.revoke_notes_read(principal, receipt, version).unwrap();
            grant_acks.send("grant").unwrap();
        });

        let mut sent = Vec::new();
        let outcome = disclose_cooperative(
            &mut guard_cred, &mut guard_grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut sent,
            |out| {
                rendezvous.wait();
                // Both independent writers are attempting to revoke while
                // the two BEGIN IMMEDIATE transactions are held.
                let first = began.recv_timeout(Duration::from_secs(2)).unwrap();
                let second = began.recv_timeout(Duration::from_secs(2)).unwrap();
                assert_ne!(first, second);
                thread::sleep(Duration::from_millis(80));
                assert!(received.try_recv().is_err(), "revoke acknowledged before emission");
                out.write_all(b"authorized response candidate")?;
                assert!(received.try_recv().is_err(), "revoke overtook transport write");
                Ok(())
            },
        );
        assert_eq!(outcome, DisclosureOutcome::Sent);
        assert_eq!(sent, b"authorized response candidate");
        assert_ne!(
            received.recv_timeout(Duration::from_secs(3)).unwrap(),
            received.recv_timeout(Duration::from_secs(3)).unwrap(),
        );
        credential_worker.join().unwrap();
        grant_worker.join().unwrap();
        assert!(f.credentials.authenticate(f.token.as_str()).is_err());
        assert!(f.grants.authorize_notes_read(f.agent, f.registration).is_err());
    }

    #[test]
    fn revoked_before_fence_never_invokes_transport() {
        let mut f = fixture();
        f.credentials.revoke(f.credential_id).unwrap();
        let (mut credential, mut grant) = fence_connections(&f);
        let mut output = Vec::<u8>::new();
        let mut called = false;
        let response = disclose_cooperative(
            &mut credential, &mut grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut output,
            |_out| { called = true; Ok(()) },
        );
        assert_eq!(response, DisclosureOutcome::Denied);
        assert!(!called);
        assert!(output.is_empty());
    }

    #[test]
    fn revoked_grant_before_fence_never_invokes_transport() {
        let mut f = fixture();
        f.grants.revoke_notes_read(
            f.principal, f.registration, f.epoch,
        ).unwrap();
        let (mut credential, mut grant) = fence_connections(&f);
        let mut output = Vec::<u8>::new();
        let response = disclose_cooperative(
            &mut credential, &mut grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut output,
            |_out| panic!("unauthorized response emission"),
        );
        assert_eq!(response, DisclosureOutcome::Denied);
        assert!(output.is_empty());
    }

    struct FailingTransport {
        accepted: usize,
        bytes: Vec<u8>,
    }

    impl Write for FailingTransport {
        fn write(&mut self, src: &[u8]) -> io::Result<usize> {
            if self.accepted != 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "disconnected"));
            }
            let n = src.len().min(4);
            self.bytes.extend_from_slice(&src[..n]);
            self.accepted += n;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partially_written_response_is_ambiguous_hold_not_retry() {
        let f = fixture();
        let (mut cred, mut grant) = fence_connections(&f);
        let mut output = FailingTransport { accepted: 0, bytes: Vec::new() };
        let result = disclose_cooperative(
            &mut cred, &mut grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut output,
            |out| out.write_all(b"abcdef-private-note"),
        );
        assert_eq!(result, DisclosureOutcome::PartialDisclosureHold);
        assert_eq!(output.bytes, b"abcd");
        // The four bytes cannot be withdrawn. An installed implementation
        // must close this HTTP connection, not retry or silently claim deny.
    }
    struct AmbiguousZeroReport;
    impl Write for AmbiguousZeroReport {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "report uncertain"))
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn zero_reported_bytes_do_not_prove_zero_disclosure() {
        let f = fixture();
        let (mut cred, mut grant) = fence_connections(&f);
        let mut sink = AmbiguousZeroReport;
        let disposition = disclose_cooperative(
            &mut cred, &mut grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut sink,
            |out| out.write_all(b"private"),
        );
        assert_eq!(disposition, DisclosureOutcome::PartialDisclosureHold);
    }

    #[derive(Default)]
    struct FlushFailure(Vec<u8>);
    impl Write for FlushFailure {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"))
        }
    }

    #[test]
    fn buffered_flush_failure_is_ambiguous_hold() {
        let f = fixture();
        let (mut cred, mut grant) = fence_connections(&f);
        let mut sink = FlushFailure::default();
        let disposition = disclose_cooperative(
            &mut cred, &mut grant, &f.credentials, &f.grants,
            f.token.as_str(), f.agent, f.lease, f.registration, &mut sink,
            |out| out.write_all(b"private"),
        );
        assert_eq!(disposition, DisclosureOutcome::PartialDisclosureHold);
        assert_eq!(sink.0, b"private");
    }

}
