"""M2: deterministic, synthetic-only work assembler and immutable publisher.

No Literotica HTTP and no implicit synchronization with Akula. Only fixture.invalid
inputs are accepted. The latest pointer is conditional; old versions never change.
"""
from __future__ import annotations

import hashlib
import html
import json
import re
from dataclasses import dataclass
from typing import Protocol

from .model import canonical_fixture_url, require_entity_id
from .objects import ObjectConflict, ObjectWriteUncertain

MAX_OBJECT_BYTES = 2 * 1024 * 1024
MAX_PAGES = 3000
MAX_CHAPTERS = 1000
_TAG_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}(?:/[a-z0-9][a-z0-9_-]{0,63})*$")


class IncompleteWork(ValueError):
    """Validation failed; no version should be published."""


class CurrentConflict(RuntimeError):
    """Concurrent/stale publisher must not replace a newer current revision."""


class PublicationUncertain(RuntimeError):
    """A write may have happened; retry with the identical draft and expectation."""


class WorkObjectSink(Protocol):
    def put_verified(self, key: str, data: bytes) -> str: ...
    def get(self, key: str) -> bytes: ...
    def cas_verified(self, key: str, data: bytes, expected: bytes | None) -> str: ...


@dataclass(frozen=True)
class ChapterPage:
    number: int
    text: str
    source_url: str


@dataclass(frozen=True)
class Chapter:
    ordinal: int
    title: str
    pages: tuple[ChapterPage, ...]


@dataclass(frozen=True)
class Work:
    source_id: str
    title: str
    author: str
    source_url: str
    language: str
    tags: tuple[str, ...]
    source_tags: tuple[str, ...]
    source_categories: tuple[str, ...]
    chapters: tuple[Chapter, ...]


@dataclass(frozen=True)
class Bundle:
    work_id: str
    revision_id: str
    root: str
    files: tuple[tuple[str, bytes], ...]
    manifest: bytes
    pointer: bytes

    @property
    def manifest_key(self) -> str:
        return f"{self.root}/manifest.json"

    @property
    def pointer_key(self) -> str:
        return f"works/{self.work_id}/current.json"


def _json_bytes(doc: dict) -> bytes:
    return (json.dumps(doc, sort_keys=True, ensure_ascii=False, separators=(",", ":"),
                       allow_nan=False) + "\n").encode("utf-8")


