#!/usr/bin/env python3
"""Isolated std-only ABBA benchmark of exact production EPUB/PalmDOC functions."""
import pathlib
import re
import subprocess
import tempfile

BASE = "e512a0f0711853bbdbfb39ec07965ad7641b2ab5"
ROOT = pathlib.Path(__file__).resolve().parents[1]

def source(ref, path):
    return subprocess.check_output(["git", "show", f"{ref}:{path}"], cwd=ROOT, text=True)

def function(text, name):
    # Stop at the next top-level item; nested blocks are indented in these files.
    match = re.search(r"^(?:pub )?fn " + name + r"\(", text, re.M)
    if match is None:
        raise RuntimeError(f"missing production function {name}")
    tail = text[match.start():]
    boundary = re.search(r"\n(?=(?:pub )?fn |\#\[cfg|mod )", tail)
    return tail[:boundary.start() if boundary else len(tail)].replace(
        f"fn {name}(", f"pub fn {name}(", 1
    ).replace("pub pub fn", "pub fn")

def module(ref, name):
    text = source(ref, "aquilum-app/native/src/reader/text.rs")
    entities = re.search(r"const ENTITIES:.*?^\];", text, re.M | re.S)
    if entities is None:
        raise RuntimeError("missing entity table")
    mobi = source(ref, "aquilum-app/native/src/reader/mobi.rs")
    return f"mod {name} {{\n" + entities.group() + "\n" + function(text, "numeric_entities") + "\n" + function(mobi, "decompress_palmdoc") + "\n}\n"

HARNESS = r'''
use std::hint::black_box;
use std::time::Instant;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn timed<T>(f: impl Fn() -> T, iterations: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations { black_box(f()); }
    start.elapsed().as_secs_f64() * 1e6 / iterations as f64
}
fn bench<T: PartialEq + std::fmt::Debug>(name: &str, bytes: usize, old: impl Fn() -> T, new: impl Fn() -> T) {
    assert_eq!(old(), new(), "output mismatch: {name}");
    for _ in 0..4 { black_box(old()); black_box(new()); }
    let mut a = Vec::new();
    let mut b = Vec::new();
    for _ in 0..11 {
        // A B B A, same inputs and runner. Each sample averages eight operations.
        a.push(timed(&old, 8));
        b.push(timed(&new, 8));
        b.push(timed(&new, 8));
        a.push(timed(&old, 8));
    }
    println!("RAW {name} old_us={a:?} new_us={b:?}");
    let old_us = median(&mut a);
    let new_us = median(&mut b);
    println!("RESULT {name} input_bytes={bytes} old_us={old_us:.3} new_us={new_us:.3} ratio_old_over_new={:.3}", old_us / new_us);
}
fn main() {
    for size in [64 * 1024, 512 * 1024] {
        for (label, unit) in [
            ("plain", "<p>Обычный текст without entities.</p>".to_owned()),
            ("sparse", "<p>Обычный текст without entities.</p>".repeat(100)),
            ("dense", "&nbsp;&mdash;&thinsp;&micro;&amp;&#160;".to_owned()),
        ] {
            let unit = if label == "sparse" { format!("{unit}&nbsp;") } else { unit.to_owned() };
            let input = unit.repeat(size / unit.len() + 1);
            bench(&format!("epub-{label}-{size}"), input.len(),
                || baseline::numeric_entities(black_box(&input)),
                || candidate::numeric_entities(black_box(&input)));
        }
        for (label, pair) in [("nonoverlap", [0x80, 0x87]), ("overlap", [0x80, 0x0f])] {
            let mut input = b"abcdefghijklmnop".to_vec();
            while input.len() < size { input.extend_from_slice(&pair); }
            bench(&format!("palmdoc-{label}-{size}"), input.len(),
                || baseline::decompress_palmdoc(black_box(&input)),
                || candidate::decompress_palmdoc(black_box(&input)));
        }
        let input = b"literal text only ".repeat(size / 18 + 1);
        bench(&format!("palmdoc-literal-{size}"), input.len(),
            || baseline::decompress_palmdoc(black_box(&input)),
            || candidate::decompress_palmdoc(black_box(&input)));
    }
}
'''

head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
print(f"BASE={BASE}\nHEAD={head}", flush=True)
subprocess.run(["rustc", "--version", "--verbose"], check=True)
with tempfile.TemporaryDirectory() as temp:
    path = pathlib.Path(temp)
    (path / "bench.rs").write_text(module(BASE, "baseline") + module("HEAD", "candidate") + HARNESS)
    subprocess.run(["rustc", "--edition=2024", "-O", str(path / "bench.rs"), "-o", str(path / "bench")], check=True)
    subprocess.run([str(path / "bench")], check=True)
