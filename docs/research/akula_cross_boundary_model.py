#!/usr/bin/env python3
"""AKCB-01: dependency-free, bounded state exploration of one cross-boundary write.

This is an ABSTRACT model, not a model of executable Akula admission logic.
Two actors (a receiver and a concurrent editor), one document, one disclosure,
one revocation and one crash are sufficient to expose several unsafe orders.
Usage: python3 akula_cross_boundary_model.py
"""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, replace
import json


@dataclass(frozen=True)
class State:
    stage: int = 0  # 0 idle, 1 received, 2 verified, 3 claimed, 4 filegate,
                    # 5 history, 6 journal, 7 applied, 8 acknowledged
    policy_epoch: int = 0
    grant_active: bool = True
    writer_epoch: int = 0
    expected_revision: int = 0
    current_revision: int = 0
    committed_revision: int = 0
    confirmed_baseline: int = 0
    verified: bool = False
    claimed: bool = False
    filegate_committed: bool = False
    history_durable: bool = False
    journal_durable: bool = False
    applied: bool = False
    ack_sent: bool = False
    publication_fenced: bool = True
    original_retained: bool = True
    hold: bool = False
    crashed: bool = False
    editor_committed: bool = False
    read_phase: int = 0  # 0 not read, 1 buffered, 2 disclosed, 3 discarded
    read_epoch: int = 0
    disclosure_authorized: bool = False
    blind_retry: bool = False


def write_authorized(s: State) -> bool:
    return (s.grant_active and s.writer_epoch == s.policy_epoch
            and s.expected_revision == s.current_revision)


def transitions(s: State) -> list[tuple[str, State]]:
    out: list[tuple[str, State]] = []
    def add(name: str, **change: object) -> None:
        out.append((name, replace(s, **change)))

    if s.stage == 0 and s.grant_active:
        add("receive", stage=1, writer_epoch=s.policy_epoch,
            expected_revision=s.current_revision)
    if s.stage == 1 and not s.hold:
        add("verify_bytes", stage=2, verified=True)
    if s.stage == 2 and not s.hold and write_authorized(s):
        add("durable_claim", stage=3, claimed=True)
    if s.stage == 3 and not s.hold and write_authorized(s):
        add("filegate_commit", stage=4, filegate_committed=True,
            current_revision=s.current_revision + 1,
            committed_revision=s.current_revision + 1,
            publication_fenced=True)
    if s.stage in (2, 3) and not s.hold and not write_authorized(s):
        add("hold_stale_or_revoked", hold=True)
    if s.stage == 4 and not s.hold:
        add("persist_history", stage=5, history_durable=True)
    if s.stage == 5 and not s.hold:
        add("persist_journal", stage=6, journal_durable=True)
    if s.stage == 6 and not s.hold:
        add("mark_applied", stage=7, applied=True,
            confirmed_baseline=s.committed_revision)
    if s.stage == 7 and not s.hold and write_authorized_after_commit(s):
        add("ack_applied", stage=8, ack_sent=True)
    if s.stage == 7 and not s.hold and not write_authorized_after_commit(s):
        add("hold_ack_on_revocation", hold=True)

    if s.policy_epoch == 0:
        add("revoke", policy_epoch=1, grant_active=False)
    if not s.editor_committed and s.stage < 4 and not s.hold:
        add("concurrent_editor_commit", current_revision=s.current_revision + 1,
            editor_committed=True)
    if s.stage in range(1, 8) and not s.hold and not s.crashed:
        # Cold start cannot infer a complete transaction from a partial journal.
        add("crash_to_hold", crashed=True, hold=True)

    if s.read_phase == 0 and s.grant_active:
        add("read_to_private_buffer", read_phase=1, read_epoch=s.policy_epoch)
    if s.read_phase == 1 and s.grant_active and s.read_epoch == s.policy_epoch:
        add("disclose_after_revalidation", read_phase=2,
            disclosure_authorized=True)
    if s.read_phase == 1 and (not s.grant_active or s.read_epoch != s.policy_epoch):
        add("discard_stale_buffer", read_phase=3)
    return out


def write_authorized_after_commit(s: State) -> bool:
    # The publication changed current_revision; recheck epoch and live grant,
    # not the now-obsolete expected_revision.
    return s.grant_active and s.writer_epoch == s.policy_epoch


