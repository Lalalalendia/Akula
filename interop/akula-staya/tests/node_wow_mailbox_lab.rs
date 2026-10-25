//! Default-off Akula NodeV1 / pinned WOW mailbox integration acceptance.
//! No installed service, production owner policy v4, or public networking.
#![cfg(all(target_os = "linux", feature = "node-wow-mailbox-lab"))]

use akula_staya_integration::node_wow_mailbox_lab::{MailRoleHold, NodeWowMailboxLab};
use akula_wire_lab::{
    crypto_identity_w3::SignedHeader,
    crypto_w3,
    custodian_discovery::{ListClaims, sign_list_for_lab},
    custodian_mailbox::{MailboxPolicy, StoreClaims, StoreLocator, sign_store_for_lab},
    custodian_wire::{WireError, bind_test_loopback, client_fetch_listed, client_list},
    journal_w4::Journal,
    private_route_w3,
    trust_w3::TrustRegistry,
};
use aquilum_core::node_v1::{NodeHost, NodeIdentity};
use aquilum_core::sync::VaultId;
use ed25519_dalek::SigningKey;
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    thread,
};

const OWNER: [u8; 16] = [1; 16];
const CUSTODIAN: [u8; 16] = [8; 16];
const RECIPIENT: [u8; 16] = [3; 16];
const MAILBOX: [u8; 16] = [11; 16];
const SCOPE: [u8; 16] = [5; 16];
const MESSAGE: [u8; 16] = [4; 16];
const EXPIRY: i64 = 10_000_000;
const NODE_ENDPOINT: [u8; 32] = [42; 32];

fn private_folder(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

struct Fixture {
    _tmp: tempfile::TempDir,
    node: NodeHost,
    node_policy: PathBuf,
    mail_state: PathBuf,
    owner: SigningKey,
    recipient: SigningKey,
    custodian: SigningKey,
    trust: TrustRegistry,
    packet: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let node_state = tmp.path().join("node");
        let node_policy = tmp.path().join("root-policy");
        let mail_state = tmp.path().join("separate-wow-state");
        for dir in [&node_state, &node_policy, &mail_state] {
            private_folder(dir);
        }
        let key = NodeIdentity::open_or_create(&node_state).unwrap();
        let owner_public = [9u8; 32];
        let roster_digest = [10u8; 32];
        let grant_digest = [11u8; 32];
        let put_grant_digest = [12u8; 32];
        let config = serde_json::json!({
            "version": 3,
            "node_public": key.public_key(),
            "iroh_endpoint_public": NODE_ENDPOINT,
            "owner_public": owner_public,
            "vault_id": VaultId::new(),
            "grant_epoch": 9u64,
            "roster_digest": roster_digest,
            "grant_digest": grant_digest,
            "put_grant_digest": put_grant_digest
        });
        let policy_file = node_policy.join("owner-policy.json");
        fs::write(&policy_file, serde_json::to_vec(&config).unwrap()).unwrap();
        fs::set_permissions(&policy_file, fs::Permissions::from_mode(0o600)).unwrap();
        let node = NodeHost::open_integration_fixture(&node_state, &node_policy).unwrap();
        node.verify_iroh_endpoint(NODE_ENDPOINT).unwrap();

        let owner = SigningKey::from_bytes(&[1; 32]);
        let recipient = SigningKey::from_bytes(&[3; 32]);
        let custodian = SigningKey::from_bytes(&[8; 32]);
        let sender = SigningKey::from_bytes(&[7; 32]);
        let mut trust = TrustRegistry::new();
        for (id, key) in [
            (OWNER, &owner),
            (RECIPIENT, &recipient),
            (CUSTODIAN, &custodian),
        ] {
            trust.enroll(id, 1, key.verifying_key()).unwrap();
        }
        let (_, hpke_public) = crypto_w3::new_recipient();
        let packet = private_route_w3::seal_private(
            &sender,
            &hpke_public,
            [9; 16],
            SignedHeader {
                sender_id: [7; 16],
                recipient_id: RECIPIENT,
                message_id: MESSAGE,
                expiry: EXPIRY as u64,
                epoch: 1,
            },
            b"independent NodeV1 and WOW mail state",
        )
        .unwrap();
        let claims = StoreClaims {
            issuer: OWNER,
            issuer_epoch: 1,
            custodian: CUSTODIAN,
            mailbox: MAILBOX,
            scope: SCOPE,
            recipient: RECIPIENT,
            message: MESSAGE,
            offer: [12; 16],
            slot: [13; 16],
            expires_at: EXPIRY,
            grant_until: EXPIRY - 1,
            packet_len: packet.len() as u32,
            digest: *blake3::hash(&packet).as_bytes(),
        };
        let db_file = mail_state.join("wow-mailbox.sqlite");
        let mut journal = Journal::open(&db_file).unwrap();
        journal
            .operator_provision_mailbox(
                MailboxPolicy {
                    mailbox: MAILBOX,
                    issuer: OWNER,
                    issuer_epoch: 1,
                    custodian: CUSTODIAN,
                    scope: SCOPE,
                    recipient: RECIPIENT,
                    max_message_bytes: 1024,
                    max_offers: 2,
                },
                &trust,
            )
            .unwrap();
        journal
            .request_custodian_store(&sign_store_for_lab(&owner, claims), &trust, 1)
            .unwrap();
        journal
            .commit_custodian_store(
                StoreLocator {
                    mailbox: MAILBOX,
                    offer: claims.offer,
                },
                &packet,
                &custodian,
                1,
                &trust,
                2,
            )
            .unwrap();
        drop(journal);
        fs::set_permissions(&db_file, fs::Permissions::from_mode(0o600)).unwrap();
        Self {
            _tmp: tmp,
            node,
            node_policy: policy_file,
            mail_state,
            owner,
            recipient,
            custodian,
            trust,
            packet,
        }
    }

    fn role(&self) -> Result<NodeWowMailboxLab, MailRoleHold> {
        let mut trust = TrustRegistry::new();
        for (id, key) in [
            (OWNER, &self.owner),
            (RECIPIENT, &self.recipient),
            (CUSTODIAN, &self.custodian),
        ] {
            trust.enroll(id, 1, key.verifying_key()).unwrap();
        }
        NodeWowMailboxLab::open_existing(
            &self.node,
            &self.mail_state,
            NODE_ENDPOINT,
            CUSTODIAN,
            1,
            self.custodian.clone(),
            trust,
        )
    }

    fn list_proof(&self) -> Vec<u8> {
        sign_list_for_lab(
            &self.recipient,
            ListClaims {
                issuer: OWNER,
                issuer_epoch: 1,
                custodian: CUSTODIAN,
                mailbox: MAILBOX,
                scope: SCOPE,
                recipient: RECIPIENT,
                recipient_epoch: 1,
                since_seq: 0,
                grant_until_ms: 100_000,
                nonce: [80; 16],
                max_items: 1,
            },
        )
    }
}

