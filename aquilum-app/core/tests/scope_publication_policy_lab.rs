//! AK-DATA-CTRL-01C: synthetic, memory-only two-lock publication laboratory.
//! No vault access, disk writes, listener, production secret, or file deletion.
//! All signing keys and content bytes are deliberately public test fixtures.

use aquilum_core::sync::{DeviceId, DocumentId, RevisionId};
use ring::signature::{self, KeyPair};
use std::collections::{BTreeMap, BTreeSet};
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
    quarantined: BTreeSet<(Scope, Role, Target)>,
}

impl PolicyLab {
    fn new(anchors: Anchors) -> Self {
        Self { anchors, latest: BTreeMap::new(), quarantined: BTreeSet::new() }
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
        if self.quarantined.contains(&key) {
            // Equivocation needs explicit root-authorized recovery, not seq+1.
            return Err(PolicyError::Equivocation);
        }
        if let Some(previous) = self.latest.get(&key) {
            if event.sequence < previous.sequence {
                return Ok(Ingest::Stale(previous.clone()));
            }
            if event.sequence == previous.sequence {
                return if *previous == event {
                    Ok(Ingest::Duplicate)
                } else {
                    self.quarantined.insert(key);
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
        // A valid conflicting owner/admin signature makes this subject unsafe.
        // No publication can use an arbitrary first-seen decision.
        let relevant = [
            (scope, Role::Owner, Target::Object(object)),
            (scope, Role::Admin, Target::Object(object)),
            (scope, Role::Owner, revision),
            (scope, Role::Admin, revision),
            (scope, Role::Admin, exact),
        ];
        if relevant.iter().any(|key| self.quarantined.contains(key)) {
            return Publication::Pending;
        }
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
fn same_sequence_equivocation_quarantines_the_object_not_the_other_scope() {
    let f = Fixture::new();
    let mut p = PolicyLab::new(f.anchors.clone());
    f.allow(&mut p, f.a, f.file);
    f.allow(&mut p, f.b, f.file);
    let deny = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2);
    assert_eq!(p.ingest(deny.clone()), Ok(Ingest::Applied));
    let clash = f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 2);
    assert_eq!(p.ingest(clash), Err(PolicyError::Equivocation));
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Pending);
    assert_eq!(p.state(f.b, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
    // A later valid signature does not silently resolve an owner's equivocation:
    // this lab has no trusted admin key-epoch recovery procedure yet.
    assert_eq!(
        p.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 3)),
        Err(PolicyError::Equivocation)
    );
    assert_eq!(p.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Pending);
    // In an independently clean lineage the admin CAN still issue seq+1.
    let mut clean = PolicyLab::new(f.anchors.clone());
    f.allow(&mut clean, f.a, f.file);
    clean.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Allow, 2)).unwrap();
    assert_eq!(clean.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Allowed);
    clean.ingest(f.event(f.a, Role::Owner, Target::Object(f.file), Action::Deny, 2)).unwrap();
    assert_eq!(clean.state(f.a, f.file, f.v1, CONTENT_V1, false), Publication::Blocked);
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
    // The actual signed conflicting proof must survive reboot, not just a flag.
    conflict_proofs: BTreeMap<(Scope, Role, Target), SignedDecision>,
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
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS witnessed_conflicts (
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

        let mut conflict_proofs = BTreeMap::new();
        {
            let mut query = conn.prepare(
                "SELECT record_key, signed_proof FROM witnessed_conflicts ORDER BY record_key",
            ).map_err(WitnessError::Sql)?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            }).map_err(WitnessError::Sql)?;
            for row in rows {
                let (key, proof) = row.map_err(WitnessError::Sql)?;
                let event = SignedDecision::decode_durable_proof(&proof)
                    .ok_or(WitnessError::CorruptProof)?;
                let identity = (event.scope, event.role, event.target);
                if event.witness_row_key() != key || conflict_proofs.contains_key(&identity) {
                    return Err(WitnessError::CorruptProof);
                }
                let original = policy.latest.get(&identity)
                    .ok_or(WitnessError::CorruptProof)?;
                if original.sequence != event.sequence || *original == event {
                    return Err(WitnessError::CorruptProof);
                }
                if policy.ingest(event.clone()) != Err(PolicyError::Equivocation) {
                    return Err(WitnessError::CorruptProof);
                }
                conflict_proofs.insert(identity, event);
            }
        }
        let store = Self { conn, policy, conflict_proofs, poisoned: false };
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
        hasher.update(b"AKULA:SCOPE:WITNESS-CHECKPOINT-V1\0");
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
        // The external pin commits to BOTH valid decisions and conflicting
        // original-signed evidence; a disk snapshot lacking a known conflict
        // is an unauthorized rollback even if the older allow is valid.
        hasher.update(&(self.conflict_proofs.len() as u64).to_be_bytes());
        for (identity, event) in &self.conflict_proofs {
            let key = event.witness_row_key();
            assert_eq!(*identity, (event.scope, event.role, event.target));
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
            quarantined: self.policy.quarantined.clone(),
        };
        let outcome = match next.ingest(event.clone()) {
            Ok(outcome) => outcome,
            Err(PolicyError::Equivocation) => {
                let identity = (event.scope, event.role, event.target);
                if self.conflict_proofs.contains_key(&identity) {
                    return Err(WitnessError::Policy(PolicyError::Equivocation));
                }
                // Invariant: first conflicting, signature-valid claim at
                // the same sequence of the same scope / role / target.
                let Some(previous) = self.policy.latest.get(&identity) else {
                    self.poisoned = true;
                    return Err(WitnessError::CorruptProof);
                };
                if previous.sequence != event.sequence || *previous == event {
                    self.poisoned = true;
                    return Err(WitnessError::CorruptProof);
                }
                let key = event.witness_row_key();
                let proof = event.durable_proof();
                let result = (|| -> rusqlite::Result<()> {
                    let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    tx.execute(
                        "INSERT INTO witnessed_conflicts(record_key, signed_proof)
                         VALUES (?1, ?2)", params![key, proof],
                    )?;
                    tx.commit()
                })();
                if let Err(error) = result {
                    self.poisoned = true;
                    return Err(WitnessError::Sql(error));
                }
                self.policy = next;
                self.conflict_proofs.insert(identity, event);
                // A committed conflict must be surfaced and must NOT publish.
                return Err(WitnessError::Policy(PolicyError::Equivocation));
            }
            Err(error) => return Err(WitnessError::Policy(error)),
        };
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


