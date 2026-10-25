"""M8 local-only API and durable queue for the existing M5-M7 ingest pipeline.

No live-source crawler, remote code loading, automatic Erida staging or
publishing outside M4's selected-manifest CAS publication is implemented here.
"""
from __future__ import annotations

import fcntl
import hashlib
import hmac
import json
import os
import re
import shutil
import sqlite3
import tempfile
import threading
import time
import uuid
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable, Protocol
from urllib.parse import urlsplit

from .objects import FileObjectSink, S3ObjectSink
from .unified_pipeline import MAX_RAW, PageLedger, UnifiedPipeline, WorkPlan, parse_plan

MAX_PLAN = 1024 * 1024
WORK_LIMIT = 3000
PROFILE_FIXTURE = "fixture-chapter-html-v1"
PROFILE_LITEROTICA = "literotica-saved-story-v1"
_ID = re.compile(r"[0-9a-f]{32}\Z")
_KEY = re.compile(r"[A-Za-z0-9_.:-]{1,128}\Z")


class InvalidJob(ValueError):
    pass


class JobConflict(RuntimeError):
    pass


class WorkerBusy(RuntimeError):
    pass


class SourceAdapter(Protocol):
    """A profile registered by the operator. API clients cannot load Python code."""
    profile: str
    def validate(self, plan: WorkPlan) -> None: ...


@dataclass(frozen=True)
class SavedPageAdapter:
    profile: str

    def validate(self, plan: WorkPlan) -> None:
        if plan.parser_profile != self.profile:
            raise InvalidJob("parser does not match approved source adapter")
        if not 1 <= plan.expected_pages <= WORK_LIMIT:
            raise InvalidJob("invalid selected page count")


class AdapterRegistry:
    def __init__(self) -> None:
        self._adapters: dict[str, SourceAdapter] = {}

    def register(self, adapter: SourceAdapter) -> None:
        if adapter.profile in self._adapters:
            raise ValueError("adapter profile already registered")
        self._adapters[adapter.profile] = adapter

    def validate(self, plan: WorkPlan) -> None:
        adapter = self._adapters.get(plan.parser_profile)
        if adapter is None:
            raise InvalidJob("unknown or disabled source adapter")
        adapter.validate(plan)

    def names(self) -> list[str]:
        return sorted(self._adapters)


def default_adapters() -> AdapterRegistry:
    registry = AdapterRegistry()
    registry.register(SavedPageAdapter(PROFILE_FIXTURE))
    registry.register(SavedPageAdapter(PROFILE_LITEROTICA))
    return registry


