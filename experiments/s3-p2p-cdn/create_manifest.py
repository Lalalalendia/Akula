#!/usr/bin/env python3
"""Publish an immutable, chunk-hashed manifest for an existing public S3 object."""
import argparse
import hashlib
import json
from pathlib import Path


def make_manifest(source_file: Path, source_url: str, chunk_size: int = 256 * 1024):
    if chunk_size < 1024 or chunk_size > 4 * 1024 * 1024:
        raise ValueError("chunk_size must be between 1 KiB and 4 MiB")
    if not (source_url.startswith("https://") or source_url.startswith("/origin/")):
        raise ValueError("Source URL must be HTTPS or local /origin/")
    if not source_file.is_file():
        raise FileNotFoundError(source_file)

    full = hashlib.sha256()
    chunks = []
    offset = 0
    with source_file.open("rb") as f:
        while data := f.read(chunk_size):
            full.update(data)
            chunks.append({"offset": offset, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
            offset += len(data)
    return {
        "version": 1,
        "content_id": full.hexdigest(),
        "name": source_file.name,
        "size": offset,
        "chunk_size": chunk_size,
        "origin_url": source_url,
        "chunks": chunks,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("file", type=Path)
    p.add_argument("--source-url", required=True, help="Immutable public HTTPS S3 URL or local /origin/...")
    p.add_argument("--chunk-size", type=int, default=256 * 1024)
    p.add_argument("--out", type=Path, default=Path("manifest.json"))
    args = p.parse_args()
    result = make_manifest(args.file, args.source_url, args.chunk_size)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.out}, {len(result['chunks'])} parts, ContentID={result['content_id']}")

if __name__ == "__main__":
    main()