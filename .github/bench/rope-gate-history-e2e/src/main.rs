//! A diagnostic of the REAL native conversion boundary and Core File Gate:
//!     clone a stable Rope -> to_string() -> gate::write -> HistoryService.
//!
//! Runs in a disposable workspace with no user data. This is NOT a second
//! writer, and does not bypass the File Gate or invoke its private API.
//! No network, S3, installed-host mode, UI queue or recovery activation here.
//! The "before" and "after" binaries are built from pinned Git revisions.
use aquilum_core::Core;
use aquilum_core::CoreEvent;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use crop::Rope;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

fn status_kib(label: &str) -> u64 {
    let Ok(text) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    text.lines()
        .find(|line| line.starts_with(label))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(0)
}

fn make_rope(target_bytes: usize) -> Rope {
    let line = "Новая заметка 😀 漢字 / large saved text, CRLF:\r\n";
    let part = line.repeat(2048);
    let mut rope = Rope::new();
    while rope.byte_len() + part.len() <= target_bytes {
        rope.insert(rope.byte_len(), &part);
    }
    let rest = target_bytes - rope.byte_len();
    if rest != 0 {
        rope.insert(rope.byte_len(), &"x".repeat(rest));
    }
    assert_eq!(rope.byte_len(), target_bytes);
    rope
}

fn original_text(target_bytes: usize) -> String {
    let line = "Старая редакция / untouched baseline \r\n";
    let mut text = String::with_capacity(target_bytes);
    while text.len() + line.len() <= target_bytes {
        text.push_str(line);
    }
    text.push_str(&"z".repeat(target_bytes - text.len()));
    text
}

fn raw_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn file_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read note bytes")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let variant = args.next().expect("usage: rope-gate-history-e2e <before|after> <1|8>");
    assert!(variant == "before" || variant == "after");
    let mib: usize = args.next().expect("missing MiB size").parse().expect("invalid size");
    assert!(matches!(mib, 1 | 8), "only bounded note fixtures are supported");
    let bytes = mib * 1024 * 1024;

    let temp = tempfile::tempdir().expect("scratch workspace");
    let root = temp.path().join("vault");
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&appdata).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let path = root.join("Реальная заметка.md");

    // The existing note has a real baseline, and its disk hash contains
    // CRLF bytes. The HistoryService's baseline is normalized to LF.
    let baseline = original_text(bytes);
    let baseline_hash = raw_hash(baseline.as_bytes());
    fs::write(&path, baseline.as_bytes()).unwrap();
    let expected_baseline_history = baseline.replace("\r\n", "\n");
    drop(baseline);

    // This is the SAME Core and File Gate entrypoint used by native::note::save.
    // Prepare the large Rope outside the timed operation just like the UI.
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let live_rope = make_rope(bytes);
    let stable_revision = live_rope.clone();
    let mut predicted = blake3::Hasher::new();
    for chunk in stable_revision.chunks() {
        predicted.update(chunk.as_bytes());
    }
    let predicted_hash = predicted.finalize().to_hex().to_string();

    let rss_before_kib = status_kib("VmRSS:");
    let peak_before_kib = status_kib("VmHWM:");

    let start = Instant::now();
    let body = std::hint::black_box(&stable_revision).to_string();
    let flatten_ms = start.elapsed().as_secs_f64() * 1000.0;

    let gate_started = Instant::now();
    let saved = gate::write(&core, &path, &body, Some(&baseline_hash), Source::Me, None)
        .expect("real File Gate first-save with baseline must succeed");
    let gate_ms = gate_started.elapsed().as_secs_f64() * 1000.0;
    let total_ms = start.elapsed().as_secs_f64() * 1000.0;

    let rss_after_kib = status_kib("VmRSS:");
    let peak_after_kib = status_kib("VmHWM:");

    // Everything below is OUTSIDE the timing. The first save must have
    // published the entire note and BOTH history versions. No partial writes.
    assert_eq!(body.as_bytes().len(), bytes);
    assert_eq!(saved.hash, predicted_hash);
    assert_eq!(raw_hash(&file_bytes(&path)), predicted_hash);
    assert_eq!(file_bytes(&path), body.as_bytes());
    let page = core.history.page(&|| vec![root.clone()], &path, 0, 8);
    assert_eq!(page.total, 2, "baseline and new full history version must persist");
    let stored_texts = page.versions.iter().map(|item| {
        core.history.version(&|| vec![root.clone()], &path, Some(&item.id))
            .expect("history version must be readable").text
    }).collect::<Vec<_>>();
    assert!(stored_texts.iter().any(|s| s == &body), "new history bytes missing");
    assert!(
        stored_texts.iter().any(|s| s == &expected_baseline_history),
        "original baseline bytes missing or not normalized as before"
    );

    // Same stale-CAS negative test in every sample and both revisions:
    // an external write must remain untouched and must not create history.
    let foreign = b"EXTERNAL revision\r\nnot ours\n";
    fs::write(&path, foreign).unwrap();
    let conflict = gate::write(
        &core, &path, &body, Some(&saved.hash), Source::Me, None,
    ).expect_err("stale raw disk hash must be rejected by the real File Gate");
    assert!(matches!(conflict, FileCommandError::Conflict { .. }));
    assert_eq!(file_bytes(&path), foreign);
    assert_eq!(
        core.history.page(&|| vec![root.clone()], &path, 0, 8).total, 2,
        "rejected save must not publish another history version"
    );

    println!(
        "REAL_GATE_SAVE variant={variant} mib={mib} bytes={bytes} \
         flatten_ms={flatten_ms:.4} gate_ms={gate_ms:.4} total_ms={total_ms:.4} \
         rss_before_kib={rss_before_kib} rss_after_kib={rss_after_kib} \
         peak_extra_kib={}",
        peak_after_kib.saturating_sub(peak_before_kib)
    );
}