class JobStore:
    """SQLite WAL leases with one immutable approved manifest per work ID.

    Plan and result metadata are persisted in SQLite; raw and chapter bytes live
    in immutable object storage. The operator-approved inputs are frozen to
    disk during submission so later source file edits do not change a queued job.
    """

    def __init__(self, state: str | Path, approved_root: str | Path, *,
                 adapters: AdapterRegistry | None = None) -> None:
        self.state = Path(state).resolve()
        self.approved = Path(approved_root).resolve()
        self.state.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.db = self.state / "service.sqlite3"
        self.adapters = adapters or default_adapters()
        with self.connection() as con:
            con.execute("PRAGMA journal_mode=WAL")
            con.execute("PRAGMA synchronous=FULL")
            con.executescript("""
            CREATE TABLE IF NOT EXISTS ingest_jobs (
                job_id TEXT PRIMARY KEY,
                work_id TEXT UNIQUE NOT NULL,
                profile TEXT NOT NULL,
                plan_sha TEXT NOT NULL,
                plan BLOB NOT NULL,
                idempotency_key TEXT UNIQUE,
                status TEXT NOT NULL,
                attempt INTEGER NOT NULL DEFAULT 0,
                lease_token TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL,
                result_json TEXT,
                error_code TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_ingest_jobs_status
            ON ingest_jobs(status, created_at);
            """)

    @contextmanager
    def connection(self):
        db = sqlite3.connect(self.db, isolation_level=None, timeout=10)
        db.row_factory = sqlite3.Row
        try:
            yield db
        finally:
            db.close()

    @contextmanager
    def transaction(self):
        with self.connection() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                yield db
                db.execute("COMMIT")
            except BaseException:
                db.execute("ROLLBACK")
                raise

    def _freeze_inputs(self, plan: WorkPlan) -> None:
        approved_dir = (self.approved / plan.source_id).resolve()
        if approved_dir.parent != self.approved or not approved_dir.is_dir():
            if any(p.filename for c in plan.chapters for p in c.pages):
                raise InvalidJob("approved source snapshot directory not found")
            return
        dest = self.state / "approved-snapshots" / plan.manifest_sha256
        for ch in plan.chapters:
            for page in ch.pages:
                if page.filename is None:
                    continue
                src = (approved_dir / page.filename).resolve()
                if src.parent != approved_dir or not src.is_file():
                    raise InvalidJob("source page is missing or escapes approval directory")
                if not 0 < src.stat().st_size <= MAX_RAW:
                    raise InvalidJob("approved source page exceeds immutable RAW size cap")
                dest.mkdir(parents=True, exist_ok=True, mode=0o700)
                out = dest / page.filename
                if out.exists():
                    # The immutable copy, not the mutable source, owns replay semantics.
                    continue
                tmp_fd, tmp_name = tempfile.mkstemp(prefix=".snapshot-", dir=dest)
                try:
                    with os.fdopen(tmp_fd, "wb") as writer, src.open("rb") as reader:
                        copied = 0
                        while block := reader.read(65536):
                            copied += len(block)
                            if copied > MAX_RAW:
                                raise InvalidJob("approved source grew beyond cap")
                            writer.write(block)
                        writer.flush()
                        os.fsync(writer.fileno())
                    if copied == 0:
                        raise InvalidJob("empty approved source")
                    try:
                        os.link(tmp_name, out)  # no replacement under concurrent submit
                    except FileExistsError:
                        pass
                finally:
                    os.unlink(tmp_name)
                if out.stat().st_size > MAX_RAW or not out.is_file():
                    raise InvalidJob("invalid immutable source copy")
        if dest.exists():
            fd = os.open(dest, os.O_RDONLY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)

    @staticmethod
    def _public(row: sqlite3.Row) -> dict:
        item = {k: row[k] for k in (
            "job_id", "work_id", "profile", "plan_sha", "status",
            "attempt", "created_at", "updated_at", "error_code")}
        if row["result_json"]:
            item["result"] = json.loads(row["result_json"])
        return item

    def submit(self, obj: dict) -> tuple[dict, bool]:
        if not isinstance(obj, dict) or set(obj) - {"plan", "idempotency_key"} or not isinstance(obj.get("plan"), dict):
            raise InvalidJob("request must contain a selected-work-v1 plan")
        key = obj.get("idempotency_key")
        if key is not None and (not isinstance(key, str) or not _KEY.fullmatch(key)):
            raise InvalidJob("unsafe idempotency key")
        try:
            raw = json.dumps(obj["plan"], ensure_ascii=False, sort_keys=True,
                             separators=(",", ":"), allow_nan=False).encode("utf-8")
            if len(raw) > MAX_PLAN:
                raise InvalidJob("selected work plan exceeds 1 MiB")
            plan = parse_plan(raw)
        except (ValueError, TypeError) as exc:
            raise InvalidJob("selected source plan is invalid") from exc
        self.adapters.validate(plan)
        with self.transaction() as db:
            if key is not None:
                old = db.execute("SELECT * FROM ingest_jobs WHERE idempotency_key=?", (key,)).fetchone()
                if old:
                    if old["plan_sha"] != plan.manifest_sha256:
                        raise JobConflict("idempotency key already bound to a different selected plan")
                    return self._public(old), True
            old = db.execute("SELECT * FROM ingest_jobs WHERE work_id=?", (plan.source_id,)).fetchone()
            if old:
                if old["plan_sha"] != plan.manifest_sha256:
                    raise JobConflict("work ID already bound to another selected catalog; operator migration required")
                if key is not None and key != old["idempotency_key"]:
                    raise JobConflict("work already uses a different idempotency key")
                return self._public(old), True
            self._freeze_inputs(plan)
            job_id = uuid.uuid4().hex
            now = time.time()
            db.execute("""INSERT INTO ingest_jobs
                (job_id,work_id,profile,plan_sha,plan,idempotency_key,status,created_at,updated_at)
                VALUES(?,?,?,?,?,?,'QUEUED',?,?)""",
                (job_id, plan.source_id, plan.parser_profile, plan.manifest_sha256,
                 raw, key, now, now))
            return self._public(db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone()), False

    def get(self, job_id: str) -> dict | None:
        if not isinstance(job_id, str) or not _ID.fullmatch(job_id):
            return None
        with self.connection() as db:
            row = db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone()
            return self._public(row) if row else None

    def list(self, limit: int = 50) -> list[dict]:
        limit = max(1, min(limit, 50))
        with self.connection() as db:
            return [self._public(x) for x in db.execute(
                "SELECT * FROM ingest_jobs ORDER BY created_at DESC LIMIT ?", (limit,))]

    def cancel(self, job_id: str) -> dict:
        with self.transaction() as db:
            row = db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone()
            if not row:
                raise InvalidJob("unknown job")
            if row["status"] != "QUEUED":
                raise JobConflict("only a queued job can be cancelled")
            db.execute("UPDATE ingest_jobs SET status='CANCELLED',updated_at=? WHERE job_id=?",
                       (time.time(), job_id))
            return self._public(db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone())

    def retry(self, job_id: str) -> dict:
        with self.transaction() as db:
            row = db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone()
            if not row:
                raise InvalidJob("unknown job")
            if row["status"] not in ("NEEDS_REVIEW", "CANCELLED"):
                raise JobConflict("only failed or cancelled jobs can be retried")
            if row["status"] == "NEEDS_REVIEW":
                path = self.ledger_path(job_id)
                if path.exists() and PageLedger(path).status(row["work_id"])["issues"]:
                    raise JobConflict("source page requires manual remediation; refusing blind retry")
            db.execute("""UPDATE ingest_jobs SET status='QUEUED',error_code=NULL,
                updated_at=? WHERE job_id=?""", (time.time(), job_id))
            return self._public(db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (job_id,)).fetchone())

    def ledger_path(self, job_id: str) -> Path:
        return self.state / "ledgers" / job_id / "ingest.sqlite3"

    def recover_orphaned(self) -> int:
        """Only call while holding the global worker flock; old process is gone."""
        with self.transaction() as db:
            count = db.execute("UPDATE ingest_jobs SET status='QUEUED',lease_token=NULL,updated_at=? WHERE status='RUNNING'",
                               (time.time(),)).rowcount
            return count

    def claim(self, job_id: str | None = None) -> tuple[dict, bytes, str] | None:
        """Atomically claim one queued task; an optional ID avoids changing jobs under a lock."""
        with self.transaction() as db:
            if job_id is None:
                row = db.execute("SELECT * FROM ingest_jobs WHERE status='QUEUED' ORDER BY created_at LIMIT 1").fetchone()
            else:
                row = db.execute("SELECT * FROM ingest_jobs WHERE job_id=? AND status='QUEUED'", (job_id,)).fetchone()
            if not row:
                return None
            token = uuid.uuid4().hex
            updated = db.execute("""UPDATE ingest_jobs SET status='RUNNING',attempt=attempt+1,lease_token=?,
                         error_code=NULL,updated_at=? WHERE job_id=? AND status='QUEUED'""",
                       (token, time.time(), row["job_id"])).rowcount
            if updated != 1:
                return None
            return self._public(db.execute("SELECT * FROM ingest_jobs WHERE job_id=?", (row["job_id"],)).fetchone()), bytes(row["plan"]), token

    def candidates(self, limit: int = 100) -> list[tuple[str, str]]:
        """Include orphan candidates, but only a holder of that job's OS lock may reclaim."""
        with self.connection() as db:
            return [(r["job_id"], r["status"]) for r in db.execute(
                """SELECT job_id,status FROM ingest_jobs WHERE status IN ('RUNNING','QUEUED')
                   ORDER BY CASE status WHEN 'RUNNING' THEN 0 ELSE 1 END,created_at LIMIT ?""",
                (limit,))]

    def recover_job(self, job_id: str) -> bool:
        """Caller MUST own the per-job flock before reclaiming RUNNING.

        Never blanket-reset all RUNNING rows while other live workers exist.
        """
        with self.transaction() as db:
            return db.execute("""UPDATE ingest_jobs SET status='QUEUED',lease_token=NULL,updated_at=?
                WHERE job_id=? AND status='RUNNING'""", (time.time(), job_id)).rowcount == 1

    def configure_worker_slots(self, max_workers: int) -> None:
        """Immutable on-disk concurrency cap: all processes sharing state must agree."""
        if type(max_workers) is not int or not 1 <= max_workers <= 8:
            raise ValueError("M9 permits 1..8 workers per shared POSIX state")
        with self.transaction() as db:
            db.execute("CREATE TABLE IF NOT EXISTS ingest_settings (name TEXT PRIMARY KEY,value TEXT NOT NULL)")
            old = db.execute("SELECT value FROM ingest_settings WHERE name='worker_slots'").fetchone()
            if old is None:
                db.execute("INSERT INTO ingest_settings(name,value) VALUES('worker_slots',?)", (str(max_workers),))
            elif int(old["value"]) != max_workers:
                raise JobConflict("worker slot configuration mismatch for shared state")

    def counts(self) -> dict[str,int]:
        with self.connection() as db:
            rows=db.execute("SELECT status,COUNT(*) AS n FROM ingest_jobs GROUP BY status").fetchall()
        result={s:0 for s in ('QUEUED','RUNNING','COMPLETED','NEEDS_REVIEW','CANCELLED')}
        result.update({r['status']:r['n'] for r in rows})
        return result

    def finish(self, job_id: str, token: str, *, result: dict | None = None,
               error_code: str | None = None) -> None:
        if (result is None) == (error_code is None):
            raise ValueError("exactly one completion outcome required")
        with self.transaction() as db:
            updated = db.execute("""UPDATE ingest_jobs SET status=?,result_json=?,error_code=?,
                 updated_at=?,lease_token=NULL WHERE job_id=? AND lease_token=? AND status='RUNNING'""",
                 ("COMPLETED" if result is not None else "NEEDS_REVIEW",
                  json.dumps(result, ensure_ascii=False, sort_keys=True) if result else None,
                  error_code, time.time(), job_id, token)).rowcount
            if updated != 1:
                raise JobConflict("stale worker cannot commit a job")


