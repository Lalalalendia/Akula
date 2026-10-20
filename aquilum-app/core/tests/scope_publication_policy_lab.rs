//! AK-DATA-CTRL-01C: synthetic, memory-only two-lock publication laboratory.
//! No vault access, disk writes, listener, production secret, or file deletion.
//! All signing keys and content bytes are deliberately public test fixtures.

use aquilum_core::sync::{DeviceId, DocumentId, RevisionId};
use ring::signature::{self, KeyPair};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Scope(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Role { Owner, Admin }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Action { Allow, Deny }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Target {
    Object(DocumentId),
    Revision(DocumentId, RevisionId),
    ExactContent([u8; 32]),
}

impl Target {
    fn document(self) -> Option<DocumentId> {
        match self {
            Self::Object(d) | Self::Revision(d, _) => Some(d),
            Self::ExactContent(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SignedDecision {
    scope: Scope,
    role: Role,
    target: Target,
    action: Action,
    sequence: u64,
    claimed_public_key: [u8; 32],
    signature: [u8; 64],
}

impl SignedDecision {
    fn bytes_to_sign(&self) -> Vec<u8> {
        // Fixed-width domain-separated lab encoding; not the shipping wire format.
        let mut out = b"AKULA:SCOPE:TWO-LOCK:LAB-V0\0".to_vec();
        out.push(self.scope.0);
        out.push(match self.role { Role::Owner => 1, Role::Admin => 2 });
        out.push(match self.action { Action::Allow => 1, Action::Deny => 2 });
        out.extend_from_slice(&self.sequence.to_be_bytes());
        match self.target {
            Target::Object(d) => {
                out.push(1);
                out.extend_from_slice(d.as_uuid().as_bytes());
            }
            Target::Revision(d, r) => {
                out.push(2);
                out.extend_from_slice(d.as_uuid().as_bytes());
                out.extend_from_slice(r.as_uuid().as_bytes());
            }
            Target::ExactContent(h) => {
                out.push(3);
                out.extend_from_slice(&h);
            }
        }
        out.extend_from_slice(&self.claimed_public_key);
        out
    }
}

fn signing_key(seed: u8) -> signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn sign(
    scope: Scope, role: Role, target: Target, action: Action, sequence: u64,
    key: &signature::Ed25519KeyPair,
) -> SignedDecision {
    let mut event = SignedDecision {
        scope, role, target, action, sequence,
        claimed_public_key: key.public_key().as_ref().try_into().unwrap(),
        signature: [0; 64],
    };
    event.signature = key.sign(&event.bytes_to_sign()).as_ref().try_into().unwrap();
    event
}

#[derive(Clone)]
struct Anchors {
    owners: BTreeMap<DocumentId, [u8; 32]>,
    admins: BTreeMap<Scope, [u8; 32]>,
}

#[derive(Debug, Eq, PartialEq)]
enum PolicyError { UnknownAnchor, WrongSigner, InvalidTarget, BadSignature, Equivocation }

#[derive(Debug, Eq, PartialEq)]
enum Ingest {
    Applied,
    Duplicate,
    // The corrective evidence is the ORIGINAL signature, not a new peer command.
    Stale(SignedDecision),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Publication { Allowed, Pending, Blocked, NeedsReview }

struct PolicyLab {
    anchors: Anchors,
    latest: BTreeMap<(Scope, Role, Target), SignedDecision>,
}

impl PolicyLab {
    fn new(anchors: Anchors) -> Self {
        Self { anchors, latest: BTreeMap::new() }
    }

    fn ingest(&mut self, event: SignedDecision) -> Result<Ingest, PolicyError> {
        let trusted = match event.role {
            Role::Owner => {
                let object = event.target.document().ok_or(PolicyError::InvalidTarget)?;
                self.anchors.owners.get(&object).ok_or(PolicyError::UnknownAnchor)?
            }
            Role::Admin => self.anchors.admins.get(&event.scope)
                .ok_or(PolicyError::UnknownAnchor)?,
        };
        if event.claimed_public_key != *trusted {
            return Err(PolicyError::WrongSigner);
        }
        signature::UnparsedPublicKey::new(&signature::ED25519, trusted)
            .verify(&event.bytes_to_sign(), &event.signature)
            .map_err(|_| PolicyError::BadSignature)?;

        let key = (event.scope, event.role, event.target);
        if let Some(previous) = self.latest.get(&key) {
            if event.sequence < previous.sequence {
                return Ok(Ingest::Stale(previous.clone()));
            }
            if event.sequence == previous.sequence {
                return if *previous == event {
                    Ok(Ingest::Duplicate)
                } else {
                    Err(PolicyError::Equivocation)
                };
            }
        }
        self.latest.insert(key, event);
        Ok(Ingest::Applied)
    }

    fn action(&self, scope: Scope, role: Role, target: Target) -> Option<Action> {
        self.latest.get(&(scope, role, target)).map(|e| e.action)
    }

    fn state(
        &self, scope: Scope, object: DocumentId, revision: RevisionId,
        plaintext: &[u8], similarity_hint: bool,
    ) -> Publication {
        let owner = self.action(scope, Role::Owner, Target::Object(object));
        let admin = self.action(scope, Role::Admin, Target::Object(object));
        let revision = Target::Revision(object, revision);
        let exact = Target::ExactContent(*blake3::hash(plaintext).as_bytes());
        if [owner, admin,
            self.action(scope, Role::Owner, revision),
            self.action(scope, Role::Admin, revision),
            self.action(scope, Role::Admin, exact),
        ].contains(&Some(Action::Deny)) {
            return Publication::Blocked;
        }
        if owner != Some(Action::Allow) || admin != Some(Action::Allow) {
            return Publication::Pending;
        }
        if similarity_hint { Publication::NeedsReview } else { Publication::Allowed }
    }
}

struct Fixture {
    a: Scope,
    b: Scope,
    file: DocumentId,
    new_file: DocumentId,
    v1: RevisionId,
    v2: RevisionId,
    owner: signature::Ed25519KeyPair,
    admin_a: signature::Ed25519KeyPair,
    admin_b: signature::Ed25519KeyPair,
    outsider: signature::Ed25519KeyPair,
    anchors: Anchors,
}

impl Fixture {
    fn new() -> Self {
        let (a, b) = (Scope(1), Scope(2));
        let (file, new_file) = (
            DocumentId::from_uuid(Uuid::from_u128(11)),
            DocumentId::from_uuid(Uuid::from_u128(12)),
        );
        let (v1, v2) = (
            RevisionId::from_uuid(Uuid::from_u128(101)),
            RevisionId::from_uuid(Uuid::from_u128(102)),
        );
        let owner = signing_key(11);
        let admin_a = signing_key(21);
        let admin_b = signing_key(22);
        let outsider = signing_key(99);
        let mut owners = BTreeMap::new();
        owners.insert(file, owner.public_key().as_ref().try_into().unwrap());
        owners.insert(new_file, owner.public_key().as_ref().try_into().unwrap());
        let mut admins = BTreeMap::new();
        admins.insert(a, admin_a.public_key().as_ref().try_into().unwrap());
        admins.insert(b, admin_b.public_key().as_ref().try_into().unwrap());
        Self {
            a, b, file, new_file, v1, v2,
            owner, admin_a, admin_b, outsider, anchors: Anchors { owners, admins },
        }
    }

    fn event(
        &self, scope: Scope, role: Role, target: Target, action: Action, seq: u64,
    ) -> SignedDecision {
        let key = match role {
            Role::Owner => &self.owner,
            Role::Admin if scope == self.a => &self.admin_a,
            Role::Admin => &self.admin_b,
        };
        sign(scope, role, target, action, seq, key)
    }

    fn allow(&self, lab: &mut PolicyLab, scope: Scope, file: DocumentId) {
        for role in [Role::Owner, Role::Admin] {
            assert_eq!(
                lab.ingest(self.event(scope, role, Target::Object(file), Action::Allow, 1)),
                Ok(Ingest::Applied),
            );
        }
    }
}

struct TestDevice {
    id: DeviceId,
    policy: PolicyLab,
}

impl TestDevice {
    fn new(n: u128, anchors: &Anchors) -> Self {
        Self {
            id: DeviceId::from_uuid(Uuid::from_u128(1000 + n)),
            policy: PolicyLab::new(anchors.clone()),
        }
    }
}

const CONTENT_V1: &[u8] = b"SYNTHETIC AKULA PUBLIC TEST DATA v1";
const CONTENT_V2: &[u8] = b"SYNTHETIC AKULA PUBLIC TEST DATA v2 - changed";

#[test]
fn two_distinct_signatures_required_before_publication() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Pending);
    p.ingest(f.event(f.a, Role::Owner, Target::Object(f.file), Action::Allow, 1)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Pending);
    p.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 1)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
    assert_eq!(p.state(f.b, f.file, f.v1, CONTENT_V1, false), Publication::Pending);
}

#[test]
fn three_devices_two_scopes_ban_remains_local_even_after_modified_revision() {
    let f = Fixture::new();
    let mut devices = [
        TestDevice::new(1, &f.anchors),
        TestDevice::new(2, &f.anchors),
        TestDevice::new(3, &f.anchors),
    ];
    assert_ne!(devices[0].id, devices[1].id);
    assert_ne!(devices[1].id, devices[2].id);
    for device in &mut devices {
        f.allow(&mut device.policy, f.a, f.file);
        f.allow(&mut device.policy, f.b, f.file);
    }
    let anti = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2);
    devices[0].policy.ingest(anti.clone()).unwrap();
    // Offline device has NOT yet received the denial: no instant-revoke claim.
    assert_eq!(devices[1].policy.state(f.a, f.file, f.v1, CONTENT_V1, false),
        Publication::Allowed);
    // A forwards the origin-signed evidence, not its own deletion command.
    for peer in &mut devices[1..] {
        assert_eq!(peer.policy.ingest(anti.clone()), Ok(Ingest::Applied));
    }
    for peer in &devices {
        for (rev, bytes) in [(f.v1, CONTENT_V1), (f.v2, CONTENT_V2)] {
            assert_eq!(peer.policy.state(f.a, f.file, rev, bytes, false),
                Publication::Blocked);
            assert_eq!(peer.policy.state(f.b, f.file, rev, bytes, false),
                Publication::Allowed);
        }
    }
}

#[test]
fn owner_withdrawal_cannot_be_overruled_by_scope_administrator() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    f.allow(&mut p, f.b, f.file);
    p.ingest(f.event(f.a, Role::Owner, Target::Object(f.file), Action::Deny, 2)).unwrap();
    p.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 90)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Blocked);
    assert_eq!(p.state(f.b, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
}

#[test]
fn revision_ban_is_narrower_than_object_ban() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    p.ingest(f.event(f.a, Role::Admin, Target::Revision(f.file, f.v1), Action::Deny, 1)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Blocked);
    assert_eq!(p.state(f.a, f.file, f.v2, CONTENT_V2, false), Publication::Allowed);
}

