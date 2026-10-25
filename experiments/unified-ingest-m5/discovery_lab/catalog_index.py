"""M4: atomic generation snapshots of *selected, verified fixture* work pointers.

This is NOT a sitewide discovery index and does not claim complete coverage.
A manifest-addressed generation contains ordered NDJSON shards, with mutable
indexes/current.json written only after each shard and the manifest are verified.
Only synthetic/offline work IDs are accepted by the underlying work publisher.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from .model import require_entity_id
from .objects import ObjectConflict, ObjectWriteUncertain
from .publisher import (_json_bytes, _sha, CurrentConflict,
                        PublicationUncertain, MAX_OBJECT_BYTES)
from .streaming_publisher import (StreamingWorkPublisher, StagedFile,
                                  StagedBundle, _Writer)

MAX_INDEX_ITEMS = 100_000
MAX_SHARD_ITEMS = 1000


def _strict_manifest(key: str, data: bytes, *, expected_digest: str):
    if _sha(data) != expected_digest:
        raise ObjectConflict(f"bad digest: {key}")
    try:
        return json.loads(data)
    except Exception as exc:
        raise PublicationUncertain(f"invalid JSON at {key}") from exc


@dataclass(frozen=True)
class CatalogGeneration:
    generation_id: str
    revision_manifest: bytes
    pointer: bytes
    shards: tuple[StagedFile, ...]
    items: int
    root: str


class CatalogIndexPublisher:
    def __init__(self, work_publisher: StreamingWorkPublisher, *, shard_items: int=MAX_SHARD_ITEMS):
        if not isinstance(work_publisher, StreamingWorkPublisher):
            raise TypeError("requires StreamingWorkPublisher")
        if type(shard_items) is not int or not 1 <= shard_items <= MAX_SHARD_ITEMS:
            raise ValueError("invalid shard items")
        self.work_publisher=work_publisher
        self.sink=work_publisher.sink
        self.shard_items=shard_items

    def _stage(self, work_ids: Iterable[str], root: Path) -> CatalogGeneration:
        """Inputs must already be sorted by work_id, no duplicates or large list in RAM."""
        root.mkdir(parents=True, exist_ok=False)
        files=[]
        count=0
        prev_id=""
        writer=None
        per_shard=0
        try:
            for raw_id in work_ids:
                ident=require_entity_id(raw_id)
                if ident <= prev_id:
                    raise ValueError("input IDs must be strictly increasing and unique")
                prev_id=ident
                count+=1
                if count>MAX_INDEX_ITEMS:
                    raise ValueError("index exceeds 100k-item lab bound")
                # Expensive by design in M4: verify all published artifact bytes
                # before claiming an indexed revision was verified.
                verification=self.work_publisher.verify_current(ident)
                pointer_bytes=self.sink.get(f"works/{ident}/current.json")
                pointer=_strict_manifest("work/current",pointer_bytes,
                                  expected_digest=_sha(pointer_bytes))
                if (pointer.get("revision_id") != verification["revision_id"] or
                    pointer.get("work_id") != ident):
                    raise ObjectConflict("work head changed during index verification")
                if writer is None:
                    rel=f"shards/{len(files):05d}.ndjson"
                    writer=_Writer(root/rel)
                    per_shard=0
                entry={"work_id":ident,"revision_id":verification["revision_id"],
                       "manifest_key":verification["manifest_key"],
                       "manifest_sha256":pointer["manifest_sha256"]}
                writer.write(_json_bytes(entry))
                per_shard+=1
                if per_shard >= self.shard_items:
                    meta=writer.finish()
                    files.append(StagedFile(rel,meta.path,meta.size,meta.sha256))
                    writer=None
            if writer is not None:
                meta=writer.finish()
                files.append(StagedFile(rel,meta.path,meta.size,meta.sha256))
                writer=None
        finally:
            if writer is not None:
                writer.close()
        if not files or count < 1:
            raise ValueError("cannot publish empty catalog generation")
        listed=[{"key": f.key,"size": f.size,"sha256":f.sha256} for f in files]
        manifest=_json_bytes({"schema_version":1,"kind":"selected_work_index",
                             "count":count,"shard_count":len(files),"files":listed,
                             "integrity":"verified_fixture","coverage_scope":"selected_work_ids_only",
                             "algorithm":"sha256"})
        if len(manifest)>MAX_OBJECT_BYTES:
            raise ValueError("manifest too big")
        rev=_sha(manifest)
        generation_root=f"indexes/versions/{rev}"
        pointer=_json_bytes({"schema_version":1,"generation_id":rev,
                             "manifest_key":generation_root+"/manifest.json",
                             "manifest_sha256":rev,"count":count})
        if len(pointer)>MAX_OBJECT_BYTES:
            raise ValueError("pointer too big")
        return CatalogGeneration(rev,manifest,pointer,tuple(files),count,generation_root)

    def verify_current(self):
        cur=self.work_publisher.small._current_bytes("indexes/current.json")
        if cur is None:
            raise PublicationUncertain("catalog has no current pointer")
        try:
            head=json.loads(cur)
            rev=head["generation_id"]
            if not isinstance(rev,str) or not re.fullmatch("[0-9a-f]{64}",rev):
                raise ValueError("invalid generation hash")
            root=f"indexes/versions/{rev}"
            manifest_key=root+"/manifest.json"
            if head["manifest_key"]!=manifest_key or head["manifest_sha256"]!=rev:
                raise ValueError("invalid manifest target")
            manifest_bytes=self.sink.get(manifest_key)
            doc=_strict_manifest(manifest_key,manifest_bytes,expected_digest=rev)
            if doc["kind"]!="selected_work_index" or doc["shard_count"]!=len(doc["files"]):
                raise ValueError("invalid manifest structure")
            if head["count"] != doc["count"]:
                raise ValueError("wrong item count")
            count=0
            prev=""
            for no,node in enumerate(doc["files"]):
                rel=node["key"]
                if rel!=f"shards/{no:05d}.ndjson":
                    raise ValueError("unordered shards")
                self.work_publisher._verify_file(root+"/"+rel,StagedFile(rel,Path(),node["size"],node["sha256"]))
                # Read shard itself only after proving digest. M4 is capped to
                # 1000 lines/shard, each line small, bounded <=2MiB.
                data=self.sink.get(root+"/"+rel)
                for line in data.splitlines():
                    item=json.loads(line)
                    wid=require_entity_id(item["work_id"])
                    if wid<=prev or not re.fullmatch("[a-f0-9]{64}",item["revision_id"]):
                        raise ValueError("duplicate/unsorted index entry")
                    expected=f"works/{wid}/versions/{item['revision_id']}/manifest.json"
                    if item["manifest_key"] != expected or item["manifest_sha256"]!=item["revision_id"]:
                        raise ValueError("bad work manifest reference")
                    prev=wid
                    count+=1
            if count!=doc["count"]:
                raise ValueError("wrong count after reading index")
        except (ValueError,TypeError,KeyError,IndexError,json.JSONDecodeError) as exc:
            raise PublicationUncertain("invalid catalog pointer/manifest") from exc
        return {"status":"VERIFIED","generation_id":rev,"count":count,
                "shards":len(doc["files"]),"manifest_key":manifest_key}

    def publish(self, work_ids: Iterable[str], *, expected_generation: str | None,
                stage_dir: str | Path):
        directory=Path(stage_dir)
        directory.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="m4-index-",dir=directory) as temp:
            staged=self._stage(work_ids,Path(temp)/"data")
            previous=self.work_publisher.small._current_bytes("indexes/current.json")
            current_rev=None
            if previous:
                try:
                    current_rev=json.loads(previous)["generation_id"]
                except (ValueError,TypeError,KeyError) as exc:
                    raise PublicationUncertain("bad existing index pointer") from exc
            if current_rev==staged.generation_id:
                self.verify_current()
                return {"status":"UNCHANGED","generation_id":staged.generation_id,
                        "count":staged.items}
            if expected_generation!=current_rev:
                raise CurrentConflict("index generation changed")
            manifest_file=Path(temp)/"data"/"manifest.json"
            with manifest_file.open("xb") as output:
                output.write(staged.revision_manifest)
                output.flush();os.fsync(output.fileno())
            allfiles=list(staged.shards)+[StagedFile("manifest.json",manifest_file,
                        len(staged.revision_manifest),staged.generation_id)]
            temp_bundle=StagedBundle("catalog",staged.generation_id,staged.root,
                                    tuple(allfiles),staged.pointer)
            for file in temp_bundle.files:
                self.work_publisher._put_stage(temp_bundle,file)
            try:
                self.sink.cas_verified("indexes/current.json",staged.pointer,previous)
            except (ObjectConflict,ObjectWriteUncertain) as exc:
                after=self.work_publisher.small._current_bytes("indexes/current.json")
                if after!=staged.pointer:
                    if isinstance(exc,ObjectConflict):
                        raise CurrentConflict("index pointer CAS rejected") from exc
                    raise PublicationUncertain("index commit uncertain") from exc
            verification=self.verify_current()
            return {"status":"PUBLISHED", **{k:v for k,v in verification.items() if k!="status"}}
