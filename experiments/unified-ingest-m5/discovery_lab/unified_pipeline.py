"""M5 offline-first ingest pipeline: approved pages -> RAW -> parse -> publish.

The authoritative source discovery, access policy and per-site HTTP limiter are
owned by the caller (ReadEra, Literotica, or another source adapter). This module
only consumes a fixed, explicitly-approved manifest and immutable raw snapshots.
It never makes a network request to an upstream web site. S3 is opt-in.

The M4 streaming publisher remains authoritative for final objects, manifests,
content-derived revision IDs and current.json compare-and-swap.
"""
from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import time
import uuid
from contextlib import contextmanager
from dataclasses import dataclass
from html.parser import HTMLParser
from pathlib import Path
from typing import Iterator, Protocol

from .model import canonical_fixture_url, require_entity_id
from .objects import FileObjectSink, S3ObjectSink, ObjectConflict
from .publisher import IncompleteWork, PublicationUncertain
from .streaming_publisher import (StreamWork, StreamChapter, StreamPage,
                                  StreamingWorkPublisher)

MAX_RAW = 1024 * 1024
PARSER_VERSION = "fixture-chapter-html-v1"


class NeedsReview(ValueError):
    """Missing, conflicting or unrecognized source. Never silently publish."""


class InjectCrash(RuntimeError):
    """Test-only crash between remote RAW acknowledgement and SQL commit."""


@dataclass(frozen=True)
class PageSpec:
    chapter: int
    page: int
    source_url: str
    filename: str | None = None
    raw_key: str | None = None
    raw_sha256: str | None = None


@dataclass(frozen=True)
class ChapterSpec:
    ordinal: int
    title: str
    pages: tuple[PageSpec, ...]


@dataclass(frozen=True)
class WorkPlan:
    source_id: str
    title: str
    author: str
    source_url: str
    language: str
    tags: tuple[str, ...]
    source_tags: tuple[str, ...]
    source_categories: tuple[str, ...]
    chapters: tuple[ChapterSpec, ...]
    manifest_sha256: str

    @property
    def expected_pages(self) -> int:
        return sum(len(c.pages) for c in self.chapters)


class RawObjectSink(Protocol):
    def put_verified(self, key: str, data: bytes) -> str: ...
    def get(self, key: str) -> bytes: ...


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _safe_ident(s: object, label: str) -> str:
    try:
        return require_entity_id(s)
    except (TypeError, ValueError) as e:
        raise NeedsReview(f"invalid {label}") from e


def _nonempty(s: object, label: str, max_chars: int = 500) -> str:
    if not isinstance(s, str) or not s.strip() or len(s) > max_chars or "\x00" in s:
        raise NeedsReview(f"invalid {label}")
    return s.strip()


