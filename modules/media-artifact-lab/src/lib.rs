//! Experimental, metadata-only media projection store for Akula LAB-17.
//!
//! This is NOT a scheduler, a File Gate, a permissions provider, an ASR engine,
//! a decoder, or an index. The caller must possess and revalidate trusted source
//! admission at every byte-reading and result-delivery boundary. This separate
//! lab holds only small metadata and caller-supplied deterministic stage outputs.
//!
//! A stage key identifies a computation; an output digest identifies bytes.
//! Different computation versions that yield equal bytes can reuse downstream
//! stages. Source timing bindings are NOT part of canonical audio identity.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum LabError {
    Invalid(&'static str),
    StaleSource,
    Revoked,
    RevisionCollision,
    NonDeterministicOutput,
    Sql(rusqlite::Error),
    Json(serde_json::Error),
}
impl fmt::Display for LabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(msg) => f.write_str(msg),
            Self::StaleSource => f.write_str("source generation/revision is stale"),
            Self::Revoked => f.write_str("source was revoked"),
            Self::RevisionCollision => f.write_str("generation reused with conflicting bytes"),
            Self::NonDeterministicOutput => f.write_str("same stage key produced different bytes"),
            Self::Sql(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for LabError {}
impl From<rusqlite::Error> for LabError { fn from(e: rusqlite::Error) -> Self { Self::Sql(e) } }
impl From<serde_json::Error> for LabError { fn from(e: serde_json::Error) -> Self { Self::Json(e) } }
pub type Result<T> = std::result::Result<T, LabError>;

/// Distinguish existing SHA-256 LAB-16 fixtures from native BLAKE3 digests.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Digest(String);
impl Digest {
    pub fn parse(input: &str) -> Result<Self> {
        let (algorithm, hex) = input.split_once(':').ok_or(LabError::Invalid("digest needs algorithm"))?;
        if !matches!(algorithm, "blake3" | "sha256") || hex.len() != 64
            || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        { return Err(LabError::Invalid("invalid digest format")); }
        Ok(Self(input.to_owned()))
    }
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("blake3:{}", blake3::hash(bytes).to_hex()))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

fn stable_hash<T: Serialize>(data: &T) -> Result<Digest> {
    Ok(Digest::of_bytes(&serde_json::to_vec(data)?))
}
fn valid_label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub scope_id: String,
    pub source_id: String,
    pub revision_id: String,
    pub source_digest: Digest,
    pub generation: u64,
}
impl SourceRef {
    fn validate(&self) -> Result<()> {
        if !valid_label(&self.scope_id) || !valid_label(&self.source_id)
            || !valid_label(&self.revision_id) || self.generation == 0
        { return Err(LabError::Invalid("bad source scope/id/revision/generation")); }
        Ok(())
    }
}

/// A source's own clock. Do not infer this from average frame rate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimeBase { pub num: i64, pub den: i64 }
impl TimeBase {
    pub fn new(num: i64, den: i64) -> Result<Self> {
        if num <= 0 || den <= 0 { return Err(LabError::Invalid("invalid time base")); }
        Ok(Self { num, den })
    }
}