#[test]
fn conflicting_real_admin_signatures_remain_quarantined_after_reboot() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for scope in [f.a, f.b] {
        for role in [Role::Owner, Role::Admin] {
            store.ingest(f.event(scope, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
        }
    }
    store.ingest(f.event(
        f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2,
    )).unwrap();
    let contradiction = f.event(
        f.a, Role::Admin, Target::Object(f.file), Action::Allow, 2,
    );
    assert!(matches!(
        store.ingest(contradiction.clone()),
        Err(WitnessError::Policy(PolicyError::Equivocation))
    ));
    assert_eq!(store.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Pending);
    assert_eq!(store.state(f.b, f.file, f.v1, CONTENT_V1), Publication::Allowed);
    let trusted_conflict_pin = store.checkpoint();
    drop(store);
    let mut restarted = DurablePolicyLab::open(
        &db, f.anchors.clone(), WitnessPin::Exact(trusted_conflict_pin),
    ).unwrap();
    assert_eq!(restarted.state(f.a, f.file, f.v2, CONTENT_V2), Publication::Pending);
    assert_eq!(restarted.state(f.b, f.file, f.v1, CONTENT_V1), Publication::Allowed);
    assert!(matches!(
        restarted.ingest(f.event(
            f.a, Role::Admin, Target::Object(f.file), Action::Allow, 3,
        )),
        Err(WitnessError::Policy(PolicyError::Equivocation))
    ));
}

#[test]
fn corrupt_conflict_proof_is_rejected_even_with_valid_previous_allow() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let contradiction = f.event(
        f.a, Role::Admin, Target::Object(f.file), Action::Deny, 1,
    );
    assert!(matches!(
        store.ingest(contradiction),
        Err(WitnessError::Policy(PolicyError::Equivocation))
    ));
    assert_eq!(store.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Pending);
    let checkpoint = store.checkpoint();
    store.conn.execute(
        "UPDATE witnessed_conflicts SET signed_proof = x'00'", [],
    ).unwrap();
    drop(store);
    assert!(matches!(
        DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Exact(checkpoint)),
        Err(WitnessError::CorruptProof)
    ));
}

