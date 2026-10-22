#!/usr/bin/env python3
"""Pinned, dependency-free Rust call-target oracle for Akula IDE graph lab.

This file lives only on a research branch. It never modifies Akula source.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import resource
import subprocess
import sys
import threading
import time
from urllib.parse import urlparse, unquote

FILES = {
    "Cargo.toml": """[package]
name = "akula-ide-graph-fixture"
version = "0.1.0"
edition = "2021"

[features]
extra = []
""",
    "src/lib.rs": """pub mod storage;
pub mod domain;
pub mod misc;
pub mod ui;

pub use domain::api as exposed_api;

pub fn root_entry() {
    crate::ui::render();
}
""",
    "src/domain.rs": """pub fn api() {
    crate::storage::public_api();
}

pub struct Service;
impl Service {
    pub fn run(&self) {
        api();
    }
    pub fn generic<T>(&self, input: T) -> T {
        input
    }
}
""",
    "src/storage.rs": """pub fn public_api() {}
pub(crate) fn internal_api() {}
pub fn same() {}
""",
    "src/misc.rs": """pub fn same() {}
""",
    "src/ui.rs": """use crate::domain::{api, Service};
use crate::storage::public_api as direct_storage;

pub fn render() {
    api();
    direct_storage();
    crate::misc::same();
    let service = Service;
    service.run();
    crate::storage::internal_api();
    crate::storage::same();
    crate::exposed_api();
    service.generic::<i32>(1);
}
pub fn same() {}
""",
}

# Golden call-sites and source definitions. Each needle must be unique
# in its file. Source ranges and target files are independently checked
# using rust-analyzer textDocument/definition.
CASES = [
    dict(file="src/lib.rs", needle="crate::ui::render();", symbol="render", target="src/ui.rs"),
    dict(file="src/domain.rs", needle="crate::storage::public_api();", symbol="public_api", target="src/storage.rs"),
    dict(file="src/domain.rs", needle="        api();", symbol="api", target="src/domain.rs"),
    dict(file="src/ui.rs", needle="    api();", symbol="api", target="src/domain.rs"),
    dict(file="src/ui.rs", needle="    direct_storage();", symbol="direct_storage", target="src/storage.rs"),
    dict(file="src/ui.rs", needle="    crate::misc::same();", symbol="same", target="src/misc.rs"),
    dict(file="src/ui.rs", needle="    service.run();", symbol="run", target="src/domain.rs"),
    dict(file="src/ui.rs", needle="    crate::storage::internal_api();", symbol="internal_api", target="src/storage.rs"),
    dict(file="src/ui.rs", needle="    crate::storage::same();", symbol="same", target="src/storage.rs"),
    dict(file="src/ui.rs", needle="    crate::exposed_api();", symbol="exposed_api", target="src/domain.rs"),
    dict(file="src/ui.rs", needle="    service.generic::<i32>(1);", symbol="generic", target="src/domain.rs"),
]
RUST_FILES = [p for p in FILES if p.endswith(".rs")]


def make_fixture(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, content in FILES.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    (root / "oracle-input.json").write_text(json.dumps(CASES, indent=2), encoding="utf-8")
    for case in CASES:
        content = FILES[case["file"]]
        assert content.count(case["needle"]) == 1, (case, "nonunique needle")
        line = next(x for x in content.splitlines() if case["needle"] in x)
        assert case["symbol"] in line, case
        assert case["target"] in FILES, case
    print(json.dumps({"fixture": str(root), "files": len(FILES), "golden_sites": len(CASES)}))


class LspClient:
    def __init__(self, root):
        self.root = root
        self.proc = subprocess.Popen(["rust-analyzer"], cwd=str(root),
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, bufsize=0)
        self.q = queue.Queue()
        self.request_id = 0
        self.lock = threading.Lock()
        self.stderr = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._drain_err, daemon=True).start()

    def _drain_err(self):
        data = self.proc.stderr.read()
        self.stderr.append(data.decode("utf-8", errors="replace")[-10000:])

    def _read(self):
        try:
            while True:
                headers = {}
                while True:
                    raw = self.proc.stdout.readline()
                    if not raw:
                        raise EOFError("rust-analyzer stdout closed")
                    if raw in (b"\r\n", b"\n"):
                        break
                    line = raw.decode("ascii", "replace").strip()
                    if ":" in line:
                        k, val = line.split(":", 1)
                        headers[k.lower()] = val.strip()
                count = int(headers["content-length"])
                parts = []
                remaining = count
                while remaining:
                    chunk = self.proc.stdout.read(remaining)
                    if not chunk:
                        raise EOFError("short LSP packet")
                    parts.append(chunk)
                    remaining -= len(chunk)
                body = b"".join(parts)
                self.q.put(json.loads(body))
        except Exception as exc:
            self.q.put({"_reader_error": str(exc)})

    def send(self, obj):
        raw = json.dumps(obj, separators=(",", ":")).encode()
        with self.lock:
            self.proc.stdin.write(f"Content-Length: {len(raw)}\r\n\r\n".encode() + raw)
            self.proc.stdin.flush()

    def notify(self, method, params):
        self.send({"jsonrpc":"2.0", "method":method, "params":params})

    def request(self, method, params, timeout=170):
        self.request_id += 1
        reqid = self.request_id
        self.send({"jsonrpc":"2.0", "id":reqid, "method":method, "params":params})
        end = time.monotonic() + timeout
        while True:
            remaining = end - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{method} request timed out")
            try:
                msg = self.q.get(timeout=remaining)
            except queue.Empty:
                raise TimeoutError(f"{method} request timed out")
            if "_reader_error" in msg:
                raise RuntimeError(msg["_reader_error"])
            if "id" in msg and "method" in msg:
                # Servers can ask for configuration or register capabilities.
                if msg["method"] == "workspace/configuration":
                    value = [None] * len(msg.get("params", {}).get("items", []))
                else:
                    value = None
                self.send({"jsonrpc":"2.0", "id":msg["id"], "result":value})
                continue
            if msg.get("id") == reqid:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result")

    def close(self):
        try:
            self.request("shutdown", None, timeout=20)
            self.notify("exit", None)
        except Exception:
            pass
        try:
            self.proc.wait(timeout=6)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=6)


def location(case, text):
    lines = text.splitlines()
    matches = [(i, line) for i, line in enumerate(lines) if case["needle"] in line]
    if len(matches) != 1:
        raise ValueError(f"needle count {len(matches)}: {case}")
    i, line = matches[0]
    at = line.index(case["symbol"])
    return {"line":i, "character":at + 1}


def paths_of_result(result, root):
    if result is None:
        return []
    items = result if isinstance(result, list) else [result]
    paths = []
    for item in items:
        uri = item.get("targetUri") or item.get("uri")
        if not uri:
            continue
        try:
            target = Path(unquote(urlparse(uri).path)).resolve().relative_to(root.resolve())
            paths.append(str(target).replace(os.sep, "/"))
        except (ValueError, OSError):
            paths.append(uri)
    return sorted(set(paths))


def resolve_targets_with_readiness(cli, root, case, source, budget_seconds):
    """Wait for asynchronous workspace load before trusting a definition result.

    rust-analyzer may return None immediately after initialized/didOpen while
    Cargo metadata and the semantic database are still loading. This retries
    a concrete, independently annotated target and reports a failure rather
    than turning missing evidence into a passing (empty) graph.
    """
    deadline = time.monotonic() + budget_seconds
    attempts = 0
    latest = []
    while True:
        attempts += 1
        response = cli.request("textDocument/definition", {
            "textDocument": {"uri": (root / case["file"]).resolve().as_uri()},
            "position": location(case, source)
        }, timeout=min(30, max(5, int(deadline - time.monotonic()) + 1)))
        latest = paths_of_result(response, root)
        if latest == [case["target"]]:
            return latest, attempts
        if time.monotonic() >= deadline:
            return latest, attempts
        time.sleep(0.4)


def lsp_oracle(root, result_dir):
    start = time.monotonic()
    cli = LspClient(root)
    results = []
    diagnostics = []
    try:
        init = {
            "processId": os.getpid(),
            "clientInfo": {"name":"akula-ide-graph-lab","version":"0.1"},
            "rootUri":root.resolve().as_uri(),
            "rootPath":str(root.resolve()),
            "workspaceFolders":[{"uri":root.resolve().as_uri(),"name":"akula-ide-graph-fixture"}],
            "capabilities":{
                "workspace":{"configuration":True},
                "textDocument":{"definition":{"linkSupport":False},
                                "callHierarchy":{"dynamicRegistration":False}}
            },
            "initializationOptions":{
                "cargo":{"buildScripts":{"enable":False}},
                "procMacro":{"enable":False}
            }
        }
        init_result = cli.request("initialize", init, timeout=180)
        cli.notify("initialized", {})
        for name in RUST_FILES:
            cli.notify("textDocument/didOpen", {
                "textDocument":{
                    "uri":(root/name).resolve().as_uri(),
                    "languageId":"rust", "version":1, "text":FILES[name]
                }
            })
        for index, case in enumerate(CASES):
            targets, attempts = resolve_targets_with_readiness(
                cli, root, case, FILES[case["file"]],
                budget_seconds=95 if index == 0 else 12)
            results.append({
                **case,
                "targets":targets,
                "attempts":attempts,
                "matched_expected":targets == [case["target"]]
            })
        # Unsaved-buffer correctness: the same call site must retarget
        # from misc::same to storage::same without any disk write.
        new_ui = FILES["src/ui.rs"].replace("crate::misc::same();",
                                            "crate::storage::same();", 1)
        cli.notify("textDocument/didChange", {
            "textDocument":{"uri":(root/"src/ui.rs").resolve().as_uri(),"version":2},
            "contentChanges":[{"text":new_ui}]
        })
        modified = dict(file="src/ui.rs", needle="    crate::storage::same();",
                        symbol="same", target="src/storage.rs")
        # Choose the FIRST occurrence: it used to call misc::same.
        # The second occurrence already called storage::same before didChange,
        # so testing the second one would be a false positive.
        dirty_targets, dirty_attempts = resolve_targets_with_readiness(
            cli, root, modified, new_ui, budget_seconds=30)
        disk_unchanged = (root / "src/ui.rs").read_text(encoding="utf-8") == FILES["src/ui.rs"]
        overlay = {"expected":"src/storage.rs",
                   "targets":dirty_targets,
                   "attempts":dirty_attempts,
                   "disk_unchanged":disk_unchanged}
        overlay["matched_expected"] = dirty_targets == [overlay["expected"]] and disk_unchanged
        data = {"initialized":True, "server_capabilities":sorted(list((init_result or {}).get("capabilities", {}))),
                "cases":results, "matched":sum(x["matched_expected"] for x in results),
                "total":len(results), "dirty_buffer_retarget":overlay}
    except Exception as exc:
        data = {"initialized":False, "error":str(exc), "cases":results}
        raise
    finally:
        cli.close()
        data = locals().get("data", {"error":"failed before result assembly","cases":results})
        data["elapsed_seconds"] = round(time.monotonic()-start,3)
        data["ra_child_ru_maxrss_kib"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        data["ra_stderr_tail"] = "\n".join(cli.stderr)[-4000:]
        (result_dir/"ra-lsp-oracle.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
        print(json.dumps({k:data.get(k) for k in ("initialized","matched","total","elapsed_seconds",
                                                   "ra_child_ru_maxrss_kib","dirty_buffer_retarget","error")}, indent=2))
    # A green Actions job MUST mean every annotated definition and the dirty
    # buffer overlay passed. The saved JSON is still uploaded on failure.
    if data.get("matched") != len(CASES) or not data.get("dirty_buffer_retarget", {}).get("matched_expected"):
        raise SystemExit("FAIL: Rust LSP golden call targets or dirty-buffer overlay are not verified")


def evaluate(result_dir):
    oracle_path = result_dir/"ra-lsp-oracle.json"
    oracle = json.loads(oracle_path.read_text()) if oracle_path.exists() else {}
    gortex = {}
    for f in sorted(result_dir.glob("gortex-*.json")):
        try:
            data=json.loads(f.read_text())
            gortex[f.stem]={"result_type":type(data).__name__,
                            "nodes":len(data.get("nodes",[])) if isinstance(data,dict) else None,
                            "edges":len(data.get("edges",[])) if isinstance(data,dict) else None,
                            "results":len(data.get("results",[])) if isinstance(data,dict) else None,
                            "error":data.get("error") if isinstance(data,dict) else None}
        except Exception as e:
            gortex[f.stem]={"parse_error":str(e)}
    summary={"lsp_case_matches":oracle.get("matched"),
             "lsp_case_total":oracle.get("total"),
             "dirty_buffer_retarget":oracle.get("dirty_buffer_retarget"),
             "gortex_query_shapes":gortex,
             "note":"Gortex query nodes are candidate call-graph projections, not an independently verified precision/recall score. Inspect raw edges and matching source ranges before CI gating."}
    (result_dir/"edge-evaluation.json").write_text(json.dumps(summary,indent=2))
    print(json.dumps(summary,indent=2))


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("operation",choices=["make-fixture","lsp-oracle","evaluate"])
    parser.add_argument("--fixture",type=Path,required=True)
    parser.add_argument("--results",type=Path,required=True)
    args=parser.parse_args()
    args.results.mkdir(parents=True,exist_ok=True)
    if args.operation=="make-fixture":
        make_fixture(args.fixture)
    elif args.operation=="lsp-oracle":
        lsp_oracle(args.fixture,args.results)
    else:
        evaluate(args.results)


if __name__=="__main__":
    main()
