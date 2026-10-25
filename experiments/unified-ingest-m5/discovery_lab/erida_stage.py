"""Explicit, operator-initiated Erida staging, with NO confirmation or translation.

This is the only optional write-side boundary of the M5/M6/M7 bridge.
Requires an independent operator action, a bounded preflight of all batches,
then reuses the existing Erida /v1/imports/stages endpoint. Never invokes
/stages/{id}/confirm or modifies the application's BookLibrary directly.
"""
from __future__ import annotations

import hashlib
import json
import re
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path

from .erida_export import ERIDA_MAX_UPLOAD
from .erida_preview import NoRedirects, _safe_base, preview_erida_batches
from .unified_pipeline import NeedsReview

_NAME = re.compile(r"[A-Za-z0-9_.-]{1,180}\.html\Z")
_SHA = re.compile(r"[0-9a-f]{64}\Z")
_MAX_UNCOMMITTED_STAGES = 3  # Existing actor-scoped Erida ImportSessions limit.


@dataclass(frozen=True)
class _Batch:
    filename: str
    body: bytes
    first_ordinal: int
    last_ordinal: int
    count: int
    sha256: str


def _checked_batches(path: str | Path) -> tuple[dict, list[_Batch]]:
    manifest_file = Path(path).resolve()
    if not manifest_file.is_file() or manifest_file.stat().st_size > 1024 * 1024:
        raise NeedsReview("missing or oversized prepared import manifest")
    try:
        document = json.loads(manifest_file.read_bytes())
    except (UnicodeDecodeError, ValueError) as exc:
        raise NeedsReview("invalid prepared import manifest") from exc
    if (not isinstance(document, dict) or document.get("schema") != "erida-import-batches-v1"
            or document.get("scope") != "selected_manifest_only"
            or document.get("requires_actor_confirmation") is not True
            or document.get("stage_automatically") is not False
            or document.get("publish_automatically") is not False
            or document.get("erida_api_stage") != "POST /v1/imports/stages"):
        raise NeedsReview("manifest does not permit explicit actor-only staging")
    entries = document.get("batches")
    if not isinstance(entries, list) or not 1 <= len(entries) <= _MAX_UNCOMMITTED_STAGES:
        raise NeedsReview("Erida supports at most three unconfirmed stages per actor")
    expected_total = document.get("expected_chapters")
    if type(expected_total) is not int or not 1 <= expected_total <= 1000:
        raise NeedsReview("invalid selected chapter count")
    batches, expected_next, used_names = [], 1, set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise NeedsReview("invalid prepared batch entry")
        name = entry.get("filename")
        if (not isinstance(name, str) or not _NAME.fullmatch(name) or ".." in name
                or name in used_names):
            raise NeedsReview("unsafe or duplicate batch file")
        used_names.add(name)
        first, last, count = (entry.get(k) for k in ("first_ordinal", "last_ordinal", "chapter_count"))
        if (any(type(n) is not int for n in (first, last, count)) or first != expected_next
                or last < first or count != last - first + 1):
            raise NeedsReview("missing, overlapping or duplicated chapter ordinals")
        expected_next = last + 1
        size, hash_expected = entry.get("bytes"), entry.get("sha256")
        if (type(size) is not int or not 0 < size <= ERIDA_MAX_UPLOAD
                or not isinstance(hash_expected, str) or not _SHA.fullmatch(hash_expected)):
            raise NeedsReview("invalid batch size or digest")
        target = (manifest_file.parent / name).resolve()
        if target.parent != manifest_file.parent or not target.is_file() or target.stat().st_size != size:
            raise NeedsReview("missing, relocated or modified import batch")
        body = target.read_bytes()
        if hashlib.sha256(body).hexdigest() != hash_expected:
            raise NeedsReview("batch SHA-256 mismatch; refusing to stage")
        batches.append(_Batch(name, body, first, last, count, hash_expected))
    if expected_next - 1 != expected_total:
        raise NeedsReview("prepared export does not cover selected chapter inventory")
    return document, batches


def _validate_preview_entries(preview: dict, batch: _Batch, *, staged: bool) -> None:
    if not isinstance(preview, dict) or preview.get("kind") != "book":
        raise NeedsReview("Erida response is not a book import")
    if preview.get("chapter_count") != batch.count or preview.get("warnings") != []:
        raise NeedsReview("Erida disagrees with selected chapter inventory")
    if (staged and preview.get("stored") is not False) or (
            not staged and preview.get("stored") is not False):
        raise NeedsReview("Erida response claims unexpected storage")
    chapters = preview.get("chapters")
    expected = list(range(batch.first_ordinal, batch.last_ordinal + 1))
    if (not isinstance(chapters, list) or len(chapters) != len(expected)
            or any(not isinstance(ch, dict) for ch in chapters)
            or [ch.get("detected_number") for ch in chapters] != expected
            or any(ch.get("has_text") is not True for ch in chapters)):
        raise NeedsReview("Erida preview has missing/empty/reordered chapters")


