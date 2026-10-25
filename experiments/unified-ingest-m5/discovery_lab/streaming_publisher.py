"""M4: replayable staged streaming publication of *synthetic* long works.

Only fixture.invalid provenance; no live website client. Streaming chunks are
bounded and all source pages and chapters must match explicitly declared counts.
The staged disk snapshot is completed before *any* remote object is published.
S3 objects are immutable; CAS of current.json happens strictly last.

Tradeoff: disk spool uses ~2x story bytes (chapters and combined text), but peak
assembler memory depends on input chunk size, not on total book size. A crash
before pointer publication leaves only immutable orphan objects, never a partial
visible current version. Real MinIO provider admission remains a separate gate.
"""
from __future__ import annotations

import hashlib
import html
import json
import os
import re
import shutil
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable
from urllib.parse import parse_qs, urlsplit

from .model import canonical_fixture_url, require_entity_id
from .literotica_snapshot import story_url, series_url, SnapshotRejected
from .multipart import MultipartFileUploader, StreamingObjectUploader, MiB, READBACK_BYTES, MAX_LAB_FILE_BYTES
from .objects import FileObjectSink, S3ObjectSink, ObjectConflict, ObjectWriteUncertain
from .publisher import (_TAG_RE, _json_bytes, _sha, _safe_heading, _clean_line,
                        IncompleteWork, CurrentConflict, PublicationUncertain, Work,
                        WorkPublisher, MAX_PAGES, MAX_CHAPTERS, MAX_OBJECT_BYTES)

MAX_INPUT_CHUNK = 256 * 1024
MAX_STAGED_BYTES = 1024 * MiB


@dataclass(frozen=True)
class StreamPage:
    number: int
    source_url: str
    chunks: Iterable[str]


@dataclass(frozen=True)
class StreamChapter:
    ordinal: int
    title: str
    expected_pages: int
    pages: Iterable[StreamPage]


@dataclass(frozen=True)
class StreamWork:
    source_id: str
    title: str
    author: str
    source_url: str
    language: str
    tags: tuple[str, ...]
    source_tags: tuple[str, ...]
    source_categories: tuple[str, ...]
    expected_chapters: int
    chapters: Iterable[StreamChapter]
    parser_profile: str = "fixture-chapter-html-v1"


@dataclass(frozen=True)
class StagedFile:
    key: str
    path: Path
    size: int
    sha256: str


@dataclass(frozen=True)
class StagedBundle:
    work_id: str
    revision_id: str
    root: str
    files: tuple[StagedFile, ...]  # story.md, metadata.json, chapters, manifest.json
    pointer: bytes

    @property
    def pointer_key(self):
        return f"works/{self.work_id}/current.json"

    @property
    def manifest_key(self):
        return self.root + "/manifest.json"