#[test]
fn exact_plaintext_match_catches_new_id_without_global_ban() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    for object in [f.file, f.new_file] {
        f.allow(&mut p, f.a, object);
        f.allow(&mut p, f.b, object);
    }
    let digest = Target::ExactContent(*blake3::hash(CONTENT_V1).as_bytes());
    p.ingest(f.event(f.a, Role::Admin, digest, Action::Deny, 1)).unwrap();
    assert_eq!(p.state(f.a, f.new_file, f.v2, CONTENT_V1, false),
        Publication::Blocked);
    assert_eq!(p.state(f.b, f.new_file, f.v2, CONTENT_V1, false),
        Publication::Allowed);
    assert_eq!(p.state(f.a, f.new_file, f.v2, CONTENT_V2, true),
        Publication::NeedsReview);
    assert_eq!(p.state(f.a, f.new_file, f.v2, CONTENT_V2, false),
        Publication::Allowed,
        "changed bytes without trusted provenance cannot be proven identical");
}

#[test]
fn forged_key_wrong_scope_and_modified_signed_payload_fail() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    let forged = sign(f.a, Role::Admin, Target::Object(f.file),
        Action::Deny, 2, &f.outsider);
    assert_eq!(p.ingest(forged), Err(PolicyError::WrongSigner));
    let wrong_scope = sign(f.a, Role::Admin, Target::Object(f.file),
        Action::Deny, 2, &f.admin_b);
    assert_eq!(p.ingest(wrong_scope), Err(PolicyError::WrongSigner));
    let mut tampered = f.event(f.a, Role::Admin, Target::Object(f.file),
        Action::Deny, 2);
    tampered.action = Action::Allow;
    assert_eq!(p.ingest(tampered), Err(PolicyError::BadSignature));
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
}