def violations(s: State) -> list[str]:
    bad: list[str] = []
    if s.claimed and not s.verified:
        bad.append("claim_before_verified_bytes")
    if s.filegate_committed and not (s.claimed and s.verified
                                      and s.publication_fenced):
        bad.append("unfenced_or_unverified_publication")
    if s.history_durable and not s.filegate_committed:
        bad.append("history_without_filegate")
    if s.journal_durable and not s.history_durable:
        bad.append("journal_without_history")
    if s.applied and not (s.filegate_committed and s.history_durable
                           and s.journal_durable and s.original_retained):
        bad.append("applied_without_durable_chain")
    if s.applied and s.confirmed_baseline != s.committed_revision:
        bad.append("false_baseline")
    if s.confirmed_baseline != 0 and not s.applied:
        bad.append("premature_baseline")
    if s.ack_sent and not (s.applied and s.verified and s.claimed
                           and s.filegate_committed and s.history_durable
                           and s.journal_durable and s.original_retained):
        bad.append("premature_applied_ack")
    if s.read_phase == 2 and not s.disclosure_authorized:
        bad.append("unauthorized_disclosure")
    if s.blind_retry:
        bad.append("blind_replay_after_unknown_outcome")
    if not s.original_retained and (s.hold or not s.ack_sent):
        bad.append("original_lost_before_proven_completion")
    return bad


def explore(max_states: int = 100_000) -> tuple[dict[State, tuple[str, ...]], int]:
    start = State()
    traces: dict[State, tuple[str, ...]] = {start: ()}
    queue = deque([start])
    edges = 0
    while queue:
        current = queue.popleft()
        assert not violations(current), (traces[current], violations(current))
        for label, nxt in transitions(current):
            edges += 1
            if nxt not in traces:
                if len(traces) >= max_states:
                    raise AssertionError("exhaustive bound exceeded; do not claim PASS")
                traces[nxt] = traces[current] + (label,)
                queue.append(nxt)
    return traces, edges


def find_trace(traces: dict[State, tuple[str, ...]], prefix: tuple[str, ...]) -> list[str]:
    candidates = [t for t in traces.values() if t[:len(prefix)] == prefix]
    if not candidates:
        raise AssertionError("expected distinguishing trace absent: " + repr(prefix))
    return list(min(candidates, key=len))


def check() -> dict[str, object]:
    traces, edges = explore()
    seqs = tuple(traces.values())
    def has_subsequence(sequence: tuple[str, ...]) -> list[str]:
        candidates = [t for t in seqs if all(x in t for x in sequence)
                      and [t.index(x) for x in sequence] == sorted(t.index(x) for x in sequence)]
        if not candidates:
            raise AssertionError("missing required witness: " + repr(sequence))
        return list(min(candidates, key=len))

    required = {
        "normal_ack": ("receive", "verify_bytes", "durable_claim", "filegate_commit",
                       "persist_history", "persist_journal", "mark_applied", "ack_applied"),
        "stale_writer": ("receive", "verify_bytes", "durable_claim",
                         "concurrent_editor_commit", "hold_stale_or_revoked"),
        "revocation_before_disclose": ("read_to_private_buffer", "revoke", "discard_stale_buffer"),
        "crash_after_filegate": ("receive", "verify_bytes", "durable_claim",
                                 "filegate_commit", "crash_to_hold"),
        "crash_after_applied_before_ack": ("mark_applied", "crash_to_hold"),
        "revocation_before_publish": ("durable_claim", "revoke", "hold_stale_or_revoked"),
    }
    witnesses = {name: has_subsequence(t) for name, t in required.items()}

    # Negative controls demonstrate that the oracle is not a vacuous checker.
    mutants = {
        "ack_before_commit": replace(State(), ack_sent=True),
        "publication_without_claim": replace(State(), filegate_committed=True),
        "stale_publication_fence": replace(State(), verified=True, claimed=True,
                                            filegate_committed=True, publication_fenced=False),
        "false_baseline": replace(State(), confirmed_baseline=1),
        "post_revocation_disclosure": replace(State(), read_phase=2),
        "blind_retry": replace(State(), blind_retry=True),
        "lost_original_on_hold": replace(State(), original_retained=False, hold=True),
    }
    detected = {name: violations(s) for name, s in mutants.items()}
    if any(not flags for flags in detected.values()):
        raise AssertionError("one or more mutation controls escaped detection")
    return {
        "model": "AKCB-01 bounded abstract state space",
        "status": "PASS (model only, not production conformance)",
        "states": len(traces),
        "edges": edges,
        "distinct_actions": len({a for t in seqs for a in t}),
        "max_trace_depth": max(len(t) for t in seqs),
        "witnesses": witnesses,
        "mutants_rejected": detected,
        "scope": "two actors; one document; at most one revoke/crash/external edit/read",
    }


if __name__ == "__main__":
    print(json.dumps(check(), indent=2, ensure_ascii=False, sort_keys=True))
