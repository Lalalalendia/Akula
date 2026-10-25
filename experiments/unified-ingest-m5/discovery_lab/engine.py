"""SQLite WAL discovery frontier + immutable raw receipts + offline fixtures.

No networking code. This package cannot crawl Literotica. Pages are generated
from synthetic, authorized fixtures only. S3Sink is an optional integration seam.
"""
from __future__ import annotations

import hashlib
import json
import sqlite3
import time
import uuid
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Mapping, Sequence

from .model import SchemaChanged, canonical_fixture_url, parse_fixture_page, parse_fixture_html
from .objects import FileObjectSink, ObjectConflict, ObjectWriteUncertain


@dataclass(frozen=True)
class FixtureResponse:
    status: int
    body: bytes = b""
    content_type: str = "application/json"
    retry_after: float | None = None


class MemoryFixtureSource:
    """No sockets: mapping {(run_id, page): FixtureResponse} with fallback (*, page)."""
    def __init__(self, pages: Mapping[tuple[str, int], FixtureResponse]) -> None:
        self.pages = dict(pages)
        self.calls: list[tuple[str, int]] = []

    def fetch(self, run_id: str, page: int) -> FixtureResponse:
        self.calls.append((run_id, page))
        return self.pages.get((run_id, page), self.pages.get(("*", page), FixtureResponse(503)))


class SimulatedCrash(RuntimeError):
    """Crash injection after uploading raw, before SQLite transaction."""


class StaleLease(RuntimeError):
    pass


