#!/usr/bin/env python3
"""Isolated Akula IDE agent-harness golden probe, not production Akula code.

Demonstrates LSP-backed direct Rust call resolution, a separately owned
architecture policy, version-bound verdicts, and bad->repaired agent overlays.
No Akula source file is modified, no external workspace code is executed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import subprocess
import time

from edge_accuracy_probe import LspClient, resolve_targets_with_readiness

CARGO = """[package]
name = "akula-ide-harness-fixture"
version = "0.1.0"
edition = "2021"
"""
FILES = {
    "Cargo.toml": CARGO,
    "src/lib.rs": "pub mod core;\npub mod storage;\npub mod ui;\n",
    "src/storage.rs": "pub(crate) fn internal_save() {}\n",
    "src/core.rs": (
        "pub fn save() { crate::storage::internal_save(); }\n"
        "pub fn save_with_receipt() { crate::storage::internal_save(); }\n"
    ),
}
BASE = "pub fn render() {\n    crate::core::save();\n}\n"
BAD = "pub fn render() {\n    crate::storage::internal_save();\n}\n"
REPAIRED = "pub fn render() {\n    crate::core::save_with_receipt();\n}\n"

# This policy is owned by the harness, not by the agent-controlled workspace.
POLICY = {
    "policy_id": "AK-IDE-LAB-Z-UI-001",
    "version": "fixture-v1",
    "source": "src/ui.rs",
    "allow_target_files": ["src/core.rs"],
    "deny_target_files": ["src/storage.rs"],
}
POLICY_HASH = hashlib.sha256(
    json.dumps(POLICY, sort_keys=True).encode("utf-8")
).hexdigest()


def source_hash(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def make_fixture(root):
    for rel, text in {**FILES, "src/ui.rs": BASE}.items():
        dst = root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(text, encoding="utf-8")


def cargo_compile_variants(root, results):
    """Demonstrate both patches are valid *Rust*; policy must be independent."""
    events = []
    cargo = root / "Cargo.toml"
    subprocess.run(
        ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(cargo)],
        check=True, capture_output=True, text=True, timeout=30
    )
    target = root / "src/ui.rs"
    try:
        for name, source in (("baseline", BASE), ("forbidden_patch", BAD),
                             ("repaired_patch", REPAIRED)):
            target.write_text(source, encoding="utf-8")
            command = ["cargo", "check", "--offline", "--locked",
                       "--manifest-path", str(cargo), "--quiet"]
            proc = subprocess.run(command, cwd=root, capture_output=True, text=True,
                                  timeout=90)
            events.append({"name": name, "compile_pass": proc.returncode == 0,
                           "exit_code": proc.returncode,
                           "stderr_tail": proc.stderr[-1600:]})
    finally:
        target.write_text(BASE, encoding="utf-8")
    (results / "cargo-variants.json").write_text(
        json.dumps(events, indent=2), encoding="utf-8"
    )
    return events


def lsp_position(source, invocation):
    locations = [(line_number, line) for line_number, line
                 in enumerate(source.splitlines()) if invocation in line]
    if len(locations) != 1:
        raise ValueError("ambiguous golden callsite: " + invocation)
    line_number, line = locations[0]
    # ASCII-only fixture means LSP UTF-16/UTF-8 column positions coincide.
    return {"line": line_number, "character": line.index(invocation) + 1}


def verdict(source, checked_hash, resolved_targets, proposed_policy=None):
    """Provider-owned allowlist + canonical deny. Empty/ambiguous != PASS.

    proposed_policy deliberately has NO effect: agents cannot grant themselves
    permission by editing their own overlay or prompting the model to allow.
    """
    if checked_hash != source_hash(source):
        return {"status": "STALE", "reason": "source revision changed"}
    if len(resolved_targets) != 1:
        return {"status": "INCOMPLETE", "reason": "definition unresolved or ambiguous"}
    target = resolved_targets[0]
    if target in POLICY["deny_target_files"]:
        return {"status": "FAIL", "target": target, "rule_id": POLICY["policy_id"],
                "reason": "UI cannot depend directly on StorageInternal; use Core API"}
    if target in POLICY["allow_target_files"]:
        return {"status": "PASS", "target": target, "rule_id": POLICY["policy_id"]}
    return {"status": "INCOMPLETE", "target": target,
            "reason": "target not classified by golden policy"}


def start_lsp(root):
    client = LspClient(root)
    init = {
        "processId": os.getpid(),
        "clientInfo": {"name": "akula-harness-lab", "version": "0.1"},
        "rootUri": root.resolve().as_uri(), "rootPath": str(root.resolve()),
        "workspaceFolders": [{"uri": root.resolve().as_uri(), "name": "akula-harness-lab"}],
        "capabilities": {
            "workspace": {"configuration": True},
            "textDocument": {"definition": {"linkSupport": False}}
        },
        "initializationOptions": {
            "cargo": {"buildScripts": {"enable": False}},
            "procMacro": {"enable": False},
            "checkOnSave": False,
        }
    }
    response = client.request("initialize", init, timeout=150)
    client.notify("initialized", {})
    for name, body in {**FILES, "src/ui.rs": BASE}.items():
        if not name.endswith(".rs"):
            continue
        client.notify("textDocument/didOpen", {"textDocument": {
            "uri": (root / name).resolve().as_uri(),
            "languageId": "rust", "version": 1, "text": body
        }})
    return client, sorted((response or {}).get("capabilities", {}).keys())


def check_variant(client, root, name, text, version, invocation,
                  expected_target, expected_verdict, wait_seconds):
    if version > 1:
        client.notify("textDocument/didChange", {
            "textDocument": {"uri": (root / "src/ui.rs").resolve().as_uri(),
                             "version": version},
            "contentChanges": [{"text": text}]
        })
    golden = {"file": "src/ui.rs", "needle": invocation,
              "symbol": invocation.split("::")[-1].split("(")[0],
              "target": expected_target}
    targets, attempts = resolve_targets_with_readiness(
        client, root, golden, text, budget_seconds=wait_seconds,
        position_override=lsp_position(text, invocation)
    )
    result = verdict(text, source_hash(text), targets)
    return {
        "name": name, "document_version": version, "source_hash": source_hash(text),
        "expected_target": expected_target, "actual_targets": targets,
        "expected_verdict": expected_verdict, "verdict": result,
        "lsp_requests": attempts,
        "pass": targets == [expected_target] and result["status"] == expected_verdict
    }


def execute(root, results):
    results.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    make_fixture(root)
    compile_cases = cargo_compile_variants(root, results)
    report = {
        "fixture": "generated controlled Cargo workspace, no external dependencies",
        "scope": "one direct Rust callsite at a time (NOT complete call graph)",
        "policy_hash": POLICY_HASH,
        "compiler_checks": compile_cases,
        "cases": [],
        "negative_checks": [],
        "trusted_fixture_only": True,
    }
    cli = None
    try:
        cli, capabilities = start_lsp(root)
        report["lsp_capabilities"] = capabilities
        for case in (
            ("baseline", BASE, 1, "crate::core::save();", "src/core.rs", "PASS", 100),
            ("forbidden_patch", BAD, 2, "crate::storage::internal_save();",
             "src/storage.rs", "FAIL", 55),
            ("repaired_patch", REPAIRED, 3, "crate::core::save_with_receipt();",
             "src/core.rs", "PASS", 55),
        ):
            report["cases"].append(check_variant(cli, root, *case))
        # No agent proposal can override the canonical rule; it is not a
        # replacement for OS-level File Gate permissions.
        fake_approval = {"deny_target_files": [], "allow_target_files": ["src/storage.rs"]}
        report["negative_checks"] = [
            {"name": "empty evidence is not PASS",
             "pass": verdict(BASE, source_hash(BASE), [])["status"] == "INCOMPLETE"},
            {"name": "stale source is not PASS",
             "pass": verdict(BASE, source_hash(BAD), ["src/core.rs"])["status"] == "STALE"},
            {"name": "agent cannot change canonical policy",
             "pass": verdict(BAD, source_hash(BAD), ["src/storage.rs"],
                             proposed_policy=fake_approval)["status"] == "FAIL"},
            {"name": "disk was not changed by LSP overlay",
             "pass": (root / "src/ui.rs").read_text(encoding="utf-8") == BASE}
        ]
    except Exception as exc:
        report["unexpected_error"] = repr(exc)
    finally:
        if cli is not None:
            cli.close()
        report["elapsed_seconds"] = round(time.monotonic() - started, 3)
        report["rust_analyzer_maxrss_kib"] = resource.getrusage(
            resource.RUSAGE_CHILDREN).ru_maxrss
        report["status"] = "PASS" if (
            len(report["cases"]) == 3 and all(c["pass"] for c in report["cases"])
            and len(report["negative_checks"]) == 4
            and all(c["pass"] for c in report["negative_checks"])
            and all(c["compile_pass"] for c in compile_cases)
            and "unexpected_error" not in report
        ) else "FAIL"
        (results / "harness-report.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        print(json.dumps(report, ensure_ascii=False, indent=2))
    if report["status"] != "PASS":
        raise SystemExit("FAIL: no green CI without exact LSP targets, policy, and overlay checks")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--fixture", type=Path, required=True)
    p.add_argument("--results", type=Path, required=True)
    args = p.parse_args()
    execute(args.fixture, args.results)


if __name__ == "__main__":
    main()
