"""M6: verified, read-only bridge from M5 immutable artifacts to Erida HTML import.

No Erida API calls, staging, translation or publication.  Erida's own
POST /v1/imports/stages and explicit /confirm remain the only write route.
"""
from __future__ import annotations

import hashlib
import html
import json
import os
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from .unified_pipeline import NeedsReview
from .publisher import _safe_heading

ERIDA_MAX_UPLOAD = 12 * 1024 * 1024
DEFAULT_BATCH_BYTES = 10 * 1024 * 1024
# M5's current S3ObjectSink.get supports <=2 MiB per read. Reject, don't truncate.
MAX_CHAPTER_BYTES = 2 * 1024 * 1024
HEX = re.compile(r"[a-f0-9]{64}\Z")
IDENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,99}\Z")


class ObjectReader(Protocol):
    def get(self, key: str) -> bytes: ...


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def checked_json(reader: ObjectReader, key: str, *, expected: str | None = None,
                 limit: int = 2 * 1024 * 1024) -> dict:
    data = reader.get(key)
    if not isinstance(data, bytes) or not data or len(data) > limit:
        raise NeedsReview("oversized or missing immutable JSON")
    if expected is not None and digest(data) != expected:
        raise NeedsReview("immutable object SHA-256 mismatch")
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise NeedsReview("invalid immutable JSON") from exc
    if not isinstance(value, dict):
        raise NeedsReview("immutable JSON must be an object")
    return value


def _chapter_html(markdown: bytes, chapter_no: int, title: str) -> bytes:
    try:
        source = markdown.decode("utf-8", "strict")
    except UnicodeDecodeError as exc:
        raise NeedsReview("chapter has invalid UTF-8") from exc
    if len(markdown) > MAX_CHAPTER_BYTES or not source.strip():
        raise NeedsReview("chapter is empty or too large for Erida staged HTML")
    expected = "# " + _safe_heading(title)
    if not source.startswith(expected + "\n"):
        raise NeedsReview("chapter title differs from verified source plan")
    body = source[len(expected):].strip("\n")
    if not body.strip() or "\x00" in body:
        raise NeedsReview("chapter has no verified text")
    blocks = []
    for paragraph in re.split(r"\n\s*\n", body):
        if paragraph.strip():
            blocks.append("<p>" + "<br>".join(html.escape(line) for line in paragraph.split("\n")) + "</p>")
    if not blocks:
        raise NeedsReview("chapter contains no paragraphs")
    heading = f"<h1>Глава {chapter_no}. {html.escape(title)}</h1>"
    return (heading + "\n" + "\n".join(blocks) + "\n").encode("utf-8")


@dataclass(frozen=True)
class VerifiedChapter:
    ordinal: int
    title: str
    html_bytes: bytes
    md_sha256: str


