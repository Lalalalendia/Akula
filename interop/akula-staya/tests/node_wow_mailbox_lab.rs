//! Default-off Akula NodeV1 / pinned WOW mailbox integration acceptance.
//! No installed service, production owner policy v4, or public networking.
#![cfg(all(target_os = "linux", feature = "node-wow-mailbox-lab"))]

use akula_staya_integration::node_wow_mailbox_lab::{
    MailFirstApprovalClaims, MailRoleClaims, MailRoleHold, NodeWowMailboxLab, PublishHold,
    PublishStop, first_mail_approval_proposal_for_lab, first_mail_publish_ci,
    mail_commit_receipt_for_lab, mail_epoch_floor_for_lab, sign_first_mail_approval_for_lab,
    sign_mail_policy_for_lab,
};
use akula_wire_lab::{
    crypto_identity_w3::SignedHeader,
    crypto_w3,
    custodian_discovery::{ListClaims, sign_list_for_lab},
    custodian_mailbox::{
        MailRoleGate, MailboxPolicy, StoreClaims, StoreLocator, sign_store_for_lab,
    },
    custodian_wire::{WireError, bind_test_loopback, client_fetch_listed, client_list},
    journal_w4::Journal,
    private_route_w3,
    trust_w3::TrustRegistry,
};
use aquilum_core::node_v1::{NodeHost, NodeIdentity};
use aquilum_core::sync::VaultId;
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
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
    mail_policy_dir: PathBuf,
    mail_state: PathBuf,
    owner: SigningKey,
    recipient: SigningKey,
    custodian: SigningKey,
    packet: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let node_state = tmp.path().join("node");
        let node_policy = tmp.path().join("root-policy");
        let mail_state = tmp.path().join("separate-wow-state");
        let mail_policy_dir = tmp.path().join("signed-mail-owner-policy");
        for dir in [&node_state, &node_policy, &mail_state, &mail_policy_dir] {
            private_folder(dir);
        }
        let owner = SigningKey::from_bytes(&[1; 32]);
        let key = NodeIdentity::open_or_create(&node_state).unwrap();
        let owner_public = owner.verifying_key().to_bytes();
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

        let policy = MailRoleClaims {
            version: 1,
            policy_epoch: 1,
            node_public: node.public_key(),
            node_endpoint: NODE_ENDPOINT,
            operator_public: owner.verifying_key().to_bytes(),
            issuer: OWNER,
            issuer_epoch: 1,
            issuer_public: owner.verifying_key().to_bytes(),
            custodian: CUSTODIAN,
            custodian_epoch: 1,
            custodian_public: custodian.verifying_key().to_bytes(),
            sender: [7; 16],
            sender_epoch: 1,
            sender_public: sender.verifying_key().to_bytes(),
            recipient: RECIPIENT,
            recipient_epoch: 1,
            recipient_public: recipient.verifying_key().to_bytes(),
            mailbox: MAILBOX,
            scope: SCOPE,
            permissions: 0b0000_1111,
            max_message_bytes: 1024,
            max_offers: 2,
            valid_from_ms: 1,
            valid_until_ms: EXPIRY,
        };
        let signed = sign_mail_policy_for_lab(&owner, policy);
        let policy_file = mail_policy_dir.join("owner-mail-policy.bin");
        fs::write(&policy_file, &signed).unwrap();
        fs::set_permissions(&policy_file, fs::Permissions::from_mode(0o600)).unwrap();
        let floor = mail_policy_dir.join("owner-mail-epoch-floor.bin");
        fs::write(
            &floor,
            mail_epoch_floor_for_lab(&signed, policy.policy_epoch),
        )
        .unwrap();
        fs::set_permissions(&floor, fs::Permissions::from_mode(0o600)).unwrap();
        let mut witness_journal = Journal::open(&db_file).unwrap();
        witness_journal
            .operator_provision_mail_role_gate(MailRoleGate {
                mailbox_policy: MailboxPolicy {
                    mailbox: policy.mailbox,
                    issuer: policy.issuer,
                    issuer_epoch: policy.issuer_epoch,
                    custodian: policy.custodian,
                    scope: policy.scope,
                    recipient: policy.recipient,
                    max_message_bytes: policy.max_message_bytes,
                    max_offers: policy.max_offers,
                },
                policy_epoch: policy.policy_epoch,
                signed_policy_digest: *blake3::hash(&signed).as_bytes(),
            })
            .unwrap();
        drop(witness_journal);
        Self {
            _tmp: tmp,
            node,
            node_policy: node_policy.join("owner-policy.json"),
            mail_policy_dir,
            mail_state,
            owner,
            recipient,
            custodian,
            packet,
        }
    }

    fn role(&self) -> Result<NodeWowMailboxLab, MailRoleHold> {
        NodeWowMailboxLab::open_existing(
            &self.node,
            &self.mail_state,
            &self.mail_policy_dir,
            NODE_ENDPOINT,
            self.custodian.clone(),
            1,
        )
    }

    fn mail_policy_file(&self) -> PathBuf {
        self.mail_policy_dir.join("owner-mail-policy.bin")
    }

    fn floor_file(&self) -> PathBuf {
        self.mail_policy_dir.join("owner-mail-epoch-floor.bin")
    }

    // Simulate the separate privileged operator provisioning step.
    // This same-UID fixture is NOT a production anti-rollback authority.
    fn provision_mail_policy(&self, signed: &[u8], epoch: u64) {
        let file = self.mail_policy_file();
        let floor = self.floor_file();
        fs::write(&file, signed).unwrap();
        fs::write(&floor, mail_epoch_floor_for_lab(signed, epoch)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&floor, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn advance_sqlite_role_witness(&self, signed: &[u8], claims: MailRoleClaims) {
        let mut journal = Journal::open(&self.mail_state.join("wow-mailbox.sqlite")).unwrap();
        journal
            .operator_provision_mail_role_gate(MailRoleGate {
                mailbox_policy: MailboxPolicy {
                    mailbox: claims.mailbox,
                    issuer: claims.issuer,
                    issuer_epoch: claims.issuer_epoch,
                    custodian: claims.custodian,
                    scope: claims.scope,
                    recipient: claims.recipient,
                    max_message_bytes: claims.max_message_bytes,
                    max_offers: claims.max_offers,
                },
                policy_epoch: claims.policy_epoch,
                signed_policy_digest: *blake3::hash(signed).as_bytes(),
            })
            .unwrap();
    }

    fn signed_claims(&self) -> MailRoleClaims {
        MailRoleClaims {
            version: 1,
            policy_epoch: 1,
            node_public: self.node.public_key(),
            node_endpoint: NODE_ENDPOINT,
            operator_public: self.owner.verifying_key().to_bytes(),
            issuer: OWNER,
            issuer_epoch: 1,
            issuer_public: self.owner.verifying_key().to_bytes(),
            custodian: CUSTODIAN,
            custodian_epoch: 1,
            custodian_public: self.custodian.verifying_key().to_bytes(),
            sender: [7; 16],
            sender_epoch: 1,
            sender_public: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
            recipient: RECIPIENT,
            recipient_epoch: 1,
            recipient_public: self.recipient.verifying_key().to_bytes(),
            mailbox: MAILBOX,
            scope: SCOPE,
            permissions: 0b0000_1111,
            max_message_bytes: 1024,
            max_offers: 2,
            valid_from_ms: 1,
            valid_until_ms: EXPIRY,
        }
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
    assert!(fixture.role().is_ok());
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
            &fixture.mail_policy_dir,
            NODE_ENDPOINT,
            SigningKey::from_bytes(&[96; 32]),
            1,
        ),
        Err(MailRoleHold::UnpinnedTrust)
    ));
}