#[test]
fn forged_contradiction_does_not_create_quarantine_or_modify_checkpoint() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("witness.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let trusted_pin = store.checkpoint();
    assert!(matches!(
        store.ingest(sign(
            f.a, Role::Admin, Target::Object(f.file), Action::Deny, 1,
            &f.outsider,
        )),
        Err(WitnessError::Policy(PolicyError::WrongSigner))
    ));
    assert_eq!(store.checkpoint(), trusted_pin);
    assert_eq!(store.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Allowed);
}


// AK-DATA-CTRL-01D: in-memory simulation of independently queried witness
// services. No real remote authority or machine-protected storage exists.
// This tests challenge freshness + at-least-two agreeing signed *latest*
// reports, not a saved quorum certificate that could itself be replayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignedWitnessReply {
    witness_id: u8,
    installation: [u8; 16],
    challenge: [u8; 16],
    generation: u64,
    policy_checkpoint: [u8; 32],
    signature: [u8; 64],
}

impl SignedWitnessReply {
    fn bytes_to_sign(&self) -> Vec<u8> {
        let mut out = b"AKULA:INDEPENDENT-WITNESS:REPLY-V0\0".to_vec();
        out.push(self.witness_id);
        out.extend_from_slice(&self.installation);
        out.extend_from_slice(&self.challenge);
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.policy_checkpoint);
        out
    }
}

struct SimulatedIndependentWitness {
    id: u8,
    key: signature::Ed25519KeyPair,
    last_generation: u64,
    last_checkpoint: [u8; 32],
}

impl SimulatedIndependentWitness {
    fn new(id: u8) -> Self {
        Self {
            id,
            key: signing_key(60 + id),
            last_generation: 0,
            last_checkpoint: [0; 32],
        }
    }

    fn public_key(&self) -> [u8; 32] {
        self.key.public_key().as_ref().try_into().unwrap()
    }

    fn advance(&mut self, generation: u64, checkpoint: [u8; 32]) -> Result<(), ()> {
        if generation < self.last_generation {
            return Err(());
        }
        if generation == self.last_generation {
            return if checkpoint == self.last_checkpoint { Ok(()) } else { Err(()) };
        }
        self.last_generation = generation;
        self.last_checkpoint = checkpoint;
        Ok(())
    }

    fn attest(&self, installation: [u8; 16], challenge: [u8; 16]) -> SignedWitnessReply {
        let mut reply = SignedWitnessReply {
            witness_id: self.id,
            installation,
            challenge,
            generation: self.last_generation,
            policy_checkpoint: self.last_checkpoint,
            signature: [0; 64],
        };
        reply.signature = self.key.sign(&reply.bytes_to_sign()).as_ref().try_into().unwrap();
        reply
    }
}

