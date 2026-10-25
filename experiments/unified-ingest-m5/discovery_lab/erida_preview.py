"""Opt-in READ-ONLY preview against Erida's documented import endpoint.

The request is only POST /v1/imports/preview, never /stages or /confirm.
No samples or API keys are returned in the resulting report.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

from .unified_pipeline import NeedsReview
from .erida_export import ERIDA_MAX_UPLOAD


class NoRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise NeedsReview("Erida API redirected preview request; refusing to forward credentials")


def _safe_base(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    if parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise NeedsReview("use an origin URL without credentials or path")
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        raise NeedsReview("invalid Erida API origin")
    if parsed.scheme != "https" and parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise NeedsReview("remote Erida preview requires HTTPS")
    return url.rstrip("/")


def preview_erida_batches(manifest_path: str | Path, *, erida_origin: str,
                          api_key: str, timeout: int = 20) -> dict:
    """Check every immutable HTML batch with Erida's server-owned parser.

    It sends chapter text to the explicitly selected Erida server. Never call
    unless the user has authorized that server/source. Does not store chapters.
    """
    origin = _safe_base(erida_origin)
    if not api_key or len(api_key) > 1024:
        raise NeedsReview("missing Erida member credential")
    root = Path(manifest_path).resolve().parent
    manifest = json.loads(Path(manifest_path).read_bytes())
    if (manifest.get("schema") != "erida-import-batches-v1" or
        manifest.get("scope") != "selected_manifest_only" or
        manifest.get("stage_automatically") is not False or
        manifest.get("requires_actor_confirmation") is not True):
        raise NeedsReview("invalid Erida prepared-import manifest")
    batches = manifest.get("batches")
    if not isinstance(batches, list) or not batches or len(batches) > 1000:
        raise NeedsReview("missing prepared batches")
    opener = urllib.request.build_opener(NoRedirects())
    report = []
    seen_ordinals = []
    for item in batches:
        name = item.get("filename") if isinstance(item, dict) else None
        if not isinstance(name, str) or not re.fullmatch(r"[a-zA-Z0-9_.-]{1,180}\.html", name) or ".." in name:
            raise NeedsReview("unsafe import filename")
        batch = (root / name).resolve()
        if batch.parent != root:
            raise NeedsReview("import file escaped prepared output directory")
        body = batch.read_bytes()
        if (len(body) > ERIDA_MAX_UPLOAD or len(body) != item.get("bytes") or
            hashlib.sha256(body).hexdigest() != item.get("sha256")):
            raise NeedsReview("modified or oversized Erida import batch")
        boundary = "M6-" + uuid.uuid4().hex
        envelope = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n"
                    "Content-Type: text/html; charset=utf-8\r\n\r\n").encode()
        payload = envelope + body + (f"\r\n--{boundary}--\r\n").encode()
        request = urllib.request.Request(
            origin + "/v1/imports/preview", payload, method="POST",
            headers={"Content-Type": f"multipart/form-data; boundary={boundary}",
                     "X-API-Key": api_key, "Accept": "application/json"})
        try:
            with opener.open(request, timeout=timeout) as response:
                if response.status != 200:
                    raise NeedsReview(f"Erida preview returned HTTP {response.status}")
                response_body = response.read(1024 * 1024 + 1)
        except urllib.error.HTTPError as exc:
            raise NeedsReview(f"Erida preview HTTP error {exc.code}") from None
        except urllib.error.URLError as exc:
            raise NeedsReview("Erida preview transport unavailable") from exc
        if len(response_body) > 1024 * 1024:
            raise NeedsReview("Erida preview response exceeds cap")
        try:
            preview = json.loads(response_body)
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise NeedsReview("invalid Erida preview response") from exc
        entries = preview.get("chapters") if isinstance(preview, dict) else None
        expected = list(range(item["first_ordinal"], item["last_ordinal"] + 1))
        if (preview.get("stored") is not False or preview.get("kind") != "book" or
            preview.get("chapter_count") != item["chapter_count"] or
            not isinstance(entries, list) or
            [row.get("detected_number") for row in entries] != expected or
            preview.get("warnings") or any(not row.get("has_text") for row in entries)):
            raise NeedsReview("Erida import preview disagrees with verified chapter inventory")
        seen_ordinals.extend(expected)
        report.append({"batch": name, "chapters_confirmed_by_preview": len(entries),
                       "first_ordinal": expected[0], "last_ordinal": expected[-1]})
    if seen_ordinals != list(range(1, manifest.get("expected_chapters", 0) + 1)):
        raise NeedsReview("Erida preview did not cover the selected chapter inventory")
    return {"status": "READ_ONLY_PREVIEW_PASSED", "stage_created": False,
            "translated": False, "published": False, "batches": report}