class IngestWorker:
    def __init__(self, jobs: JobStore, *, s3_bucket: str | None = None,
                 s3_endpoint: str | None = None, s3_prefix: str = "unified-lab/",
                 max_workers: int = 1) -> None:
        self.jobs = jobs
        self.s3_bucket, self.s3_endpoint, self.s3_prefix = s3_bucket, s3_endpoint, s3_prefix
        if s3_endpoint and not s3_bucket:
            raise ValueError("S3 endpoint requires explicit operator-owned bucket")
        self.max_workers = max_workers
        self.jobs.configure_worker_slots(max_workers)
        (self.jobs.state / "worker-slots").mkdir(exist_ok=True, mode=0o700)
        (self.jobs.state / "job-locks").mkdir(exist_ok=True, mode=0o700)

    @contextmanager
    def _exclusive(self):
        """Reserve one capacity slot across processes. M8 one-worker mode remains valid."""
        for n in range(self.max_workers):
            path = self.jobs.state / "worker-slots" / f"slot-{n}.lock"
            fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                os.close(fd)
                continue
            try:
                yield
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
                os.close(fd)
            return
        raise WorkerBusy("all configured worker slots are occupied")

    @contextmanager
    def _job_exclusive(self, job_id: str):
        """An OS lock for each work; never overlap recovery and active processing."""
        if not _ID.fullmatch(job_id):
            raise InvalidJob("invalid job ID")
        path = self.jobs.state / "job-locks" / (job_id + ".lock")
        fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
        try:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as exc:
                raise WorkerBusy("job owned by another executor") from exc
            yield
        finally:
            os.close(fd)

    def _sink(self):
        if self.s3_bucket:
            import boto3
            from botocore.client import Config
            client = boto3.client("s3", endpoint_url=self.s3_endpoint,
                                  config=Config(s3={"addressing_style": "path"}))
            return S3ObjectSink(client, self.s3_bucket, self.s3_prefix)
        return FileObjectSink(self.jobs.state / "objects")

    def _process(self, info: dict, plan_bytes: bytes) -> dict:
        plan = parse_plan(plan_bytes)
        self.jobs.adapters.validate(plan)
        work_ledger = PageLedger(self.jobs.ledger_path(info["job_id"]))
        # A killed worker can leave a valid page lease in FETCHING/PARSING.
        # Exclusive worker.lock proves no older worker can still execute.
        with work_ledger.tx() as db:
            db.execute("""UPDATE source_pages SET state=CASE state
                WHEN 'FETCHING' THEN 'READY' ELSE 'RAW' END,
                lease_token=NULL,lease_until=0 WHERE state IN ('FETCHING','PARSING')""")
        journal = (self.jobs.state / "multipart" / (info["job_id"] + ".sqlite3")) if self.s3_bucket else None
        if journal:
            journal.parent.mkdir(parents=True, exist_ok=True)
        engine = UnifiedPipeline(work_ledger, self._sink(),
                                 self.jobs.state / "approved-snapshots" / plan.manifest_sha256,
                                 self.jobs.state / "staging" / info["job_id"],
                                 multipart_journal=journal)
        return engine.run(plan)

    def run_once(self) -> dict | None:
        with self._exclusive():
            # Only a worker holding this job's OS lock may requeue an orphan.
            for job_id, _ in self.jobs.candidates():
                try:
                    with self._job_exclusive(job_id):
                        self.jobs.recover_job(job_id)  # no-op unless abandoned RUNNING
                        work = self.jobs.claim(job_id)
                        if not work:
                            continue
                        info, manifest, token = work
                        try:
                            result = self._process(info, manifest)
                        except Exception as exc:
                            # Return only stable classification, not source or credentials.
                            self.jobs.finish(info["job_id"], token, error_code=type(exc).__name__)
                        else:
                            self.jobs.finish(info["job_id"], token, result=result)
                        return self.jobs.get(info["job_id"])
                except WorkerBusy:
                    continue
            return None

    def run_loop(self, stop: threading.Event, *, poll_interval: float = 0.5) -> None:
        while not stop.is_set():
            try:
                got = self.run_once()
            except WorkerBusy:
                got = None
            if not got:
                stop.wait(poll_interval)


