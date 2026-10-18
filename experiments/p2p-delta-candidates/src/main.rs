//! P2P-DELTA-04: research-only, in-memory candidate A/B against real public Git edits.
//! No QUIC, network admission, filesystem apply, File Gate, cloud writes, or trusted baseline.
//! Sizes include a *modelled* 120-byte metadata header, not actual future P2P wire format.

use copia::{Delta, Sync, SyncBuilder};
use std::collections::BTreeSet;
use std::error::Error;
use std::io::{self, Cursor, Write};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

const MAX_TEXT: usize = 2 * 1024 * 1024;
const MODEL_HEADER_BYTES: usize = 120; // same size as experimental AKTDLT01 header
const BASE_MIN_SAVING: usize = 128;
const BASE_MIN_PERCENT: usize = 15;
const PATHS: [&str; 5] = [
    "docs/performance-roadmap.md",
    "README.ru.md",
    "docs/sync/adr-0006-cross-process-mutation-boundary.md",
    "docs/sync/adr-0005-inprocess-history-files-interlock.md",
    "aquilum-app/native/testdata/markdown-test.md",
];

fn reject(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// The independent caller must authenticate the claimed base RevisionId and
/// VaultId before using this function. A hash match alone is not authority.
fn guarded_restore(
    sync: &impl Sync,
    base: &[u8],
    expected_base_hash: [u8; 32],
    delta: &Delta,
    expected_target_hash: [u8; 32],
) -> Result<Vec<u8>, Box<dyn Error>> {
    if base.len() > MAX_TEXT || sha(base) != expected_base_hash {
        return Err(reject("unverified or changed base bytes").into());
    }
    let mut output = LimitedWriter {
        bytes: Vec::new(),
        limit: MAX_TEXT,
    };
    sync.patch(Cursor::new(base), delta, &mut output)?;
    if sha(&output.bytes) != expected_target_hash {
        return Err(reject("reconstructed target BLAKE3 mismatch").into());
    }
    Ok(output.bytes)
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let size = self.bytes.len().checked_add(data.len())
            .ok_or_else(|| reject("patch output overflow"))?;
        if size > self.limit {
            return Err(reject("patch output exceeded 2MiB budget"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct Measured {
    full_wire: usize,
    dictionary_wire: usize,
    copia_signature_wire: usize,
    copia_delta_wire: usize,
    full_us: u128,
    dictionary_us: u128,
    copia_us: u128,
}

fn compare(base: &[u8], target: &[u8]) -> Result<Measured, Box<dyn Error>> {
    if base.len() > MAX_TEXT || target.len() > MAX_TEXT || base.len() < 64 {
        return Err(reject("text shape outside 2MiB/64B comparison bounds").into());
    }
    std::str::from_utf8(base)?;
    std::str::from_utf8(target)?;

    let started = Instant::now();
    let full = zstd::bulk::compress(target, 3)?;
    let full_us = started.elapsed().as_micros();
    let full_restored = zstd::bulk::decompress(&full, target.len())?;
    if full_restored != target {
        return Err(reject("full Zstd restored different bytes").into());
    }

    let started = Instant::now();
    let mut compressor = zstd::bulk::Compressor::with_dictionary(3, base)?;
    let dictionary = compressor.compress(target)?;
    let dictionary_us = started.elapsed().as_micros();
    let mut decompressor = zstd::bulk::Decompressor::with_dictionary(base)?;
    let dict_restored = decompressor.decompress(&dictionary, target.len())?;
    if dict_restored != target {
        return Err(reject("dictionary Zstd restored different bytes").into());
    }

    // Deliberately do NOT import copia's directory scanner, mirror, delete,
    // conflict handling, or on-disk writes into Akula. Test its pure primitives.
    let started = Instant::now();
    let sync = SyncBuilder::new()
        .block_size(2048)
        .strong_hash_len(32)
        .build();
    let signature = sync.signature(Cursor::new(base))?;
    let delta = sync.delta(Cursor::new(target), &signature)?;
    let copia_us = started.elapsed().as_micros();
    let signature_bytes = bincode::serialize(&signature)?;
    let delta_bytes = bincode::serialize(&delta)?;
    let restored = guarded_restore(&sync, base, sha(base), &delta, sha(target))?;
    if restored != target {
        return Err(reject("copia restored different bytes").into());
    }

    // Modelled metadata overhead, not a validated protocol schema. Real iroh
    // framing/TLS, peer admission, RTT, grants and retransmits are NOT measured.
    Ok(Measured {
        full_wire: MODEL_HEADER_BYTES + full.len(),
        dictionary_wire: MODEL_HEADER_BYTES + dictionary.len(),
        copia_signature_wire: MODEL_HEADER_BYTES + signature_bytes.len(),
        copia_delta_wire: MODEL_HEADER_BYTES + delta_bytes.len(),
        full_us,
        dictionary_us,
        copia_us,
    })
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let result = Command::new("git").arg("-C").arg(root).args(args).output()?;
    if !result.status.success() {
        return Err(reject("real Git revision unavailable (complete checkout required)").into());
    }
    Ok(result.stdout)
}

fn percentile(numbers: &mut [u128], pct: usize) -> u128 {
    numbers.sort_unstable();
    if numbers.is_empty() {
        return 0;
    }
    let index = ((numbers.len() - 1) * pct).div_euclid(100);
    numbers[index]
}

fn run(root: &Path) -> Result<(), Box<dyn Error>> {
    if !root.join(".git").exists() {
        return Err(reject("complete source Git checkout required").into());
    }
    let mut files = BTreeSet::new();
    let mut pairs = 0usize;
    let mut full_total = 0usize;
    let mut dict_total = 0usize;
    let mut sig_total = 0usize;
    let mut copia_delta_total = 0usize;
    let mut dictionary_wins = 0usize;
    let mut full_times = Vec::new();
    let mut dict_times = Vec::new();
    let mut copia_times = Vec::new();

    for path in PATHS {
        let log = git(root, &["log", "--format=%H", "--", path])?;
        let ids = String::from_utf8(log)?;
        let commits: Vec<&str> = ids.lines().collect();
        let mut added = 0usize;

        for adjacent in commits.windows(2) {
            if pairs >= 20 || added >= 4 {
                break;
            }
            let new_spec = format!("{}:{}", adjacent[0], path);
            let old_spec = format!("{}:{}", adjacent[1], path);
            let (Ok(new), Ok(old)) = (
                git(root, &["show", &new_spec]),
                git(root, &["show", &old_spec]),
            ) else {
                continue; // deleted/renamed fixture: not a text-pair
            };
            if old == new || old.len() < 64 || new.len() < 64
                || old.len() > MAX_TEXT || new.len() > MAX_TEXT
                || std::str::from_utf8(&old).is_err()
                || std::str::from_utf8(&new).is_err()
            {
                continue;
            }

            let m = compare(&old, &new)?;
            // Same threshold as DELTA-TEXT-03; cannot pick a dependent format
            // solely because its payload is one or two bytes shorter.
            let saved = m.full_wire.saturating_sub(m.dictionary_wire);
            let select_dictionary = saved >= BASE_MIN_SAVING
                && saved * 100 >= m.full_wire * BASE_MIN_PERCENT;
            if select_dictionary {
                dictionary_wins += 1;
            }
            println!(
                "P2P_REAL_PAIR path={} from={} to={} old={} new={} full_wire={} dictionary_wire={} copia_signature_wire={} copia_delta_wire={} copia_total_wire={} eligible_dictionary={} full_encode_us={} dictionary_encode_us={} copia_signature_delta_us={}",
                path, &adjacent[1][..12], &adjacent[0][..12], old.len(), new.len(),
                m.full_wire, m.dictionary_wire, m.copia_signature_wire,
                m.copia_delta_wire, m.copia_signature_wire + m.copia_delta_wire,
                select_dictionary, m.full_us, m.dictionary_us, m.copia_us
            );
            full_total += m.full_wire;
            dict_total += m.dictionary_wire;
            sig_total += m.copia_signature_wire;
            copia_delta_total += m.copia_delta_wire;
            full_times.push(m.full_us);
            dict_times.push(m.dictionary_us);
            copia_times.push(m.copia_us);
            pairs += 1;
            added += 1;
            files.insert(path);
        }
    }

    if pairs < 9 || files.len() < 3 {
        return Err(reject("fixture insufficient: fewer than 9 pairs or 3 paths").into());
    }
    println!(
        "P2P_CANDIDATE_SUMMARY pairs={} paths={} full_wire={} dictionary_wire={} dictionary_eligible={} copia_signature_wire={} copia_delta_wire={} copia_total_wire={} full_encode_p50_us={} dictionary_encode_p50_us={} copia_encode_p50_us={} full_encode_p95_us={} dictionary_encode_p95_us={} copia_encode_p95_us={} production_transport=false",
        pairs, files.len(), full_total, dict_total, dictionary_wins,
        sig_total, copia_delta_total, sig_total + copia_delta_total,
        percentile(&mut full_times.clone(), 50),
        percentile(&mut dict_times.clone(), 50),
        percentile(&mut copia_times.clone(), 50),
        percentile(&mut full_times, 95),
        percentile(&mut dict_times, 95),
        percentile(&mut copia_times, 95),
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    run(Path::new(&root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_edits_restore_exact_bytes() {
        let base = "Раздел 1: проверяем текст и Unicode — Акула.\n".repeat(100);
        let target = base.replace("Акула", "Akula ↔ Akula");
        let m = compare(base.as_bytes(), target.as_bytes()).unwrap();
        assert!(m.full_wire > 0 && m.dictionary_wire > 0);
        assert!(m.copia_signature_wire > MODEL_HEADER_BYTES);
        assert!(m.copia_delta_wire > MODEL_HEADER_BYTES);
    }

    #[test]
    fn wrong_or_changed_base_rejected_before_patch() {
        let base = "source document ".repeat(200);
        let target = format!("{}additional text", base);
        let sync = SyncBuilder::new().block_size(2048).build();
        let sig = sync.signature(Cursor::new(base.as_bytes())).unwrap();
        let delta = sync.delta(Cursor::new(target.as_bytes()), &sig).unwrap();
        let bad = base.replace("source", "stolen");
        let outcome = guarded_restore(
            &sync, bad.as_bytes(), sha(base.as_bytes()), &delta, sha(target.as_bytes())
        );
        assert!(outcome.is_err());
    }

    #[test]
    fn target_integrity_and_2mib_budget_are_independent() {
        let base = "document \n".repeat(400);
        let target = format!("{}last line", base);
        let sync = SyncBuilder::new().build();
        let sig = sync.signature(Cursor::new(base.as_bytes())).unwrap();
        let delta = sync.delta(Cursor::new(target.as_bytes()), &sig).unwrap();
        assert!(guarded_restore(
            &sync, base.as_bytes(), sha(base.as_bytes()), &delta, sha(b"wrong target")
        ).is_err());
        let oversized = vec![b'x'; MAX_TEXT + 1];
        assert!(compare(base.as_bytes(), &oversized).is_err());
    }

    #[test]
    fn full_fallback_never_uses_remote_unverified_base() {
        let target = "no prior verified version".repeat(100);
        let full = zstd::bulk::compress(target.as_bytes(), 3).unwrap();
        let restored = zstd::bulk::decompress(&full, target.len()).unwrap();
        assert_eq!(restored, target.as_bytes());
    }
}