def parse_plan(document: bytes) -> WorkPlan:
    """Selected, immutable catalog; never silently infer completeness."""
    if len(document) > 1024 * 1024:
        raise NeedsReview("plan exceeds 1 MiB")
    try:
        o = json.loads(document)
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        raise NeedsReview("invalid JSON plan") from e
    if not isinstance(o, dict) or o.get("schema") != "selected-work-v1":
        raise NeedsReview("unknown plan schema")
    source_id = _safe_ident(o.get("source_id"), "source ID")
    source_url = canonical_fixture_url(o.get("source_url", ""))
    title, author = _nonempty(o.get("title"), "title"), _nonempty(o.get("author"), "author")
    language = _nonempty(o.get("language"), "language", 30)
    if not re.fullmatch(r"[a-z]{2,3}(?:-[a-z]{2})?", language):
        raise NeedsReview("invalid language")
    tags = o.get("tags", [])
    source_tags = o.get("source_tags", [])
    categories = o.get("source_categories", [])
    for seq, label, maximum in ((tags, "tags", 100), (source_tags, "source_tags", 500),
                                (categories, "source_categories", 100)):
        if not isinstance(seq, list) or len(seq) > maximum or len(set(map(str, seq))) != len(seq):
            raise NeedsReview(f"invalid {label}")
        if any(not isinstance(x, str) or not x or len(x) > 200 for x in seq):
            raise NeedsReview(f"invalid {label} entries")
    chs = o.get("chapters")
    if not isinstance(chs, list) or not 1 <= len(chs) <= 1000:
        raise NeedsReview("unknown or empty chapter inventory")
    chapters = []
    total = 0
    for num, chapter in enumerate(chs, 1):
        if not isinstance(chapter, dict) or chapter.get("ordinal") != num:
            raise NeedsReview("chapters must have consecutive ordinals")
        caption = _nonempty(chapter.get("title"), "chapter title")
        raw_pages = chapter.get("pages")
        if not isinstance(raw_pages, list) or not raw_pages or len(raw_pages) > 3000:
            raise NeedsReview("invalid chapter page inventory")
        pages = []
        for n, page in enumerate(raw_pages, 1):
            if not isinstance(page, dict) or page.get("number") != n:
                raise NeedsReview("pages must be consecutive")
            src = canonical_fixture_url(page.get("source_url", ""))
            filename, key, digest = page.get("fixture_file"), page.get("raw_key"), page.get("raw_sha256")
            if bool(filename) == bool(key):
                raise NeedsReview("exactly one of fixture_file or raw_key required")
            if filename is not None:
                if not isinstance(filename, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,140}\.html", filename) or ".." in filename:
                    raise NeedsReview("unsafe fixture file")
                if digest is not None:
                    raise NeedsReview("fixture_file has no independent S3 digest")
            else:
                if not isinstance(key, str) or not key.startswith("raw/") or ".." in key.split("/") or "\\" in key or len(key) > 500:
                    raise NeedsReview("invalid preloaded S3 key")
                if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
                    raise NeedsReview("preloaded RAW requires verified sha256")
            pages.append(PageSpec(num, n, src, filename, key, digest))
        chapters.append(ChapterSpec(num, caption, tuple(pages)))
        total += len(pages)
    if total > 3000:
        raise NeedsReview("too many pages")
    # Hash the raw manifest bytes: restart must use the exact same approved plan.
    return WorkPlan(source_id, title, author, source_url, language, tuple(tags),
                    tuple(source_tags), tuple(categories), tuple(chapters), _sha(document))


class _ChapterReader(HTMLParser):
    """Strict fixture adapter; not a live Literotica DOM parser."""
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.start_count = 0
        self.end_count = 0
        self.attrs: dict[str, str] = {}
        self.in_article = False
        self.in_paragraph = False
        self.chunk: list[str] = []
        self.paragraphs: list[str] = []
        self.script_depth = 0
        self.forbidden = False

    def handle_starttag(self, tag, attrs) -> None:
        values = dict(attrs)
        if tag in ("script", "iframe", "form"):
            self.forbidden = True
        if tag == "article":
            self.start_count += 1
            if self.in_article or self.start_count != 1:
                self.forbidden = True
            self.in_article = True
            self.attrs = values
        elif tag == "p" and self.in_article:
            if self.in_paragraph:
                self.forbidden = True
            self.in_paragraph = True
            self.chunk = []
        elif tag == "br" and self.in_paragraph:
            self.chunk.append("\n")

    def handle_endtag(self, tag) -> None:
        if tag == "p" and self.in_paragraph:
            t = "".join(self.chunk).strip()
            if t:
                self.paragraphs.append(t)
            self.in_paragraph = False
        elif tag == "article" and self.in_article:
            self.end_count += 1
            self.in_article = False

    def handle_data(self, data: str) -> None:
        if self.in_paragraph:
            self.chunk.append(data)