#[test]
fn independently_pinned_operator_signature_rejects_untrusted_mail_roles() {
    let fixture = Fixture::new();
    let file = fixture.mail_policy_file();
    let baseline = fs::read(&file).unwrap();
    assert!(fixture.role().is_ok());

    // The same valid-looking policy signed by an unrelated key cannot
    // become an active WOW trust registry.
    let attacker = SigningKey::from_bytes(&[99; 32]);
    fixture.provision_mail_policy(
        &sign_mail_policy_for_lab(&attacker, fixture.signed_claims()),
        1,
    );
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fixture.provision_mail_policy(&baseline, 1);

    // Binding mail to a different Akula Node endpoint is a hard refusal.
    let mut wrong_endpoint = fixture.signed_claims();
    wrong_endpoint.node_endpoint = [88; 32];
    fixture.provision_mail_policy(&sign_mail_policy_for_lab(&fixture.owner, wrong_endpoint), 1);
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fixture.provision_mail_policy(&baseline, 1);

    // A broad permission bit or an unexpectedly enlarged quota cannot
    // silently upgrade the schema/role, even when signed.
    let mut unknown_right = fixture.signed_claims();
    unknown_right.permissions |= 0b1000_0000;
    fixture.provision_mail_policy(&sign_mail_policy_for_lab(&fixture.owner, unknown_right), 1);
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fixture.provision_mail_policy(&baseline, 1);

    let mut too_many = fixture.signed_claims();
    too_many.max_offers = 100;
    fixture.provision_mail_policy(&sign_mail_policy_for_lab(&fixture.owner, too_many), 1);
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fixture.provision_mail_policy(&baseline, 1);
    assert!(fixture.role().is_ok());
}