#[test]
fn owner_cannot_issue_digest_wide_bans_against_other_owners() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    let forged = f.event(f.a, Role::Owner,
        Target::ExactContent(*blake3::hash(CONTENT_V1).as_bytes()), Action::Deny, 1);
    assert_eq!(p.ingest(forged), Err(PolicyError::InvalidTarget));
}

#[test]
fn old_allow_returns_origin_signed_anti_to_the_stale_source() {
    let f = Fixture::new();
    let mut a = TestDevice::new(1, &f.anchors);
    let mut b = TestDevice::new(2, &f.anchors);
    let mut c = TestDevice::new(3, &f.anchors);
    for device in [&mut a, &mut b, &mut c] {
        f.allow(&mut device.policy, f.a, f.file);
        f.allow(&mut device.policy, f.b, f.file);
    }
    let old = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 1);
    let anti = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2);
    assert_eq!(a.policy.ingest(anti.clone()), Ok(Ingest::Applied));
    assert_eq!(a.policy.ingest(old), Ok(Ingest::Stale(anti.clone())));
    for peer in [&mut b, &mut c] {
        assert_eq!(peer.policy.ingest(anti.clone()), Ok(Ingest::Applied));
        assert_eq!(peer.policy.ingest(anti.clone()), Ok(Ingest::Duplicate));
        assert_eq!(peer.policy.state(f.a, f.file, f.v1, CONTENT_V1, false),
            Publication::Blocked);
        assert_eq!(peer.policy.state(f.b, f.file, f.v1, CONTENT_V1, false),
            Publication::Allowed);
    }
}

