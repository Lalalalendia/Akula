#!/usr/bin/env python3
"""Compare actual old/new SSH helpers in one optimized Rust binary on one runner."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "aquilum-app/core/src/"
SHARED = SOURCE + "server_bootstrap_ssh_common.rs"


def git_file(ref, path):
    result = subprocess.run(["git", "show", f"{ref}:{path}"], cwd=ROOT, capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else None


def function(source, name):
    start = source.index(f"fn {name}(")
    opening = source.index("{", start)
    depth = 0
    for end in range(opening, len(source)):
        depth += (source[end] == "{") - (source[end] == "}")
        if not depth:
            return "pub(super) " + source[start:end + 1]
    raise ValueError(f"unclosed function {name}")


def command(source):
    block = re.search(r"const FACTS_COMMAND: &str = concat!\((.*?)\n\);", source, re.S)
    return "".join(json.loads(value) for value in re.findall(r'"(?:\\.|[^"\\])*"', block[1]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    args = parser.parse_args()
    current = (ROOT / SHARED).read_text()
    old_shared = git_file(args.base, SHARED)
    modules = []
    for os_name, filename in [("linux", "server_bootstrap_ssh_lab.rs"), ("windows", "server_bootstrap_ssh_windows_lab.rs")]:
        previous = old_shared if old_shared is not None else git_file(args.base, SOURCE + filename)
        if previous is None:
            raise RuntimeError(f"baseline source missing: {args.base}:{filename}")
        assert command(previous) == command(current), f"remote command changed: {os_name}"
        assert "const MAX_OUTPUT: u64 = 8192;" in previous and "const MAX_OUTPUT: u64 = 8192;" in current
        helpers = "\n".join(function(previous, name) for name in ["decode_ed25519_wire", "parse_facts"])
        modules.append(f"mod old_{os_name} {{ use crate::server_bootstrap_lab::{{B0Error, MachineArch, ReadOnlyHostFacts, SupportedOs}}; use std::collections::BTreeMap; const MAX_OUTPUT: u64 = 8192; {helpers} }}")
    build = subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(ROOT / "aquilum-app/Cargo.toml"), "-p", "aquilum-core", "--features", "server-bootstrap-b0-openssh-lab", "--message-format=json"], cwd=ROOT, check=True, capture_output=True, text=True)
    library = None
    for line in build.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item["target"]["name"] == "akula_server_bootstrap_b0":
            library = next((Path(p) for p in item["filenames"] if p.endswith(".rlib")), library)
    if library is None:
        raise RuntimeError("Cargo did not report the actual bootstrap contract library")
    with tempfile.TemporaryDirectory(prefix="akula-ssh-ab-") as directory:
        directory = Path(directory)
        (directory / "common.rs").write_text(current)
        harness = directory / "bench.rs"
        harness.write_text('#![allow(dead_code)]\nuse akula_server_bootstrap_b0 as server_bootstrap_lab;\n' + "\n".join(modules) + '\nmod common;\n' + HARNESS)
        binary = directory / ("bench.exe" if os.name == "nt" else "bench")
        subprocess.run(["rustc", "--edition=2021", "-O", "-C", "codegen-units=1", "-L", f"dependency={library.parent}", "--extern", f"akula_server_bootstrap_b0={library}", str(harness), "-o", str(binary)], check=True)
        result = subprocess.run([str(binary)], check=True, capture_output=True, text=True)
    print(json.dumps({"base": args.base, "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "os": platform.platform(), "cpu": platform.processor(), "rustc": subprocess.check_output(["rustc", "-Vv"], text=True), "remote_command_bytes_identical": True}))
    samples = {}
    for line in result.stdout.splitlines():
        if line.startswith("PARITY"):
            print(line)
            continue
        kernel, baseline, cycle, phase, ns = line.split(",")
        samples.setdefault((kernel, baseline, int(cycle)), {})[phase] = float(ns)
        print(line)
    for kernel, baseline in sorted({(k, b) for k, b, _ in samples}):
        pairs = [v for (k, b, _), v in samples.items() if (k, b) == (kernel, baseline)]
        old = [statistics.mean([v["A1"], v["A2"]]) for v in pairs]
        new = [statistics.mean([v["B1"], v["B2"]]) for v in pairs]
        print(json.dumps({"kernel": kernel, "baseline": baseline, "old_ns_median": statistics.median(old), "new_ns_median": statistics.median(new), "paired_ratios": [b / a for a, b in zip(old, new)], "new_over_old_median": statistics.median([b / a for a, b in zip(old, new)])}))


HARNESS = r'''
use std::hint::black_box;
use std::time::Instant;
use server_bootstrap_lab::{B0Error, ReadOnlyHostFacts};
const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcH";
const FACTS: &str = "AKULA_B0_OS=ID=ubuntu\nAKULA_B0_ARCH=x86_64\nAKULA_B0_SYSTEMD=yes\nAKULA_B0_SUDO=no\nAKULA_B0_CPU=8\nAKULA_B0_RAM=8192\nAKULA_B0_DISK=65536\n";
fn measure<T>(f: fn(&str) -> T, values: &[String]) -> f64 {
    let f = black_box(f);
    let start = Instant::now();
    for _ in 0..20000 {
        for value in values { black_box(f(black_box(value.as_str()))); }
    }
    start.elapsed().as_nanos() as f64 / (values.len() * 20000) as f64
}
fn compare<T>(kernel: &str, baseline: &str, old: fn(&str) -> T, new: fn(&str) -> T, values: &[String]) {
    black_box(measure(old, values)); black_box(measure(new, values));
    for cycle in 0..7 {
        for (phase, f) in [("A1", old), ("B1", new), ("B2", new), ("A2", old)] {
            println!("{kernel},{baseline},{cycle},{phase},{}", measure(f, values));
        }
    }
}
fn main() {
    let mut keys = vec![KEY.to_owned(), String::new(), format!("{KEY}=")];
    for position in 0..KEY.len() {
        for byte in 0x20..=0x7e {
            let mut candidate = KEY.as_bytes().to_vec(); candidate[position] = byte;
            keys.push(String::from_utf8(candidate).unwrap());
        }
    }
    let mut facts = vec![FACTS.to_owned(), FACTS.replace('\n', "\r\n"), String::new(), "x".repeat(8193)];
    for field in ["CPU=8", "RAM=8192", "DISK=65536", "SYSTEMD=yes", "SUDO=no", "OS=ID=ubuntu", "ARCH=x86_64"] {
        let key = field.split('=').next().unwrap();
        for value in ["", "0", "1", "65535", "65536", "18446744073709551615", "18446744073709551616", "-1", "yes", "no", "ID=debian", "unknown"] {
            facts.push(FACTS.replace(field, &format!("{key}={value}")));
        }
    }
    for value in &keys { assert_eq!(old_linux::decode_ed25519_wire(value), common::decode_ed25519_wire(value)); assert_eq!(old_windows::decode_ed25519_wire(value), common::decode_ed25519_wire(value)); }
    for value in &facts { assert_eq!(old_linux::parse_facts(value), common::parse_facts(value)); assert_eq!(old_windows::parse_facts(value), common::parse_facts(value)); }
    println!("PARITY keys={} facts={} BOTH_BASELINES_PASS", keys.len(), facts.len());
    let key_calls: [(&str, fn(&str) -> Result<Vec<u8>, B0Error>); 2] = [("linux", old_linux::decode_ed25519_wire), ("windows", old_windows::decode_ed25519_wire)];
    let fact_calls: [(&str, fn(&str) -> Result<ReadOnlyHostFacts, B0Error>); 2] = [("linux", old_linux::parse_facts), ("windows", old_windows::parse_facts)];
    let valid_keys = [KEY.to_owned()];
    let invalid_keys = [format!("{KEY}="), "!".repeat(68)];
    let valid_facts = [FACTS.to_owned(), FACTS.replace("OS=ID=ubuntu", "OS=ID=debian")];
    let invalid_facts = [FACTS.replace("CPU=8", "CPU=65536"), FACTS.replace("RAM=8192", "RAM=-1")];
    for (name, old) in key_calls {
        compare("decode_valid", name, old, common::decode_ed25519_wire, &valid_keys);
        compare("decode_invalid", name, old, common::decode_ed25519_wire, &invalid_keys);
    }
    for (name, old) in fact_calls {
        compare("facts_valid", name, old, common::parse_facts, &valid_facts);
        compare("facts_invalid", name, old, common::parse_facts, &invalid_facts);
    }
}
'''

if __name__ == "__main__":
    main()