#[test]
fn mail_policy_change_revocation_and_expiry_fence_before_socket_accept() {
    let fixture = Fixture::new();
    let mut role = fixture.role().unwrap();
    let file = fixture.mail_policy_file();

    let mut revised = fixture.signed_claims();
    revised.policy_epoch += 1;
    let signed_epoch_two = sign_mail_policy_for_lab(&fixture.owner, revised);
    fs::write(&file, &signed_epoch_two).unwrap();
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        role.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    // Even a valid owner signature is rejected on cold open while the
    // independently provisioned monotonic witness remains at epoch one.
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));

    // Explicit operator action updates the separate witness. Live
    // instances must NOT auto-adopt the new signed policy.
    fixture.provision_mail_policy(&signed_epoch_two, 2);
    fixture.advance_sqlite_role_witness(&signed_epoch_two, revised);
    assert!(matches!(
        role.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    let restarted = fixture.role().unwrap();
    drop(restarted);
    // This signed fixture is short-lived. An expired one cannot serve.
    let mut role = fixture.role().unwrap();
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        role.serve_one(&fixture.node, &listener, EXPIRY),
        Err(MailRoleHold::MailPolicy)
    ));
}

#[test]
fn unsafe_mail_role_policy_file_never_bootstraps_keys_or_mailbox() {
    let fixture = Fixture::new();
    let path = fixture.mail_policy_file();
    let original = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let hardlink = fixture.mail_policy_dir.join("extra-hardlink");
    fs::hard_link(&path, &hardlink).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fs::remove_file(hardlink).unwrap();

    fs::remove_file(&path).unwrap();
    symlink("/etc/passwd", &path).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fs::remove_file(&path).unwrap();
    fs::write(&path, original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(fixture.role().is_ok());
}

#[test]
fn signed_mail_scope_must_match_durable_sqlite_grants_exactly() {
    let fixture = Fixture::new();
    let db_path = fixture.mail_state.join("wow-mailbox.sqlite");
    assert!(fixture.role().is_ok());

    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute(
            "UPDATE custodian_mailbox_policy SET max_offers=3 WHERE mailbox=?1",
            [MAILBOX.as_slice()],
        )
        .unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    connection
        .execute(
            "UPDATE custodian_mailbox_policy SET max_offers=2 WHERE mailbox=?1",
            [MAILBOX.as_slice()],
        )
        .unwrap();

    let mut accepted = fixture.role().unwrap();
    connection
        .execute(
            "UPDATE custodian_mailbox_policy SET max_bytes=2048 WHERE mailbox=?1",
            [MAILBOX.as_slice()],
        )
        .unwrap();
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        accepted.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::MailPolicy),
    ));

    connection
        .execute(
            "UPDATE custodian_mailbox_policy SET max_bytes=1024 WHERE mailbox=?1",
            [MAILBOX.as_slice()],
        )
        .unwrap();
    assert!(fixture.role().is_ok());

    // A second locally provisioned mailbox cannot borrow this one-mailbox
    // signed operator policy, even if the issuer key is otherwise valid.
    connection
        .execute(
            "INSERT INTO custodian_mailbox_policy
         (mailbox,issuer,issuer_epoch,custodian,scope,recipient,max_bytes,max_offers)
         VALUES(?1,?2,1,?3,?4,?5,1024,2)",
            rusqlite::params![
                [77u8; 16].as_slice(),
                OWNER.as_slice(),
                CUSTODIAN.as_slice(),
                SCOPE.as_slice(),
                RECIPIENT.as_slice()
            ],
        )
        .unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
}