def make_handler(jobs: JobStore, token: str):
    if not isinstance(token, str) or len(token) < 24:
        raise ValueError("API bearer token must have at least 24 characters")

    class Handler(BaseHTTPRequestHandler):
        server_version = "IngestM8/0.1"
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            return  # access logs may contain sensitive source IDs

        def answer(self, code: int, obj: dict):
            out = json.dumps(obj, ensure_ascii=False, sort_keys=True).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(out)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            self.wfile.write(out)

        def authorized(self) -> bool:
            claimed = self.headers.get("Authorization", "")
            if not hmac.compare_digest(claimed, "Bearer " + token):
                self.answer(401, {"error": "unauthorized"})
                return False
            return True

        def checked_path(self) -> str | None:
            parts = urlsplit(self.path)
            if parts.query or parts.fragment:
                self.answer(400, {"error": "query parameters are not supported"})
                return None
            return parts.path

        def do_GET(self):
            path = self.checked_path()
            if path is None:
                return
            if path == "/healthz":
                return self.answer(200, {"status": "ok", "service": "ingest-m8"})
            if not self.authorized():
                return
            if path == "/v1/queue":
                return self.answer(200, {"counts": jobs.counts(), "lease_strategy": "per-job-flock",
                                         "live_crawling": False})
            if path == "/v1/adapters":
                return self.answer(200, {"profiles": jobs.adapters.names(), "live_crawling": False})
            if path == "/v1/jobs":
                return self.answer(200, {"jobs": jobs.list()})
            match = re.fullmatch(r"/v1/jobs/([0-9a-f]{32})", path)
            if match:
                job = jobs.get(match[1])
                return self.answer(200, job) if job else self.answer(404, {"error": "not found"})
            return self.answer(404, {"error": "not found"})

        def do_POST(self):
            path = self.checked_path()
            if path is None:
                return
            if not self.authorized():
                return
            length = self.headers.get("Content-Length")
            if length is None or not length.isascii() or not length.isdecimal():
                return self.answer(411, {"error": "Content-Length required"})
            n = int(length)
            if n < 0 or n > MAX_PLAN + 4096:
                self.close_connection = True
                return self.answer(413, {"error": "request exceeds 1 MiB limit"})
            body = self.rfile.read(n)
            try:
                if path == "/v1/jobs":
                    if self.headers.get("Content-Type", "").split(";")[0].strip().lower() != "application/json":
                        return self.answer(415, {"error": "expected application/json"})
                    request = json.loads(body)
                    result, reused = jobs.submit(request)
                    return self.answer(200 if reused else 201, {"job": result, "reused": reused})
                match = re.fullmatch(r"/v1/jobs/([0-9a-f]{32})/(retry|cancel)", path)
                if not match:
                    return self.answer(404, {"error": "not found"})
                if n != 0:
                    return self.answer(400, {"error": "no body accepted"})
                result = jobs.retry(match[1]) if match[2] == "retry" else jobs.cancel(match[1])
                return self.answer(200, {"job": result})
            except JobConflict:
                return self.answer(409, {"error": "job conflict"})
            except json.JSONDecodeError:
                return self.answer(400, {"error": "invalid JSON"})
            except (InvalidJob, ValueError, TypeError, UnicodeDecodeError):
                return self.answer(422, {"error": "invalid job or plan"})
            except Exception:
                return self.answer(500, {"error": "internal error"})

    return Handler


def serve(jobs: JobStore, token: str, *, port: int, worker: IngestWorker | None = None,
          bind: str = "127.0.0.1", worker_count: int = 1) -> None:
    if bind != "127.0.0.1":
        raise ValueError("M8 experiment listens on loopback only; use authenticated TLS proxy for remote use")
    if not 0 <= port <= 65535:
        raise ValueError("invalid listening port")
    if type(worker_count) is not int or not 1 <= worker_count <= 8:
        raise ValueError("worker_count must be 1..8")
    if worker and worker_count > worker.max_workers:
        raise ValueError("requested worker threads exceed shared configured capacity")
    stop = threading.Event()
    threads = []
    with ThreadingHTTPServer((bind, port), make_handler(jobs, token)) as server:
        if worker:
            for _ in range(worker_count):
                thread = threading.Thread(target=worker.run_loop, args=(stop,), daemon=True)
                thread.start()
                threads.append(thread)
        try:
            server.serve_forever(poll_interval=0.2)
        finally:
            stop.set()
            for thread in threads:
                thread.join(timeout=2)