class DiscoveryStore:
    def __init__(self, db_file: str | Path) -> None:
        self.path = Path(db_file)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self._conn() as db:
            db.executescript("""
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS runs (
                run_id TEXT PRIMARY KEY,
                partition_key TEXT NOT NULL,
                document_kind TEXT NOT NULL,
                seed_url TEXT NOT NULL,
                max_pages INTEGER NOT NULL,
                scope TEXT NOT NULL,
                started_at REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS jobs (
                job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES runs(run_id),
                page INTEGER NOT NULL CHECK(page>=1),
                state TEXT NOT NULL DEFAULT 'READY',
                attempt INTEGER NOT NULL DEFAULT 0,
                available_at REAL NOT NULL DEFAULT 0,
                lease_token TEXT,
                lease_until REAL NOT NULL DEFAULT 0,
                error TEXT,
                UNIQUE(run_id,page)
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_claim ON jobs (run_id,state,available_at,lease_until,page);
            CREATE TABLE IF NOT EXISTS snapshots (
                job_id INTEGER PRIMARY KEY REFERENCES jobs(job_id),
                run_id TEXT NOT NULL,
                page INTEGER NOT NULL,
                object_key TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                raw_size INTEGER NOT NULL,
                last_page INTEGER NOT NULL,
                declared_total INTEGER,
                received_at REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS entities (
                entity_key TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                display_title TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS observations (
                run_id TEXT NOT NULL REFERENCES runs(run_id),
                job_id INTEGER NOT NULL REFERENCES jobs(job_id),
                position INTEGER NOT NULL,
                entity_key TEXT NOT NULL REFERENCES entities(entity_key),
                PRIMARY KEY(run_id,job_id,position)
            );
            CREATE INDEX IF NOT EXISTS idx_obs_entity ON observations(entity_key,run_id);
            CREATE TABLE IF NOT EXISTS relationships (
                run_id TEXT NOT NULL REFERENCES runs(run_id),
                parent_key TEXT NOT NULL,
                child_key TEXT NOT NULL,
                relation_kind TEXT NOT NULL,
                ordinal INTEGER NOT NULL DEFAULT 0,
                job_id INTEGER NOT NULL REFERENCES jobs(job_id),
                UNIQUE(run_id,parent_key,child_key,relation_kind,ordinal)
            );
            CREATE TABLE IF NOT EXISTS outbox (
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES runs(run_id),
                entity_key TEXT NOT NULL,
                event_type TEXT NOT NULL,
                created_at REAL NOT NULL,
                UNIQUE(run_id,entity_key,event_type)
            );
            """)

    @contextmanager
    def _conn(self) -> Iterator[sqlite3.Connection]:
        conn = sqlite3.connect(str(self.path), isolation_level=None, timeout=10.0)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA foreign_keys=ON")
        try:
            yield conn
        finally:
            conn.close()

    @contextmanager
    def _tx(self) -> Iterator[sqlite3.Connection]:
        with self._conn() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                yield db
                db.execute("COMMIT")
            except BaseException:
                db.execute("ROLLBACK")
                raise

    def register(self, partition_key: str, document_kind: str, seed_url: str, *, max_pages: int = 100,
                 scope: str = "selected_partition", when: float | None = None) -> str:
        if document_kind not in ("author_catalog", "category_alpha", "series_index", "recent_window"):
            raise ValueError("invalid discovery document kind")
        if not 1 <= max_pages <= 5000 or not partition_key or not scope:
            raise ValueError("invalid run scope/bounds")
        canonical = canonical_fixture_url(seed_url)
        run_id = uuid.uuid4().hex
        with self._tx() as db:
            db.execute("INSERT INTO runs VALUES(?,?,?,?,?,?,?)",
                       (run_id, partition_key, document_kind, canonical, max_pages, scope,
                        time.time() if when is None else when))
            db.execute("INSERT INTO jobs(run_id,page) VALUES(?,1)", (run_id,))
        return run_id

    def run_details(self, run_id: str) -> dict:
        with self._conn() as db:
            row = db.execute("SELECT * FROM runs WHERE run_id=?", (run_id,)).fetchone()
            if row is None:
                raise KeyError(run_id)
            return dict(row)

    def claim(self, run_id: str, now: float, *, lease_seconds: float = 60.0) -> dict | None:
        if lease_seconds <= 0:
            raise ValueError("invalid lease")
        with self._tx() as db:
            job = db.execute("""SELECT * FROM jobs WHERE run_id=? AND
                             ((state='READY' AND available_at<=?) OR
                              (state='LEASED' AND lease_until<=?))
                             ORDER BY page LIMIT 1""", (run_id, now, now)).fetchone()
            if job is None:
                return None
            token = uuid.uuid4().hex
            db.execute("""UPDATE jobs SET state='LEASED',lease_token=?,lease_until=?,attempt=attempt+1,error=NULL
                          WHERE job_id=?""", (token, now + lease_seconds, job["job_id"]))
            claim = dict(job)
            claim.update(lease_token=token, attempt=job["attempt"] + 1, lease_until=now+lease_seconds)
            return claim

    def _verify_claim(self, db: sqlite3.Connection, job_id: int, lease_token: str, now: float) -> sqlite3.Row:
        job = db.execute("SELECT * FROM jobs WHERE job_id=?", (job_id,)).fetchone()
        if job is None or job["state"] != "LEASED" or (job["lease_token"] != lease_token or job["lease_until"] <= now):
            raise StaleLease("obsolete worker cannot update claimed job")
        return job

    def complete(self, claim: dict, *, key: str, digest: str, raw_size: int, page_data,
                 now: float) -> None:
        """Atomic snapshot, observations, relationships, outbox and next-page job."""
        with self._tx() as db:
            job = self._verify_claim(db, claim["job_id"], claim["lease_token"], now)
            run = db.execute("SELECT * FROM runs WHERE run_id=?", (job["run_id"],)).fetchone()
            if page_data.document_kind != run["document_kind"] or page_data.current_page != job["page"]:
                raise SchemaChanged("page type/ordinal changed before completion")
            db.execute("""INSERT INTO snapshots (job_id,run_id,page,object_key,sha256,raw_size,last_page,declared_total,received_at)
                          VALUES(?,?,?,?,?,?,?,?,?)""", (job["job_id"], job["run_id"], job["page"], key,
                                                             digest, raw_size, page_data.last_page,
                                                             page_data.declared_total, now))
            for pos, item in enumerate(page_data.items, 1):
                db.execute("INSERT OR IGNORE INTO entities(entity_key,kind,source_id,display_title) VALUES(?,?,?,?)",
                           (item.key, item.kind, item.entity_id, item.title))
                db.execute("INSERT INTO observations(run_id,job_id,position,entity_key) VALUES(?,?,?,?)",
                           (job["run_id"], job["job_id"], pos, item.key))
                if item.kind in ("work", "series", "chapter", "author"):
                    db.execute("INSERT OR IGNORE INTO outbox(run_id,entity_key,event_type,created_at) VALUES(?,?,?,?)",
                               (job["run_id"], item.key, "ENTITY_DISCOVERED", now))
                for rel in item.relations:
                    db.execute("""INSERT OR IGNORE INTO relationships
                               (run_id,parent_key,child_key,relation_kind,ordinal,job_id)
                               VALUES(?,?,?,?,?,?)""",
                               (job["run_id"], f"{rel.parent_kind}:{rel.parent_id}", item.key,
                                rel.rel, rel.ordinal or 0, job["job_id"]))
            db.execute("""UPDATE jobs SET state='DONE',lease_token=NULL,lease_until=0,error=NULL
                          WHERE job_id=?""", (job["job_id"],))
            if job["page"] < page_data.last_page:
                db.execute("INSERT OR IGNORE INTO jobs(run_id,page) VALUES(?,?)",
                           (job["run_id"], job["page"] + 1))

    def record_problem(self, claim: dict, reason: str, *, status: str, now: float,
                       retry_after: float | None = None, max_attempts: int = 4) -> None:
        with self._tx() as db:
            job = self._verify_claim(db, claim["job_id"], claim["lease_token"], now)
            if status == "RETRY" and job["attempt"] < max_attempts:
                # Deterministic, bounded backoff for tests. Production adds jitter.
                delay = max(1.0, min(3600.0, retry_after if retry_after is not None else (2**job["attempt"])))
                db.execute("""UPDATE jobs SET state='READY',available_at=?,lease_token=NULL,lease_until=0,error=?
                              WHERE job_id=?""", (now+delay, reason, job["job_id"]))
            else:
                final = status if status in ("BLOCKED", "MISSING_UPSTREAM", "NEEDS_REVIEW") else "NEEDS_REVIEW"
                db.execute("""UPDATE jobs SET state=?,lease_token=NULL,lease_until=0,error=? WHERE job_id=?""",
                           (final, reason, job["job_id"]))

    def report(self, run_id: str) -> dict:
        info = self.run_details(run_id)
        with self._conn() as db:
            jobs = [dict(r) for r in db.execute("SELECT * FROM jobs WHERE run_id=? ORDER BY page", (run_id,))]
            raw = [dict(r) for r in db.execute("SELECT * FROM snapshots WHERE run_id=? ORDER BY page", (run_id,))]
            observations = [r[0] for r in db.execute("SELECT entity_key FROM observations WHERE run_id=? ORDER BY job_id,position", (run_id,))]
            rel_count = db.execute("SELECT COUNT(*) FROM relationships WHERE run_id=?", (run_id,)).fetchone()[0]
            event_count = db.execute("SELECT COUNT(*) FROM outbox WHERE run_id=?", (run_id,)).fetchone()[0]
        lastpages = {r["last_page"] for r in raw}
        totals = {r["declared_total"] for r in raw if r["declared_total"] is not None}
        declared_missing = any(r["declared_total"] is None for r in raw)
        expected_pages = max(lastpages) if lastpages else None
        actual_pages = {r["page"] for r in raw}
        missing_pages = list(sorted(set(range(1, expected_pages + 1)) - actual_pages)) if expected_pages else []
        duplicates = len(observations) - len(set(observations))
        observed_unique = len(set(observations))
        error_states = {x["state"] for x in jobs}
        if "BLOCKED" in error_states:
            status = "BLOCKED"
        elif "MISSING_UPSTREAM" in error_states:
            status = "MISSING_UPSTREAM"
        elif "NEEDS_REVIEW" in error_states:
            status = "NEEDS_REVIEW"
        elif error_states.intersection({"READY", "LEASED"}):
            status = "ACTIVE"
        elif len(lastpages) > 1 or len(totals) > 1 or duplicates > 0:
            status = "DRIFT"
        elif missing_pages or (not declared_missing and totals and observed_unique != next(iter(totals))):
            status = "PARTIAL"
        elif not raw:
            status = "NEEDS_REVIEW"
        elif declared_missing or not totals:
            status = "OBSERVED_UNKNOWN_TOTAL"
        else:
            status = "MATCHED_DECLARED"
        return {
            "run_id": run_id, "partition": info["partition_key"], "scope": info["scope"],
            "status": status, "pages_fetched": len(raw), "pages_expected": expected_pages,
            "missing_pages": missing_pages, "items_observed": len(observations),
            "entities_unique": observed_unique, "duplicate_within_partition": duplicates,
            "declared_totals_seen": sorted(totals), "last_pages_seen": sorted(lastpages),
            "relations": rel_count, "outbox_events": event_count,
            "job_states": {st: sum(int(j["state"] == st) for j in jobs) for st in sorted(error_states)},
            "notes": "Partition/author coverage is not proof of full website coverage.",
        }

    def assess_passes(self, run_ids: Sequence[str]) -> dict:
        """Independently validate two or more successive *complete* fixture passes.

        Identical totals alone are insufficient: both identity *and order* must
        agree in the final two passes. A drifted earlier pass is retained as
        evidence. This never claims whole-site or historical completeness.
        """
        if not 2 <= len(run_ids) <= 8 or len(set(run_ids)) != len(run_ids):
            raise ValueError("use two to eight distinct completed runs")
        runs = [self.run_details(run_id) for run_id in run_ids]
        identity = lambda x: (x["partition_key"], x["document_kind"], x["seed_url"], x["scope"])
        if any(identity(x) != identity(runs[0]) for x in runs[1:]):
            raise ValueError("all scan passes must have identical partition, kind, seed and scope")
        reports = [self.report(run_id) for run_id in run_ids]
        with self._conn() as db:
            ordered = [[r[0] for r in db.execute("""SELECT o.entity_key FROM observations o
                    JOIN jobs j ON j.job_id=o.job_id WHERE o.run_id=?
                    ORDER BY j.page,o.position""", (run_id,))] for run_id in run_ids]
        latest = ordered[-1]
        earlier = ordered[-2]
        sets = [set(items) for items in ordered]
        seen = set().union(*sets)
        clean = [r["status"] == "MATCHED_DECLARED" for r in reports]
        if not clean[-1] or not clean[-2]:
            status = "HOLD_INCOMPLETE"
        elif sets[-1] != sets[-2]:
            status = "DRIFT"
        elif latest != earlier:
            status = "ORDER_DRIFT"
        elif all(clean) and all(items == latest for items in ordered):
            status = "STABLE_OBSERVED"
        else:
            status = "RECOVERED_STABLE_OBSERVED"
        return {
            "partition": runs[0]["partition_key"], "scope": runs[0]["scope"],
            "status": status,
            "pass_ids": list(run_ids),
            "pass_statuses": [r["status"] for r in reports],
            "observed_unique_by_pass": [len(s) for s in sets],
            "last_two_identity_equal": sets[-1] == sets[-2],
            "last_two_order_equal": latest == earlier,
            "union_ever_observed": len(seen),
            "latest_unique": len(sets[-1]),
            "ever_seen_missing_from_latest": sorted(seen-sets[-1]),
            "new_since_initial": sorted(sets[-1]-sets[0]),
            "absent_since_initial": sorted(sets[0]-sets[-1]),
            "notes": ("Two matching bounded observations support local stability only; "
                      "they do not prove all site content was discovered."),
        }

    def diff(self, earlier_run: str, later_run: str) -> dict:
        e, l = self.run_details(earlier_run), self.run_details(later_run)
        if e["partition_key"] != l["partition_key"]:
            raise ValueError("only compare the same discovery partition")
        with self._conn() as db:
            def entities(run: str) -> set[str]:
                return {r[0] for r in db.execute("SELECT DISTINCT entity_key FROM observations WHERE run_id=?", (run,))}
            a, b = entities(earlier_run), entities(later_run)
        return {"newly_observed": sorted(b-a), "absent_from_new_observation": sorted(a-b),
                "retained": len(a&b), "warning": "Absent in a new listing does not prove deletion."}

    def count_entities(self) -> int:
        with self._conn() as db:
            return db.execute("SELECT COUNT(*) FROM entities").fetchone()[0]

    def snapshot_keys(self, run_id: str) -> list[str]:
        with self._conn() as db:
            return [r[0] for r in db.execute("SELECT object_key FROM snapshots WHERE run_id=? ORDER BY page", (run_id,))]

    def relationships(self) -> list[tuple[str, str, str, int]]:
        with self._conn() as db:
            return [tuple(r) for r in db.execute("SELECT parent_key,child_key,relation_kind,ordinal FROM relationships ORDER BY parent_key,child_key")]