fn exchange<T>(
    node: &NodeHost,
    role: &mut NodeWowMailboxLab,
    f: impl FnOnce(&mut TcpStream) -> Result<T, WireError> + Send + 'static,
) -> Result<T, WireError>
where
    T: Send + 'static,
{
    let listener = bind_test_loopback().unwrap();
    NodeWowMailboxLab::ensure_local_only(&listener).unwrap();
    let address = listener.local_addr().unwrap();
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        f(&mut stream)
    });
    role.serve_one(node, &listener, 3).unwrap();
    client.join().unwrap()
}

#[test]
fn current_node_policy_and_independent_mailbox_survive_restart() {
    let fixture = Fixture::new();
    let mut role = fixture.role().unwrap();
    let proof = fixture.list_proof();
    let found = exchange(&fixture.node, &mut role, move |sock| {
        client_list(sock, &proof)
    })
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, MESSAGE);
    assert_eq!(*blake3::hash(&fixture.packet).as_bytes(), found[0].digest);
    let proof = fixture.list_proof();
    let entry = found[0];
    let fetched = exchange(&fixture.node, &mut role, move |sock| {
        client_fetch_listed(sock, &proof, entry)
    })
    .unwrap();
    assert_eq!(fetched, Some(fixture.packet.clone()));
    drop(role);

    // Service restart must not generate a new mailbox, signing key or Node ID.
    let mut restarted = fixture.role().unwrap();
    let proof = fixture.list_proof();
    let later = exchange(&fixture.node, &mut restarted, move |sock| {
        client_list(sock, &proof)
    })
    .unwrap();
    assert_eq!(later, found);
}

#[test]
fn changed_node_owner_policy_fences_mailbox_before_accept() {
    let fixture = Fixture::new();
    let mut role = fixture.role().unwrap();
    assert!(matches!(fixture.role(), Ok(_)));
    // Same JSON object with a different byte commitment invalidates Node V1.
    let mut bytes = fs::read(&fixture.node_policy).unwrap();
    bytes.push(b'\n');
    fs::write(&fixture.node_policy, bytes).unwrap();
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        role.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::NodePolicy)
    ));
}

#[test]
fn mail_role_rejects_public_bind_and_unapproved_or_swapped_state() {
    let fixture = Fixture::new();
    let mut role = fixture.role().unwrap();
    let public = TcpListener::bind("0.0.0.0:0").unwrap();
    assert!(NodeWowMailboxLab::ensure_local_only(&public).is_err());
    assert!(matches!(
        role.serve_one(&fixture.node, &public, 3),
        Err(MailRoleHold::Wire)
    ));

    // The already-admitted DB inode cannot be replaced behind the role.
    let db = fixture.mail_state.join("wow-mailbox.sqlite");
    fs::rename(&db, fixture.mail_state.join("old.sqlite")).unwrap();
    symlink("/etc/passwd", &db).unwrap();
    let private = bind_test_loopback().unwrap();
    assert!(matches!(
        role.serve_one(&fixture.node, &private, 3),
        Err(MailRoleHold::UnsafeDatabase)
    ));
    assert!(matches!(fixture.role(), Err(MailRoleHold::UnsafeDatabase)));
}

#[test]
fn missing_world_readable_or_wrong_signing_key_fails_closed() {
    let fixture = Fixture::new();
    let db = fixture.mail_state.join("wow-mailbox.sqlite");
    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::UnsafeDatabase)));
    fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(matches!(
        NodeWowMailboxLab::open_existing(
            &fixture.node,
            &fixture.mail_state,
            NODE_ENDPOINT,
            CUSTODIAN,
            1,
            SigningKey::from_bytes(&[96; 32]),
            fixture.trust,
        ),
        Err(MailRoleHold::UnpinnedTrust)
    ));
}