#[test]
fn epoch_floor_cannot_silently_rollback_one_signed_policy() {
    let fixture = Fixture::new();
    let original = fs::read(fixture.mail_policy_file()).unwrap();
    let mut live = fixture.role().unwrap();

    let mut next = fixture.signed_claims();
    next.policy_epoch += 1;
    let signed_next = sign_mail_policy_for_lab(&fixture.owner, next);
    fixture.provision_mail_policy(&signed_next, 2);
    fixture.advance_sqlite_role_witness(&signed_next, next);
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        live.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    assert!(
        fixture.role().is_ok(),
        "explicitly provisioned new epoch is authorized"
    );
    // Reverting only the signed manifest does not restore old permissions.
    fs::write(fixture.mail_policy_file(), &original).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));

    // An omitted or world-readable witness cannot bootstrap a new role.
    fs::remove_file(fixture.floor_file()).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    let floor = mail_epoch_floor_for_lab(&signed_next, 2);
    fs::write(fixture.floor_file(), floor).unwrap();
    fs::set_permissions(fixture.floor_file(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
    fs::set_permissions(fixture.floor_file(), fs::Permissions::from_mode(0o600)).unwrap();

    // The original manifest and the newer authorized epoch floor cannot pair.
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));
}

#[test]
fn revoked_sqlite_mail_role_fences_live_node_even_with_valid_owner_files() {
    let fixture = Fixture::new();
    let mut live = fixture.role().unwrap();
    assert!(fixture.role().is_ok());
    let db = fixture.mail_state.join("wow-mailbox.sqlite");

    // The operator changes the exact SQLite authority while BOTH signed
    // files remain unchanged. The live Node bridge must re-check the gate.
    let mut operator = Journal::open(&db).unwrap();
    operator.operator_revoke_mail_role_gate(MAILBOX).unwrap();
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        live.serve_one(&fixture.node, &listener, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    assert!(matches!(fixture.role(), Err(MailRoleHold::MailPolicy)));

    // No automatic un-revoke at the same owner epoch.
    let claims = fixture.signed_claims();
    let signed = sign_mail_policy_for_lab(&fixture.owner, claims);
    assert!(
        operator
            .operator_provision_mail_role_gate(MailRoleGate {
                mailbox_policy: MailboxPolicy {
                    mailbox: claims.mailbox,
                    issuer: claims.issuer,
                    issuer_epoch: claims.issuer_epoch,
                    custodian: claims.custodian,
                    scope: claims.scope,
                    recipient: claims.recipient,
                    max_message_bytes: claims.max_message_bytes,
                    max_offers: claims.max_offers,
                },
                policy_epoch: claims.policy_epoch,
                signed_policy_digest: *blake3::hash(&signed).as_bytes(),
            })
            .is_err()
    );
}

/// This OS test is explicitly opt-in because it stages root-owned,
/// PUBLIC signed policy files under a disposable CI runner's /etc tree.
/// It refuses any pre-existing Akula Node installation, never installs a
/// daemon, binary, key, service or listens on an external network address.
/// Run ONLY through a sudo child of the disposable CI OS test. The test
/// executable and public source are synthetic; no installed Node is touched.
#[test]
fn root_mail_cas_publisher_ci_child() {
    if std::env::var("AKULA_WOW_ROOT_CAS_CHILD").ok().as_deref() != Some("1") {
        return;
    }
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let source = std::env::var("AKULA_WOW_ROOT_CAS_SOURCE").unwrap();
    let bytes = fs::read(source).unwrap();
    let action = fs::read(std::env::var("AKULA_WOW_ROOT_CAS_APPROVAL").unwrap()).unwrap();
    let which = std::env::var("AKULA_WOW_ROOT_CAS_STOP").unwrap();
    let (point, expected) = match which.as_str() {
        "pending" => (
            PublishStop::AfterPendingSync,
            Err(PublishHold::InjectedStop),
        ),
        "policy" => (
            PublishStop::AfterPolicyRename,
            Err(PublishHold::InjectedStop),
        ),
        "floor" => (
            PublishStop::AfterFloorRename,
            Err(PublishHold::InjectedStop),
        ),
        "receipt" => (
            PublishStop::AfterReceiptRename,
            Err(PublishHold::InjectedStop),
        ),
        "existing" => (PublishStop::None, Err(PublishHold::ExistingOrInterrupted)),
        "forged" => (PublishStop::None, Err(PublishHold::InvalidOwnerApproval)),
        "none" => (PublishStop::None, Ok(())),
        _ => panic!("invalid CI-only crash point"),
    };
    assert_eq!(first_mail_publish_ci(&bytes, &action, 1, point), expected);
}

#[test]
fn root_owned_mail_policy_is_read_only_and_current() {
    if std::env::var("AKULA_WOW_ROOT_MAIL_OS_TEST").ok().as_deref() != Some("1") {
        return;
    }
    use std::os::unix::fs::MetadataExt;
    assert_ne!(
        rustix::process::geteuid().as_raw(),
        0,
        "nonroot service required"
    );
    const PARENT: &str = "/etc/akula-node";
    const ROOT: &str = "/etc/akula-node/wow-mail";
    const POLICY: &str = "/etc/akula-node/wow-mail/owner-mail-policy.bin";
    const FLOOR: &str = "/etc/akula-node/wow-mail/owner-mail-epoch-floor.bin";
    const RECEIPT: &str = "/etc/akula-node/wow-mail/owner-mail-commit.bin";
    const PENDING: &str = "/etc/akula-node/wow-mail/owner-mail-policy.pending";
    const PENDING_FLOOR: &str = "/etc/akula-node/wow-mail/owner-mail-epoch-floor.pending";
    const PENDING_RECEIPT: &str = "/etc/akula-node/wow-mail/owner-mail-commit.pending";
    const ROOT_POLICY: &str = "/etc/akula-node/owner-policy.json";
    assert!(
        fs::symlink_metadata(PARENT).is_err(),
        "refuse to touch an existing root-owned Akula Node installation"
    );
    fn sudo(args: &[&str]) {
        let result = Command::new("sudo").arg("-n").args(args).status().unwrap();
        assert!(
            result.success(),
            "CI root-owned fixture provisioning failed"
        );
    }
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for args in [
                vec![
                    "rm",
                    "-f",
                    "--",
                    POLICY,
                    FLOOR,
                    RECEIPT,
                    PENDING,
                    PENDING_FLOOR,
                    PENDING_RECEIPT,
                ],
                vec!["rmdir", "--", ROOT],
                vec!["rm", "-f", "--", ROOT_POLICY],
                vec!["rmdir", "--", PARENT],
            ] {
                let _ = Command::new("sudo").arg("-n").args(args).status();
            }
        }
    }
    let fixture = Fixture::new();
    let group = rustix::process::getegid().as_raw().to_string();
    sudo(&[
        "install", "-d", "-o", "root", "-g", "root", "-m", "0755", PARENT,
    ]);
    let _cleanup = Cleanup;
    sudo(&[
        "install", "-d", "-o", "root", "-g", &group, "-m", "0750", ROOT,
    ]);
    let signed_source = fixture.mail_policy_file();
    let floor_source = fixture.floor_file();
    // RootTrust is independently pinned: the root actor reads this
    // root-owned Node policy, not a key from the caller's mail manifest.
    sudo(&[
        "install",
        "-o",
        "root",
        "-g",
        "root",
        "-m",
        "0600",
        fixture.node_policy.to_str().unwrap(),
        ROOT_POLICY,
    ]);
    let signed_bytes = fs::read(&signed_source).unwrap();
    let action_claims = MailFirstApprovalClaims {
        version: 1,
        operation: 1,
        node_public: fixture.node.public_key(),
        node_endpoint: NODE_ENDPOINT,
        signed_policy_digest: *blake3::hash(&signed_bytes).as_bytes(),
        policy_epoch: 1,
        expected_witness_revision: 0,
        approval_nonce: [91; 16],
        helper_challenge: [92; 32],
        issued_at_ms: 0,
        valid_until_ms: 200_000,
    };
    let approval_source = fixture._tmp.path().join("owner-action-approval.bin");
    fs::write(
        &approval_source,
        sign_first_mail_approval_for_lab(&fixture.owner, action_claims),
    )
    .unwrap();
    let forged_approval = fixture._tmp.path().join("forged-owner-action.bin");
    fs::write(
        &forged_approval,
        sign_first_mail_approval_for_lab(&SigningKey::from_bytes(&[99; 32]), action_claims),
    )
    .unwrap();
    let root_child = |stop: &str| {
        eprintln!("ROOT_MAIL_CI: root subprocess start {stop}");
        let exe = std::env::current_exe().unwrap();
        let status = Command::new("sudo")
            .arg("-n")
            .arg("timeout")
            .arg("45s")
            .arg("env")
            .arg("GITHUB_ACTIONS=true")
            .arg("AKULA_WOW_ROOT_CAS_CI=1")
            .arg("AKULA_WOW_ROOT_CAS_CHILD=1")
            .arg(format!(
                "AKULA_WOW_ROOT_CAS_SOURCE={}",
                signed_source.display()
            ))
            .arg(format!(
                "AKULA_WOW_ROOT_CAS_APPROVAL={}",
                if stop == "forged" {
                    &forged_approval
                } else {
                    &approval_source
                }
                .display()
            ))
            .arg(format!("AKULA_WOW_ROOT_CAS_STOP={stop}"))
            .arg(exe)
            .arg("--exact")
            .arg("root_mail_cas_publisher_ci_child")
            .arg("--nocapture")
            .status()
            .unwrap();
        assert!(status.success(), "root-only publisher subprocess: {stop}");
        eprintln!("ROOT_MAIL_CI: root subprocess done {stop}");
    };
    // Wrong detached owner approval must FAIL BEFORE ANY root files exist.
    root_child("forged");
    for path in [
        POLICY,
        FLOOR,
        RECEIPT,
        PENDING,
        PENDING_FLOOR,
        PENDING_RECEIPT,
    ] {
        assert!(
            fs::symlink_metadata(path).is_err(),
            "forged approval created a root mail file"
        );
    }

    // All three early crash points must refuse admission. A privileged
    // human repair is simulated only by clearing the disposable CI tree.
    for crash in ["pending", "policy", "floor"] {
        root_child(crash);
        assert!(matches!(
            NodeWowMailboxLab::open_existing_root_provisioned(
                &fixture.node,
                &fixture.mail_state,
                NODE_ENDPOINT,
                fixture.custodian.clone(),
                1,
            ),
            Err(MailRoleHold::MailPolicy)
        ));
        sudo(&[
            "rm",
            "-f",
            "--",
            POLICY,
            FLOOR,
            RECEIPT,
            PENDING,
            PENDING_FLOOR,
            PENDING_RECEIPT,
        ]);
    }
    // The last rename was already fsynced: a lost response must not
    // permit re-provisioning, even though read-only admission is complete.
    root_child("receipt");
    root_child("existing");
    let signed_bytes = fs::read(&signed_source).unwrap();
    let floor_bytes = fs::read(&floor_source).unwrap();
    let receipt_fixture = fixture._tmp.path().join("mail-commit-fixture.bin");
    fs::write(
        &receipt_fixture,
        mail_commit_receipt_for_lab(&signed_bytes, &floor_bytes, 1),
    )
    .unwrap();
    eprintln!("ROOT_MAIL_CI: final signed root bundle published");
    let meta = fs::metadata(POLICY).unwrap();
    assert_eq!(meta.uid(), 0);
    assert_eq!(meta.gid(), rustix::process::getegid().as_raw());
    assert_eq!(meta.permissions().mode() & 0o7777, 0o640);
    assert!(
        fs::read(POLICY).is_ok(),
        "service must read the signed public manifest"
    );
    assert!(fs::write(POLICY, b"unprivileged replacement attempt").is_err());
    assert!(fs::remove_file(POLICY).is_err());

    let open = || {
        NodeWowMailboxLab::open_existing_root_provisioned(
            &fixture.node,
            &fixture.mail_state,
            NODE_ENDPOINT,
            fixture.custodian.clone(),
            1,
        )
    };
    eprintln!("ROOT_MAIL_CI: verifying nonroot mailbox reader");
    let mut running = open().expect("root-owned policy and SQLite witness must match");
    let proof = fixture.list_proof();
    let listed = exchange(&fixture.node, &mut running, move |sock| {
        client_list(sock, &proof)
    })
    .unwrap();
    assert_eq!(listed.len(), 1);
    let info = listed[0];
    let proof = fixture.list_proof();
    let retrieved = exchange(&fixture.node, &mut running, move |sock| {
        client_fetch_listed(sock, &proof, info)
    })
    .unwrap();
    assert_eq!(retrieved, Some(fixture.packet.clone()));
    eprintln!("ROOT_MAIL_CI: root-owned LIST/FETCH PASS");

    // A new pending file is an unresolved root transaction and must not
    // be treated as a valid previous receipt.
    eprintln!("ROOT_MAIL_CI: pending touch start");
    sudo(&["touch", PENDING]);
    eprintln!("ROOT_MAIL_CI: pending touch done, deny check");
    assert!(matches!(open(), Err(MailRoleHold::MailPolicy)));
    eprintln!("ROOT_MAIL_CI: pending denial PASS, remove pending");
    sudo(&["rm", "-f", "--", PENDING]);
    eprintln!("ROOT_MAIL_CI: pending removed, clean reopen");
    assert!(open().is_ok());
    eprintln!("ROOT_MAIL_CI: pending gate PASS");

    // The receipt checksum binds both exact files. A missing or foreign
    // receipt cannot turn a partially published manifest into an active role.
    eprintln!("ROOT_MAIL_CI: receipt removal start");
    sudo(&["rm", "-f", "--", RECEIPT]);
    eprintln!("ROOT_MAIL_CI: receipt removed, deny check");
    assert!(matches!(open(), Err(MailRoleHold::MailPolicy)));
    eprintln!("ROOT_MAIL_CI: missing receipt denied");
    sudo(&[
        "install",
        "-o",
        "root",
        "-g",
        &group,
        "-m",
        "0640",
        receipt_fixture.to_str().unwrap(),
        RECEIPT,
    ]);
    // Even byte-identical receipt replacement forces existing instances
    // to re-enrol; the original receipt inode is pinned in memory.
    eprintln!("ROOT_MAIL_CI: replacement receipt re-open");
    assert!(open().is_ok());
    eprintln!("ROOT_MAIL_CI: prior instance receipt replay denial");
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        running.serve_one(&fixture.node, &listener, 4),
        Err(MailRoleHold::MailPolicy)
    ));
    let mut running = open().unwrap();

    // Writable group path cannot authorize a mailbox, even if signature
    // and verified SQLite row remain unchanged.
    sudo(&["chmod", "0770", ROOT]);
    assert!(matches!(open(), Err(MailRoleHold::MailPolicy)));
    sudo(&["chmod", "0750", ROOT]);
    assert!(open().is_ok());

    // Root changes the signed bytes: an existing role must stop even if
    // the new file has a valid (but unreviewed) signature. No auto-enrolment.
    sudo(&["rm", "-f", "--", POLICY]);
    sudo(&["ln", "-s", "/etc/passwd", POLICY]);
    assert!(matches!(open(), Err(MailRoleHold::MailPolicy)));
    let listener = bind_test_loopback().unwrap();
    assert!(matches!(
        running.serve_one(&fixture.node, &listener, 4),
        Err(MailRoleHold::MailPolicy)
    ));
}

