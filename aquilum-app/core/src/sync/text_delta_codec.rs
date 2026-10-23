//! DELTA-TEXT-03: isolated, bounded, versioned *representation*, NOT sync.
//!
//! This byte codec cannot identify a publisher, establish a base as trusted,
//! grant File Gate access, publish an S3 root, remove conflicts or acknowledge
//! a baseline. The caller MUST independently pin the exact base RevisionId
//! and BLAKE3 to verified V1 history before supplying old bytes.
//!
//! The legacy AKPKEXP1 V2 pack member always contains RAW blobs. This new
//! format has a different magic and is deliberately not wired into it.
//! New codecs require negotiated capabilities and a separate security review.

use super::RevisionId;
use uuid::Uuid;

pub(crate) const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = MAX_TEXT_BYTES + 64 * 1024;
const MAGIC: &[u8; 8] = b"AKTDLT01";
const SCHEMA: u8 = 1;
const LEVEL: i32 = 3;
const HEADER_BYTES: usize = 8 + 1 + 1 + 2 + 16 + 16 + 32 + 32 + 4 + 4 + 4;
const DELTA_MIN_SAVING_BYTES: usize = 128;
const DELTA_MIN_SAVING_PERCENT: usize = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum TextDeltaMode {
    FullZstd = 0,
    SpliceZstd = 1,
    DictionaryZstd = 2,
}

