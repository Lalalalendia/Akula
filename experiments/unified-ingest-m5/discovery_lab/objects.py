"""Immutable raw-object sink with verified readback; optional S3 backend."""
from __future__ import annotations

import hashlib
import os
import uuid
import fcntl
from pathlib import Path


class ObjectConflict(RuntimeError):
    pass


class ObjectWriteUncertain(OSError):
    """Object may have been stored, but acknowledgement/readback was lost."""
    pass


class FileObjectSink:
    """A local object-store simulator; O_EXCL enforces create-if-absent."""

    def __init__(self, root: str | Path) -> None:
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)

    def _path(self, key: str) -> Path:
        if not key or key.startswith("/") or any(x in ("", ".", "..") for x in key.split("/")):
            raise ValueError("unsafe key")
        if "\\" in key:
            raise ValueError("unsafe key")
        return self.root.joinpath(*key.split("/"))

    def put_verified(self, key: str, data: bytes) -> str:
        """Expose the immutable key only after a full, fsynced temporary write."""
        digest = hashlib.sha256(data).hexdigest()
        out = self._path(key)
        out.parent.mkdir(parents=True, exist_ok=True)
        temp = out.with_name(out.name + ".partial-" + uuid.uuid4().hex)
        try:
            with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as fh:
                fh.write(data)
                fh.flush()
                os.fsync(fh.fileno())
            try:
                # Atomic publication with no overwrites. Old writer's file wins.
                os.link(temp, out)
            except FileExistsError:
                pass
        finally:
            temp.unlink(missing_ok=True)
        received = out.read_bytes()
        if received != data or hashlib.sha256(received).hexdigest() != digest:
            raise ObjectConflict(f"object exists with conflicting/corrupt content: {key}")
        return digest

    def cas_verified(self, key: str, data: bytes, expected: bytes | None) -> str:
        """Cross-process CAS, POSIX/Linux fixture only; fsync + atomic replace."""
        out = self._path(key)
        out.parent.mkdir(parents=True, exist_ok=True)
        lockfile = out.with_name(out.name + ".m2-lock")
        tmp = out.with_name(out.name + ".m2-tmp-" + uuid.uuid4().hex)
        with open(lockfile, "a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                current = out.read_bytes() if out.exists() else None
                if current != expected:
                    raise ObjectConflict("CAS expected bytes changed")
                with os.fdopen(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as fh:
                    fh.write(data)
                    fh.flush()
                    os.fsync(fh.fileno())
                os.replace(tmp, out)
                dirfd = os.open(out.parent, os.O_RDONLY)
                try:
                    os.fsync(dirfd)
                finally:
                    os.close(dirfd)
                received = out.read_bytes()
                if received != data:
                    raise ObjectWriteUncertain("CAS readback mismatch")
                return hashlib.sha256(data).hexdigest()
            finally:
                tmp.unlink(missing_ok=True)
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)

    def get(self, key: str) -> bytes:
        return self._path(key).read_bytes()

    def keys(self) -> list[str]:
        return sorted(str(x.relative_to(self.root)) for x in self.root.rglob("*") if x.is_file())


class S3ObjectSink:
    """OPTIONAL real S3 sink: inert unless instantiated by the caller with an authorized client.

    Limited M0 interface for SMALL objects only, no multipart or eventual-publication logic.
    Not yet tested against a live S3/MinIO server.
    """

    def __init__(self, client: object, bucket: str, prefix: str = "literotica-lab/") -> None:
        if not bucket or not prefix or prefix.startswith("/") or ".." in prefix.split("/"):
            raise ValueError("invalid bucket/prefix")
        self.client, self.bucket, self.prefix = client, bucket, prefix

    def put_verified(self, key: str, data: bytes) -> str:
        if len(data) > 2 * 1024 * 1024:
            raise ValueError("M0 S3 sink requires <=2 MiB")
        if key.startswith("/") or ".." in key.split("/") or "\\" in key:
            raise ValueError("unsafe key")
        full_key = self.prefix + key
        digest = hashlib.sha256(data).hexdigest()
        try:
            self.client.put_object(Bucket=self.bucket, Key=full_key, Body=data,
                                   ContentType=("text/markdown; charset=utf-8" if key.endswith(".md") else
                                                "text/html; charset=utf-8" if key.endswith(".html") else
                                                "application/json"),
                                   IfNoneMatch="*")
        except Exception:
            # The write may already have completed despite a lost response.
            # Readback below is the only proof; never issue a second write here.
            pass
        try:
            received = self.get(key)
        except Exception as exc:
            raise ObjectWriteUncertain("S3 readback failed after put") from exc
        if received != data or hashlib.sha256(received).hexdigest() != digest:
            raise ObjectConflict("S3 immutable key exists with different bytes")
        return digest

    def cas_verified(self, key: str, data: bytes, expected: bytes | None) -> str:
        """S3 conditional head-pointer write; provider CAS must be capability-tested."""
        if len(data) > 2 * 1024 * 1024:
            raise ValueError("M2 S3 CAS requires <=2 MiB")
        if not key or key.startswith("/") or ".." in key.split("/") or "\\" in key:
            raise ValueError("unsafe key")
        params = dict(Bucket=self.bucket, Key=self.prefix + key, Body=data,
                      ContentType="application/json")
        if expected is None:
            params["IfNoneMatch"] = "*"
        else:
            try:
                current = self.get(key)
                if current != expected:
                    raise ObjectConflict("CAS changed")
                head = self.client.head_object(Bucket=self.bucket, Key=self.prefix + key)
                etag = head.get("ETag")
                if not isinstance(etag, str) or not etag:
                    raise ObjectWriteUncertain("S3 HEAD omitted ETag for CAS")
                params["IfMatch"] = etag
            except ObjectConflict:
                raise
            except ObjectWriteUncertain:
                raise
            except Exception as exc:
                raise ObjectWriteUncertain("S3 current read/HEAD unavailable") from exc
        try:
            self.client.put_object(**params)
        except Exception as exc:
            code = (getattr(exc, "response", None) or {}).get("Error", {}).get("Code")
            if code in ("PreconditionFailed", "412", "ConditionalRequestConflict", "409"):
                raise ObjectConflict("S3 CAS precondition rejected") from exc
            # A CAS acknowledgement can be lost AFTER the pointer was stored.
            # Never replay a possibly-committed pointer update blindly.
            try:
                present = self.get(key)
            except Exception as check_exc:
                raise ObjectWriteUncertain("S3 pointer acknowledgement/readback unavailable") from check_exc
            if present != data:
                raise ObjectWriteUncertain("S3 pointer changed or was not committed") from exc
            return hashlib.sha256(data).hexdigest()
        try:
            readback = self.get(key)
        except Exception as exc:
            raise ObjectWriteUncertain("S3 CAS readback unavailable") from exc
        if readback != data:
            raise ObjectWriteUncertain("S3 pointer changed before readback")
        return hashlib.sha256(data).hexdigest()

    def get(self, key: str) -> bytes:
        if key.startswith("/") or ".." in key.split("/") or "\\" in key:
            raise ValueError("unsafe key")
        try:
            result = self.client.get_object(Bucket=self.bucket, Key=self.prefix + key)
        except Exception as exc:
            code = (getattr(exc, "response", None) or {}).get("Error", {}).get("Code")
            if code in ("NoSuchKey", "NoSuchBucket", "404", "NotFound", "AccessDenied",
                        "InvalidAccessKeyId", "SignatureDoesNotMatch"):
                raise  # Preserve 404 and permanent failures for publisher/quality gates.
            raise ObjectWriteUncertain("S3 read unavailable") from exc
        body = result["Body"]
        limit = 2 * 1024 * 1024
        expected = result.get("ContentLength")
        if expected is not None and (not isinstance(expected, int) or expected < 0 or expected > limit):
            if hasattr(body, "close"):
                body.close()
            raise ObjectConflict("invalid or oversized S3 ContentLength")
        output = bytearray()
        try:
            try:
                while len(output) <= limit:
                    block = body.read(min(256 * 1024, limit + 1 - len(output)))
                    if not block:
                        break
                    output.extend(block)
            except Exception as exc:
                raise ObjectWriteUncertain("S3 body read interrupted") from exc
            if len(output) > limit:
                raise ObjectConflict("S3 object exceeds small object cap")
            if expected is not None and len(output) != expected:
                raise ObjectWriteUncertain("S3 GET truncated: ContentLength mismatch")
            return bytes(output)
        finally:
            if hasattr(body, "close"):
                body.close()
