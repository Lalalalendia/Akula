#!/usr/bin/env python3
"""Correlate two opt-in B2 QA logs; never certify physical WAN delivery.

No dependencies or network calls. The source log is untrusted diagnostic text,
not a signed offer, cryptographic receipt, independent device witness or
power-loss durability claim. Never print raw log lines or content hashes.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys
import unittest
import uuid

MARKER = "AKULA_B2_QA_EVIDENCE "
MAX_BYTES = 8 * 1024 * 1024
MAX_LOG_BYTES = 8 * 1024 * 1024
FIELDS = frozenset({
    "phase", "bytes", "blake3", "inbox_receipt", "selected_quic_path_snapshot",
    "durable_ack", "signed_offer",
})
HEX = re.compile(r"[0-9a-f]{64}\Z")


class EvidenceRejected(ValueError):
    """A log fails a strict, local diagnostic-contract check."""


@dataclass(frozen=True)
class Evidence:
    phase: str
    size: int
    digest: str
    receipt: str
    route: str


def extract(log: str, expected_phase: str) -> Evidence:
    """Require exactly one matching B2 record in each independent log."""
    lines = [line.split(MARKER, 1)[1] for line in log.splitlines() if MARKER in line]
    if len(lines) != 1:
        raise EvidenceRejected("expected exactly one B2 diagnostic line per endpoint")
    if len(lines[0]) > 2048:
        raise EvidenceRejected("oversized B2 diagnostic line")
    pairs = {}
    for token in lines[0].split():
        if token.count("=") != 1:
            raise EvidenceRejected("malformed diagnostic field")
        key, value = token.split("=", 1)
        if key in pairs or not value:
            raise EvidenceRejected("duplicate or empty diagnostic field")
        pairs[key] = value
    if pairs.keys() != FIELDS:
        raise EvidenceRejected("missing or unexpected B2 diagnostic fields")
    if pairs["phase"] != expected_phase:
        raise EvidenceRejected("unexpected sender/receiver phase")
    if pairs["durable_ack"] != "false" or pairs["signed_offer"] != "false":
        raise EvidenceRejected("unsupported durable or signed delivery claim")
    raw_size = pairs["bytes"]
    if not raw_size.isascii() or not raw_size.isdecimal() or len(raw_size) > 8:
        raise EvidenceRejected("invalid byte count")
    size = int(raw_size)
    if str(size) != raw_size:
        raise EvidenceRejected("non-canonical byte count")
    if not 1 <= size <= MAX_BYTES:
        raise EvidenceRejected("byte count outside one-file beta contract")
    if not HEX.fullmatch(pairs["blake3"]):
        raise EvidenceRejected("invalid full lowercase BLAKE3")
    if pairs["selected_quic_path_snapshot"] not in {"direct", "relay", "unknown"}:
        raise EvidenceRejected("unrecognized QUIC path snapshot")
    receipt = pairs["inbox_receipt"]
    if expected_phase == "sender_verified_peer_ack":
        if receipt != "none":
            raise EvidenceRejected("sender must not assert a local receiver receipt")
    else:
        try:
            parsed = uuid.UUID(receipt)
        except (ValueError, AttributeError) as exc:
            raise EvidenceRejected("receiver receipt UUID missing or invalid") from exc
        if parsed.int == 0 or str(parsed) != receipt:
            raise EvidenceRejected("receiver receipt UUID is nil or not canonical")
    return Evidence(expected_phase, size, pairs["blake3"], receipt,
                    pairs["selected_quic_path_snapshot"])


def correlate(sender_log: str, receiver_log: str) -> str:
    """Return CORRELATED or HOLD; neither means actual WAN B2 PASS."""
    sender = extract(sender_log, "sender_verified_peer_ack")
    receiver = extract(receiver_log, "receiver_local_inbox_verified")
    if (sender.size, sender.digest) != (receiver.size, receiver.digest):
        raise EvidenceRejected("sender and receiver byte count/BLAKE3 disagree")
    if "unknown" in (sender.route, receiver.route):
        return "HOLD"
    return "CORRELATED"


def read_log(path: Path) -> str:
    if not path.is_file() or path.stat().st_size > MAX_LOG_BYTES:
        raise EvidenceRejected("log missing or exceeds local QA size limit")
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise EvidenceRejected("cannot read UTF-8 QA log") from exc


class ProofContractTests(unittest.TestCase):
    def proof(self, phase="sender_verified_peer_ack", **overrides):
        fields = {
            "phase": phase,
            "bytes": "42",
            "blake3": "ab" * 32,
            "inbox_receipt": "none" if phase == "sender_verified_peer_ack"
            else "05ec121e-0f1b-4a63-8c4a-5e11c32a1b41",
            "selected_quic_path_snapshot": "direct",
            "durable_ack": "false", "signed_offer": "false",
        }
        fields.update(overrides)
        return "2026-11-09 INFO " + MARKER + " ".join(f"{k}={v}" for k, v in fields.items())

    def test_valid_pair_is_only_correlated(self):
        self.assertEqual(correlate(self.proof(), self.proof("receiver_local_inbox_verified")), "CORRELATED")

    def test_unknown_route_stays_hold(self):
        self.assertEqual(correlate(self.proof(), self.proof("receiver_local_inbox_verified", selected_quic_path_snapshot="unknown")), "HOLD")

    def test_wrong_size(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(), self.proof("receiver_local_inbox_verified", bytes="43"))

    def test_wrong_hash(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(), self.proof("receiver_local_inbox_verified", blake3="cd" * 32))

    def test_missing_receipt(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(), self.proof("receiver_local_inbox_verified", inbox_receipt="none"))

    def test_replay_duplicate_line(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof() + "\n" + self.proof(), self.proof("receiver_local_inbox_verified"))

    def test_untrusted_extra_field(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof() + " leaked=1", self.proof("receiver_local_inbox_verified"))

    def test_no_false_signed_or_durable_claim(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(signed_offer="true"), self.proof("receiver_local_inbox_verified"))
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(), self.proof("receiver_local_inbox_verified", durable_ack="true"))

    def test_empty_oversized_and_invalid_hash(self):
        for count in ("0", "8388609", "01", "-4"):
            with self.subTest(count=count), self.assertRaises(EvidenceRejected):
                correlate(self.proof(bytes=count), self.proof("receiver_local_inbox_verified"))
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(blake3="AB" * 32), self.proof("receiver_local_inbox_verified"))

    def test_wrong_phase_and_nil_receipt(self):
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof("receiver_local_inbox_verified"), self.proof("receiver_local_inbox_verified"))
        with self.assertRaises(EvidenceRejected):
            correlate(self.proof(), self.proof("receiver_local_inbox_verified", inbox_receipt=str(uuid.UUID(int=0))))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sender", type=Path, help="sender QA console log")
    parser.add_argument("--receiver", type=Path, help="receiver QA console log")
    parser.add_argument("--self-test", action="store_true", help="run fail-closed contract tests")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=1).run(
            unittest.defaultTestLoader.loadTestsFromTestCase(ProofContractTests))
        return 0 if result.wasSuccessful() else 1
    if args.sender is None or args.receiver is None:
        parser.error("--sender and --receiver are required unless --self-test is set")
    try:
        if args.sender.samefile(args.receiver):
            raise EvidenceRejected("two independent log files are required")
        outcome = correlate(read_log(args.sender), read_log(args.receiver))
    except (EvidenceRejected, OSError) as exc:
        print(f"B2_CORRELATION=REJECTED PHYSICAL_WAN=UNVERIFIED: {exc}", file=sys.stderr)
        return 1
    if outcome == "HOLD":
        print("B2_CORRELATION=HOLD PHYSICAL_WAN=UNVERIFIED: unknown selected QUIC route")
        return 2
    print("B2_CORRELATION=MATCHED PHYSICAL_WAN=UNVERIFIED: manual two-PC witness and cold restart still required")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
