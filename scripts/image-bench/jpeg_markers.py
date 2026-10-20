"""Inspect JPEG headers safely and register a verified progressive derivative.

This probes marker metadata, not visual fidelity. No original image is modified.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path

SOF = {0xc0: "baseline", 0xc1: "extended", 0xc2: "progressive", 0xc3: "lossless"}

def _read_exact(f, size):
    result = f.read(size)
    if len(result) != size:
        raise ValueError("truncated JPEG header")
    return result

def inspect(path):
    with Path(path).open("rb") as f:
        if _read_exact(f, 2) != bytes((0xff, 0xd8)):
            raise ValueError("not a JPEG SOI")
        width = height = None
        frame = None
        exif = icc = False
        segments = 0
        while segments < 10000:
            if _read_exact(f, 1) != bytes((0xff,)):
                raise ValueError("invalid JPEG marker alignment")
            marker = _read_exact(f, 1)[0]
            while marker == 0xff:
                marker = _read_exact(f, 1)[0]
            if marker in (0xd9, 0xda):
                break
            if marker in (0xd8, 0x01) or 0xd0 <= marker <= 0xd7:
                segments += 1
                continue
            length = int.from_bytes(_read_exact(f, 2), "big")
            if length < 2:
                raise ValueError("malformed segment length")
            payload_length = length - 2
            prefix = _read_exact(f, min(payload_length, 24))
            if payload_length > len(prefix):
                f.seek(payload_length - len(prefix), 1)
            if marker in SOF:
                if len(prefix) < 5:
                    raise ValueError("bad frame header")
                height = int.from_bytes(prefix[1:3], "big")
                width = int.from_bytes(prefix[3:5], "big")
                frame = SOF[marker]
            if marker == 0xe1 and prefix.startswith(b"Exif\x00\x00"):
                exif = True
            if marker == 0xe2 and prefix.startswith(b"ICC_PROFILE\x00"):
                icc = True
            segments += 1
        if width is None or height is None or frame is None:
            raise ValueError("missing supported JPEG frame header")
        return {"frame": frame, "width": width, "height": height, "exif": exif, "icc": icc}

def sha256(path):
    digest=hashlib.sha256()
    with Path(path).open("rb") as f:
        for part in iter(lambda: f.read(1 << 20), b""):
            digest.update(part)
    return digest.hexdigest()

def append_derivative(cases_path, source_id, derived_path, record_path):
    with Path(cases_path).open("r", encoding="utf-8", newline="") as f:
        cases=list(csv.DictReader(f, delimiter="\t"))
    orig=[r for r in cases if r["id"] == source_id]
    if len(orig) != 1 or orig[0]["format"] != "jpg":
        raise ValueError("expected one pinned JPEG source")
    row=orig[0]
    if sha256(row["file"]) != row["sha256"]:
        raise ValueError("source SHA-256 changed during transcoding")
    before=inspect(row["file"])
    after=inspect(derived_path)
    if before["width"] != after["width"] or before["height"] != after["height"]:
        raise ValueError("derivative dimensions changed")
    if after["frame"] != "progressive":
        raise ValueError("jpegtran did not create progressive JPEG")
    if before["exif"] != after["exif"] or before["icc"] != after["icc"]:
        raise ValueError("metadata markers not preserved")
    name=source_id + "-progressive"
    if any(r["id"] == name for r in cases):
        raise ValueError("duplicate derived entry")
    digest=sha256(derived_path)
    size=Path(derived_path).stat().st_size
    if not (0 < size <= 32_000_000):
        raise ValueError("derived JPEG violates the 32 MB corpus read limit")
    with Path(cases_path).open("a", encoding="utf-8", newline="") as f:
        csv.writer(f, delimiter="\t", lineterminator="\n").writerow((
            name, row["format"], row["width"], row["height"], digest,
            str(derived_path), row["license"], str(size)))
    provenance={"source_id":source_id,"source_sha256":row["sha256"],
                "derived_sha256":digest,"source":before,"derived":after,
                "transformation":"jpegtran -progressive -copy all (coefficient transcode)"}
    Path(record_path).write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    print("DERIVED " + json.dumps(provenance, sort_keys=True))

def main():
    p=argparse.ArgumentParser()
    p.add_argument("--probe", type=Path)
    p.add_argument("--cases", type=Path)
    p.add_argument("--source-id", default="1407")
    p.add_argument("--derived", type=Path)
    p.add_argument("--record", type=Path)
    a=p.parse_args()
    if a.probe:
        print("JPEG_MARKERS " + json.dumps(inspect(a.probe), sort_keys=True))
    elif a.cases and a.derived and a.record:
        append_derivative(a.cases, a.source_id, a.derived, a.record)
    else:
        p.error("supply --probe or --cases / --derived / --record")

if __name__ == "__main__":
    main()