#[test]
fn sequence_equivocation_rejected_but_newer_allow_can_restore_own_role_only() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    let deny = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2);
    assert_eq!(p.ingest(deny.clone()), Ok(Ingest::Applied));
    let clash = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 2);
    assert_eq!(p.ingest(clash), Err(PolicyError::Equivocation));
    assert_eq!(p.ingest(deny), Ok(Ingest::Duplicate));
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Blocked);
    p.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 3)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
    p.ingest(f.event(f.a, Role::Owner, Target::Object(f.file), Action::Deny, 2)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Blocked);
}

#[test]
fn no_remote_policy_op_can_mutate_original_owner_bytes() {
    let f = Fixture::new();
    let original = b"PRIVATE OWNER ORIGINAL IS NOT SUBJECT TO REMOTE DELETE".to_vec();
    let snapshot = original.clone();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    p.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2)).unwrap();
    assert_eq!(p.state(f.a, f.file, f.v1, &original, false), Publication::Blocked);
    assert_eq!(original, snapshot);
}


// The next tests add a durable, in-memory-verified negative witness. Only a
// synthetic temp SQLite DB is written: never a user's vault or content file.
// The trusted checkpoint MUST be held independently of that DB on reopen.
// No production checkpoint publisher, key-custody or filesystem binding exists.
use rusqlite::{params, Connection, TransactionBehavior};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WitnessPin {
    Genesis,
    // Only a checkpoint supplied by an INDEPENDENT trusted caller may be used.
    Exact([u8; 32]),
}

#[derive(Debug)]
enum WitnessError {
    Sql(rusqlite::Error),
    Policy(PolicyError),
    CorruptProof,
    AbsentWitness,
    UnanchoredReopen,
    RollbackOrTamper,
    PoisonedAfterStorageFailure,
}

fn take_bytes<'a>(remaining: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if remaining.len() < n { return None; }
    let (head, rest) = remaining.split_at(n);
    *remaining = rest;
    Some(head)
}

impl SignedDecision {
    fn durable_proof(&self) -> Vec<u8> {
        let mut proof = self.bytes_to_sign();
        proof.extend_from_slice(&self.signature);
        proof
    }

    fn decode_durable_proof(data: &[u8]) -> Option<Self> {
        const DOMAIN: &[u8] = b"AKULA:SCOPE:TWO-LOCK:LAB-V0\0";
        let mut remaining = data.strip_prefix(DOMAIN)?;
        let scope = Scope(*take_bytes(&mut remaining, 1)?.first()?);
        let role = match take_bytes(&mut remaining, 1)?[0] {
            1 => Role::Owner,
            2 => Role::Admin,
            _ => return None,
        };
        let action = match take_bytes(&mut remaining, 1)?[0] {
            1 => Action::Allow,
            2 => Action::Deny,
            _ => return None,
        };
        let sequence = u64::from_be_bytes(take_bytes(&mut remaining, 8)?.try_into().ok()?);
        let tag = take_bytes(&mut remaining, 1)?[0];
        let target = match tag {
            1 => Target::Object(DocumentId::from_uuid(Uuid::from_bytes(
                take_bytes(&mut remaining, 16)?.try_into().ok()?,
            ))),
            2 => Target::Revision(
                DocumentId::from_uuid(Uuid::from_bytes(
                    take_bytes(&mut remaining, 16)?.try_into().ok()?,
                )),
                RevisionId::from_uuid(Uuid::from_bytes(
                    take_bytes(&mut remaining, 16)?.try_into().ok()?,
                )),
            ),
            3 => Target::ExactContent(take_bytes(&mut remaining, 32)?.try_into().ok()?),
            _ => return None,
        };
        let claimed_public_key = take_bytes(&mut remaining, 32)?.try_into().ok()?;
        let signature = take_bytes(&mut remaining, 64)?.try_into().ok()?;
        if !remaining.is_empty() { return None; }
        let event = Self { scope, role, target, action, sequence, claimed_public_key, signature };
        // A decoding ambiguity must never create a new meaning for signed bytes.
        if event.durable_proof() != data { return None; }
        Some(event)
    }