/// Same PCM content may have distinct bindings in remuxed versions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AudioBinding {
    pub source: SourceRef,
    pub stream_index: u32,
    pub start_pts: i64,
    pub time_base: TimeBase,
    pub pcm_digest: Digest,
}
impl AudioBinding {
    pub fn binding_digest(&self) -> Result<Digest> {
        self.source.validate()?;
        TimeBase::new(self.time_base.num, self.time_base.den)?;
        stable_hash(self)
    }
    /// Convert a local PCM sample offset to container nanoseconds; keep signed
    /// ticks and use i128 to avoid long-video overflow and `frame/fps` drift.
    /// Integer nanoseconds may round downward by at most 1ns per term.
    pub fn presentation_ns(&self, sample_index: i64, sample_rate: u32) -> Result<i128> {
        if sample_index < 0 || sample_rate == 0 { return Err(LabError::Invalid("bad PCM offset/rate")); }
        TimeBase::new(self.time_base.num, self.time_base.den)?;
        let start = (self.start_pts as i128)
            .checked_mul(self.time_base.num as i128)
            .and_then(|x| x.checked_mul(1_000_000_000))
            .ok_or(LabError::Invalid("time overflow"))?
            / (self.time_base.den as i128);
        let offset = (sample_index as i128) * 1_000_000_000 / (sample_rate as i128);
        start.checked_add(offset).ok_or(LabError::Invalid("time overflow"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind { AsrOriginal, Translation, FrameSet, Ocr, VisualFeatures, SoundTags }

/// Inputs are keyed by explicit roles and use content (not job) digests.
/// BTreeMap makes the serialized key independent of caller insertion order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StageRequest {
    pub kind: StageKind,
    pub code_version: String,
    pub model_id: String,
    pub params_digest: Digest,
    pub output_schema: u32,
    pub inputs: BTreeMap<String, Digest>,
}
impl StageRequest {
    fn validate(&self) -> Result<()> {
        if !valid_label(&self.code_version) || !valid_label(&self.model_id)
            || self.output_schema == 0 || self.inputs.is_empty()
            || self.inputs.len() > 64 || self.inputs.keys().any(|k| !valid_label(k))
        { return Err(LabError::Invalid("invalid stage specification")); }
        Ok(())
    }
    pub fn stage_key(&self, scope: &str) -> Result<Digest> {
        self.validate()?;
        if !valid_label(scope) { return Err(LabError::Invalid("invalid scope")); }
        stable_hash(&(scope, self))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageOutput {
    pub stage_key: Digest,
    pub content_digest: Digest,
    pub payload: String,
    pub cached: bool,
}

pub struct ArtifactStore { conn: Connection }
impl ArtifactStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> { Self::prepare(Connection::open(path)?) }
    pub fn memory() -> Result<Self> { Self::prepare(Connection::open_in_memory()?) }
    fn prepare(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS media_lab_sources(
              scope_id TEXT NOT NULL, source_id TEXT NOT NULL, revision_id TEXT NOT NULL,
              source_digest TEXT NOT NULL, generation INTEGER NOT NULL CHECK(generation>0),
              revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
              PRIMARY KEY(scope_id, source_id));
            CREATE TABLE IF NOT EXISTS media_lab_bindings(
              scope_id TEXT NOT NULL, source_id TEXT NOT NULL, revision_id TEXT NOT NULL,
              binding_digest TEXT NOT NULL, stream_index INTEGER NOT NULL,
              start_pts INTEGER NOT NULL, time_num INTEGER NOT NULL, time_den INTEGER NOT NULL,
              pcm_digest TEXT NOT NULL,
              PRIMARY KEY(scope_id,source_id,revision_id,stream_index),
              FOREIGN KEY(scope_id,source_id) REFERENCES media_lab_sources(scope_id,source_id));
            CREATE TABLE IF NOT EXISTS media_lab_outputs(
              scope_id TEXT NOT NULL, stage_key TEXT NOT NULL, kind TEXT NOT NULL,
              content_digest TEXT NOT NULL, payload TEXT NOT NULL,
              PRIMARY KEY(scope_id,stage_key));
            CREATE TABLE IF NOT EXISTS media_lab_edges(
              scope_id TEXT NOT NULL, child_stage_key TEXT NOT NULL,
              input_role TEXT NOT NULL, parent_digest TEXT NOT NULL,
              PRIMARY KEY(scope_id, child_stage_key, input_role),
              FOREIGN KEY(scope_id,child_stage_key)
                 REFERENCES media_lab_outputs(scope_id,stage_key));")?;
        Ok(Self { conn })
    }

    /// Caller MUST obtain this snapshot from Akula trusted File Gate; the lab
    /// database is not an independent authority for identity or permissions.
    pub fn register_source(&mut self, source: &SourceRef) -> Result<()> {
        source.validate()?;
        let generation = i64::try_from(source.generation)
            .map_err(|_| LabError::Invalid("generation overflow"))?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String,String,i64,i64)> = tx.query_row(
            "SELECT revision_id,source_digest,generation,revoked FROM media_lab_sources WHERE scope_id=?1 AND source_id=?2",
            params![source.scope_id,source.source_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).optional()?;
        if let Some((revision,digest,previous,revoked)) = old {
            if revoked != 0 { return Err(LabError::Revoked); }
            if previous > generation { return Err(LabError::StaleSource); }
            if previous == generation {
                if revision != source.revision_id || digest != source.source_digest.as_str() {
                    return Err(LabError::RevisionCollision);
                }
            } else {
                tx.execute("UPDATE media_lab_sources SET revision_id=?3,source_digest=?4,generation=?5 WHERE scope_id=?1 AND source_id=?2",
                    params![source.scope_id, source.source_id, source.revision_id, source.source_digest.as_str(), generation])?;
                // The old track timeline belongs to the prior source revision.
                tx.execute("DELETE FROM media_lab_bindings WHERE scope_id=?1 AND source_id=?2",
                    params![source.scope_id,source.source_id])?;
            }
        } else {
            tx.execute("INSERT INTO media_lab_sources(scope_id,source_id,revision_id,source_digest,generation) VALUES (?1,?2,?3,?4,?5)",
                params![source.scope_id, source.source_id, source.revision_id, source.source_digest.as_str(), generation])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn revoke_source(&mut self, source: &SourceRef) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_source(&tx, source)?;
        tx.execute("UPDATE media_lab_sources SET revoked=1 WHERE scope_id=?1 AND source_id=?2",
            params![source.scope_id,source.source_id])?;
        tx.commit()?;
        Ok(())
    }
    fn check_source(conn: &Connection, source: &SourceRef) -> Result<()> {
        source.validate()?;
        let row: Option<(String,String,i64,i64)> = conn.query_row(
            "SELECT revision_id,source_digest,generation,revoked FROM media_lab_sources WHERE scope_id=?1 AND source_id=?2",
            params![source.scope_id,source.source_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).optional()?;
        let Some((rev,dig,generation,revoked)) = row else { return Err(LabError::StaleSource); };
        if revoked != 0 { return Err(LabError::Revoked); }
        if rev != source.revision_id || dig != source.source_digest.as_str()
            || u64::try_from(generation).ok() != Some(source.generation)
        { return Err(LabError::StaleSource); }
        Ok(())
    }
    pub fn record_audio_binding(&mut self, binding: &AudioBinding) -> Result<Digest> {
        let binding_digest = binding.binding_digest()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_source(&tx, &binding.source)?;
        let prior: Option<String> = tx.query_row(
            "SELECT binding_digest FROM media_lab_bindings WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND stream_index=?4",
            params![binding.source.scope_id,binding.source.source_id,binding.source.revision_id,binding.stream_index],
            |r| r.get(0)).optional()?;
        if let Some(existing) = prior {
            if existing != binding_digest.as_str() { return Err(LabError::RevisionCollision); }
        } else {
            tx.execute("INSERT INTO media_lab_bindings VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![binding.source.scope_id,binding.source.source_id,binding.source.revision_id,
                        binding_digest.as_str(),binding.stream_index,binding.start_pts,
                        binding.time_base.num,binding.time_base.den,binding.pcm_digest.as_str()])?;
        }
        tx.commit()?;
        Ok(binding_digest)
    }
    /// A caller-supplied deterministic byte payload. No ASR, translation or
    /// actual expensive work is performed by this metadata-only crate.
    pub fn publish(&mut self, source: &SourceRef, request: &StageRequest, payload: &str) -> Result<StageOutput> {
        let stage_key = request.stage_key(&source.scope_id)?;
        let content_digest = Digest::of_bytes(payload.as_bytes());
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_source(&tx, source)?;
        let old: Option<(String,String)> = tx.query_row(
            "SELECT content_digest,payload FROM media_lab_outputs WHERE scope_id=?1 AND stage_key=?2",
            params![source.scope_id,stage_key.as_str()], |r| Ok((r.get(0)?,r.get(1)?))
        ).optional()?;
        let cached = if let Some((dig,text)) = old {
            if dig != content_digest.as_str() || text != payload {
                return Err(LabError::NonDeterministicOutput);
            }
            true
        } else {
            tx.execute("INSERT INTO media_lab_outputs(scope_id,stage_key,kind,content_digest,payload) VALUES (?1,?2,?3,?4,?5)",
                params![source.scope_id,stage_key.as_str(),format!("{:?}",request.kind),content_digest.as_str(),payload])?;
            for (role,digest) in &request.inputs {
                tx.execute("INSERT INTO media_lab_edges(scope_id,child_stage_key,input_role,parent_digest) VALUES (?1,?2,?3,?4)",
                    params![source.scope_id,stage_key.as_str(),role,digest.as_str()])?;
            }
            false
        };
        tx.commit()?;
        Ok(StageOutput { stage_key,content_digest,payload:payload.to_owned(),cached })
    }
    /// A cache read always requires a fresh source snapshot. The Akula host
    /// must additionally enforce LIVE rights before returning payload to user.
    pub fn lookup(&self, source: &SourceRef, request: &StageRequest) -> Result<Option<StageOutput>> {
        let key = request.stage_key(&source.scope_id)?;
        Self::check_source(&self.conn, source)?;
        let old: Option<(String,String)> = self.conn.query_row(
            "SELECT content_digest,payload FROM media_lab_outputs WHERE scope_id=?1 AND stage_key=?2",
            params![source.scope_id,key.as_str()], |r| Ok((r.get(0)?,r.get(1)?))
        ).optional()?;
        old.map(|(digest,payload)| Ok(StageOutput {
            stage_key:key,content_digest:Digest::parse(&digest)?,payload,cached:true
        })).transpose()
    }
    pub fn stage_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM media_lab_outputs",[],|r|r.get(0))?)
    }
    pub fn binding_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM media_lab_bindings",[],|r|r.get(0))?)
    }
}
