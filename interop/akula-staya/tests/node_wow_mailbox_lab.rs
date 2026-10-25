//! Default-off Akula NodeV1 / pinned WOW mailbox integration acceptance.
//! No installed service, production owner policy v4, or public networking.
#![cfg(all(target_os = "linux", feature = "node-wow-mailbox-lab"))]

use akula_staya_integration::node_wow_mailbox_lab::{
    MailRoleClaims, MailRoleHold, NodeWowMailboxLab, mail_epoch_floor_for_lab,
    sign_mail_policy_for_lab,
};
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
use rusqlite::Connection;
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