def _multipart(batch: _Batch) -> tuple[bytes, str]:
    boundary = "M7-" + uuid.uuid4().hex
    header = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; "
              f"filename=\"{batch.filename}\"\r\nContent-Type: text/html; charset=utf-8\r\n\r\n").encode()
    return header + batch.body + (f"\r\n--{boundary}--\r\n").encode(), boundary


def stage_erida_batches(manifest_path: str | Path, *, erida_origin: str, api_key: str,
                        allow_stage: bool = False, timeout: int = 20) -> dict:
    """Stage source only after explicit operator opt-in; return redacted receipts.

    A stage is NOT a library import. The existing actor still must manually map
    chapters and confirm them in Erida. At most three uncommitted batches are
    staged; a failure yields a PARTIAL report with previously created stage IDs.
    """
    if allow_stage is not True:
        raise NeedsReview("staging requires explicit --allow-stage acknowledgment")
    if not isinstance(api_key, str) or not 1 <= len(api_key) <= 1024:
        raise NeedsReview("Erida member credential is missing")
    if type(timeout) is not int or not 1 <= timeout <= 120:
        raise NeedsReview("invalid timeout")
    origin = _safe_base(erida_origin)
    manifest, batches = _checked_batches(manifest_path)
    # The preview only calls the read-only endpoint. Do it for ALL batches
    # before sending any write. This preflight includes remote chapter checks.
    preview = preview_erida_batches(manifest_path, erida_origin=origin, api_key=api_key, timeout=timeout)
    if preview.get("status") != "READ_ONLY_PREVIEW_PASSED" or len(preview.get("batches", [])) != len(batches):
        raise NeedsReview("read-only Erida preflight incomplete; no stage created")
    opener = urllib.request.build_opener(NoRedirects())
    report = {
        "schema": "erida-stage-receipts-v1",
        "status": "STAGING",
        "work_id": manifest["work_id"],
        "revision_id": manifest["revision_id"],
        "requires_actor_confirmation": True,
        "confirmed": False,
        "translated": False,
        "published": False,
        "stages": [],
    }
    for batch in batches:
        envelope, boundary = _multipart(batch)
        # Actor-scoped server idempotency; key depends on exact bytes and immutable revision.
        replay_key = "ingest-m7-" + hashlib.sha256(
            (manifest["work_id"] + ":" + manifest["revision_id"] + ":" + batch.sha256).encode()
        ).hexdigest()
        req = urllib.request.Request(
            origin + "/v1/imports/stages", envelope, method="POST",
            headers={"Content-Type": f"multipart/form-data; boundary={boundary}",
                     "X-API-Key": api_key, "Idempotency-Key": replay_key,
                     "Accept": "application/json"})
        try:
            with opener.open(req, timeout=timeout) as response:
                if response.status != 201:
                    raise NeedsReview(f"Erida staging HTTP {response.status}")
                raw = response.read(1024 * 1024 + 1)
            if len(raw) > 1024 * 1024:
                raise NeedsReview("Erida stage response exceeds limit")
            staged = json.loads(raw)
            if not isinstance(staged, dict) or staged.get("status") != "staged":
                raise NeedsReview("Erida did not create an uncommitted stage")
            stage_id = staged.get("stage_id")
            try:
                stage_id = str(uuid.UUID(stage_id))
            except (ValueError, TypeError, AttributeError) as exc:
                raise NeedsReview("Erida returned invalid stage identifier") from exc
            if staged.get("file_sha256") != batch.sha256:
                raise NeedsReview("Erida staged bytes do not match verified batch")
            expires = staged.get("expires_at")
            if type(expires) not in (float, int) or expires < time.time() or expires > time.time() + 3600:
                raise NeedsReview("Erida stage has invalid expiration")
            if staged.get("unassigned_prefix_saved") is not False:
                raise NeedsReview("Erida found unassigned text; explicit review required")
            _validate_preview_entries(staged.get("preview"), batch, staged=True)
            report["stages"].append({
                "batch": batch.filename,
                "sha256": batch.sha256,
                "stage_id": stage_id,
                "expires_at": expires,
                "chapters": batch.count,
                "status": "staged",
            })
        except (urllib.error.HTTPError, urllib.error.URLError, ValueError, NeedsReview) as exc:
            report["status"] = "PARTIAL_STAGE_NEEDS_REVIEW"
            report["error"] = (
                "Erida staging HTTP error " + str(exc.code) if isinstance(exc, urllib.error.HTTPError)
                else "Erida staging transport failed" if isinstance(exc, urllib.error.URLError)
                else "Erida stage response failed validation"
            )
            return report
    report["status"] = "STAGED_AWAITING_ACTOR_CONFIRMATION"
    return report