def parse_snapshot(payload: bytes, *, source_id: str, chapter: int, page: int) -> str:
    if not payload or len(payload) > MAX_RAW:
        raise NeedsReview("empty or oversized RAW snapshot")
    try:
        html = payload.decode("utf-8", "strict")
    except UnicodeDecodeError as exc:
        raise NeedsReview("invalid UTF-8") from exc
    parser = _ChapterReader()
    parser.feed(html)
    parser.close()
    attrs = parser.attrs
    if (parser.forbidden or parser.start_count != 1 or parser.end_count != 1 or
        parser.in_article or parser.in_paragraph or
        attrs.get("data-fixture-work") != source_id or
        attrs.get("data-chapter") != str(chapter) or
        attrs.get("data-page") != str(page) or
        not parser.paragraphs):
        raise NeedsReview("unrecognized chapter fixture or metadata; review parser")
    text = "\n\n".join(parser.paragraphs)
    if not text.strip() or "\x00" in text or len(text.encode("utf-8")) > MAX_RAW:
        raise NeedsReview("invalid chapter text")
    return text


class PageLedger:
    """Durable small-pointer-only fetch/parse queue; no text stored in SQLite."""
    def __init__(self, db_file: str | Path) -> None:
        self.path = Path(db_file)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.conn() as db:
            db.executescript("""
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS selected_works(
                work_id TEXT PRIMARY KEY, manifest_sha TEXT NOT NULL, created_at REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS source_pages(
                work_id TEXT NOT NULL, chapter INTEGER NOT NULL, page INTEGER NOT NULL,
                state TEXT NOT NULL DEFAULT 'READY', attempt INTEGER NOT NULL DEFAULT 0,
                lease_token TEXT, lease_until REAL NOT NULL DEFAULT 0,
                raw_key TEXT, raw_sha TEXT, raw_size INTEGER,
                parsed_key TEXT, parsed_sha TEXT, error TEXT,
                PRIMARY KEY(work_id,chapter,page));
            CREATE INDEX IF NOT EXISTS idx_pipeline_claim ON source_pages(work_id,state,chapter,page);
            CREATE TABLE IF NOT EXISTS stage_events(
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                work_id TEXT NOT NULL, chapter INTEGER NOT NULL, page INTEGER NOT NULL,
                phase TEXT NOT NULL, object_key TEXT NOT NULL, sha256 TEXT NOT NULL,
                UNIQUE(work_id,chapter,page,phase));
            """)

    @contextmanager
    def conn(self) -> Iterator[sqlite3.Connection]:
        db = sqlite3.connect(self.path, timeout=10.0, isolation_level=None)
        db.row_factory = sqlite3.Row
        try:
            yield db
        finally:
            db.close()

    @contextmanager
    def tx(self) -> Iterator[sqlite3.Connection]:
        with self.conn() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                yield db
                db.execute("COMMIT")
            except BaseException:
                db.execute("ROLLBACK")
                raise

    def register(self, plan: WorkPlan) -> None:
        with self.tx() as db:
            old = db.execute("SELECT manifest_sha FROM selected_works WHERE work_id=?", (plan.source_id,)).fetchone()
            if old and old[0] != plan.manifest_sha256:
                raise NeedsReview("same work ID registered with different selected catalog; use a new ledger")
            db.execute("INSERT OR IGNORE INTO selected_works VALUES(?,?,?)", (plan.source_id, plan.manifest_sha256, time.time()))
            for chapter in plan.chapters:
                for page in chapter.pages:
                    db.execute("INSERT OR IGNORE INTO source_pages(work_id,chapter,page) VALUES(?,?,?)",
                               (plan.source_id, chapter.ordinal, page.page))

    def claim(self, work_id: str, state: str, *, now: float | None = None, seconds: float = 90) -> dict | None:
        if state not in ("READY", "RAW"):
            raise ValueError("unknown stage")
        now = time.time() if now is None else now
        with self.tx() as db:
            row = db.execute("""SELECT * FROM source_pages WHERE work_id=? AND
                         (state=? OR (state=? AND lease_until<=? AND error IS NULL))
                         ORDER BY chapter,page LIMIT 1""",
                         (work_id, state, "FETCHING" if state == "READY" else "PARSING", now)).fetchone()
            if not row:
                return None
            token = uuid.uuid4().hex
            db.execute("UPDATE source_pages SET state=?,lease_token=?,lease_until=?,attempt=attempt+1 WHERE work_id=? AND chapter=? AND page=?",
                       ("FETCHING" if state == "READY" else "PARSING", token, now + seconds, work_id, row["chapter"], row["page"]))
            return {**dict(row), "lease_token": token}

    def commit(self, claimed: dict, *, phase: str, key: str, sha: str, size: int | None = None) -> None:
        if phase not in ("RAW", "PARSED"):
            raise ValueError("bad phase")
        expected = "FETCHING" if phase == "RAW" else "PARSING"
        with self.tx() as db:
            cur = db.execute("SELECT state,lease_token,lease_until FROM source_pages WHERE work_id=? AND chapter=? AND page=?",
                             (claimed["work_id"], claimed["chapter"], claimed["page"])).fetchone()
            if (not cur or cur["state"] != expected or cur["lease_token"] != claimed["lease_token"]
                    or cur["lease_until"] <= time.time()):
                raise NeedsReview("stale lease cannot commit")
            fields = ("state='RAW',raw_key=?,raw_sha=?,raw_size=?," if phase == "RAW" else
                      "state='PARSED',parsed_key=?,parsed_sha=?,")
            values = (key, sha, size) if phase == "RAW" else (key, sha)
            db.execute(f"UPDATE source_pages SET {fields} lease_token=NULL,lease_until=0,error=NULL WHERE work_id=? AND chapter=? AND page=?",
                       values + (claimed["work_id"], claimed["chapter"], claimed["page"]))
            db.execute("INSERT OR IGNORE INTO stage_events(work_id,chapter,page,phase,object_key,sha256) VALUES(?,?,?,?,?,?)",
                       (claimed["work_id"], claimed["chapter"], claimed["page"], phase, key, sha))

    def fail(self, claimed: dict, exc: Exception) -> None:
        with self.tx() as db:
            db.execute("""UPDATE source_pages SET state='NEEDS_REVIEW',lease_token=NULL,lease_until=0,error=?
                          WHERE work_id=? AND chapter=? AND page=? AND lease_token=?""",
                       (f"{type(exc).__name__}:{str(exc)[:160]}", claimed["work_id"], claimed["chapter"],
                        claimed["page"], claimed["lease_token"]))

    def rows(self, work_id: str) -> list[dict]:
        with self.conn() as db:
            return [dict(r) for r in db.execute("SELECT * FROM source_pages WHERE work_id=? ORDER BY chapter,page", (work_id,))]

    def status(self, work_id: str) -> dict:
        rows = self.rows(work_id)
        return {"work_id": work_id, "total": len(rows), "states":
                {s: sum(r["state"] == s for r in rows) for s in sorted({r["state"] for r in rows})},
                "issues": [{"chapter": r["chapter"], "page": r["page"], "code": r["error"]}
                           for r in rows if r["error"]]}