    fn witness_row_key(&self) -> Vec<u8> {
        let mut key = vec![
            self.scope.0,
            match self.role { Role::Owner => 1, Role::Admin => 2 },
        ];
        match self.target {
            Target::Object(doc) => {
                key.push(1);
                key.extend_from_slice(doc.as_uuid().as_bytes());
            }
            Target::Revision(doc, revision) => {
                key.push(2);
                key.extend_from_slice(doc.as_uuid().as_bytes());
                key.extend_from_slice(revision.as_uuid().as_bytes());
            }
            Target::ExactContent(hash) => {
                key.push(3);
                key.extend_from_slice(&hash);
            }
        }
        key
    }
}

struct DurablePolicyLab {
    conn: Connection,
    policy: PolicyLab,
    poisoned: bool,
}

impl DurablePolicyLab {
    fn open(path: &Path, anchors: Anchors, pin: WitnessPin) -> Result<Self, WitnessError> {
        let existed = path.exists();
        match (existed, pin) {
            (true, WitnessPin::Genesis) => return Err(WitnessError::UnanchoredReopen),
            (false, WitnessPin::Exact(_)) => return Err(WitnessError::AbsentWitness),
            _ => {}
        }
        let conn = Connection::open(path).map_err(WitnessError::Sql)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS witnessed_decisions (
                 record_key BLOB PRIMARY KEY NOT NULL,
                 signed_proof BLOB NOT NULL
             ) WITHOUT ROWID;",
        ).map_err(WitnessError::Sql)?;

        let mut policy = PolicyLab::new(anchors);
        {
            let mut query = conn.prepare(
                "SELECT record_key, signed_proof FROM witnessed_decisions ORDER BY record_key",
            ).map_err(WitnessError::Sql)?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            }).map_err(WitnessError::Sql)?;
            for row in rows {
                let (key, proof) = row.map_err(WitnessError::Sql)?;
                let event = SignedDecision::decode_durable_proof(&proof)
                    .ok_or(WitnessError::CorruptProof)?;
                if event.witness_row_key() != key {
                    return Err(WitnessError::CorruptProof);
                }
                if policy.ingest(event).map_err(WitnessError::Policy)? != Ingest::Applied {
                    return Err(WitnessError::CorruptProof);
                }
            }
        }

        let store = Self { conn, policy, poisoned: false };
        if let WitnessPin::Exact(expected) = pin {
            if store.checkpoint() != expected {
                return Err(WitnessError::RollbackOrTamper);
            }
        }
        Ok(store)
    }

    // A snapshot commitment is not itself an independent root of trust.
    // A production caller must keep the expected hash outside this SQLite DB.
    fn checkpoint(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"AKULA:SCOPE:WITNESS-CHECKPOINT-V0\0");
        hasher.update(&(self.policy.latest.len() as u64).to_be_bytes());
        for ((scope, role, subject), event) in &self.policy.latest {
            // Include key and exact signed proof, in deterministic BTree order.
            let key = event.witness_row_key();
            assert_eq!((*scope, *role, *subject), (event.scope, event.role, event.target));
            hasher.update(&(key.len() as u32).to_be_bytes());
            hasher.update(&key);
            let proof = event.durable_proof();
            hasher.update(&(proof.len() as u32).to_be_bytes());
            hasher.update(&proof);
        }
        *hasher.finalize().as_bytes()
    }

    fn ingest(&mut self, event: SignedDecision) -> Result<Ingest, WitnessError> {
        if self.poisoned {
            return Err(WitnessError::PoisonedAfterStorageFailure);
        }
        // Check signature and causality on a clone BEFORE attempting persistence.
        let mut next = PolicyLab {
            anchors: self.policy.anchors.clone(),
            latest: self.policy.latest.clone(),
        };
        let outcome = next.ingest(event.clone()).map_err(WitnessError::Policy)?;
        if outcome != Ingest::Applied {
            return Ok(outcome);
        }
        let key = event.witness_row_key();
        let proof = event.durable_proof();
        // Commit durably BEFORE exposing the next policy state to callers.
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO witnessed_decisions(record_key, signed_proof)
                 VALUES (?1, ?2)
                 ON CONFLICT(record_key) DO UPDATE SET signed_proof=excluded.signed_proof",
                params![key, proof],
            )?;
            tx.commit()
        })();
        if let Err(error) = result {
            // An uncertain DB outcome must NOT authorize a new publication.
            self.poisoned = true;
            return Err(WitnessError::Sql(error));
        }
        self.policy = next;
        Ok(Ingest::Applied)
    }

    fn state(
        &self, scope: Scope, doc: DocumentId, revision: RevisionId, plaintext: &[u8],
    ) -> Publication {
        if self.poisoned {
            Publication::Pending
        } else {
            self.policy.state(scope, doc, revision, plaintext, false)
        }
    }
}