#[test]
fn detached_owner_first_publish_approval_is_not_a_reusable_mail_grant() {
    let fixture = Fixture::new();
    let signed_policy = fs::read(fixture.mail_policy_file()).unwrap();
    let digest = *blake3::hash(&signed_policy).as_bytes();
    let claim = MailFirstApprovalClaims {
        version: 1,
        operation: 1,
        node_public: fixture.node.public_key(),
        node_endpoint: NODE_ENDPOINT,
        signed_policy_digest: digest,
        policy_epoch: 1,
        expected_witness_revision: 0,
        approval_nonce: [91; 16],
        helper_challenge: [92; 32],
        issued_at_ms: 1,
        valid_until_ms: 200_000,
    };
    let verify = |bytes: &[u8], revision: u64, now: i64| {
        first_mail_approval_proposal_for_lab(
            &fixture.node,
            &fixture.mail_policy_dir,
            NODE_ENDPOINT,
            bytes,
            revision,
            now,
        )
    };
    let approved = sign_first_mail_approval_for_lab(&fixture.owner, claim);
    let proposal = verify(&approved, 0, 3).unwrap();
    assert!(proposal.requires_independent_cas_commit);
    assert_eq!(proposal.required_next_revision, 1);
    assert_eq!(proposal.approved_digest, digest);
    assert_eq!(proposal.approval_nonce, claim.approval_nonce);

    // An owner-signed mailbox policy is NOT consent for the separate action.
    assert!(matches!(
        verify(&signed_policy, 0, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    let forged = sign_first_mail_approval_for_lab(&SigningKey::from_bytes(&[99; 32]), claim);
    assert!(matches!(
        verify(&forged, 0, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    assert!(matches!(
        verify(&approved, 1, 3),
        Err(MailRoleHold::MailPolicy)
    ));
    assert!(matches!(
        verify(&approved, 0, 200_000),
        Err(MailRoleHold::MailPolicy)
    ));

    let mut bad_digest = claim;
    bad_digest.signed_policy_digest = [55; 32];
    assert!(
        verify(
            &sign_first_mail_approval_for_lab(&fixture.owner, bad_digest),
            0,
            3,
        )
        .is_err()
    );
    let mut bad_endpoint = claim;
    bad_endpoint.node_endpoint = [77; 32];
    assert!(
        verify(
            &sign_first_mail_approval_for_lab(&fixture.owner, bad_endpoint),
            0,
            3,
        )
        .is_err()
    );
    let mut zero_nonce = claim;
    zero_nonce.approval_nonce = [0; 16];
    assert!(
        verify(
            &sign_first_mail_approval_for_lab(&fixture.owner, zero_nonce),
            0,
            3,
        )
        .is_err()
    );
    let mut oversize_window = claim;
    oversize_window.valid_until_ms = 1_000_000;
    assert!(
        verify(
            &sign_first_mail_approval_for_lab(&fixture.owner, oversize_window),
            0,
            3,
        )
        .is_err()
    );
    assert!(matches!(
        verify(&approved, 0, 0),
        Err(MailRoleHold::MailPolicy)
    ));
}
