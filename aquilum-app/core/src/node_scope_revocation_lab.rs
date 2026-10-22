//! AK-DATA-CTRL-01D: signed, scope-bound anti-memes on a real Node read gate.
//!
//! Opt-in Linux laboratory ONLY. The sole mutable data is a SQLite record in
//! a separately operator-selected appdata directory. No files are erased,
//! rewritten, quarantined on disk or accepted for upload. A signed revocation
//! suppresses future reads offered by a cooperating node in ONE VaultId.
//!
//! NOT production anti-rollback, owner signature, device ACL or Roha transport.
use aquilum_core::sync::VaultId;
use ring::signature::{self, UnparsedPublicKey};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::path::Path;

const DOMAIN: &[u8] = b"AKULA:NODE:READ-SCOPE-REVOKE:LAB-V0\0";
const WIRE_LEN: usize = 32 + 8 + 64;

pub fn decode_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let raw = s.as_bytes();
    if raw.len() != N.checked_mul(2)? { return None; }
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (nibble(raw[i * 2])? << 4) | nibble(raw[i * 2 + 1])?;
    }
    Some(out)
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(ALPHABET[(byte >> 4) as usize] as char);
        result.push(ALPHABET[(byte & 15) as usize] as char);
    }
    result
}

/// Only the pinned scope administrator may issue this type of suppression.
/// Owner grants, unbans, file deletion and broader content bans are excluded.
pub fn bytes_to_sign(vault: VaultId, document_id: &str, sequence: u64) -> Option<Vec<u8>> {
    if sequence == 0 || sequence > i64::MAX as u64 { return None; }
    let document = decode_hex::<32>(document_id)?;
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(vault.as_uuid().as_bytes());
    bytes.extend_from_slice(&document);
    bytes.extend_from_slice(&sequence.to_be_bytes());
    Some(bytes)
}

#[derive(Clone)]
struct Proof {
    seq: u64,
    sig: [u8; 64],
}

impl Proof {
    fn wire(&self, doc: &[u8; 32]) -> String {
        let mut bytes = Vec::with_capacity(WIRE_LEN);
        bytes.extend_from_slice(doc);
        bytes.extend_from_slice(&self.seq.to_be_bytes());
        bytes.extend_from_slice(&self.sig);
        encode_hex(&bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ingest {
    Applied,
    Duplicate,
    Stale { correction: String },
}

pub struct ScopedRevocations {
    scope: VaultId,
    admin: [u8; 32],
    conn: Connection,
    current: BTreeMap<[u8; 32], Proof>,
    poisoned: bool,
}

impl ScopedRevocations {
    pub fn open(path: &Path, scope: VaultId, admin: [u8; 32]) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|_| "cannot open local policy database")?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS lab_scope_anchor (
                 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                 scope BLOB NOT NULL,
                 admin BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS lab_revocations (
                 doc BLOB PRIMARY KEY NOT NULL CHECK(length(doc)=32),
                 seq INTEGER NOT NULL CHECK(seq > 0),
                 sig BLOB NOT NULL CHECK(length(sig)=64)
             ) WITHOUT ROWID;"
        ).map_err(|_| "cannot prepare scoped policy tables")?;

        let scope_bytes = scope.as_uuid().as_bytes().to_vec();
        conn.execute(
            "INSERT OR IGNORE INTO lab_scope_anchor(singleton, scope, admin)
             VALUES (1, ?1, ?2)",
            params![&scope_bytes, &admin[..]],
        ).map_err(|_| "cannot pin local scope authority")?;
        let (found_scope, found_admin): (Vec<u8>, Vec<u8>) = conn.query_row(
            "SELECT scope, admin FROM lab_scope_anchor WHERE singleton=1",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| "missing local policy anchor")?;
        if found_scope != scope_bytes || found_admin != admin {
            return Err("refusing changed scope or administrator anchor".into());
        }

        let mut current = BTreeMap::new();
        {
            let mut query = conn.prepare(
                "SELECT doc, seq, sig FROM lab_revocations ORDER BY doc"
            ).map_err(|_| "cannot read stored policy")?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?))
            }).map_err(|_| "cannot iterate stored policy")?;
            for row in rows {
                let (doc, seq, sig) = row.map_err(|_| "corrupt policy row")?;
                let doc: [u8; 32] = doc.try_into().map_err(|_| "bad document length")?;
                let sig: [u8; 64] = sig.try_into().map_err(|_| "bad signature length")?;
                if seq <= 0 { return Err("invalid stored sequence".into()); }
                let signed = bytes_to_sign(scope, &encode_hex(&doc), seq as u64)
                    .ok_or("invalid stored document or sequence")?;
                UnparsedPublicKey::new(&signature::ED25519, &admin)
                    .verify(&signed, &sig)
                    .map_err(|_| "stored policy signature invalid")?;
                if current.insert(doc, Proof { seq: seq as u64, sig }).is_some() {
                    return Err("duplicate persisted policy".into());
                }
            }
        }
        Ok(Self { scope, admin, conn, current, poisoned: false })
    }

    /// Accept one signed, exact-document suppression. It NEVER mutates files.
    pub fn ingest(&mut self, wire: &str) -> Result<Ingest, String> {
        if self.poisoned { return Err("policy needs recovery".into()); }
        let bytes = decode_hex::<WIRE_LEN>(wire).ok_or("invalid anti-meme framing")?;
        let doc: [u8; 32] = bytes[..32].try_into().map_err(|_| "bad document")?;
        let seq = u64::from_be_bytes(bytes[32..40].try_into()
            .map_err(|_| "bad sequence")?);
        let sig: [u8; 64] = bytes[40..].try_into().map_err(|_| "bad signature")?;
        let signed = bytes_to_sign(self.scope, &encode_hex(&doc), seq)
            .ok_or("bad signed anti-meme fields")?;
        UnparsedPublicKey::new(&signature::ED25519, &self.admin)
            .verify(&signed, &sig)
            .map_err(|_| "signature does not match scope authority")?;

        if let Some(existing) = self.current.get(&doc) {
            if seq < existing.seq {
                return Ok(Ingest::Stale { correction: existing.wire(&doc) });
            }
            if seq == existing.seq {
                return if sig == existing.sig {
                    Ok(Ingest::Duplicate)
                } else {
                    Err("same signed sequence has conflicting signature".into())
                };
            }
        }
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.transaction()?;
            tx.execute(
                "INSERT INTO lab_revocations(doc, seq, sig)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(doc) DO UPDATE SET
                   seq=excluded.seq, sig=excluded.sig
                 WHERE excluded.seq > lab_revocations.seq",
                params![&doc[..], seq as i64, &sig[..]],
            )?;
            tx.commit()
        })();
        if result.is_err() {
            self.poisoned = true;
            return Err("failed to persist signed scope revocation".into());
        }
        self.current.insert(doc, Proof { seq, sig });
        Ok(Ingest::Applied)
    }

    pub fn is_poisoned(&self) -> bool { self.poisoned }

    pub fn is_revoked(&self, document_id: &str) -> bool {
        // Malformed IDs are not implicitly authorized elsewhere: ReadCatalog
        // validates them independently. An uncertain SQL state fails closed.
        if self.poisoned { return true; }
        decode_hex::<32>(document_id).is_some_and(|doc| self.current.contains_key(&doc))
    }

    pub fn signed_proof(&self, document_id: &str) -> Option<String> {
        if self.poisoned { return None; }
        let doc = decode_hex::<32>(document_id)?;
        self.current.get(&doc).map(|p| p.wire(&doc))
    }
}