def verify_handoff(reader: ObjectReader, handoff_key: str, handoff_sha256: str) -> tuple[dict, list[VerifiedChapter]]:
    """Fail closed on mismatched selected manifest, all source receipts and chapters.

    This is NOT a statement of full site coverage. It verifies only selected pages.
    """
    if not isinstance(handoff_sha256, str) or not HEX.fullmatch(handoff_sha256):
        raise NeedsReview("expected immutable handoff sha256")
    handoff = checked_json(reader, handoff_key, expected=handoff_sha256)
    if (handoff.get("schema") != "verified-source-handoff-v1" or
        handoff.get("scope") != "selected_manifest_only" or
        handoff.get("requires_actor_confirmation") is not True or
        handoff.get("publication_permitted") is not False or
        handoff.get("translation_started") is not False):
        raise NeedsReview("handoff is not approved for read-only export")
    work_id = handoff.get("work_id")
    revision = handoff.get("revision_id")
    if not isinstance(work_id, str) or not IDENT.fullmatch(work_id):
        raise NeedsReview("unsafe work ID")
    if not isinstance(revision, str) or not HEX.fullmatch(revision):
        raise NeedsReview("invalid revision ID")
    if handoff_key != f"handoffs/{work_id}/{revision}.json":
        raise NeedsReview("handoff key does not match claimed work and revision")
    selected_hash = handoff.get("manifest_sha256")
    if not isinstance(selected_hash, str) or not HEX.fullmatch(selected_hash):
        raise NeedsReview("invalid selected catalog hash")
    chapters = handoff.get("chapters")
    items = handoff.get("items")
    if (not isinstance(chapters, list) or not chapters or len(chapters) > 1000 or
        not isinstance(items, list) or not 1 <= len(items) <= 3000 or
        handoff.get("chapter_count") != len(chapters) or handoff.get("pages") != len(items)):
        raise NeedsReview("incomplete handoff inventory")
    expected_pairs = []
    for ordinal, chapter in enumerate(chapters, 1):
        if (not isinstance(chapter, dict) or chapter.get("ordinal") != ordinal or
            not isinstance(chapter.get("title"), str) or not chapter["title"].strip() or
            not isinstance(chapter.get("pages"), int) or chapter["pages"] < 1):
            raise NeedsReview("invalid chapter inventory")
        expected_pairs.extend((ordinal, p) for p in range(1, chapter["pages"] + 1))
    if len(expected_pairs) != len(items):
        raise NeedsReview("chapter/page inventory count mismatch")
    for expected_pair, item in zip(expected_pairs, items):
        if not isinstance(item, dict) or (item.get("chapter"), item.get("page")) != expected_pair:
            raise NeedsReview("page receipts reordered, duplicated or missing")
        raw_key, parsed_key = item.get("raw_key"), item.get("parsed_key")
        raw_hash, parsed_hash = item.get("raw_sha256"), item.get("parsed_sha256")
        if (not isinstance(raw_key, str) or not raw_key.startswith(f"raw/{work_id}/") or
            not isinstance(parsed_key, str) or not parsed_key.startswith(f"parsed/{work_id}/") or
            not all(isinstance(v, str) and HEX.fullmatch(v) for v in (raw_hash, parsed_hash))):
            raise NeedsReview("untrusted RAW/parsed receipt")
        raw = reader.get(raw_key)
        if not isinstance(raw, bytes) or len(raw) > 1024 * 1024 or digest(raw) != raw_hash:
            raise NeedsReview("RAW receipt verification failed")
        parsed = checked_json(reader, parsed_key, expected=parsed_hash)
        if (parsed.get("schema") != "parsed-page-v1" or parsed.get("work_id") != work_id or
            (parsed.get("chapter"), parsed.get("page")) != expected_pair or
            parsed.get("raw_sha256") != raw_hash or not isinstance(parsed.get("text"), str) or
            not parsed["text"].strip()):
            raise NeedsReview("parsed receipt does not match RAW/page")
    manifest_key = f"works/{work_id}/versions/{revision}/manifest.json"
    if handoff.get("artifact_manifest_key") != manifest_key:
        raise NeedsReview("manifest key is not bound to handoff revision")
    manifest = checked_json(reader, manifest_key, expected=revision)
    if (manifest.get("schema_version") != 1 or manifest.get("algorithm") != "sha256" or
        manifest.get("work_id") != work_id):
        raise NeedsReview("unsupported artifact manifest")
    records = manifest.get("files")
    required = {"metadata.json", "story.md"} | {f"chapters/{n:04}.md" for n in range(1, len(chapters) + 1)}
    if (not isinstance(records, list) or manifest.get("file_count") != len(required) or
        len(records) != len(required)):
        raise NeedsReview("artifact inventory differs from expected chapters")
    files = {}
    for record in records:
        if not isinstance(record, dict):
            raise NeedsReview("invalid artifact descriptor")
        key = record.get("key")
        if key not in required or key in files or not isinstance(record.get("sha256"), str) or not HEX.fullmatch(record["sha256"]):
            raise NeedsReview("unexpected/duplicate artifact key")
        files[key] = record
    if set(files) != required:
        raise NeedsReview("missing artifact chapter")
    root = f"works/{work_id}/versions/{revision}/"

    def read_file(name: str) -> bytes:
        record = files[name]
        blob = reader.get(root + name)
        if (not isinstance(blob, bytes) or len(blob) != record.get("size") or
            digest(blob) != record["sha256"]):
            raise NeedsReview(f"artifact file corrupted: {name}")
        return blob

    metadata = checked_json(reader, root + "metadata.json", expected=files["metadata.json"]["sha256"])
    if (metadata.get("source_work_id") != work_id or metadata.get("source_url") != handoff.get("source_url") or
        metadata.get("title") != handoff.get("title") or metadata.get("language") != handoff.get("language") or
        not isinstance(metadata.get("chapters"), list) or len(metadata["chapters"]) != len(chapters)):
        raise NeedsReview("published metadata does not match selected handoff")
    # Validate the combined artifact too: the immutable manifest covers the entire published work.
    read_file("story.md")
    verified = []
    for chapter, md_meta in zip(chapters, metadata["chapters"]):
        ordinal, title, expected_pages = chapter["ordinal"], chapter["title"], chapter["pages"]
        if (md_meta.get("ordinal") != ordinal or md_meta.get("title") != title or
            md_meta.get("pages_verified") != expected_pages or md_meta.get("pages_expected") != expected_pages or
            len(md_meta.get("pages", [])) != expected_pages):
            raise NeedsReview("published chapter metadata incomplete")
        name = f"chapters/{ordinal:04}.md"
        body = read_file(name)
        rendered = _chapter_html(body, ordinal, title)
        verified.append(VerifiedChapter(ordinal, title, rendered, files[name]["sha256"]))
    return handoff, verified