fn test_witnesses() -> (Vec<SimulatedIndependentWitness>, BTreeMap<u8, [u8; 32]>) {
    let witnesses: Vec<_> = (1..=3).map(SimulatedIndependentWitness::new).collect();
    let pinned_keys = witnesses.iter().map(|w| (w.id, w.public_key())).collect();
    (witnesses, pinned_keys)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WitnessQuorumError {
    WrongInstallation,
    WrongChallenge,
    UnknownWitness,
    BadSignature,
    DuplicateWitness,
    SplitHeads,
    InsufficientWitnesses,
}

fn verify_fresh_witness_quorum(
    known_witness_keys: &BTreeMap<u8, [u8; 32]>,
    expected_installation: [u8; 16],
    expected_challenge: [u8; 16],
    replies: &[SignedWitnessReply],
) -> Result<(u64, [u8; 32]), WitnessQuorumError> {
    // A fixed, independently pinned test configuration. Never learn quorum
    // participants, installation ID or signer public keys from a reply.
    if known_witness_keys.len() != 3 {
        return Err(WitnessQuorumError::InsufficientWitnesses);
    }
    let mut observed_ids = BTreeSet::new();
    let mut common_head = None;
    for reply in replies {
        let trusted_key = known_witness_keys.get(&reply.witness_id)
            .ok_or(WitnessQuorumError::UnknownWitness)?;
        if !observed_ids.insert(reply.witness_id) {
            return Err(WitnessQuorumError::DuplicateWitness);
        }
        if reply.installation != expected_installation {
            return Err(WitnessQuorumError::WrongInstallation);
        }
        if reply.challenge != expected_challenge {
            return Err(WitnessQuorumError::WrongChallenge);
        }
        signature::UnparsedPublicKey::new(&signature::ED25519, trusted_key)
            .verify(&reply.bytes_to_sign(), &reply.signature)
            .map_err(|_| WitnessQuorumError::BadSignature)?;
        let value = (reply.generation, reply.policy_checkpoint);
        if let Some(previous) = common_head {
            if previous != value {
                // A partial update or a malicious witness must NOT silently
                // choose the most convenient old/first head.
                return Err(WitnessQuorumError::SplitHeads);
            }
        } else {
            common_head = Some(value);
        }
    }
    if observed_ids.len() < 2 {
        return Err(WitnessQuorumError::InsufficientWitnesses);
    }
    common_head.ok_or(WitnessQuorumError::InsufficientWitnesses)
}

#[derive(Debug)]
enum WitnessQuorumOpenError {
    Quorum(WitnessQuorumError),
    Storage(WitnessError),
}

fn open_from_current_witness_quorum(
    path: &Path,
    anchors: Anchors,
    trusted_keys: &BTreeMap<u8, [u8; 32]>,
    installation: [u8; 16],
    challenge: [u8; 16],
    replies: &[SignedWitnessReply],
) -> Result<DurablePolicyLab, WitnessQuorumOpenError> {
    let (_generation, checkpoint) =
        verify_fresh_witness_quorum(trusted_keys, installation, challenge, replies)
            .map_err(WitnessQuorumOpenError::Quorum)?;
    DurablePolicyLab::open(path, anchors, WitnessPin::Exact(checkpoint))
        .map_err(WitnessQuorumOpenError::Storage)
}

const TEST_INSTALLATION: [u8; 16] = [0x4a; 16];
const TEST_CHALLENGE: [u8; 16] = [0xb1; 16];

fn durable_revoked_fixture() -> (Fixture, tempfile::TempDir, std::path::PathBuf, [u8; 32]) {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("signed-policy.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    store.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2)).unwrap();
    let checkpoint = store.checkpoint();
    drop(store);
    (f, temp, db, checkpoint)
}

#[test]
fn two_independent_fresh_witnesses_can_open_after_one_is_lost() {
    let (f, _temp, db, checkpoint) = durable_revoked_fixture();
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses {
        witness.advance(1, checkpoint).unwrap();
    }
    // Witness 2 is offline; two *different* sources still agree on latest.
    let replies = [
        witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE),
        witnesses[2].attest(TEST_INSTALLATION, TEST_CHALLENGE),
    ];
    let open = open_from_current_witness_quorum(
        &db, f.anchors.clone(), &trusted, TEST_INSTALLATION, TEST_CHALLENGE, &replies,
    ).unwrap();
    assert_eq!(open.state(f.a, f.file, f.v1, CONTENT_V1), Publication::Blocked);
}

#[test]
fn one_remaining_witness_is_never_a_fallback_trust_root() {
    let (f, _temp, db, checkpoint) = durable_revoked_fixture();
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses { witness.advance(1, checkpoint).unwrap(); }
    let replies = [witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE)];
    assert!(matches!(
        open_from_current_witness_quorum(
            &db, f.anchors.clone(), &trusted, TEST_INSTALLATION, TEST_CHALLENGE, &replies,
        ),
        Err(WitnessQuorumOpenError::Quorum(WitnessQuorumError::InsufficientWitnesses))
    ));
}

#[test]
fn recorded_signed_witness_reply_does_not_satisfy_a_new_challenge() {
    let (_f, _temp, _db, checkpoint) = durable_revoked_fixture();
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses { witness.advance(1, checkpoint).unwrap(); }
    let recorded = [
        witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE),
        witnesses[1].attest(TEST_INSTALLATION, TEST_CHALLENGE),
    ];
    let fresh_challenge = [0xc2; 16];
    assert_eq!(
        verify_fresh_witness_quorum(&trusted, TEST_INSTALLATION, fresh_challenge, &recorded),
        Err(WitnessQuorumError::WrongChallenge)
    );
    assert_eq!(
        verify_fresh_witness_quorum(&trusted, [0xd1; 16], TEST_CHALLENGE, &recorded),
        Err(WitnessQuorumError::WrongInstallation)
    );
}