impl TextDeltaMode {
    fn decode(value: u8) -> Result<Self, TextDeltaError> {
        match value {
            0 => Ok(Self::FullZstd),
            1 => Ok(Self::SpliceZstd),
            2 => Ok(Self::DictionaryZstd),
            _ => Err(TextDeltaError::Unsupported),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextDeltaError {
    Unsupported,
    InvalidEnvelope,
    InvalidUtf8,
    OverBudget,
    MissingBase,
    WrongBase,
    WrongRevision,
    Integrity,
    CorruptCompression,
}

#[derive(Debug)]
pub(crate) struct EncodedTextDelta {
    pub(crate) mode: TextDeltaMode,
    pub(crate) bytes: Vec<u8>,
}

fn digest(bytes: &[u8]) -> [u8; 32] { *blake3::hash(bytes).as_bytes() }

fn check_text(bytes: &[u8]) -> Result<(), TextDeltaError> {
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(TextDeltaError::OverBudget);
    }
    std::str::from_utf8(bytes).map_err(|_| TextDeltaError::InvalidUtf8)?;
    Ok(())
}

fn splice(base: &[u8], target: &[u8]) -> Result<Vec<u8>, TextDeltaError> {
    let mut prefix = 0;
    while prefix < base.len().min(target.len()) && base[prefix] == target[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < base.len().min(target.len()) - prefix
        && base[base.len() - 1 - suffix] == target[target.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let inserted = target.get(prefix..target.len() - suffix)
        .ok_or(TextDeltaError::InvalidEnvelope)?;
    let removed = base.len() - prefix - suffix;
    let mut bytes = Vec::with_capacity(12 + inserted.len());
    bytes.extend_from_slice(&(prefix as u32).to_le_bytes());
    bytes.extend_from_slice(&(removed as u32).to_le_bytes());
    bytes.extend_from_slice(&(inserted.len() as u32).to_le_bytes());
    bytes.extend_from_slice(inserted);
    Ok(bytes)
}

fn splice_apply(base: &[u8], bytes: &[u8]) -> Result<Vec<u8>, TextDeltaError> {
    if bytes.len() < 12 {
        return Err(TextDeltaError::InvalidEnvelope);
    }
    let take = |index: usize| -> Result<usize, TextDeltaError> {
        let field = bytes.get(index..index + 4)
            .ok_or(TextDeltaError::InvalidEnvelope)?;
        let word: [u8; 4] = field.try_into().map_err(|_| TextDeltaError::InvalidEnvelope)?;
        Ok(u32::from_le_bytes(word) as usize)
    };
    let prefix = take(0)?;
    let removed = take(4)?;
    let inserted = take(8)?;
    let end = prefix.checked_add(removed).ok_or(TextDeltaError::InvalidEnvelope)?;
    if end > base.len() || bytes.len() != 12 + inserted {
        return Err(TextDeltaError::InvalidEnvelope);
    }
    let result_len = base.len() - removed + inserted;
    if result_len > MAX_TEXT_BYTES {
        return Err(TextDeltaError::OverBudget);
    }
    let mut result = Vec::with_capacity(result_len);
    result.extend_from_slice(&base[..prefix]);
    result.extend_from_slice(&bytes[12..]);
    result.extend_from_slice(&base[end..]);
    Ok(result)
}

fn to_header(
    mode: TextDeltaMode,
    base: Option<(RevisionId, &[u8])>,
    target_id: RevisionId,
    target: &[u8],
    compressed: Vec<u8>,
) -> Result<Vec<u8>, TextDeltaError> {
    if compressed.is_empty() || compressed.len() > MAX_FRAME_BYTES {
        return Err(TextDeltaError::OverBudget);
    }
    let mut wire = Vec::with_capacity(HEADER_BYTES + compressed.len());
    wire.extend_from_slice(MAGIC);
    wire.push(SCHEMA);
    wire.push(mode as u8);
    wire.extend_from_slice(&0u16.to_le_bytes()); // reserved, MUST be zero
    match (mode, base) {
        (TextDeltaMode::FullZstd, _) => {
            wire.extend_from_slice(&[0; 16]);
        }
        (_, Some((id, _))) => {
            wire.extend_from_slice(id.as_uuid().as_bytes());
        }
        (_, None) => return Err(TextDeltaError::MissingBase),
    }
    wire.extend_from_slice(target_id.as_uuid().as_bytes());
    match (mode, base) {
        (TextDeltaMode::FullZstd, _) => wire.extend_from_slice(&[0; 32]),
        (_, Some((_, bytes))) => wire.extend_from_slice(&digest(bytes)),
        (_, None) => return Err(TextDeltaError::MissingBase),
    }
    wire.extend_from_slice(&digest(target));
    let old_len = if mode == TextDeltaMode::FullZstd { 0 }
                  else { base.ok_or(TextDeltaError::MissingBase)?.1.len() };
    wire.extend_from_slice(&(old_len as u32).to_le_bytes());
    wire.extend_from_slice(&(target.len() as u32).to_le_bytes());
    wire.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    debug_assert_eq!(wire.len(), HEADER_BYTES);
    wire.extend_from_slice(&compressed);
    Ok(wire)
}

/// The optional old bytes must come from the caller's independently trusted
/// V1 checkpoint. This function merely verifies their own BLAKE3 for encoding.
/// A malicious caller cannot obtain publish/apply rights by calling it.
///
/// Full Zstd is always an available fallback. A delta is selected only if
/// the *whole versioned frame* saves >=128 bytes AND >=15%.
pub(crate) fn encode(
    target_id: RevisionId,
    target: &[u8],
    old: Option<(RevisionId, &[u8])>,
) -> Result<EncodedTextDelta, TextDeltaError> {
    check_text(target)?;
    if target_id.as_uuid().is_nil() {
        return Err(TextDeltaError::WrongRevision);
    }
    if let Some((old_id, old_bytes)) = old {
        check_text(old_bytes)?;
        if old_id.as_uuid().is_nil() || old_id == target_id {
            return Err(TextDeltaError::WrongRevision);
        }
    }
    let full = zstd::bulk::compress(target, LEVEL)
        .map_err(|_| TextDeltaError::CorruptCompression)?;
    let fallback = to_header(TextDeltaMode::FullZstd, None, target_id, target, full)?;
    let mut chosen = fallback.clone();
    let mut mode = TextDeltaMode::FullZstd;
    if let Some((old_id, old_bytes)) = old {
        let patch = splice(old_bytes, target)?;
        if let Ok(compressed) = zstd::bulk::compress(&patch, LEVEL) {
            if let Ok(candidate) = to_header(
                TextDeltaMode::SpliceZstd, Some((old_id, old_bytes)),
                target_id, target, compressed,
            ) {
                if candidate.len() < chosen.len() {
                    chosen = candidate;
                    mode = TextDeltaMode::SpliceZstd;
                }
            }
        }
        if old_bytes.len() >= 64 {
            // Raw dictionary is NOT zstd CLI --patch-from byte-for-byte:
            // this explicit Rust codec must be measured independently.
            if let Ok(mut compressor) =
                zstd::bulk::Compressor::with_dictionary(LEVEL, old_bytes)
            {
                if let Ok(compressed) = compressor.compress(target) {
                    if let Ok(candidate) = to_header(
                        TextDeltaMode::DictionaryZstd,
                        Some((old_id, old_bytes)), target_id, target, compressed,
                    ) {
                        if candidate.len() < chosen.len() {
                            chosen = candidate;
                            mode = TextDeltaMode::DictionaryZstd;
                        }
                    }
                }
            }
        }
    }
    // A small size win is not sufficient to justify a dependent transfer.
    if mode != TextDeltaMode::FullZstd {
        let saved = fallback.len().saturating_sub(chosen.len());
        if saved < DELTA_MIN_SAVING_BYTES ||
            saved * 100 < fallback.len() * DELTA_MIN_SAVING_PERCENT
        {
            chosen = fallback;
            mode = TextDeltaMode::FullZstd;
        }
    }
    Ok(EncodedTextDelta { mode, bytes: chosen })
}

fn u32_at(wire: &[u8], offset: usize) -> Result<usize, TextDeltaError> {
    let bytes: [u8; 4] = wire.get(offset..offset + 4)
        .ok_or(TextDeltaError::InvalidEnvelope)?
        .try_into().map_err(|_| TextDeltaError::InvalidEnvelope)?;
    Ok(u32::from_le_bytes(bytes) as usize)
}

/// Decode returns untrusted *bytes only*. Passing old bytes is not proof they
/// belong to an authenticated history; callers must establish that separately.
/// This API has NO local mutation, S3 I/O, acknowledged baseline or writer role.
pub(crate) fn decode(
    wire: &[u8],
    expected_revision: RevisionId,
    old: Option<(RevisionId, &[u8])>,
) -> Result<Vec<u8>, TextDeltaError> {
    if wire.len() < HEADER_BYTES + 1 || wire.len() > HEADER_BYTES + MAX_FRAME_BYTES
        || wire.get(..8) != Some(MAGIC.as_slice())
    {
        return Err(TextDeltaError::InvalidEnvelope);
    }
    if wire[8] != SCHEMA {
        return Err(TextDeltaError::Unsupported);
    }
    let mode = TextDeltaMode::decode(wire[9])?;
    if wire[10] != 0 || wire[11] != 0 {
        return Err(TextDeltaError::InvalidEnvelope);
    }
    let base_id = &wire[12..28];
    let target_id = RevisionId::from_uuid(
        Uuid::from_bytes(wire[28..44].try_into()
            .map_err(|_| TextDeltaError::InvalidEnvelope)?)
    );
    if target_id.as_uuid().is_nil() || target_id != expected_revision {
        return Err(TextDeltaError::WrongRevision);
    }
    let base_digest = &wire[44..76];
    let target_digest = &wire[76..108];
    let old_len = u32_at(wire,108)?;
    let new_len = u32_at(wire,112)?;
    let compressed_len = u32_at(wire,116)?;
    if old_len > MAX_TEXT_BYTES || new_len > MAX_TEXT_BYTES
        || compressed_len == 0 || compressed_len > MAX_FRAME_BYTES
        || wire.len() != HEADER_BYTES + compressed_len
    {
        return Err(TextDeltaError::OverBudget);
    }
    let base = match mode {
        TextDeltaMode::FullZstd => {
            if base_id != [0;16] || base_digest != [0;32] || old_len != 0 {
                return Err(TextDeltaError::InvalidEnvelope);
            }
            None
        }
        _ => {
            let (old_id, bytes) = old.ok_or(TextDeltaError::MissingBase)?;
            if old_id.as_uuid().is_nil()
                || old_id.as_uuid().as_bytes().as_slice() != base_id
                || old_id == target_id
                || bytes.len() != old_len
                || !std::str::from_utf8(bytes).is_ok()
                || digest(bytes).as_slice() != base_digest
            {
                return Err(TextDeltaError::WrongBase);
            }
            Some(bytes)
        }
    };
    let payload = &wire[HEADER_BYTES..];
    let output = match mode {
        TextDeltaMode::FullZstd => zstd::bulk::decompress(payload, new_len)
            .map_err(|_| TextDeltaError::CorruptCompression)?,
        TextDeltaMode::SpliceZstd => {
            let patch = zstd::bulk::decompress(payload, MAX_TEXT_BYTES + 12)
                .map_err(|_| TextDeltaError::CorruptCompression)?;
            splice_apply(base.ok_or(TextDeltaError::MissingBase)?, &patch)?
        }
        TextDeltaMode::DictionaryZstd => {
            let mut decoder = zstd::bulk::Decompressor::with_dictionary(
                base.ok_or(TextDeltaError::MissingBase)?,
            ).map_err(|_| TextDeltaError::CorruptCompression)?;
            decoder.decompress(payload, new_len)
                .map_err(|_| TextDeltaError::CorruptCompression)?
        }
    };
    if output.len() != new_len || digest(&output).as_slice() != target_digest {
        return Err(TextDeltaError::Integrity);
    }
    check_text(&output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_like_base() -> String {
        // Deterministic incompressible *text*: the delta threshold must
        // be tested against a nontrivial full-Zstd competitor, not a tiny
        // Zstd frame for a repeated single phrase.
        let mut text = String::from("shared words\n");
        for i in 0u64..300 {
            text.push_str(&blake3::hash(&i.to_le_bytes()).to_hex().to_string());
            text.push('\n');
        }
        text
    }

    #[test]
    fn byte_exact_utf8_edits_and_full_fallback() {
        let parent_id = RevisionId::new();
        let left_id = RevisionId::new();
        let base = "Глава 1 — проверяем обычные изменения 🙂\n".repeat(80);
        for target in [
            base.replace("обычные", "очень интересные"),
            format!("{}После главы появился новый абзац.\n", base),
            String::new(),
            "Полная новая версия другого размера. Здесь новая история.".repeat(60),
        ] {
            let wire = encode(left_id, target.as_bytes(),
                              Some((parent_id, base.as_bytes()))).unwrap();
            assert_eq!(
                decode(&wire.bytes, left_id, Some((parent_id, base.as_bytes()))).unwrap(),
                target.as_bytes()
            );
            let full = encode(left_id, target.as_bytes(), None).unwrap();
            assert_eq!(full.mode, TextDeltaMode::FullZstd);
            assert_eq!(decode(&full.bytes, left_id, None).unwrap(), target.as_bytes());
            assert!(wire.bytes.len() <= full.bytes.len());
        }
    }

    #[test]
    fn independent_offline_siblings_share_only_an_exact_parent() {
        let parent_id = RevisionId::new();
        let left_id = RevisionId::new();
        let right_id = RevisionId::new();
        let parent = real_like_base();
        let left = parent.replacen("shared", "left", 1);
        let right = parent.replacen("shared", "right", 1);
        for (revision, target, wrong) in [
            (left_id, &left, &right), (right_id, &right, &left),
        ] {
            let envelope = encode(revision, target.as_bytes(),
                                  Some((parent_id, parent.as_bytes()))).unwrap();
            assert_ne!(envelope.mode, TextDeltaMode::FullZstd);
            assert_eq!(decode(&envelope.bytes, revision,
                              Some((parent_id, parent.as_bytes()))).unwrap(),
                       target.as_bytes());
            assert_eq!(decode(&envelope.bytes, revision,
                              Some((parent_id, wrong.as_bytes()))),
                       Err(TextDeltaError::WrongBase));
            assert_eq!(decode(&envelope.bytes, revision, None),
                       Err(TextDeltaError::MissingBase));
        }
    }

    #[test]
    fn reject_downgrade_spoofed_uuid_unknown_version_and_corrupt_payload() {
        let parent_id = RevisionId::new();
        let target_id = RevisionId::new();
        let base = real_like_base();
        let target = base.replace("words", "slightly different words");
        let envelope = encode(target_id, target.as_bytes(),
                              Some((parent_id, base.as_bytes()))).unwrap();
        assert_ne!(envelope.mode, TextDeltaMode::FullZstd);
        let correct_base = Some((parent_id, base.as_bytes()));
        let mut bad = envelope.bytes.clone();
        bad[0] = b'X';
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::InvalidEnvelope));
        bad = envelope.bytes.clone();
        bad[8] = 2;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Unsupported));
        bad = envelope.bytes.clone();
        bad[9] = 255;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Unsupported));
        bad = envelope.bytes.clone();
        bad[10] = 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::InvalidEnvelope));
        bad = envelope.bytes.clone();
        bad[28] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::WrongRevision));
        bad = envelope.bytes.clone();
        bad[44] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::WrongBase));
        bad = envelope.bytes.clone();
        bad[76] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Integrity));
        bad = envelope.bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad.pop();
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad.push(0);
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad[112..116].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::OverBudget));
        bad = envelope.bytes.clone();
        bad[116..120].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::OverBudget));
        assert_eq!(decode(&envelope.bytes,RevisionId::new(),correct_base),
                   Err(TextDeltaError::WrongRevision));
        assert_eq!(decode(&envelope.bytes,target_id,
                          Some((RevisionId::new(),base.as_bytes()))),
                   Err(TextDeltaError::WrongBase));
        assert!(encode(target_id, &[0xff,0xfe], correct_base).is_err());
        assert_eq!(
            encode(target_id, target.as_bytes(), Some((target_id,base.as_bytes())))
                .err(), Some(TextDeltaError::WrongRevision)
        );
    }

    #[test]
    fn direct_dictionary_and_splice_modes_have_independent_decoders() {
        // Explicitly exercise modes 1 and 2 even when automatic selection
        // would choose a different encoding for a particular text.
        let parent_id = RevisionId::new();
        let target_id = RevisionId::new();
        let parent = real_like_base();
        let target = parent.replacen("shared", "новое слово", 1);
        let base = Some((parent_id, parent.as_bytes()));
        for mode in [TextDeltaMode::SpliceZstd, TextDeltaMode::DictionaryZstd] {
            let payload = match mode {
                TextDeltaMode::SpliceZstd => {
                    let patch = splice(parent.as_bytes(), target.as_bytes()).unwrap();
                    zstd::bulk::compress(&patch, LEVEL).unwrap()
                }
                TextDeltaMode::DictionaryZstd => {
                    let mut encoder = zstd::bulk::Compressor::with_dictionary(
                        LEVEL, parent.as_bytes(),
                    ).unwrap();
                    encoder.compress(target.as_bytes()).unwrap()
                }
                TextDeltaMode::FullZstd => unreachable!(),
            };
            let frame = to_header(mode, base, target_id, target.as_bytes(), payload)
                .unwrap();
            assert_eq!(
                decode(&frame, target_id, base).unwrap(), target.as_bytes(),
            );
            assert_eq!(
                decode(&frame, target_id, None),
                Err(TextDeltaError::MissingBase),
            );
            assert_eq!(
                decode(&frame, target_id,
                       Some((parent_id, "untrusted replacement".as_bytes()))),
                Err(TextDeltaError::WrongBase),
            );
        }
    }

    #[test]
    fn empty_long_random_repetitive_and_limit_edges() {
        let a = RevisionId::new();
        let b = RevisionId::new();
        let parent = "Русский markdown 🙂".repeat(1024);
        let target = format!("{}\n### end\n",parent);
        for bytes in [
            b"".as_slice(),
            "🙂🙂".as_bytes(),
            "changed arbitrary content".as_bytes(),
            target.as_bytes(),
        ] {
            let encoded = encode(b, bytes, Some((a, parent.as_bytes()))).unwrap();
            assert_eq!(decode(&encoded.bytes,b,Some((a,parent.as_bytes()))).unwrap(),bytes);
        }
        assert_eq!(
            encode(b,&vec![b'x';MAX_TEXT_BYTES+1],None).err(),
            Some(TextDeltaError::OverBudget)
        );
        let big = vec![b'A'; MAX_TEXT_BYTES];
        let encoded = encode(b,&big,None).unwrap();
        assert_eq!(decode(&encoded.bytes,b,None).unwrap(),big);
        let mut falsified = encoded.bytes.clone();
        falsified[108..112].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode(&falsified,b,None).is_err());
        assert!(!encoded.bytes.starts_with(b"AKPKEXP1"));
    }

    #[test]
    #[ignore = "Run only in the dedicated complete-Git-history codec A/B workflow"]
    fn real_git_markdown_pairs_numeric_ab() {
        use std::{path::Path, process::Command};

        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap();
        let git = |args: &[&str]| -> Vec<u8> {
            let output = Command::new("git")
                .current_dir(repo)
                .args(args).output().expect("Git unavailable for actual history A/B");
            assert!(output.status.success(), "Git revision read failed: {:?}",
                    String::from_utf8_lossy(&output.stderr));
            output.stdout
        };
        let paths = [
            "docs/performance-roadmap.md",
            "README.ru.md",
            "docs/sync/adr-0006-cross-process-mutation-boundary.md",
            "docs/sync/adr-0005-inprocess-history-files-interlock.md",
            "aquilum-app/native/testdata/markdown-test.md",
        ];
        let mut cases = 0usize;
        let mut full_bytes = 0usize;
        let mut selected_bytes = 0usize;
        let mut full_count = 0usize;
        let mut splice_count = 0usize;
        let mut dictionary_count = 0usize;
        for path in paths {
            let raw_log = git(&["log", "--format=%H", "--", path]);
            let commits = std::str::from_utf8(&raw_log).unwrap()
                .lines().take(5).collect::<Vec<_>>();
            for pair in commits.windows(2) {
                let previous = format!("{}:{path}", pair[1]);
                let next = format!("{}:{path}", pair[0]);
                let old = git(&["show", &previous]);
                let new = git(&["show", &next]);
                if old.len() < 64 || new.len() < 64 || old == new
                    || old.len() > MAX_TEXT_BYTES || new.len() > MAX_TEXT_BYTES
                    || std::str::from_utf8(&old).is_err()
                    || std::str::from_utf8(&new).is_err()
                {
                    continue;
                }
                let base = RevisionId::new();
                let target = RevisionId::new();
                let chosen = encode(target, &new, Some((base, &old))).unwrap();
                let full = encode(target, &new, None).unwrap();
                assert_eq!(decode(&chosen.bytes, target, Some((base, &old))).unwrap(), new);
                assert_eq!(decode(&full.bytes, target, None).unwrap(), new);
                assert!(chosen.bytes.len() <= full.bytes.len());
                cases += 1;
                full_bytes += full.bytes.len();
                selected_bytes += chosen.bytes.len();
                match chosen.mode {
                    TextDeltaMode::FullZstd => full_count += 1,
                    TextDeltaMode::SpliceZstd => splice_count += 1,
                    TextDeltaMode::DictionaryZstd => dictionary_count += 1,
                }
            }
        }
        assert!(cases >= 9, "test requires checkout fetch-depth: 0");
        println!(
            "DELTA_TEXT_RUST_REAL_GIT pairs={cases} full_wire={full_bytes} selected_wire={selected_bytes} full={full_count} splice={splice_count} dictionary={dictionary_count}"
        );
    }
    #[test]
    fn forged_splice_offsets_and_trailing_bytes_refused() {
        let base=b"Unicode \xF0\x9F\x99\x82 content".to_vec();
        assert_eq!(splice_apply(&base,&[0;7]),Err(TextDeltaError::InvalidEnvelope));
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&u32::MAX.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.push(b'A');
        assert_eq!(splice_apply(&base,&invalid),Err(TextDeltaError::InvalidEnvelope));
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&0u32.to_le_bytes());
        invalid.push(b'A');
        assert_eq!(splice_apply(&base,&invalid),Err(TextDeltaError::InvalidEnvelope));
    }
    include!("text_delta_codec_profile_tests.rs");
}