class _Writer:
    def __init__(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self.fp = path.open("xb")
        self.hash = hashlib.sha256()
        self.size = 0

    def write(self, data: bytes) -> None:
        if not isinstance(data, bytes):
            raise TypeError("bytes required")
        if self.size + len(data) > MAX_LAB_FILE_BYTES:
            raise IncompleteWork("staged file exceeds 512 MiB lab cap")
        self.fp.write(data)
        self.hash.update(data)
        self.size += len(data)

    def finish(self) -> StagedFile:
        self.fp.flush()
        os.fsync(self.fp.fileno())
        self.fp.close()
        return StagedFile(str(self.path.name), self.path, self.size, self.hash.hexdigest())

    def close(self) -> None:
        self.fp.close()


def _normalized_chunks(chunks: Iterable[str]):
    """CRLF-normalize across chunk boundaries; strip only boundary LFs."""
    if chunks is None:
        raise IncompleteWork("missing page source chunks")
    try:
        it = iter(chunks)
    except TypeError as exc:
        raise IncompleteWork("page chunks must be iterable") from exc
    pending_cr = False
    started = False
    pending_nl = 0
    for incoming in it:
        if not isinstance(incoming, str) or len(incoming.encode("utf-8", errors="surrogatepass")) > MAX_INPUT_CHUNK:
            raise IncompleteWork("invalid/oversized input chunk")
        if "\x00" in incoming or any(0xD800 <= ord(c) <= 0xDFFF for c in incoming):
            raise IncompleteWork("invalid page Unicode")
        if pending_cr:
            incoming = "\r" + incoming
            pending_cr = False
        if incoming.endswith("\r"):
            incoming = incoming[:-1]
            pending_cr = True
        chunk = incoming.replace("\r\n", "\n").replace("\r", "\n")
        if not started:
            chunk = chunk.lstrip("\n")
        if not chunk:
            continue
        core = chunk.rstrip("\n")
        nl = len(chunk) - len(core)
        if core:
            if pending_nl:
                yield "\n" * pending_nl
                pending_nl = 0
            yield core
            started = True
            pending_nl = nl
        elif started:
            pending_nl += nl
    if pending_cr:
        # This normalizes a dangling CR to LF, then strips trailing newlines.
        pass
    # Do not emit pending trailing LF; M2 uses strip("\n").


def _safe_source_url(url: str, profile: str, *, is_page: bool = False) -> str:
    if profile == "fixture-chapter-html-v1":
        return canonical_fixture_url(url)
    if profile != "literotica-saved-story-v1":
        raise ValueError("unknown source parser profile")
    # Provenance validation ONLY. Never fetch this URL.
    if is_page:
        story_url(url)
    else:
        try:
            story_url(url)
        except SnapshotRejected:
            series_url(url)
    return url


def _validate_head(work: StreamWork) -> tuple[str, str, str, str, str, tuple[str, ...], tuple[str, ...]]:
    try:
        ident = require_entity_id(work.source_id)
        source_url = _safe_source_url(work.source_url, work.parser_profile)
    except (ValueError, TypeError) as exc:
        raise IncompleteWork("unsafe ID or non-fixture URL") from exc
    title = _clean_line(work.title, "title")
    author = _clean_line(work.author, "author")
    language = _clean_line(work.language, "language", 25).lower()
    if not re.fullmatch(r"[a-z]{2,3}(?:-[a-z]{2})?", language):
        raise IncompleteWork("invalid language")
    if not isinstance(work.tags, tuple) or len(work.tags) > 100 or len(set(work.tags)) != len(work.tags):
        raise IncompleteWork("invalid tags")
    if any(not isinstance(t, str) or not _TAG_RE.fullmatch(t) for t in work.tags):
        raise IncompleteWork("invalid normalized tag")
    if not isinstance(work.source_tags, tuple) or not isinstance(work.source_categories, tuple):
        raise IncompleteWork("invalid source labels")
    source_tags = tuple(_clean_line(x, "source tag", 200) for x in work.source_tags)
    categories = tuple(_clean_line(x, "source category", 200) for x in work.source_categories)
    if len(source_tags) > 500 or len(categories) > 100:
        raise IncompleteWork("too many source labels")
    if type(work.expected_chapters) is not int or not 1 <= work.expected_chapters <= MAX_CHAPTERS:
        raise IncompleteWork("invalid declared chapter count")
    return ident, source_url, title, author, language, source_tags, categories


def spool_work(work: StreamWork, target: str | Path) -> StagedBundle:
    """Consume a possibly one-shot chapter/page iterator into immutable spool files.

    All validation completes locally; caller must not publish until this returns.
    If it raises, no remote key has been created.
    """
    ident, source_url, title, author, lang, source_tags, cats = _validate_head(work)
    base = Path(target)
    if base.exists():
        if not base.is_dir() or any(base.iterdir()):
            raise ValueError("stage directory must be empty")
    else:
        base.mkdir(parents=True)
    sources: set[str] = set()
    pages_total = 0
    stage_total = 0
    recorded_chapters = []
    finished: list[StagedFile] = []
    story = _Writer(base / "story.md")
    try:
        preamble = ["---", "type: book", "title: " + json.dumps(title, ensure_ascii=False),
                    "author: " + json.dumps(author, ensure_ascii=False),
                    "tags: " + json.dumps(list(work.tags), ensure_ascii=False),
                    "source: literotica", "source_id: " + json.dumps(ident, ensure_ascii=False),
                    "language: " + json.dumps(lang), "---", "", "# " + _safe_heading(title), ""]
        story.write(("\n".join(preamble) + "\n").encode("utf-8"))
        chapter_count = 0
        for chapter in work.chapters:
            chapter_count += 1
            if chapter_count > work.expected_chapters or chapter_count > MAX_CHAPTERS:
                raise IncompleteWork("unexpected chapter")
            if chapter.ordinal != chapter_count:
                raise IncompleteWork("missing or out-of-order chapter")
            if type(chapter.expected_pages) is not int or not 1 <= chapter.expected_pages <= MAX_PAGES:
                raise IncompleteWork("invalid declared page count")
            chapter_title = _clean_line(chapter.title, "chapter title")
            if chapter_count > 1:
                story.write(b"\n\n")
            story.write(("## " + _safe_heading(chapter_title) + "\n\n").encode("utf-8"))
            rel = f"chapters/{chapter_count:04d}.md"
            output = _Writer(base / rel)
            chapter_hash = hashlib.sha256()
            page_meta = []
            page_hashes = set()
            page_count = 0
            try:
                output.write(("# " + _safe_heading(chapter_title) + "\n\n").encode("utf-8"))
                for page in chapter.pages:
                    page_count += 1
                    pages_total += 1
                    if pages_total > MAX_PAGES or page_count > chapter.expected_pages:
                        raise IncompleteWork("excess pages")
                    if page.number != page_count:
                        raise IncompleteWork("missing/repeated/out-of-order page")
                    try:
                        url = _safe_source_url(page.source_url, work.parser_profile, is_page=True)
                        if work.parser_profile == "literotica-saved-story-v1":
                            url_page = story_url(url).page
                        else:
                            page_query = parse_qs(urlsplit(url).query).get("page", ["1"])
                            url_page = int(page_query[0])
                    except (ValueError, TypeError, IndexError) as exc:
                        raise IncompleteWork("unsafe page URL") from exc
                    if url_page != page_count or url in sources:
                        raise IncompleteWork("duplicate/inconsistent page URL")
                    sources.add(url)
                    if page_count > 1:
                        output.write(b"\n\n")
                        story.write(b"\n\n")
                        chapter_hash.update(b"\n\n")
                    body_hash = hashlib.sha256()
                    body_chars = 0
                    meaningful = False
                    for piece in _normalized_chunks(page.chunks):
                        meaningful = meaningful or bool(piece.strip())
                        body_chars += len(piece)
                        raw = piece.encode("utf-8")
                        body_hash.update(raw)
                        safe = html.escape(piece, quote=False).encode("utf-8")
                        output.write(safe)
                        story.write(safe)
                        chapter_hash.update(safe)
                    if not meaningful:
                        raise IncompleteWork("empty/whitespace-only page")
                    signature = (body_hash.hexdigest(), body_chars)
                    if signature in page_hashes:
                        raise IncompleteWork("duplicate page body")
                    page_hashes.add(signature)
                    page_meta.append({"number": page_count, "source_url": url,
                                      "text_sha256": body_hash.hexdigest(), "chars": body_chars})
                if page_count != chapter.expected_pages:
                    raise IncompleteWork("missing chapter pages")
                output.write(b"\n")
                st = output.finish()
                finished.append(StagedFile(rel, st.path, st.size, st.sha256))
                stage_total += st.size
                if stage_total + story.size > MAX_STAGED_BYTES:
                    raise IncompleteWork("total stage bytes exceeded")
                recorded_chapters.append({"ordinal": chapter_count, "title": chapter_title,
                                          "file": rel, "body_sha256": chapter_hash.hexdigest(),
                                          "pages_expected": page_count, "pages_verified": page_count,
                                          "pages": page_meta})
            finally:
                output.close()
        if chapter_count != work.expected_chapters:
            raise IncompleteWork("missing declared chapters")
        story.write(b"\n")
        st = story.finish()
        stage_total += st.size
        if stage_total > MAX_STAGED_BYTES:
            raise IncompleteWork("total stage bytes exceeded")
    finally:
        story.close()

    integrity, coverage = ("verified_fixture", "all_fixture_pages") if work.parser_profile == "fixture-chapter-html-v1" else ("verified_selected_snapshot", "selected_manifest_only")
    metadata = {"schema_version": 1, "source": "literotica", "source_work_id": ident,
                "source_url": source_url, "kind": "story", "title": title,
                "authors": [{"display_name": author}], "language": lang,
                "tags": list(work.tags), "source_tags": list(source_tags),
                "source_categories": list(cats), "chapters": recorded_chapters,
                "integrity": integrity, "coverage_scope": coverage,
                "parser_version": "synthetic-m2-v1" if work.parser_profile == "fixture-chapter-html-v1" else work.parser_profile}
    meta_blob = _json_bytes(metadata)
    if len(meta_blob) > MAX_OBJECT_BYTES:
        raise IncompleteWork("metadata exceeds configured small-object limit")
    meta_file = base / "metadata.json"
    with meta_file.open("xb") as out:
        out.write(meta_blob)
        out.flush(); os.fsync(out.fileno())
    finished.insert(0, StagedFile("story.md", base / "story.md", st.size, st.sha256))
    finished.insert(1, StagedFile("metadata.json", meta_file, len(meta_blob), _sha(meta_blob)))
    files = [{"key": x.key, "size": x.size, "sha256": x.sha256} for x in finished]
    manifest = _json_bytes({"schema_version": 1, "source": "literotica", "work_id": ident,
                            "file_count": len(files), "files": files,
                            "integrity": integrity, "coverage_scope": coverage,
                            "algorithm": "sha256"})
    if len(manifest) > MAX_OBJECT_BYTES:
        raise IncompleteWork("manifest exceeds configured small-object limit")
    manifest_path = base / "manifest.json"
    with manifest_path.open("xb") as out:
        out.write(manifest)
        out.flush(); os.fsync(out.fileno())
    revision = _sha(manifest)
    root = f"works/{ident}/versions/{revision}"
    pointer = _json_bytes({"schema_version": 1, "work_id": ident,
                           "revision_id": revision, "manifest_sha256": _sha(manifest),
                           "manifest_key": root + "/manifest.json"})
    if len(pointer) > MAX_OBJECT_BYTES:
        raise IncompleteWork("pointer too large")
    return StagedBundle(ident, revision, root,
                        tuple(finished + [StagedFile("manifest.json", manifest_path, len(manifest), _sha(manifest))]), pointer)


def _hash_file(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as inp:
        while block := inp.read(READBACK_BYTES):
            digest.update(block)
            size += len(block)
            if size > MAX_LAB_FILE_BYTES:
                raise ObjectConflict("file exceeds lab cap")
    return digest.hexdigest(), size


class StreamingWorkPublisher:
    """Integration: staged assembly + small PUT or resumable S3 multipart + CAS."""

    def __init__(self, sink: FileObjectSink | S3ObjectSink, *,
                 multipart_journal: str | Path | None = None) -> None:
        if not isinstance(sink, (FileObjectSink, S3ObjectSink)):
            raise TypeError("unsupported sink")
        self.sink = sink
        self.small = WorkPublisher(sink)
        self.multipart = (MultipartFileUploader(sink, multipart_journal) if isinstance(sink, S3ObjectSink)
                          and multipart_journal is not None else None)
        self.stream = StreamingObjectUploader(sink) if isinstance(sink, S3ObjectSink) else None
        if isinstance(sink, S3ObjectSink) and multipart_journal is None:
            raise ValueError("S3 publisher requires a durable multipart journal")

    def _verify_file(self, key: str, staged: StagedFile):
        digest = hashlib.sha256()
        size = 0
        if isinstance(self.sink, FileObjectSink):
            reader = self.sink._path(key).open("rb")
        else:
            try:
                reader = self.sink.client.get_object(Bucket=self.sink.bucket,
                              Key=self.sink.prefix + key)["Body"]
            except Exception as exc:
                raise PublicationUncertain(f"remote object unavailable {key}") from exc
        try:
            while data := reader.read(READBACK_BYTES):
                size += len(data)
                if size > MAX_LAB_FILE_BYTES:
                    raise ObjectConflict("remote object oversize")
                digest.update(data)
        except Exception as exc:
            if isinstance(exc, ObjectConflict):
                raise
            raise PublicationUncertain(f"streamed readback failed {key}") from exc
        finally:
            reader.close()
        if (size, digest.hexdigest()) != (staged.size, staged.sha256):
            raise ObjectConflict(f"bad stored size or digest: {key}")

    def _file_put_verified(self, key: str, source: Path):
        """Create-only disk object, copy in 256KiB chunks and verify without big RAM."""
        out = self.sink._path(key)
        out.parent.mkdir(parents=True, exist_ok=True)
        temp = out.with_name(out.name + ".partial-" + uuid.uuid4().hex)
        try:
            with temp.open("xb") as fout, source.open("rb") as fin:
                shutil.copyfileobj(fin, fout, READBACK_BYTES)
                fout.flush(); os.fsync(fout.fileno())
            try:
                os.link(temp, out)
                dirfd = os.open(out.parent, os.O_RDONLY)
                try: os.fsync(dirfd)
                finally: os.close(dirfd)
            except FileExistsError:
                pass
        finally:
            temp.unlink(missing_ok=True)

    def _put_stage(self, bundle: StagedBundle, item: StagedFile):
        key = bundle.root + "/" + item.key
        if _hash_file(item.path) != (item.sha256, item.size):
            raise IncompleteWork("spool data modified before remote upload")
        if isinstance(self.sink, FileObjectSink):
            self._file_put_verified(key, item.path)
        elif item.size <= MAX_OBJECT_BYTES:
            self.sink.put_verified(key, item.path.read_bytes())
        elif item.size < 5 * MiB:
            with item.path.open("rb") as f:
                r = self.stream.put_stream(key, f, content_type="text/markdown; charset=utf-8" if key.endswith(".md") else "application/json")
                if (r["size"], r["sha256"]) != (item.size, item.sha256):
                    raise ObjectConflict("stream input digest mismatch")
        else:
            r = self.multipart.upload_file(key, item.path, content_type="text/markdown; charset=utf-8" if key.endswith(".md") else "application/json")
            if (r["size"], r["sha256"]) != (item.size, item.sha256):
                raise ObjectConflict("multipart input digest mismatch")
        self._verify_file(key, item)

    def verify_current(self, work_id: str):
        """Check pointer, manifest and every artifact in bounded memory."""
        ident = require_entity_id(work_id)
        pointer = self.small._current_bytes(f"works/{ident}/current.json")
        if pointer is None:
            raise PublicationUncertain("work is not published")
        try:
            doc = json.loads(pointer)
            revision = doc["revision_id"]
            if not isinstance(revision, str) or not re.fullmatch("[a-f0-9]{64}", revision):
                raise ValueError("bad revision")
            root = f"works/{ident}/versions/{revision}"
            manifest_key = root + "/manifest.json"
            if doc["work_id"] != ident or doc["manifest_key"] != manifest_key:
                raise ValueError("bad pointer")
            manifest_blob = self.sink.get(manifest_key)
            if _sha(manifest_blob) != revision or _sha(manifest_blob) != doc["manifest_sha256"]:
                raise ObjectConflict("manifest corrupt")
            manifest = json.loads(manifest_blob)
            files = manifest["files"]
            if manifest.get("work_id") != ident or manifest.get("file_count") != len(files):
                raise ValueError("wrong manifest")
            keys = set()
            for node in files:
                rel = node["key"]
                if (rel in keys or (rel not in ("story.md", "metadata.json")
                        and not re.fullmatch(r"chapters/[0-9]{4}\.md", rel))):
                    raise ValueError("bad manifest entries")
                keys.add(rel)
                self._verify_file(root + "/" + rel, StagedFile(rel, Path(), node["size"], node["sha256"]))
            if "story.md" not in keys or "metadata.json" not in keys:
                raise ValueError("missing required artifacts")
        except (TypeError, ValueError, KeyError, json.JSONDecodeError) as exc:
            raise PublicationUncertain("pointer/manifest malformed") from exc
        return {"status":"VERIFIED", "work_id":ident, "revision_id":revision,
                "files_verified":len(files), "manifest_key":manifest_key}

    def publish(self, work: StreamWork, *, expected_revision: str | None, stage_dir: str | Path) -> dict:
        """Stage all data first; only after validation and verified immutable writes CAS head."""
        path = Path(stage_dir)
        path.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="m4-spool-", dir=path) as temp:
            bundle = spool_work(work, Path(temp) / "artifacts")
            previous = self.small._current_bytes(bundle.pointer_key)
            previous_rev = None
            if previous is not None:
                try:
                    doc = json.loads(previous)
                    if doc["work_id"] != bundle.work_id:
                        raise ValueError("wrong work id")
                    previous_rev = doc["revision_id"]
                except (ValueError, TypeError, KeyError) as exc:
                    raise PublicationUncertain("invalid current pointer") from exc
            if previous_rev == bundle.revision_id:
                self.verify_current(bundle.work_id)
                return {"status":"UNCHANGED", "work_id":bundle.work_id,
                        "revision_id":bundle.revision_id, "files_verified":len(bundle.files)-1}
            if previous_rev != expected_revision:
                raise CurrentConflict("stale current revision")
            # Commit manifest last among immutable version objects.
            for item in bundle.files:
                self._put_stage(bundle, item)
            # Only now is a new version visible through the mutable head pointer.
            try:
                self.sink.cas_verified(bundle.pointer_key, bundle.pointer, previous)
            except (ObjectConflict, ObjectWriteUncertain) as exc:
                after = self.small._current_bytes(bundle.pointer_key)
                if after != bundle.pointer:
                    if isinstance(exc, ObjectConflict):
                        raise CurrentConflict("CAS rejected concurrent update") from exc
                    raise PublicationUncertain("pointer commit outcome unknown") from exc
            receipt = self.verify_current(bundle.work_id)
            return {"status":"PUBLISHED", "work_id":bundle.work_id,
                    "revision_id":bundle.revision_id, "files_verified":receipt["files_verified"],
                    "manifest_key":receipt["manifest_key"]}


def stream_from_work(work: Work, *, chunk_size: int = MAX_INPUT_CHUNK // 4) -> StreamWork:
    """Compatibility helper for existing M2 fixtures (not a RAM optimization)."""
    if not isinstance(chunk_size, int) or not 1 <= chunk_size <= MAX_INPUT_CHUNK // 4:
        raise ValueError("chunk size invalid")
    chapters = (StreamChapter(ch.ordinal, ch.title, len(ch.pages),
                (StreamPage(p.number, p.source_url,
                            (p.text[i:i + chunk_size] for i in range(0, len(p.text), chunk_size)))
                 for p in ch.pages)) for ch in work.chapters)
    return StreamWork(work.source_id, work.title, work.author, work.source_url,
                      work.language, work.tags, work.source_tags, work.source_categories,
                      len(work.chapters), chapters)