#[test]
fn stale_sqlite_snapshot_refused_when_quorum_remembers_newer_checkpoint() {
    let f = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("signed-policy.sqlite");
    let backup = tmp.path().join("old.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let old_checkpoint = store.checkpoint();
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(store);
    std::fs::copy(&db, &backup).unwrap();
    let mut store = DurablePolicyLab::open(
        &db, f.anchors.clone(), WitnessPin::Exact(old_checkpoint),
    ).unwrap();
    store.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2)).unwrap();
    let newest = store.checkpoint();
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(store);
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses { witness.advance(2, newest).unwrap(); }
    std::fs::copy(&backup, &db).unwrap();
    let replies = [
        witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE),
        witnesses[1].attest(TEST_INSTALLATION, TEST_CHALLENGE),
    ];
    assert!(matches!(
        open_from_current_witness_quorum(
            &db, f.anchors.clone(), &trusted, TEST_INSTALLATION, TEST_CHALLENGE, &replies,
        ),
        Err(WitnessQuorumOpenError::Storage(WitnessError::RollbackOrTamper))
    ));
}

#[test]
fn stale_or_split_witnesses_cannot_silently_select_an_old_head() {
    let (_f, _tmp, _db, checkpoint) = durable_revoked_fixture();
    let (mut witnesses, trusted) = test_witnesses();
    witnesses[0].advance(1, checkpoint).unwrap();
    witnesses[1].advance(1, checkpoint).unwrap();
    // One witness is on a different signed epoch/head, so no silent fallback.
    witnesses[2].advance(2, [0x42; 32]).unwrap();
    let replies = [
        witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE),
        witnesses[2].attest(TEST_INSTALLATION, TEST_CHALLENGE),
    ];
    assert_eq!(
        verify_fresh_witness_quorum(&trusted, TEST_INSTALLATION, TEST_CHALLENGE, &replies),
        Err(WitnessQuorumError::SplitHeads)
    );
}

#[test]
fn duplicate_or_forged_witness_cannot_fake_a_quorum() {
    let (_f, _tmp, _db, checkpoint) = durable_revoked_fixture();
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses { witness.advance(1, checkpoint).unwrap(); }
    let one = witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE);
    assert_eq!(
        verify_fresh_witness_quorum(&trusted, TEST_INSTALLATION, TEST_CHALLENGE, &[one, one]),
        Err(WitnessQuorumError::DuplicateWitness)
    );
    let mut forged = witnesses[1].attest(TEST_INSTALLATION, TEST_CHALLENGE);
    forged.signature[0] ^= 1;
    assert_eq!(
        verify_fresh_witness_quorum(&trusted, TEST_INSTALLATION, TEST_CHALLENGE, &[one, forged]),
        Err(WitnessQuorumError::BadSignature)
    );
}

#[test]
fn witness_generations_never_roll_back_or_equivocate_locally() {
    let (mut witnesses, _trusted) = test_witnesses();
    let w = &mut witnesses[0];
    assert!(w.advance(1, [11; 32]).is_ok());
    assert!(w.advance(1, [11; 32]).is_ok());
    assert!(w.advance(0, [0; 32]).is_err());
    assert!(w.advance(1, [12; 32]).is_err());
    assert!(w.advance(2, [13; 32]).is_ok());
}


#[test]
fn policy_commit_without_witness_quorum_confirmation_fails_closed_after_reboot() {
    let f = Fixture::new();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("signed-policy.sqlite");
    let mut store = DurablePolicyLab::open(&db, f.anchors.clone(), WitnessPin::Genesis).unwrap();
    for role in [Role::Owner, Role::Admin] {
        store.ingest(f.event(f.a, role, Target::Object(f.file), Action::Allow, 1)).unwrap();
    }
    let old_head = store.checkpoint();
    let (mut witnesses, trusted) = test_witnesses();
    for witness in &mut witnesses { witness.advance(1, old_head).unwrap(); }
    // Model failure: the fresh deny reached SQLite, but 2-of-3 witnesses
    // were not updated. Continuing with the OLD checkpoint would be unsafe.
    store.ingest(f.event(f.a, Role::Admin, Target::Object(f.file), Action::Deny, 2)).unwrap();
    drop(store);
    let still_old = [
        witnesses[0].attest(TEST_INSTALLATION, TEST_CHALLENGE),
        witnesses[1].attest(TEST_INSTALLATION, TEST_CHALLENGE),
    ];
    assert!(matches!(
        open_from_current_witness_quorum(
            &db, f.anchors.clone(), &trusted, TEST_INSTALLATION, TEST_CHALLENGE, &still_old,
        ),
        Err(WitnessQuorumOpenError::Storage(WitnessError::RollbackOrTamper))
    ));
}
