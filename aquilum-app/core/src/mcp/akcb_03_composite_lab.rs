//! AKCB-03: test-only two-source disclosure and derived-output lineage laboratory.
//!
//! This is not a production search, MCP route, Host Broker or credential issuer.
//! The registrations below are deliberately synthetic cfg(test) host fixtures.
//! A title/count/snippet is a disclosure; all sources must still be authorized
//! when a candidate is emitted, not just while its bytes are being prepared.

use super::agent_principals::{AgentCredentialStore, AuthenticatedAgent};
use super::agent_vault_grants::{AgentReadLease, AgentVaultGrantStore, HostVaultRegistration};
use crate::plugins::{Capability, GrantScope, VaultId};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy)]
struct Origin {
    id: &'static str,
    root: HostVaultRegistration,
}

#[derive(Clone)]
struct Candidate {
    agent: AuthenticatedAgent,
    origins: Vec<Origin>,
    leases: Vec<AgentReadLease>,
    body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResultKind {
    Sent,
    Denied,
    PartialDisclosureHold,
}

impl ResultKind {
    fn name(self) -> &'static str {
        match self {
            Self::Sent => "SENT",
            Self::Denied => "DENIED",
            Self::PartialDisclosureHold => "PARTIAL_DISCLOSURE_HOLD",
        }
    }
}

// Test-only query prefilter. In product code this MUST be accomplished by a
// host-issued source resolver before JOIN/COUNT/SORT/TOP and before any bytes
// from forbidden sources are available to the query engine.
fn scoped_sources(
    grants: &AgentVaultGrantStore,
    agent: AuthenticatedAgent,
    sources: &[Origin],
) -> Vec<Origin> {
    sources.iter().copied()
        .filter(|origin| grants.authorize_notes_read(agent, origin.root).is_ok())
        .collect()
}

// The proposed derived result contains its exact trusted source lineage.
// A stale grant snapshot is NOT authority to emit the resulting COUNT/title.
fn prepare(
    credentials: &AgentCredentialStore,
    grants: &AgentVaultGrantStore,
    bearer: &str,
    origins: &[Origin],
) -> Option<Candidate> {
    if origins.is_empty() {
        return None;
    }
    let agent = credentials.authenticate(bearer).ok()?;
    let mut seen = HashSet::new();
    let mut leases = Vec::with_capacity(origins.len());
    for origin in origins {
        if !seen.insert(origin.id) {
            return None;
        }
        leases.push(grants.authorize_notes_read(agent, origin.root).ok()?);
    }
    let titles: Vec<&str> = origins.iter().map(|source| source.id).collect();
    let total = titles.len();
    Some(Candidate {
        agent,
        origins: origins.to_vec(),
        leases,
        body: serde_json::to_vec(&json!({"total": total, "titles": titles})).ok()?,
    })
}

