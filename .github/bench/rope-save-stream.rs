// Diagnostic only: compare Akula's current Rope flatten-before-save boundary with
// incremental Rope-chunk hashing/writing inspired by Freaction/Aquilum v0.3.10.
// No File Gate, HistoryService, locks, fsync, or production write path is changed.
use crop::Rope;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::Instant;

const CHUNK: usize = 64 * 1024;
const BUFFER: usize = 1024 * 1024;

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

// Matches the representation boundary in Akula native/src/note/save.rs:
 // text.to_string() -> core::gate::write(&body), with a full-sized temporary.
fn flattened(rope: &Rope, out: &mut impl Write) -> io::Result<String> {
    let body = rope.to_string();
    let digest = blake3::hash(body.as_bytes()).to_hex().to_string();
    out.write_all(body.as_bytes())?;
    Ok(digest)
}

// Tests the streaming-serialization mechanism, NOT the full upstream
// Content/Pieces history subsystem or an authorized Akula save.
fn streamed(rope: &Rope, out: &mut impl Write) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    for piece in rope.chunks() {
        let bytes = piece.as_bytes();
        hasher.update(bytes);
        out.write_all(bytes)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("usage: rope-save-stream <flat|stream> <size MiB>")?;
    let mib: usize = args.next().ok_or("missing MiB")?.parse()?;
    if !matches!(mode.as_str(), "flat" | "stream") || !matches!(mib, 4 | 64) {
        return Err("unsupported mode/size".into());
    }
    let bytes = mib * 1024 * 1024;
    let sentence = "Глава 27 — текст, проверка UTF-8: 漢字 и 😀.\n";
    let mut input = String::with_capacity(bytes);
    while input.len() + sentence.len() <= bytes {
        input.push_str(sentence);
    }
    input.push_str(&"x".repeat(bytes - input.len()));
    let expected = blake3::hash(input.as_bytes()).to_hex().to_string();
    let rope = Rope::from(input.as_str());
    assert_eq!(rope.byte_len(), bytes);
    drop(input);

    let before_kib = status_kib("VmRSS:");
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
        _ => unreachable!(),
    };
    output.flush()?;
    drop(output);
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_kib = status_kib("VmRSS:");
    let peak_kib = status_kib("VmHWM:");

    assert_eq!(digest, expected, "computed hash must match original UTF-8 bytes");
    assert_eq!(fs::metadata(&outfile)?.len(), bytes as u64);
    assert_eq!(hash_on_disk(&outfile)?, expected, "persisted bytes must match the source");
    fs::remove_file(&outfile)?;

    println!(
        "BENCH-ROPE-SAVE mode={mode} mib={mib} bytes={bytes} elapsed_ms={elapsed_ms:.4} \
         rss_before_kib={before_kib} rss_after_kib={after_kib} peak_kib={peak_kib}"
    );
    Ok(())
}