def _sha(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()


def _clean_line(value: str, name: str, limit: int = 500) -> str:
    if (not isinstance(value, str) or not value.strip() or len(value) > limit or
            any(ord(c) < 32 or ord(c) == 127 or 0xD800 <= ord(c) <= 0xDFFF for c in value)):
        raise IncompleteWork(f"invalid {name}")
    return value.strip()


def _page_text(value: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise IncompleteWork("empty/missing page body")
    body = value.replace("\r\n", "\n").replace("\r", "\n").strip("\n")
    if "\x00" in body or any(0xD800 <= ord(c) <= 0xDFFF for c in body):
        raise IncompleteWork("invalid page body")
    encoded = body.encode("utf-8")
    if not encoded or len(encoded) > MAX_OBJECT_BYTES:
        raise IncompleteWork("page body exceeds safety limit")
    return body


def _safe_heading(value: str) -> str:
    # Escape markdown heading controls so titles stay a single heading.
    return html.escape(re.sub(r"([\\`*\[\]_#])", r"\\\1", value), quote=False)


def _check_size(key: str, blob: bytes) -> None:
    if len(blob) > MAX_OBJECT_BYTES:
        raise IncompleteWork(f"{key} exceeds M2 small-object limit (multipart not yet supported)")


def assemble(work: Work) -> Bundle:
    """Validate the entire input before doing I/O, then deterministically assemble."""
    try:
        ident = require_entity_id(work.source_id)
        source_url = canonical_fixture_url(work.source_url)
    except (ValueError, TypeError) as exc:
        raise IncompleteWork("unsafe source id or non-fixture URL") from exc
    title = _clean_line(work.title, "title")
    author = _clean_line(work.author, "author")
    language = _clean_line(work.language, "language", 25).lower()
    if not re.fullmatch(r"[a-z]{2,3}(?:-[a-z]{2})?", language):
        raise IncompleteWork("unsupported fixture language")
    if not isinstance(work.tags, tuple) or any(not isinstance(t, str) or not _TAG_RE.fullmatch(t) for t in work.tags):
        raise IncompleteWork("invalid normalized tags")
    if len(work.tags) != len(set(work.tags)) or len(work.tags) > 100:
        raise IncompleteWork("duplicate/excess tags")
    if not isinstance(work.source_tags, tuple) or not isinstance(work.source_categories, tuple):
        raise IncompleteWork("invalid source labels")
    source_tags = tuple(_clean_line(t, "source tag", 200) for t in work.source_tags)
    categories = tuple(_clean_line(t, "source category", 200) for t in work.source_categories)
    if len(source_tags) > 500 or len(categories) > 100:
        raise IncompleteWork("excess source metadata")
    if not isinstance(work.chapters, tuple) or not 1 <= len(work.chapters) <= MAX_CHAPTERS:
        raise IncompleteWork("missing/excess chapters")
    if tuple(ch.ordinal for ch in work.chapters) != tuple(range(1, len(work.chapters) + 1)):
        raise IncompleteWork("missing/duplicate/out-of-order chapter")

    source_pages: set[str] = set()
    md = [
        "---", "type: book",
        "title: " + json.dumps(title, ensure_ascii=False),
        "author: " + json.dumps(author, ensure_ascii=False),
        "tags: " + json.dumps(list(work.tags), ensure_ascii=False),
        "source: literotica", "source_id: " + json.dumps(ident, ensure_ascii=False),
        "language: " + json.dumps(language), "---", "", "# " + _safe_heading(title), "",
    ]
    files: list[tuple[str, bytes]] = []
    recorded_chapters = []
    total_page_count = 0
    for chapter in work.chapters:
        chapter_title = _clean_line(chapter.title, "chapter title")
        if not isinstance(chapter.pages, tuple) or not chapter.pages:
            raise IncompleteWork("missing chapter pages")
        if total_page_count + len(chapter.pages) > MAX_PAGES:
            raise IncompleteWork("too many pages")
        if tuple(page.number for page in chapter.pages) != tuple(range(1, len(chapter.pages) + 1)):
            raise IncompleteWork("missing/duplicate/out-of-order chapter page")
        total_page_count += len(chapter.pages)
        page_meta = []
        parts = []
        for page in chapter.pages:
            try:
                url = canonical_fixture_url(page.source_url)
            except (ValueError, TypeError) as exc:
                raise IncompleteWork("chapter page is not a fixture URL") from exc
            # Require the explicit page number when applicable; avoid two URLs
            # referring to a single page and false completeness.
            from urllib.parse import parse_qs, urlsplit
            url_page = int(parse_qs(urlsplit(url).query).get("page", ["1"])[0])
            if url_page != page.number:
                raise IncompleteWork("source URL page number disagrees with page")
            if url in source_pages:
                raise IncompleteWork("duplicate source page URL across chapters")
            source_pages.add(url)
            body = _page_text(page.text)
            if body in parts:
                raise IncompleteWork("duplicate page body in chapter")
            parts.append(body)
            page_meta.append({"number": page.number, "source_url": url,
                              "text_sha256": _sha(body.encode("utf-8")),
                              "chars": len(body)})
        chapter_text = "\n\n".join(html.escape(part, quote=False) for part in parts)
        chapter_md = f"# {_safe_heading(chapter_title)}\n\n{chapter_text}\n"
        key = f"chapters/{chapter.ordinal:04d}.md"
        file_data = chapter_md.encode("utf-8")
        _check_size(key, file_data)
        files.append((key, file_data))
        md.extend(["## " + _safe_heading(chapter_title), "", chapter_text, ""])
        recorded_chapters.append({"ordinal": chapter.ordinal, "title": chapter_title,
                                  "file": key, "body_sha256": _sha(chapter_text.encode("utf-8")),
                                  "pages_expected": len(page_meta), "pages_verified": len(page_meta),
                                  "pages": page_meta})
    story_md = ("\n".join(md).rstrip("\n") + "\n").encode("utf-8")
    _check_size("story.md", story_md)
    metadata = {
        "schema_version": 1, "source": "literotica", "source_work_id": ident,
        "source_url": source_url, "kind": "story", "title": title,
        "authors": [{"display_name": author}], "language": language,
        "tags": list(work.tags), "source_tags": list(source_tags),
        "source_categories": list(categories), "chapters": recorded_chapters,
        "integrity": "verified_fixture", "coverage_scope": "all_fixture_pages",
        "parser_version": "synthetic-m2-v1",
    }
    metadata_blob = _json_bytes(metadata)
    _check_size("metadata.json", metadata_blob)
    files.insert(0, ("story.md", story_md))
    files.insert(1, ("metadata.json", metadata_blob))
    file_listing = [{"key": k, "size": len(v), "sha256": _sha(v)} for k, v in files]
    manifest = _json_bytes({"schema_version": 1, "source": "literotica", "work_id": ident,
                            "file_count": len(files), "files": file_listing,
                            "integrity": "verified_fixture", "coverage_scope": "all_fixture_pages",
                            "algorithm": "sha256"})
    _check_size("manifest.json", manifest)
    revision = _sha(manifest)  # content-addressed and no self-referential digest.
    root = f"works/{ident}/versions/{revision}"
    pointer = _json_bytes({"schema_version": 1, "work_id": ident,
                           "revision_id": revision, "manifest_sha256": _sha(manifest),
                           "manifest_key": root + "/manifest.json"})
    _check_size("current.json", pointer)
    return Bundle(ident, revision, root, tuple(files), manifest, pointer)


class WorkPublisher:
    def __init__(self, sink: WorkObjectSink):
        self.sink = sink

    def _current_bytes(self, key: str) -> bytes | None:
        try:
            return self.sink.get(key)
        except FileNotFoundError:
            return None
        except Exception as exc:
            response = getattr(exc, "response", None) or {}
            code = response.get("Error", {}).get("Code")
            if code in ("NoSuchKey", "NotFound", "404"):
                return None
            raise PublicationUncertain("cannot read current pointer") from exc

    def publish(self, work: Work, *, expected_revision: str | None) -> dict:
        """Caller must pass the revision it observed, or None for first publish."""
        bundle = assemble(work)  # Fail before any I/O on incomplete or unsafe input.
        previous = self._current_bytes(bundle.pointer_key)
        previous_rev = None
        if previous is not None:
            try:
                head = json.loads(previous)
                previous_rev = head["revision_id"]
                if head["work_id"] != bundle.work_id:
                    raise ValueError("wrong current object")
            except (ValueError, KeyError, TypeError) as exc:
                raise PublicationUncertain("invalid current pointer") from exc
        if previous_rev == bundle.revision_id:
            self.verify_bundle(bundle)
            return {"status": "UNCHANGED", "work_id": bundle.work_id, "revision_id": bundle.revision_id,
                    "pointer_key": bundle.pointer_key, "manifest_key": bundle.manifest_key,
                    "objects_verified": len(bundle.files) + 1}
        if previous_rev != expected_revision:
            raise CurrentConflict("current revision changed; expected revision is stale")

        # Every object is immutable and checked. If an ACK is lost after PUT,
        # recover from a matching readback instead of refetching the source.
        for relkey, blob in bundle.files + (("manifest.json", bundle.manifest),):
            key = bundle.root + "/" + relkey
            self._put_recoverable(key, blob)
        # The manifest is now valid. The pointer is the last and only mutable key.
        try:
            self.sink.cas_verified(bundle.pointer_key, bundle.pointer, previous)
        except ObjectConflict as exc:
            # May be a concurrent writer OR a lost acknowledgement. Re-check.
            after = self._current_bytes(bundle.pointer_key)
            if after != bundle.pointer:
                raise CurrentConflict("current pointer CAS failed") from exc
        except ObjectWriteUncertain as exc:
            after = self._current_bytes(bundle.pointer_key)
            if after != bundle.pointer:
                raise PublicationUncertain("pointer commit outcome unknown") from exc
        self.verify_bundle(bundle)
        return {"status": "PUBLISHED", "work_id": bundle.work_id, "revision_id": bundle.revision_id,
                "pointer_key": bundle.pointer_key, "manifest_key": bundle.manifest_key,
                "objects_verified": len(bundle.files) + 1}

    def _put_recoverable(self, key: str, blob: bytes) -> None:
        try:
            result = self.sink.put_verified(key, blob)
        except ObjectWriteUncertain as exc:
            try:
                got = self.sink.get(key)
            except Exception:
                raise PublicationUncertain(f"readback unavailable after writing {key}") from exc
            if got != blob or _sha(got) != _sha(blob):
                raise PublicationUncertain(f"uncertain/corrupt object {key}") from exc
        else:
            if result != _sha(blob):
                raise PublicationUncertain(f"incorrect upload checksum {key}")

    def verify_bundle(self, bundle: Bundle) -> None:
        for rel, data in bundle.files + (("manifest.json", bundle.manifest),):
            key = bundle.root + "/" + rel
            try:
                got = self.sink.get(key)
            except Exception as exc:
                raise PublicationUncertain(f"missing/cannot read object {key}") from exc
            if got != data or _sha(got) != _sha(data):
                raise ObjectConflict(f"corrupt or conflicting verified object {key}")

    def verify_current(self, work_id: str) -> dict:
        """External reader: validate manifest and each member by stored digest."""
        try:
            ident = require_entity_id(work_id)
            pointer_key = f"works/{ident}/current.json"
            pointer_bytes = self._current_bytes(pointer_key)
            if pointer_bytes is None:
                raise PublicationUncertain("unpublished work")
            pointer = json.loads(pointer_bytes)
            revision = pointer["revision_id"]
            if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{64}", revision):
                raise ValueError("invalid revision")
            expected_root = f"works/{ident}/versions/{revision}"
            if pointer.get("work_id") != ident or pointer.get("manifest_key") != expected_root + "/manifest.json":
                raise ValueError("unexpected pointer path")
            manifest_bytes = self.sink.get(pointer["manifest_key"])
            if _sha(manifest_bytes) != pointer["manifest_sha256"] or _sha(manifest_bytes) != revision:
                raise ObjectConflict("manifest digest mismatch")
            manifest = json.loads(manifest_bytes)
            if manifest["work_id"] != ident or manifest["file_count"] != len(manifest["files"]):
                raise ValueError("invalid manifest")
            for f in manifest["files"]:
                rel = f["key"]
                if rel not in ("story.md", "metadata.json") and not re.fullmatch(r"chapters/[0-9]{4}\.md", rel):
                    raise ValueError("invalid manifest path")
                data = self.sink.get(expected_root + "/" + rel)
                if len(data) != f["size"] or _sha(data) != f["sha256"]:
                    raise ObjectConflict(f"bad digest: {rel}")
        except (KeyError, IndexError, TypeError, json.JSONDecodeError, ValueError) as exc:
            raise PublicationUncertain("invalid published manifest or pointer") from exc
        return {"status": "VERIFIED", "work_id": ident, "revision_id": revision,
                "manifest_key": expected_root + "/manifest.json", "files_verified": len(manifest["files"])}
