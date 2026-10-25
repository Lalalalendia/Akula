"""M3: opt-in, bounded-memory S3 multipart uploader of existing local files.

Only one worker owns a local SQLite journal (POSIX flock). M3 does NOT stream
arbitrary Internet responses or fetch Literotica. Conditional completion must
be separately capability-tested against the chosen S3-compatible provider.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
import sqlite3
import stat
import time
from contextlib import contextmanager
from pathlib import Path

from .objects import ObjectConflict, ObjectWriteUncertain, S3ObjectSink

MiB = 1024 * 1024
MIN_PART_BYTES = 5 * MiB
DEFAULT_PART_BYTES = 8 * MiB
MAX_LAB_FILE_BYTES = 512 * MiB
READBACK_BYTES = 256 * 1024
MAX_PARTS = 10_000


def _sha_file(path: Path) -> tuple[str, int]:
    size = 0
    hasher = hashlib.sha256()
    with path.open("rb") as inp:
        while block := inp.read(READBACK_BYTES):
            size += len(block)
            if size > MAX_LAB_FILE_BYTES:
                raise ValueError("fixture file exceeds 512 MiB safety cap")
            hasher.update(block)
    return hasher.hexdigest(), size


def _error_code(exc: Exception) -> str:
    return str((getattr(exc, "response", None) or {}).get("Error", {}).get("Code", ""))


def _safe_key(key: str) -> str:
    if (not isinstance(key, str) or not key or len(key) > 750 or
            key.startswith("/") or "\\" in key or any(seg in ("", ".", "..") for seg in key.split("/"))):
        raise ValueError("invalid object key")
    return key


class MultipartFileUploader:
    """Durable upload/resume/abort for *replayable, local, regular* files.

    Completion uses IfNoneMatch='*' to prevent overwrite. This is a protocol
    request, not proof that a particular backend enforces it; run a disposable
    conditional MPU provider probe before production use.
    """

    def __init__(self, sink: S3ObjectSink, journal: str | Path, *,
                 part_bytes: int = DEFAULT_PART_BYTES) -> None:
        if not isinstance(sink, S3ObjectSink):
            raise TypeError("multipart uploader requires S3ObjectSink")
        if not isinstance(part_bytes, int) or not MIN_PART_BYTES <= part_bytes <= 64 * MiB:
            raise ValueError("part size must be 5..64 MiB")
        self.sink = sink
        self.path = Path(journal)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.part_bytes = part_bytes
        with self._conn() as con:
            con.execute("""CREATE TABLE IF NOT EXISTS multipart_journal (
                bucket TEXT NOT NULL, object_key TEXT NOT NULL,
                source_sha256 TEXT NOT NULL, source_size INTEGER NOT NULL,
                part_bytes INTEGER NOT NULL, upload_id TEXT,
                parts_json TEXT NOT NULL DEFAULT '{}',
                state TEXT NOT NULL, updated_at REAL NOT NULL,
                PRIMARY KEY (bucket, object_key)
            )""")

    @contextmanager
    def _conn(self):
        con = sqlite3.connect(self.path, timeout=15)
        try:
            con.row_factory = sqlite3.Row
            con.execute("PRAGMA journal_mode=WAL")
            con.execute("PRAGMA synchronous=FULL")
            con.execute("PRAGMA busy_timeout=15000")
            yield con
            con.commit()
        except Exception:
            con.rollback()
            raise
        finally:
            con.close()

    @contextmanager
    def _lock(self):
        # OS releases the lock on process crash. The SQLite journal remains.
        with open(str(self.path) + ".lock", "a+b") as guard:
            fcntl.flock(guard.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(guard.fileno(), fcntl.LOCK_UN)

    def _row(self, full_key: str):
        with self._conn() as con:
            return con.execute("SELECT * FROM multipart_journal WHERE bucket=? AND object_key=?",
                               (self.sink.bucket, full_key)).fetchone()

    def _update(self, full_key: str, *, state: str, upload_id: str | None,
                source_sha256: str, source_size: int, parts: dict):
        with self._conn() as con:
            con.execute("""INSERT INTO multipart_journal
              (bucket,object_key,source_sha256,source_size,part_bytes,upload_id,parts_json,state,updated_at)
              VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(bucket,object_key) DO UPDATE SET
              source_sha256=excluded.source_sha256,source_size=excluded.source_size,
              part_bytes=excluded.part_bytes,upload_id=excluded.upload_id,
              parts_json=excluded.parts_json,state=excluded.state,updated_at=excluded.updated_at""",
              (self.sink.bucket,full_key,source_sha256,source_size,self.part_bytes,
               upload_id,json.dumps(parts,sort_keys=True),state,time.time()))

    def _verify_existing(self, full_key: str, expected_digest: str, expected_size: int) -> bool:
        """GET and hash incrementally. HEAD/ETag alone is not an integrity proof."""
        try:
            response = self.sink.client.get_object(Bucket=self.sink.bucket, Key=full_key)
        except Exception as exc:
            if _error_code(exc) in ("404", "NoSuchKey", "NotFound"):
                return False
            raise ObjectWriteUncertain("S3 cannot confirm object presence") from exc
        body = response["Body"]
        digest, count = hashlib.sha256(), 0
        try:
            while piece := body.read(READBACK_BYTES):
                count += len(piece)
                if count > MAX_LAB_FILE_BYTES:
                    raise ObjectConflict("S3 object exceeds configured safety bound")
                digest.update(piece)
        except Exception as exc:
            if isinstance(exc, ObjectConflict):
                raise
            raise ObjectWriteUncertain("S3 stream readback failed") from exc
        finally:
            if hasattr(body, "close"):
                body.close()
        if count != expected_size or digest.hexdigest() != expected_digest:
            raise ObjectConflict("S3 key exists with different/corrupt content")
        return True

    def _abort_own(self, full_key: str, upload_id: str | None) -> bool:
        if not upload_id:
            return False
        try:
            self.sink.client.abort_multipart_upload(Bucket=self.sink.bucket, Key=full_key,
                                                     UploadId=upload_id)
            return True
        except Exception as exc:
            # Once committed, S3 may return NoSuchUpload, which is harmless.
            return _error_code(exc) in ("NoSuchUpload", "404")

    def pending(self) -> list[dict]:
        with self._conn() as con:
            return [dict(r) for r in con.execute("SELECT * FROM multipart_journal WHERE state NOT IN ('COMMITTED','ABORTED') ORDER BY updated_at")]

    def abort_pending(self, key: str) -> str:
        """Operator action: abort *only* this journal's recorded UploadId."""
        full = self.sink.prefix + _safe_key(key)
        with self._lock():
            row = self._row(full)
            if row is None:
                return "ABSENT"
            if row["state"] == "COMMITTED":
                raise ObjectConflict("cannot abort already published object")
            if row["state"] == "INITIATING":
                raise ObjectWriteUncertain("unidentified upload may exist; provider lifecycle cleanup required")
            if row["state"] == "ABORTED":
                return "ABORTED"
            if not self._abort_own(full, row["upload_id"]):
                raise ObjectWriteUncertain("abort result unknown: preserve journal")
            self._update(full,state="ABORTED",upload_id=row["upload_id"],
                         source_sha256=row["source_sha256"],source_size=row["source_size"],
                         parts=json.loads(row["parts_json"]))
            return "ABORTED"

    def upload_file(self, key: str, source: str | Path, *, content_type: str = "application/octet-stream") -> dict:
        """Stream a replayable file in 8-MiB chunks; never load whole file in RAM.

        A preflight SHA pass binds the resume journal to file contents. Each resumed
        part is compared against its own saved digest. A final GET checksum is
        required even when AWS returns a successful CompleteMultipartUpload.
        """
        full_key = self.sink.prefix + _safe_key(key)
        src = Path(source)
        if src.is_symlink() or not src.is_file() or not stat.S_ISREG(src.stat().st_mode):
            raise ValueError("source must be an existing non-symlink regular file")
        if not isinstance(content_type, str) or not content_type or len(content_type) > 255:
            raise ValueError("invalid MIME type")
        source_digest, size = _sha_file(src)
        if size < MIN_PART_BYTES:
            raise ValueError("multipart requires >=5 MiB; use put_verified for small objects")
        if (size + self.part_bytes - 1)//self.part_bytes > MAX_PARTS:
            raise ValueError("too many parts")
        with self._lock():
            row = self._row(full_key)
            if row is not None and (row["source_sha256"],row["source_size"],row["part_bytes"]) != (
                    source_digest,size,self.part_bytes):
                raise ObjectConflict("same key has a different source or part-size journal; abort manually")
            if row is not None and row["state"] == "ABORTED":
                raise ObjectConflict("upload was aborted; use a different immutable key")
            if row is not None and row["state"] == "INITIATING":
                raise ObjectWriteUncertain("crash during initiation: potential orphan needs provider lifecycle")
            if self._verify_existing(full_key,source_digest,size):
                if row is not None and row["state"] != "COMMITTED":
                    if not self._abort_own(full_key,row["upload_id"]):
                        raise ObjectWriteUncertain("existing object verified, pending MPU cleanup uncertain")
                self._update(full_key,state="COMMITTED",upload_id=row["upload_id"] if row else None,
                             source_sha256=source_digest,source_size=size,
                             parts=json.loads(row["parts_json"]) if row else {})
                return {"status":"UNCHANGED", "key":key,"sha256":source_digest,"size":size}
            if row is not None and row["state"] == "COMMITTED":
                raise ObjectWriteUncertain("journal says committed but object vanished")
            if row is None:
                # This intent precedes remote initiation. Crash between remote
                # creation and UploadId commit can only be cleaned by lifecycle.
                self._update(full_key,state="INITIATING",upload_id=None,
                             source_sha256=source_digest,source_size=size,parts={})
                try:
                    init = self.sink.client.create_multipart_upload(
                        Bucket=self.sink.bucket,Key=full_key,ContentType=content_type)
                    upload_id = init["UploadId"]
                except Exception as exc:
                    raise ObjectWriteUncertain("multipart initiation result unknown") from exc
                self._update(full_key,state="UPLOADING",upload_id=upload_id,
                             source_sha256=source_digest,source_size=size,parts={})
                parts: dict[str,dict] = {}
                resumed = False
            else:
                upload_id = row["upload_id"]
                parts = json.loads(row["parts_json"])
                resumed = True
                if not isinstance(upload_id,str) or not upload_id:
                    raise ObjectWriteUncertain("journal lost upload ID")
                try:
                    listed=self.sink.client.list_parts(
                        Bucket=self.sink.bucket,Key=full_key,UploadId=upload_id)
                    if listed.get("IsTruncated"):
                        raise ObjectWriteUncertain("too many remote parts to reconcile in bounded M3 lab")
                    remote={str(p["PartNumber"]):p.get("ETag") for p in listed.get("Parts",[])}
                    # Retry missing or divergent parts using the same part number
                    # and source bytes. Never trust an ETag as a content digest.
                    for number,saved in list(parts.items()):
                        if remote.get(number) != saved["etag"]:
                            del parts[number]
                    self._update(full_key,state="UPLOADING",upload_id=upload_id,
                                 source_sha256=source_digest,source_size=size,parts=parts)
                except ObjectWriteUncertain:
                    raise
                except Exception as exc:
                    raise ObjectWriteUncertain("cannot reconcile remote multipart parts") from exc
            try:
                # Saved parts will be rechecked against the current input.
                with src.open("rb") as stream:
                    part_no = 0
                    while chunk := stream.read(self.part_bytes):
                        part_no += 1
                        checksum = hashlib.sha256(chunk).hexdigest()
                        previous = parts.get(str(part_no))
                        if previous is not None:
                            if previous["size"] != len(chunk) or previous["sha256"] != checksum:
                                raise ObjectConflict("source part changed since previous upload")
                            continue
                        result = self.sink.client.upload_part(
                            Bucket=self.sink.bucket,Key=full_key,UploadId=upload_id,
                            PartNumber=part_no,Body=chunk,ContentLength=len(chunk))
                        etag = result.get("ETag")
                        if not isinstance(etag,str) or not etag:
                            raise ObjectWriteUncertain("upload_part omitted ETag")
                        parts[str(part_no)]={"sha256":checksum,"size":len(chunk),"etag":etag}
                        self._update(full_key,state="UPLOADING",upload_id=upload_id,
                                     source_sha256=source_digest,source_size=size,parts=parts)
                # Guard against mutation after the preflight pass.
                if _sha_file(src) != (source_digest,size):
                    raise ObjectConflict("source file changed during upload")
                numbered=[{"PartNumber":int(n),"ETag":p["etag"]} for n,p in sorted(parts.items(),key=lambda x:int(x[0]))]
                if len(numbered) != (size+self.part_bytes-1)//self.part_bytes:
                    raise ObjectWriteUncertain("journal is missing required parts")
                self._update(full_key,state="COMPLETING",upload_id=upload_id,
                             source_sha256=source_digest,source_size=size,parts=parts)
                try:
                    self.sink.client.complete_multipart_upload(
                        Bucket=self.sink.bucket,Key=full_key,UploadId=upload_id,
                        MultipartUpload={"Parts":numbered},IfNoneMatch="*")
                except Exception as exc:
                    if _error_code(exc) in ("412","PreconditionFailed","409","ConditionalRequestConflict"):
                        # Expected if another writer completed first. Existing
                        # bytes must match; never overwrite or delete them.
                        try:
                            matching=self._verify_existing(full_key,source_digest,size)
                        except ObjectConflict:
                            if not self._abort_own(full_key,upload_id):
                                raise ObjectWriteUncertain("conflict confirmed but pending MPU cleanup failed")
                            self._update(full_key,state="ABORTED",upload_id=upload_id,
                                         source_sha256=source_digest,source_size=size,parts=parts)
                            raise
                        if not matching:
                            raise ObjectWriteUncertain("conditional complete failed but object absent") from exc
                        if not self._abort_own(full_key,upload_id):
                            raise ObjectWriteUncertain("matching object found; failed to clean own unfinished MPU")
                    elif not self._verify_existing(full_key,source_digest,size):
                        # Unknown ACK, or NoSuchUpload with no verified object.
                        # Preserve journal and UploadId for operator/resume.
                        raise ObjectWriteUncertain("multipart completion outcome unknown") from exc
                if not self._verify_existing(full_key,source_digest,size):
                    raise ObjectWriteUncertain("complete returned but object missing")
                self._update(full_key,state="COMMITTED",upload_id=upload_id,
                             source_sha256=source_digest,source_size=size,parts=parts)
                return {"status":"RESUMED" if resumed else "PUBLISHED",
                        "key":key,"sha256":source_digest,"size":size,
                        "parts":len(parts),"part_bytes":self.part_bytes}
            except (ObjectConflict,ObjectWriteUncertain):
                raise
            except Exception as exc:
                # UploadPart outcome unknown: retry the exact part on resume,
                # with same upload ID and source SHA, no new source crawl.
                raise ObjectWriteUncertain("multipart network/SDK result uncertain; journal preserved") from exc