class DiscoveryEngine:
    """Deterministic fixture-driven worker, no HTTP client and no live URL input."""
    def __init__(self, store: DiscoveryStore, sink: FileObjectSink, source: MemoryFixtureSource,
                 *, max_attempts: int = 4) -> None:
        self.store, self.sink, self.source = store, sink, source
        self.max_attempts = max_attempts

    def _recover_raw_before_refetch(self, claim: dict, run: dict, now: float) -> bool:
        """After uncertain acknowledgement/crash, use retained S3 RAW before HTTP.

        Returns True only when this step handled the job by reconciliation or
        a bounded retry. A genuine 404 from both possible object keys is a
        signal that a normal refetch may proceed. Other S3 errors defer it.
        """
        if claim["attempt"] <= 1:
            return False
        page = claim["page"]
        for extension, parser in (("json",parse_fixture_page),("html",parse_fixture_html)):
            key = f"raw/{run['run_id']}/{page:06}.{extension}"
            try:
                raw = self.sink.get(key)
            except (FileNotFoundError, KeyError):
                continue
            except Exception as exc:
                code = (getattr(exc, "response", None) or {}).get("Error", {}).get("Code")
                if code in ("NoSuchKey", "404", "NotFound"):
                    continue
                self.store.record_problem(claim,"s3_recovery_unavailable:"+type(exc).__name__,
                    status="RETRY",now=now,max_attempts=self.max_attempts)
                return True
            if len(raw) > 2*1024*1024:
                self.store.record_problem(claim,"oversized_recovered_raw",status="NEEDS_REVIEW",now=now)
                return True
            try:
                parsed=parser(raw,expected_kind=run["document_kind"],requested_page=page,
                              max_pages=run["max_pages"])
                self.store.complete(claim,key=key,digest=hashlib.sha256(raw).hexdigest(),
                                    raw_size=len(raw),page_data=parsed,now=now)
            except StaleLease:
                raise
            except (SchemaChanged,ValueError,OSError,RuntimeError) as exc:
                self.store.record_problem(claim,"raw_recovery_invalid:"+type(exc).__name__,
                        status="NEEDS_REVIEW",now=now)
            return True
        return False

    def step(self, run_id: str, *, now: float | None = None, fail_after_put: bool = False) -> bool:
        now = time.time() if now is None else now
        claim = self.store.claim(run_id, now)
        if claim is None:
            return False
        page = claim["page"]
        run = self.store.run_details(run_id)
        if self._recover_raw_before_refetch(claim,run,now):
            return True
        response = self.source.fetch(run_id, page)
        if response.status == 429 or response.status >= 500:
            self.store.record_problem(claim, "upstream_"+str(response.status), status="RETRY",
                                      now=now, retry_after=response.retry_after,
                                      max_attempts=self.max_attempts)
            return True
        if response.status in (401, 403, 451):
            self.store.record_problem(claim, "permission_"+str(response.status), status="BLOCKED", now=now)
            return True
        if response.status in (404, 410):
            self.store.record_problem(claim, "missing_"+str(response.status), status="MISSING_UPSTREAM", now=now)
            return True
        if response.status != 200 or response.content_type not in ("application/json", "text/html"):
            self.store.record_problem(claim, "unsupported_mime_or_status", status="NEEDS_REVIEW", now=now)
            return True
        extension = "html" if response.content_type == "text/html" else "json"
        key = f"raw/{run_id}/{page:06}.{extension}"
        try:
            try:
                digest = self.sink.put_verified(key, response.body)
            except (ObjectWriteUncertain, OSError, TimeoutError) as exc:
                # Unknown ACK: reconcile by deterministic key + exact readback.
                try:
                    stored = self.sink.get(key)
                except (OSError, RuntimeError, KeyError) as second_exc:
                    self.store.record_problem(claim, "uncertain_object_put:"+type(second_exc).__name__,
                                              status="RETRY", now=now, max_attempts=self.max_attempts)
                    return True
                if stored != response.body:
                    raise ObjectConflict("uncertain PUT contained other/corrupt bytes") from exc
                digest = hashlib.sha256(stored).hexdigest()
            if fail_after_put:
                raise SimulatedCrash("crash between raw S3 put and SQLite transaction")
            parser = parse_fixture_html if response.content_type == "text/html" else parse_fixture_page
            parsed = parser(response.body, expected_kind=run["document_kind"],
                            requested_page=page, max_pages=run["max_pages"])
            self.store.complete(claim, key=key, digest=digest, raw_size=len(response.body),
                                page_data=parsed, now=now)
        except SimulatedCrash:
            raise
        except StaleLease:
            raise
        except (SchemaChanged, ValueError, RuntimeError, OSError) as exc:
            self.store.record_problem(claim, type(exc).__name__+":"+str(exc)[:200],
                                      status="NEEDS_REVIEW", now=now)
        return True

    def drain(self, run_id: str, *, start: float = 10_000.0, ticks: int = 20_000) -> dict:
        """Synthetic clock advances; never sleeps or accesses the internet."""
        now = start
        for _ in range(ticks):
            if not self.step(run_id, now=now):
                break
            now += 10.0
        else:
            raise RuntimeError("fixture cycle did not quiesce")
        return self.store.report(run_id)