def prepare_erida_batches(reader: ObjectReader, handoff_key: str, handoff_sha256: str,
                          output_dir: str | Path, *, max_batch_bytes: int = DEFAULT_BATCH_BYTES) -> dict:
    """Write deterministic immutable HTML batches for manual Erida import/stage.

    Never makes an HTTP request and never confirms/stages any Erida session.
    """
    if type(max_batch_bytes) is not int or not 250 <= max_batch_bytes <= ERIDA_MAX_UPLOAD:
        raise ValueError("max_batch_bytes must be within [250, 12 MiB]")
    handoff, chapters = verify_handoff(reader, handoff_key, handoff_sha256)
    head = b"<!doctype html><html><head><meta charset='utf-8'></head><body>\n"
    tail = b"</body></html>\n"
    batch_data = []
    data = bytearray(head)
    ids: list[int] = []
    for chapter in chapters:
        if len(head) + len(chapter.html_bytes) + len(tail) > max_batch_bytes:
            raise NeedsReview(f"chapter {chapter.ordinal} exceeds maximum Erida upload size")
        if len(data) + len(chapter.html_bytes) + len(tail) > max_batch_bytes:
            batch_data.append((bytes(data) + tail, ids))
            data, ids = bytearray(head), []
        data.extend(chapter.html_bytes)
        ids.append(chapter.ordinal)
    if ids:
        batch_data.append((bytes(data) + tail, ids))
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    prefix = handoff["work_id"] + "-" + handoff["revision_id"][:16]
    staged = []
    with tempfile.TemporaryDirectory(prefix=".erida-m6-", dir=out) as tmp:
        temp = Path(tmp)
        for n, (body, chapter_ids) in enumerate(batch_data, 1):
            filename = f"{prefix}-batch-{n:04}.html"
            path = temp / filename
            path.write_bytes(body)
            staged.append((path, out / filename, body, chapter_ids))
        manifest = {
            "schema": "erida-import-batches-v1", "scope": "selected_manifest_only",
            "work_id": handoff["work_id"], "revision_id": handoff["revision_id"],
            "handoff_key": handoff_key, "handoff_sha256": handoff_sha256,
            "expected_chapters": len(chapters),
            "batches": [
                {"filename": path.name, "sha256": digest(body), "bytes": len(body),
                 "first_ordinal": ords[0], "last_ordinal": ords[-1], "chapter_count": len(ords)}
                for path, _, body, ords in staged
            ],
            "requires_actor_confirmation": True,
            "stage_automatically": False,
            "publish_automatically": False,
            "erida_api_preview": "POST /v1/imports/preview",
            "erida_api_stage": "POST /v1/imports/stages",
        }
        encoded = (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")
        manifest_name = f"{prefix}-import.json"
        (temp / manifest_name).write_bytes(encoded)
        for temp_path, final_path, body, _ in staged:
            try:
                os.link(temp_path, final_path)
            except FileExistsError:
                if final_path.read_bytes() != body:
                    raise NeedsReview("existing import batch conflicts with immutable output")
        target_manifest = out / manifest_name
        try:
            os.link(temp / manifest_name, target_manifest)
        except FileExistsError:
            if target_manifest.read_bytes() != encoded:
                raise NeedsReview("existing import manifest changed")
    return {"status": "PREPARED_NOT_STAGED", "manifest": str(target_manifest), **manifest}