class StreamingObjectUploader:
    """One-shot HTTP/file-like stream -> immutable S3, with bounded RAM.

    Unlike MultipartFileUploader, a non-replayable source CANNOT be resumed
    across process crash. This class attempts AbortMultipartUpload on errors.
    The provider must enforce CompleteMultipartUpload IfNoneMatch='*' (probe it).
    A bucket lifecycle rule should abort orphan multipart uploads after crashes.
    """

    def __init__(self, sink: S3ObjectSink, *, part_bytes: int=DEFAULT_PART_BYTES):
        if not isinstance(sink,S3ObjectSink):
            raise TypeError("requires S3ObjectSink")
        if not isinstance(part_bytes,int) or not MIN_PART_BYTES <= part_bytes <= 64*MiB:
            raise ValueError("part size must be 5..64 MiB")
        self.sink=sink
        self.part_bytes=part_bytes

    def _verify(self, full: str, digest: str, size: int) -> bool:
        """Incremental S3 GET readback, not ETag comparison."""
        hasher=hashlib.sha256()
        total=0
        try:
            response=self.sink.client.get_object(Bucket=self.sink.bucket,Key=full)
        except Exception as exc:
            if _error_code(exc) in ("404","NoSuchKey","NotFound"):
                return False
            raise ObjectWriteUncertain("streaming remote GET unavailable") from exc
        body=response["Body"]
        try:
            while part := body.read(READBACK_BYTES):
                total+=len(part)
                if total > MAX_LAB_FILE_BYTES:
                    raise ObjectConflict("remote size exceeds laboratory cap")
                hasher.update(part)
        except Exception as exc:
            if isinstance(exc,ObjectConflict):
                raise
            raise ObjectWriteUncertain("streaming readback interrupted") from exc
        finally:
            if hasattr(body,"close"):
                body.close()
        if (total,hasher.hexdigest()) != (size,digest):
            raise ObjectConflict("S3 object content/size conflicts with input stream")
        return True

    def put_stream(self,key: str, source, *, content_type: str="application/octet-stream") -> dict:
        """Read at most 256 KiB from a readable source per call.

        No seek, file descriptor, source replay or local file is required.
        S3 is only updated after successful PUT or complete. Return status
        VERIFIED or UNCHANGED, plus full SHA-256 and payload byte size.
        """
        full=self.sink.prefix+_safe_key(key)
        if not hasattr(source,"read") or not callable(source.read):
            raise ValueError("source must support read(n)")
        if not isinstance(content_type,str) or not content_type or len(content_type)>255:
            raise ValueError("invalid MIME type")
        hasher=hashlib.sha256()
        total=0
        buffer=bytearray()
        upload_id=None
        parts=[]
        try:
            while True:
                chunk=source.read(READBACK_BYTES)
                if not isinstance(chunk,bytes):
                    raise ValueError("stream read(n) must return bytes")
                if len(chunk)>READBACK_BYTES:
                    raise ValueError("source violated bounded read(n) contract")
                if not chunk:
                    break
                total+=len(chunk)
                if total > MAX_LAB_FILE_BYTES:
                    raise ValueError("stream exceeds 512 MiB lab limit")
                hasher.update(chunk)
                buffer.extend(chunk)
                if upload_id is None and len(buffer)>self.part_bytes:
                    initiated=self.sink.client.create_multipart_upload(
                        Bucket=self.sink.bucket,Key=full,ContentType=content_type)
                    upload_id=initiated["UploadId"]
                while upload_id and len(buffer)>=self.part_bytes:
                    no=len(parts)+1
                    if no>MAX_PARTS:
                        raise ValueError("too many multipart parts")
                    block=bytes(buffer[:self.part_bytes])
                    del buffer[:self.part_bytes]
                    response=self.sink.client.upload_part(Bucket=self.sink.bucket,Key=full,
                        UploadId=upload_id,PartNumber=no,Body=block,ContentLength=len(block))
                    etag=response.get("ETag")
                    if not etag:
                        raise ObjectWriteUncertain("S3 upload_part omitted ETag")
                    parts.append({"PartNumber":no,"ETag":etag})
            if total == 0:
                raise ValueError("empty stream is not a valid archive object")
            digest=hasher.hexdigest()
            if upload_id:
                if buffer:
                    no=len(parts)+1
                    response=self.sink.client.upload_part(Bucket=self.sink.bucket,Key=full,
                         UploadId=upload_id,PartNumber=no,Body=bytes(buffer),ContentLength=len(buffer))
                    if not response.get("ETag"):
                        raise ObjectWriteUncertain("S3 final part omitted ETag")
                    parts.append({"PartNumber":no,"ETag":response["ETag"]})
                try:
                    self.sink.client.complete_multipart_upload(
                        Bucket=self.sink.bucket,Key=full,UploadId=upload_id,
                        MultipartUpload={"Parts":parts},IfNoneMatch="*")
                except Exception as exc:
                    # On unknown completion ACK, use exact streaming GET proof.
                    if not self._verify(full,digest,total):
                        raise ObjectWriteUncertain("multipart completion outcome unknown") from exc
                    if _error_code(exc) in ("409","412","ConditionalRequestConflict","PreconditionFailed"):
                        try:
                            self.sink.client.abort_multipart_upload(
                                Bucket=self.sink.bucket,Key=full,UploadId=upload_id)
                        except Exception as abort_exc:
                            if _error_code(abort_exc) not in ("404","NoSuchUpload"):
                                raise ObjectWriteUncertain("multipart duplicate cleanup failed") from abort_exc
                        return {"status":"UNCHANGED","key":key,"size":total,"sha256":digest,"parts":len(parts)}
            else:
                try:
                    self.sink.client.put_object(Bucket=self.sink.bucket,Key=full,Body=bytes(buffer),
                                                ContentType=content_type,IfNoneMatch="*")
                except Exception as exc:
                    if not self._verify(full,digest,total):
                        raise ObjectWriteUncertain("small stream put outcome unknown") from exc
                    return {"status":"UNCHANGED","key":key,"size":total,"sha256":digest,"parts":0}
            if not self._verify(full,digest,total):
                raise ObjectWriteUncertain("stream finished but object not readable")
            return {"status":"PUBLISHED","key":key,"size":total,"sha256":digest,"parts":len(parts)}
        except Exception:
            if upload_id:
                try:
                    self.sink.client.abort_multipart_upload(
                        Bucket=self.sink.bucket,Key=full,UploadId=upload_id)
                except Exception:
                    # Provider lifecycle policy must clean any orphan upload.
                    pass
            raise