#[test]
fn reboot_keeps_signed_anti_meme_and_refuses_old_grant() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let deny = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2);
    assert_eq!(store.ingest(deny.clone()).unwrap(), Ingest::Applied);
    assert_eq!(store.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Blocked);
    let pinned_outside_database = store.checkpoint();
    drop(store);
    let mut restarted = DurablePolicyLab::open(
        &db, f.anchors.clone(), WitnessPin::Exact(pinned_outside_database),
    ).unwrap();
    assert_eq!(restarted.state(f.a, f.file, f.v2, CONTENT_V2), Publication::Blocked);
    let replayed = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 1);
    assert_eq!(restarted.ingest(replayed).unwrap(), Ingest::Stale(deny));
}

#[test]
fn restored_old_sqlite_snapshot_is_detected_with_external_checkpoint() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let backup = temp.path().join("older_snapshot.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let earlier_pin = store.checkpoint();
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(store);
    std::fs::copy(&db, &backup).unwrap();
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(),
        WitnessPin::Exact(earlier_pin)).unwrap();
    store.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2)).unwrap();
    let after_deny_pin = store.checkpoint();
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(store);
    std::fs::copy(&backup, &db).unwrap();
    assert!(matches!(
        DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Exact(after_deny_pin)),
        Err(WitnessError::RollbackOrTamper)
    ));
    assert!(matches!(
        DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis),
        Err(WitnessError::UnanchoredReopen)
    ));
}

#[test]
fn corrupt_signed_proof_or_unpinned_root_cannot_boot() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    store.ingest(f.event(f.a, Role::Owner, Target::Object(f.file), Action::Allow, 1)).unwrap();
    let pin = store.checkpoint();
    store.conn.execute(
        "UPDATE witnessed_decisions SET signed_proof = x'00' WHERE record_key =
         (SELECT record_key FROM witnessed_decisions LIMIT 1)", [],
    ).unwrap();
    drop(store);
    assert!(matches!(
        DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Exact(pin)),
        Err(WitnessError::CorruptProof)
    ));
}

#[test]
fn failed_sqlite_commit_does_not_permit_unverified_policy_state() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let trusted_before = store.checkpoint();
    store.conn.execute_batch(
        "CREATE TRIGGER synthetic_storage_failure
         BEFORE UPDATE ON witnessed_decisions
         BEGIN SELECT RAISE(ABORT, 'synthetic injected failure'); END;",
    ).unwrap();
    let failure = store.ingest(f.event(
        f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2,
    ));
    assert!(matches!(failure, Err(WitnessError::Sql(_))));
    assert_eq!(store.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Pending);
    assert!(matches!(
        store.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 3)),
        Err(WitnessError::PoisonedAfterStorageFailure)
    ));
    drop(store);
    // A new independently pinned open safely recovers the last committed state;
    // it must still query owner/admin scope permissions before any real serving.
    let restarted = DurablePolicyLab::open(
        &db, f.anchors.clone(), WitnessPin::Exact(trusted_before),
    ).unwrap();
    assert_eq!(restarted.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Allowed);
}

#[test]
fn removed_witness_cannot_silently_recreate_genesis_from_old_disk() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("deleted.sqlite");
    let phantom = [7u8; 32];
    assert!(matches!(
        DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Exact(phantom)),
        Err(WitnessError::AbsentWitness)
    ));
}