// Reuses the existing AKAI-AUTH-01d cooperative SQLite epoch idea, but checks
// *every* source lease before a derived response is written. This is a test-only
// simulation. It does not pin a real search index, OS root or HTTP socket, and
// it does not protect against uncooperative writers or hostile same-UID peers.
fn emit_with_epoch_fence<W: Write>(
    credential_db: &Path,
    grant_db: &Path,
    credentials: &AgentCredentialStore,
    grants: &AgentVaultGrantStore,
    bearer: &str,
    candidate: &Candidate,
    writer: &mut W,
    emit: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> ResultKind {
    let Ok(mut credential_conn) = Connection::open(credential_db) else {
        return ResultKind::Denied;
    };
    let Ok(mut grant_conn) = Connection::open(grant_db) else {
        return ResultKind::Denied;
    };
    if credential_conn.busy_timeout(Duration::from_secs(3)).is_err()
        || grant_conn.busy_timeout(Duration::from_secs(3)).is_err()
    {
        return ResultKind::Denied;
    }
    let Ok(_credential_epoch) =
        credential_conn.transaction_with_behavior(TransactionBehavior::Immediate)
    else {
        return ResultKind::Denied;
    };
    let Ok(_grant_epoch) =
        grant_conn.transaction_with_behavior(TransactionBehavior::Immediate)
    else {
        return ResultKind::Denied;
    };
    if candidate.origins.is_empty() || candidate.origins.len() != candidate.leases.len() {
        return ResultKind::Denied;
    }
    if candidate.origins.iter().map(|origin| origin.id).collect::<HashSet<_>>().len()
        != candidate.origins.len()
    {
        return ResultKind::Denied;
    }
    let Ok(fresh) = credentials.authenticate(bearer) else {
        return ResultKind::Denied;
    };
    if fresh != candidate.agent {
        return ResultKind::Denied;
    }
    for (origin, lease) in candidate.origins.iter().zip(&candidate.leases) {
        if grants.revalidate_read_lease(lease, fresh, origin.root).is_err() {
            return ResultKind::Denied;
        }
    }
    // These locks are held until emit AND flush finish. Errors are ambiguous:
    // even zero reported bytes cannot prove that nothing crossed a socket.
    match emit(writer).and_then(|()| writer.flush()) {
        Ok(()) => ResultKind::Sent,
        Err(_) => ResultKind::PartialDisclosureHold,
    }
}

fn attempt(
    credentials_path: &Path,
    grants_path: &Path,
    credentials: &AgentCredentialStore,
    grants: &AgentVaultGrantStore,
    bearer: &str,
    candidate: Option<&Candidate>,
    partial: bool,
) -> (ResultKind, Vec<u8>) {
    let Some(candidate) = candidate else {
        return (ResultKind::Denied, Vec::new());
    };
    let mut output = Vec::new();
    let outcome = emit_with_epoch_fence(
        credentials_path, grants_path, credentials, grants, bearer, candidate,
        &mut output,
        |writer| {
            if partial {
                writer.write_all(b"part")?;
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "test-only failed send"));
            }
            writer.write_all(&candidate.body)
        },
    );
    (outcome, output)
}

fn evidence(
    rows: &mut Vec<Value>,
    name: &'static str,
    expected: ResultKind,
    expected_total: Option<usize>,
    (actual, output): (ResultKind, Vec<u8>),
) {
    assert_eq!(actual, expected, "unexpected outcome in {name}");
    let (total, titles) = if actual == ResultKind::Sent {
        let value: Value = serde_json::from_slice(&output).expect("valid bounded derived JSON");
        let total = value["total"].as_u64().expect("total");
        let titles = value["titles"].as_array().expect("titles");
        assert_eq!(total as usize, titles.len(), "source count mismatch in {name}");
        assert_eq!(Some(total as usize), expected_total, "scope mismatch in {name}");
        (Some(total), Some(titles.iter().map(|x| x.as_str().unwrap().to_owned()).collect::<Vec<_>>()))
    } else {
        assert!(expected_total.is_none(), "unproven total for {name}");
        assert!(
            !matches!(actual, ResultKind::Denied) || output.is_empty(),
            "denied output leaked bytes in {name}"
        );
        (None, None)
    };
    rows.push(json!({
        "scenario": name,
        "outcome": actual.name(),
        "bytes": output.len(),
        "total": total,
        "titles": titles,
    }));
}

