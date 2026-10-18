#!/usr/bin/env python3
"""CI-only Vello 0.3.0 half-tie experiment. Does not alter committed sources.

Run *after* cargo test --locked has fetched the original registry dependency.
Copies that dependency into an untracked workspace directory, changes one
constant, and installs a local Cargo [patch.crates-io] for this CI process only.
Do not reuse as a production patch without separate regression acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import shutil


def main() -> None:
    cli = argparse.ArgumentParser()
    cli.add_argument(
        "--bias",
        default="0.5001",
        choices=["0.5001"],
        help="The previously verified positive half-tie offset in alpha byte units.",
    )
    args = cli.parse_args()
    workspace = Path(__file__).resolve().parents[2] / "aquilum-app"
    manifest = workspace / "Cargo.toml"
    src_base = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry/src"
    candidates = sorted(src_base.glob("*/vello_common-0.3.0"))
    if len(candidates) != 1:
        raise SystemExit(f"Expected precisely one Cargo registry Vello Common 0.3.0: {candidates}")
    dest = workspace / ".vello_half_tie_ci"
    if dest.exists():
        raise SystemExit(f"Refusing to overwrite existing CI override: {dest}")
    shutil.copytree(candidates[0], dest)
    target = dest / "src/strip.rs"
    source = target.read_text(encoding="utf-8")
    anchor = "Fill::NonZero => {\n                    let p1 = f32x4::splat(s, 0.5);"
    if source.count(anchor) != 1:
        raise SystemExit("Unsupported Vello Common strip.rs: exact NonZero quantizer missing")
    modified = source.replace(anchor, anchor.replace("0.5", args.bias), 1)
    if modified.count(f"let p1 = f32x4::splat(s, {args.bias});") != 1:
        raise SystemExit("Refusing ambiguous quantizer patch")
    target.write_text(modified, encoding="utf-8")

    original_manifest = manifest.read_text(encoding="utf-8")
    if original_manifest.count("[patch.crates-io]") != 1:
        raise SystemExit("Unexpected Cargo.toml patch section")
    if "vello_common = { path =" in original_manifest:
        raise SystemExit("Workspace already has a local vello_common patch")
    # Currently [patch.crates-io] is the last section. Enforce that assumption
    # to avoid accidentally inserting our entry under another TOML section.
    if "[" in original_manifest.split("[patch.crates-io]", 1)[1]:
        raise SystemExit("Cargo.toml patch section is not the final TOML section")
    manifest.write_text(
        original_manifest.rstrip() + '\nvello_common = { path = ".vello_half_tie_ci" }\n',
        encoding="utf-8",
    )
    digest = hashlib.sha256(source.encode("utf-8")).hexdigest()[:16]
    print(f"VELLO_HALF_TIE_OVERLAY source={candidates[0]} hash={digest}")
    print(f"VELLO_HALF_TIE_OVERLAY changed_only=Fill::NonZero alpha addend 0.5 -> {args.bias}")
    print(f"VELLO_HALF_TIE_OVERLAY destination={dest}")
    # Both Cargo.toml and the copied source remain local modifications only.


if __name__ == "__main__":
    main()