class UnifiedPipeline:
    """Operate on chosen source pages, returning a complete immutable work receipt."""
    def __init__(self, ledger: PageLedger, sink: FileObjectSink | S3ObjectSink,
                 fixture_dir: str | Path, stage_dir: str | Path,
                 *, multipart_journal: str | Path | None = None) -> None:
        self.ledger, self.sink = ledger, sink
        self.fixture_dir = Path(fixture_dir).resolve()
        self.stage_dir = Path(stage_dir)
        self.publisher = StreamingWorkPublisher(sink, multipart_journal=multipart_journal)

    def _find_page(self, plan: WorkPlan, chapter: int, page: int) -> PageSpec:
        return plan.chapters[chapter-1].pages[page-1]

    def _raw_input(self, spec: PageSpec) -> tuple[bytes, str | None]:
        if spec.filename is not None:
            path = (self.fixture_dir/spec.filename).resolve()
            if path.parent != self.fixture_dir or not path.is_file():
                raise NeedsReview("fixture path not allowed")
            if path.stat().st_size > MAX_RAW:
                raise NeedsReview("fixture page exceeds cap")
            body = path.read_bytes()
            if not body:
                raise NeedsReview("empty source page")
            return body, None
        assert spec.raw_key and spec.raw_sha256
        body = self.sink.get(spec.raw_key)
        if _sha(body) != spec.raw_sha256:
            raise ObjectConflict("preloaded RAW sha256 mismatch")
        return body, spec.raw_key

    def fetch(self, plan: WorkPlan, *, crash_after_raw_put: bool = False) -> int:
        count = 0
        while claimed := self.ledger.claim(plan.source_id, "READY"):
            try:
                p = self._find_page(plan, claimed["chapter"], claimed["page"])
                body, preloaded = self._raw_input(p)
                if not body or len(body) > MAX_RAW:
                    raise NeedsReview("invalid source page size")
                sha = _sha(body)
                key = preloaded or f"raw/{plan.source_id}/{plan.manifest_sha256[:16]}/{claimed['chapter']:04}-{claimed['page']:04}-{sha}.html"
                if not preloaded:
                    stored_sha = self.sink.put_verified(key, body)
                    if stored_sha != sha:
                        raise ObjectConflict("RAW put digest mismatch")
                if crash_after_raw_put:
                    raise InjectCrash("after RAW upload, before SQLite commit")
                self.ledger.commit(claimed, phase="RAW", key=key, sha=sha, size=len(body))
                count += 1
            except InjectCrash:
                raise
            except Exception as exc:
                self.ledger.fail(claimed, exc)
                raise
        return count

    def parse(self, plan: WorkPlan) -> int:
        count = 0
        while claimed := self.ledger.claim(plan.source_id, "RAW"):
            try:
                data = self.sink.get(claimed["raw_key"])
                if len(data) != claimed["raw_size"] or _sha(data) != claimed["raw_sha"]:
                    raise ObjectConflict("immutable RAW changed")
                text = parse_snapshot(data, source_id=plan.source_id,
                                      chapter=claimed["chapter"], page=claimed["page"])
                parsed = {"schema": "parsed-page-v1", "parser": PARSER_VERSION,
                          "work_id": plan.source_id, "chapter": claimed["chapter"],
                          "page": claimed["page"], "source_url": self._find_page(plan, claimed["chapter"], claimed["page"]).source_url,
                          "raw_sha256": claimed["raw_sha"], "text": text}
                blob = json.dumps(parsed, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
                sha = _sha(blob)
                key = f"parsed/{plan.source_id}/{plan.manifest_sha256[:16]}/{claimed['chapter']:04}-{claimed['page']:04}-{sha}.json"
                if self.sink.put_verified(key, blob) != sha:
                    raise ObjectConflict("PARSED put digest mismatch")
                self.ledger.commit(claimed, phase="PARSED", key=key, sha=sha)
                count += 1
            except Exception as exc:
                self.ledger.fail(claimed, exc)
                raise
        return count

    def _work(self, plan: WorkPlan) -> StreamWork:
        """Lazy S3-readback: at most one parsed page is materialized per iterator.

        Existing M4 spool_work consumes each source page once to a verified
        temporary disk spool before it attempts any irreversible publish.
        """
        rows = self.ledger.rows(plan.source_id)
        if len(rows) != plan.expected_pages or any(r["state"] != "PARSED" for r in rows):
            raise NeedsReview("cannot assemble incomplete selection")
        by_id = {(r["chapter"], r["page"]): r for r in rows}

        def page_iter(chapter: ChapterSpec):
            for p in chapter.pages:
                r = by_id[(chapter.ordinal, p.page)]
                blob = self.sink.get(r["parsed_key"])
                if _sha(blob) != r["parsed_sha"]:
                    raise ObjectConflict("PARSED receipt changed")
                d = json.loads(blob)
                if (d.get("schema") != "parsed-page-v1" or d.get("parser") != PARSER_VERSION or
                    d.get("work_id") != plan.source_id or d.get("chapter") != chapter.ordinal or
                    d.get("page") != p.page or d.get("source_url") != p.source_url or
                    d.get("raw_sha256") != r["raw_sha"]):
                    raise NeedsReview("parsed provenance mismatch")
                text = d.get("text")
                if not isinstance(text, str) or not text.strip():
                    raise NeedsReview("empty parsed text")
                # The page iterator is consumed before the next page is read.
                yield StreamPage(p.page, p.source_url,
                                 (text[i:i + 32768] for i in range(0, len(text), 32768)))

        chapters = (StreamChapter(c.ordinal, c.title, len(c.pages), page_iter(c))
                    for c in plan.chapters)
        return StreamWork(plan.source_id, plan.title, plan.author, plan.source_url,
                          plan.language, plan.tags, plan.source_tags, plan.source_categories,
                          len(plan.chapters), chapters)

    def publish(self, plan: WorkPlan, *, expected_revision: str | None = None) -> dict:
        stream = self._work(plan)
        result = self.publisher.publish(stream, expected_revision=expected_revision,
                                        stage_dir=self.stage_dir)
        verified = self.publisher.verify_current(plan.source_id)
        if verified["revision_id"] != result["revision_id"]:
            raise PublicationUncertain("published manifest changed")
        # Neutral read-only handoff: neither ReadEra imports nor another product publishes automatically.
        rows = self.ledger.rows(plan.source_id)
        handoff = {"schema": "verified-source-handoff-v1", "scope": "selected_manifest_only",
                   "work_id": plan.source_id, "source_url": plan.source_url,
                   "title": plan.title, "author": plan.author, "language": plan.language,
                   "tags": list(plan.tags), "source_tags": list(plan.source_tags),
                   "source_categories": list(plan.source_categories),
                   "manifest_sha256": plan.manifest_sha256,
                   "revision_id": result["revision_id"], "artifact_manifest_key": verified["manifest_key"],
                   "chapter_count": len(plan.chapters), "pages": len(rows),
                   "chapters": [{"ordinal": c.ordinal, "title": c.title,
                                 "pages": len(c.pages)} for c in plan.chapters],
                   "items": [{"chapter": r["chapter"], "page": r["page"], "raw_key": r["raw_key"],
                              "raw_sha256": r["raw_sha"], "parsed_key": r["parsed_key"],
                              "parsed_sha256": r["parsed_sha"]} for r in rows],
                   "requires_actor_confirmation": True, "translation_started": False,
                   "publication_permitted": False}
        blob = json.dumps(handoff, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
        key = f"handoffs/{plan.source_id}/{result['revision_id']}.json"
        self.sink.put_verified(key, blob)
        return {**result, "handoff_key": key, "handoff_sha256": _sha(blob),
                "pages_verified": len(rows), "selected_manifest_sha256": plan.manifest_sha256}

    def run(self, plan: WorkPlan, *, stop_after: str | None = None) -> dict:
        if stop_after not in (None, "fetch", "parse"):
            raise ValueError("bad stop stage")
        self.ledger.register(plan)
        fetched = self.fetch(plan)
        if stop_after == "fetch":
            return {"status": "RAW_READY", "fetched": fetched, **self.ledger.status(plan.source_id)}
        parsed = self.parse(plan)
        if stop_after == "parse":
            return {"status": "PARSED_READY", "fetched": fetched, "parsed": parsed,
                    **self.ledger.status(plan.source_id)}
        receipt = self.publish(plan)
        return {**receipt, "fetched": fetched, "parsed": parsed}