#[test]
fn akcb_03_native_grants_and_derived_disclosure_trace() {
    let directory = tempfile::tempdir().unwrap();
    let credentials_path = directory.path().join("akcb03-credentials.sqlite3");
    let grants_path = directory.path().join("akcb03-grants.sqlite3");
    let mut credentials = AgentCredentialStore::open(&credentials_path).unwrap();
    let mut agent_a = credentials.issue_principal("AKCB A", None).unwrap();
    let mut agent_b = credentials.issue_principal("AKCB B", None).unwrap();
    let key_a = agent_a.take_bearer_once().unwrap();
    let key_b = agent_b.take_bearer_once().unwrap();
    let root_x = HostVaultRegistration::from_host_custody(
        VaultId::new_for_host(), [11; 32],
    ).unwrap();
    let root_y = HostVaultRegistration::from_host_custody(
        VaultId::new_for_host(), [22; 32],
    ).unwrap();
    let x = Origin { id: "X/one.md", root: root_x };
    let y = Origin { id: "Y/two.md", root: root_y };
    let mut grants = AgentVaultGrantStore::provision_new(&grants_path).unwrap();
    grants.register_host_vault(root_x).unwrap();
    grants.register_host_vault(root_y).unwrap();
    grants.grant_notes_read(
        agent_a.principal, root_x, Capability::NotesRead, GrantScope::WholeVault, 0, None,
    ).unwrap();
    grants.grant_notes_read(
        agent_b.principal, root_x, Capability::NotesRead, GrantScope::WholeVault, 0, None,
    ).unwrap();
    let b_y_epoch = grants.grant_notes_read(
        agent_b.principal, root_y, Capability::NotesRead, GrantScope::WholeVault, 0, None,
    ).unwrap();

    let mut rows = Vec::new();
    let a_x = prepare(&credentials, &grants, key_a.as_str(), &[x]).unwrap();
    let b_xy = prepare(&credentials, &grants, key_b.as_str(), &[x, y]).unwrap();
    let b_y = prepare(&credentials, &grants, key_b.as_str(), &[y]).unwrap();

    let run = |token: &str, candidate: Option<&Candidate>, partial: bool| attempt(
        &credentials_path, &grants_path, &credentials, &grants,
        token, candidate, partial,
    );

    evidence(&mut rows, "a_only_x", ResultKind::Sent, Some(1),
        run(key_a.as_str(), Some(&a_x), false));
    evidence(&mut rows, "a_only_y", ResultKind::Denied, None,
        run(key_a.as_str(), prepare(&credentials, &grants, key_a.as_str(), &[y]).as_ref(), false));
    evidence(&mut rows, "a_mixed", ResultKind::Denied, None,
        run(key_a.as_str(), prepare(&credentials, &grants, key_a.as_str(), &[x, y]).as_ref(), false));

    // Authorized-sources-first: hidden Y must not change count, titles, TOP or
    // the cached candidate bytes seen by agent A.
    let a = credentials.authenticate(key_a.as_str()).unwrap();
    let visible = scoped_sources(&grants, a, &[x, y]);
    assert_eq!(visible.len(), 1);
    let filtered = prepare(&credentials, &grants, key_a.as_str(), &visible).unwrap();
    assert_eq!(filtered.body, a_x.body);
    evidence(&mut rows, "a_filtered_all", ResultKind::Sent, Some(1),
        run(key_a.as_str(), Some(&filtered), false));
    let changed_hidden = Origin { id: "Y/changed-private-title.md", root: root_y };
    let visible_again = scoped_sources(&grants, a, &[x, changed_hidden]);
    let filtered_again =
        prepare(&credentials, &grants, key_a.as_str(), &visible_again).unwrap();
    assert_eq!(filtered_again.body, filtered.body);
    evidence(&mut rows, "a_filtered_hidden_changed", ResultKind::Sent, Some(1),
        run(key_a.as_str(), Some(&filtered_again), false));

    evidence(&mut rows, "b_mixed", ResultKind::Sent, Some(2),
        run(key_b.as_str(), Some(&b_xy), false));
    evidence(&mut rows, "b_y_only", ResultKind::Sent, Some(1),
        run(key_b.as_str(), Some(&b_y), false));

    let mut independent = AgentVaultGrantStore::open_existing(&grants_path).unwrap();
    let revoked_epoch =
        independent.revoke_notes_read(agent_b.principal, root_y, b_y_epoch).unwrap();
    evidence(&mut rows, "b_old_mixed_after_y_revoke", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_xy), false));
    let b_x = prepare(&credentials, &grants, key_b.as_str(), &[x]).unwrap();
    evidence(&mut rows, "b_x_after_y_revoke", ResultKind::Sent, Some(1),
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_x), false));
    evidence(&mut rows, "a_x_unaffected_by_b_revoke", ResultKind::Sent, Some(1),
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_a.as_str(),
            Some(&a_x), false));
    evidence(&mut rows, "b_y_new_while_revoked", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            prepare(&credentials, &grants, key_b.as_str(), &[y]).as_ref(), false));

    let regrant_epoch = independent.grant_notes_read(
        agent_b.principal, root_y, Capability::NotesRead, GrantScope::WholeVault,
        revoked_epoch, None,
    ).unwrap();
    evidence(&mut rows, "b_old_mixed_after_regrant", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_xy), false));
    let b_new_xy = prepare(&credentials, &grants, key_b.as_str(), &[x, y]).unwrap();
    evidence(&mut rows, "b_new_mixed_after_regrant", ResultKind::Sent, Some(2),
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_new_xy), false));

    let mut incomplete = b_new_xy.clone();
    incomplete.leases.pop();
    evidence(&mut rows, "b_missing_lineage_lease", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&incomplete), false));
    let mut swapped = b_new_xy.clone();
    swapped.leases.swap(0, 1);
    evidence(&mut rows, "b_swapped_lineage_lease", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&swapped), false));
    evidence(&mut rows, "b_partially_written_response", ResultKind::PartialDisclosureHold, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_new_xy), true));

    // A competing issuer's durable revoke ACK must not overtake the whole
    // composed emission under the two cooperative SQLite BEGIN IMMEDIATE locks.
    let (signal_tx, signal_rx) = mpsc::channel();
    let (acked_tx, acked_rx) = mpsc::channel();
    let revoke_path = grants_path.clone();
    let b_principal = agent_b.principal;
    let revoke_thread = thread::spawn(move || {
        signal_rx.recv_timeout(Duration::from_secs(4)).unwrap();
        let mut issuing_host = AgentVaultGrantStore::open_existing(&revoke_path).unwrap();
        issuing_host.revoke_notes_read(b_principal, root_y, regrant_epoch).unwrap();
        acked_tx.send(()).unwrap();
    });
    let mut written = Vec::new();
    let raced = emit_with_epoch_fence(
        &credentials_path, &grants_path, &credentials, &grants,
        key_b.as_str(), &b_new_xy, &mut written,
        |writer| {
            signal_tx.send(()).unwrap();
            thread::sleep(Duration::from_millis(90));
            assert!(acked_rx.try_recv().is_err(), "revoke ACK overtook composed emission");
            writer.write_all(&b_new_xy.body)
        },
    );
    evidence(&mut rows, "b_emit_during_revoke", ResultKind::Sent, Some(2), (raced, written));
    acked_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    revoke_thread.join().unwrap();

    evidence(&mut rows, "b_cached_after_concurrent_revoke", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_new_xy), false));

    let b_x_candidate = prepare(&credentials, &grants, key_b.as_str(), &[x]).unwrap();
    let mut independent_credentials = AgentCredentialStore::open(&credentials_path).unwrap();
    independent_credentials.revoke(agent_b.credential).unwrap();
    evidence(&mut rows, "b_x_after_credential_revoke", ResultKind::Denied, None,
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_b.as_str(),
            Some(&b_x_candidate), false));
    evidence(&mut rows, "a_after_b_credential_revoke", ResultKind::Sent, Some(1),
        attempt(&credentials_path, &grants_path, &credentials, &grants, key_a.as_str(),
            Some(&a_x), false));

    assert_eq!(rows.len(), 20);
    let trace = json!({
        "schema": "akcb-03/derived-disclosure-lab/v1",
        "producer": "aquilum_core::mcp::akcb_03_composite_lab",
        "cases": rows,
    });
    if let Some(path) = std::env::var_os("AKCB_03_TRACE_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&trace).unwrap())
            .expect("write bounded AKCB-03 lab evidence");
    }
    println!("AKCB_03_COMPOSITE_GRANTS_LAB cases=20 (test-only, no live MCP)");
}
