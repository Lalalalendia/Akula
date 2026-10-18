// Diagnostic only: compare the current Akula Rope flatten-before-save boundary
// against unbuffered and bounded-batch streaming inspired by Aquilum v0.3.10.
// No File Gate, HistoryService, durable intent, or production save path changes.
use crop::Rope;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::Instant;

const CHUNK: usize = 64 * 1024;
const BUFFER: usize = 1024 * 1024;
const BATCH: usize = 256 * 1024;

fn status_kib(field: &str) -> u64 {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn hash_on_disk(path: &Path) -> io::Result<String> {
    let mut reader = BufReader::with_capacity(CHUNK, File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; CHUNK];
    loop {
        let size = reader.read(&mut buf)?;
        if size == 0 {
            break;
        }
        hasher.update(&buf[..size]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn fixture(bytes: usize) -> Rope {
    // Build the rope incrementally; a giant input String would contaminate
    // VmHWM and hide the extra full-sized String allocated by the flat path.
    let sentence = "Глава 27 — текст, проверка UTF-8: 漢字 и 😀.\n";
    let part = sentence.repeat(2048);
    let mut rope = Rope::new();
    while rope.byte_len() + part.len() <= bytes {
        rope.insert(rope.byte_len(), &part);
    }
    let left = bytes - rope.byte_len();
    let mut tail = String::with_capacity(left);
    while tail.len() + sentence.len() <= left {
        tail.push_str(sentence);
    }
    tail.push_str(&"x".repeat(left - tail.len()));
    rope.insert(rope.byte_len(), &tail);
    assert_eq!(rope.byte_len(), bytes);
    rope
}

// Mirrors Akula native/src/note/save.rs: Rope::to_string() before gate::write.
fn flattened(rope: &Rope, out: &mut impl Write) -> io::Result<String> {
    let body = rope.to_string();
    let digest = blake3::hash(body.as_bytes()).to_hex().to_string();
    out.write_all(body.as_bytes())?;
    Ok(digest)
}

// Direct per-Rope-chunk incremental hashing and output.
fn streamed(rope: &Rope, out: &mut impl Write) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    for chunk in rope.chunks() {
        let bytes = chunk.as_bytes();
        hasher.update(bytes);
        out.write_all(bytes)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

// Bounded batching tests whether chunk-wise BLAKE3 overhead can be reduced
// without returning to an O(file size) flatten allocation.
fn batched(rope: &Rope, out: &mut impl Write) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = Vec::with_capacity(BATCH);
    for chunk in rope.chunks() {
        let mut bytes = chunk.as_bytes();
        while !bytes.is_empty() {
            let take = bytes.len().min(BATCH - buffer.len());
            buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if buffer.len() == BATCH {
                hasher.update(&buffer);
                out.write_all(&buffer)?;
                buffer.clear();
            }
        }
    }
    if !buffer.is_empty() {
        hasher.update(&buffer);
        out.write_all(&buffer)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("usage: rope-save-stream <flat|stream|batch> <size MiB>")?;
    let mib: usize = args.next().ok_or("missing MiB")?.parse()?;
    if !matches!(mode.as_str(), "flat" | "stream" | "batch") || !matches!(mib, 4 | 64) {
        return Err("unsupported mode/size".into());
    }
    let bytes = mib * 1024 * 1024;
    let rope = fixture(bytes);
    let mut expected_hasher = blake3::Hasher::new();
    for chunk in rope.chunks() {
        expected_hasher.update(chunk.as_bytes());
    }
    let expected = expected_hasher.finalize().to_hex().to_string();

    let before_kib = status_kib("VmRSS:");
    let before_peak_kib = status_kib("VmHWM:");
    let outfile = std::env::temp_dir().join(format!(
        "akula-upstream-stream-probe-{}-{}-{mib}.md",
        std::process::id(),
        mode
    ));

    let start = Instant::now();
    let mut output = BufWriter::with_capacity(BUFFER, File::create(&outfile)?);
    let digest = match mode.as_str() {
        "flat" => flattened(std::hint::black_box(&rope), &mut output)?,
        "stream" => streamed(std::hint::black_box(&rope), &mut output)?,
        "batch" => batched(std::hint::black_box(&rope), &mut output)?,
        _ => unreachable!(),
    };
    output.flush()?;
    drop(output);
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_kib = status_kib("VmRSS:");
    let peak_kib = status_kib("VmHWM:");
    let peak_extra_kib = peak_kib.saturating_sub(before_peak_kib);

    assert_eq!(digest, expected, "computed hash must match original UTF-8 bytes");
    assert_eq!(fs::metadata(&outfile)?.len(), bytes as u64);
    assert_eq!(hash_on_disk(&outfile)?, expected, "persisted bytes must match the source");
    fs::remove_file(&outfile)?;

    println!(
        "BENCH-ROPE-SAVE mode={mode} mib={mib} bytes={bytes} elapsed_ms={elapsed_ms:.4} \
         rss_before_kib={before_kib} rss_after_kib={after_kib} peak_kib={peak_kib} peak_extra_kib={peak_extra_kib}"
    );
    Ok(())
}
