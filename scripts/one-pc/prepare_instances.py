"""Prepare two synthetic Akula process profiles without touching user data."""

from __future__ import annotations

import json
from pathlib import Path
from uuid import uuid4


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTANCE_ROOT = REPO_ROOT / ".artifacts" / "one-pc-instances"
MARKER_NAME = ".akula-one-pc-instances"
MARKER_CONTENT = b"AKULA-ONE-PC-PLAN-01 isolated process profiles v1\n"
P2P_TARGETS_PATH = INSTANCE_ROOT / "p2p-test-targets.json"
P2P_TEST_PREFIX = "Lalalalendia/Akula/P2P/LocalHarness/"

PROFILES = (
    {
        "name": "A",
        "profile_id": "akula-one-pc-a",
        "port_hint": 48101,
        "note": "Synthetic workspace A. No personal data.",
    },
    {
        "name": "B",
        "profile_id": "akula-one-pc-b",
        "port_hint": 48102,
        "note": "Synthetic workspace B. No personal data.",
    },
)


def write_once(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_bytes() != data:
            raise RuntimeError(f"Refusing to replace different profile data: {path}")
        return
    path.write_bytes(data)


def ensure_p2p_test_targets() -> dict[str, str]:
    if P2P_TARGETS_PATH.exists():
        manifest = json.loads(P2P_TARGETS_PATH.read_text(encoding="utf-8"))
    else:
        session_id = uuid4().hex
        manifest = {
            "namespace_prefix": P2P_TEST_PREFIX,
            "session_id": session_id,
            "targets": {
                profile["name"]: f"{P2P_TEST_PREFIX}{session_id}/{profile['name']}"
                for profile in PROFILES
            },
            "private_key_material_in_manifest": False,
        }
        write_once(
            P2P_TARGETS_PATH,
            (json.dumps(manifest, indent=2) + "\n").encode("utf-8"),
        )

    targets = manifest.get("targets")
    expected_names = {profile["name"] for profile in PROFILES}
    if (
        manifest.get("namespace_prefix") != P2P_TEST_PREFIX
        or not isinstance(targets, dict)
        or set(targets) != expected_names
        or any(
            not isinstance(target, str)
            or not target.startswith(P2P_TEST_PREFIX)
            or len(target) > 256
            for target in targets.values()
        )
        or targets["A"] == targets["B"]
    ):
        raise RuntimeError(f"Invalid local P2P test-target manifest: {P2P_TARGETS_PATH}")
    return targets


def main() -> None:
    if INSTANCE_ROOT.exists() and not (INSTANCE_ROOT / MARKER_NAME).exists():
        raise RuntimeError(f"Refusing to use an unmarked directory: {INSTANCE_ROOT}")
    INSTANCE_ROOT.mkdir(parents=True, exist_ok=True)
    marker = INSTANCE_ROOT / MARKER_NAME
    if marker.exists() and marker.read_bytes() != MARKER_CONTENT:
        raise RuntimeError(f"Unexpected marker contents: {marker}")
    write_once(marker, MARKER_CONTENT)

    for profile in PROFILES:
        name = profile["name"]
        base = INSTANCE_ROOT / name
        appdata = base / "appdata"
        vault = base / "vault"
        appdata.mkdir(parents=True, exist_ok=True)
        vault.mkdir(parents=True, exist_ok=True)
        write_once(
            vault / "START-HERE.md",
            (profile["note"] + "\n").encode("utf-8"),
        )

        manifest = {
            "profile_id": profile["profile_id"],
            "app_data_dir": str(appdata.resolve()),
            "vault_root": str(vault.resolve()),
            "loopback_endpoint_hint": f"127.0.0.1:{profile['port_hint']}",
            "identity_status": "not_enrolled",
            "identity_note": (
                "Mainline P2P identity uses one fixed Windows Credential Manager "
                "target per logon session, not --data-dir. Do not enroll or replace "
                "the user's identity for this simulation. Use distinct ephemeral "
                "test keys in a process-level transport harness."
            ),
            "synthetic_only": True,
        }
        write_once(
            base / "profile.json",
            (json.dumps(manifest, indent=2) + "\n").encode("utf-8"),
        )

    p2p_targets = ensure_p2p_test_targets()

    a = json.loads((INSTANCE_ROOT / "A" / "profile.json").read_text(encoding="utf-8"))
    b = json.loads((INSTANCE_ROOT / "B" / "profile.json").read_text(encoding="utf-8"))
    if a["app_data_dir"] == b["app_data_dir"] or a["vault_root"] == b["vault_root"]:
        raise RuntimeError("The two local process profiles are not isolated")
    if a["loopback_endpoint_hint"] == b["loopback_endpoint_hint"]:
        raise RuntimeError("The two loopback endpoint hints must be distinct")

    print(
        json.dumps(
            {
                "instance_root": str(INSTANCE_ROOT),
                "profiles": [a["profile_id"], b["profile_id"]],
                "app_data_dirs_distinct": True,
                "vault_roots_distinct": True,
                "loopback_hints_distinct": True,
                "p2p_test_targets_distinct": p2p_targets["A"] != p2p_targets["B"],
                "p2p_target_namespace": P2P_TEST_PREFIX,
                "production_identity_keys_provisioned": False,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
