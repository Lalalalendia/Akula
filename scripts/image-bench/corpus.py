#!/usr/bin/env python3
"""Download a hash-pinned, license-reviewed subset of the imazen-26 test split.

Never execute remote content or commit corpus bytes. Only manifest-selected images
are downloaded, with strict HTTPS origin, path, size, time and SHA-256 checks.
"""
import argparse
import csv
import hashlib
import io
import os
from pathlib import Path
import re
import tempfile
import urllib.parse
import urllib.request

COMMIT = "36da2bfbd35fe9f816ecdfa47070daf2685af64e"
MANIFEST_URL = (
    f"https://raw.githubusercontent.com/imazen/imazen-26/{COMMIT}/manifests/test.tsv"
)
RAW_ROOT = "https://codec-corpus.r2.imazen.org/imazen-26-unprocessed/"
MAX_MANIFEST_BYTES = 500_000
MAX_IMAGE_BYTES = 32_000_000
# All from the canonical 'test' split and categories licensed PD-own/PD-USGov.
# Preserve these choices until results are reviewed; don't select by filename.
CASES = {
    "1019": ("jpg", "1000-lilith-photos-general", "PD-own"),
    "1059": ("jpg", "1000-lilith-photos-general", "PD-own"),
    "5007": ("png", "5000-national-park-service-brochures", "PD-USGov"),
    "7007": ("png", "7000-lilith-plots", "PD-own"),
    "9007": ("png", "9000-lilith-ai-clipart", "PD-own"),
    "1407": ("jpg", "1400-lilith-nature", "PD-own"),
}
SMOKE = ("1019", "1059", "5007", "7007", "9007")
EXTENDED = SMOKE + ("1407",)
REQUIRED = (
    "id", "split", "content_class", "path", "width", "height", "format",
    "bytes_actual", "sha256", "raw_url",
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError(f"redirects forbidden for corpus download (HTTP {code})")


def validate_manifest(text: str, ids: tuple[str, ...]) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(text), delimiter="\t")
    if reader.fieldnames is None or not set(REQUIRED).issubset(reader.fieldnames):
        raise ValueError("missing canonical manifest fields")
    wanted = set(ids)
    rows = {}
    for row in reader:
        image_id = row.get("id", "")
        if image_id not in wanted:
            continue
        if image_id in rows:
            raise ValueError(f"duplicate corpus id: {image_id}")
        fmt, content_class, license_name = CASES[image_id]
        if row["split"] != "test" or row["format"] != fmt:
            raise ValueError(f"unexpected split or format: {image_id}")
        if row["content_class"] != content_class:
            raise ValueError(f"unexpected provenance: {image_id}")
        path = row["path"]
        if not path.startswith(content_class + "/") or ".." in Path(path).parts:
            raise ValueError(f"invalid corpus relative path: {image_id}")
        if not path.endswith("." + fmt) or not re.fullmatch(r"[a-zA-Z0-9_./-]+", path):
            raise ValueError(f"invalid corpus filename: {image_id}")
        url = row["raw_url"]
        if url != RAW_ROOT + path:
            raise ValueError(f"unapproved corpus origin or path: {image_id}")
        sha = row["sha256"]
        if not re.fullmatch(r"[a-f0-9]{64}", sha):
            raise ValueError(f"invalid sha256: {image_id}")
        length = int(row["bytes_actual"])
        if not 0 < length <= MAX_IMAGE_BYTES:
            raise ValueError(f"file too large for research CI: {image_id}")
        w, h = int(row["width"]), int(row["height"])
        if not (1 <= w <= 100_000 and 1 <= h <= 100_000 and w * h <= 100_000_000):
            raise ValueError(f"invalid image dimensions: {image_id}")
        rows[image_id] = {
            "id": image_id, "format": fmt, "width": str(w), "height": str(h),
            "sha256": sha, "url": url, "bytes": str(length),
            "license": license_name, "class": content_class,
        }
    if set(rows) != wanted:
        raise ValueError(f"missing corpus ids: {sorted(wanted - set(rows))}")
    return [rows[i] for i in ids]


def read_limited(response, max_bytes: int) -> bytes:
    data = response.read(max_bytes + 1)
    if len(data) > max_bytes:
        raise ValueError("manifest exceeds size cap")
    return data


def verified_download(case: dict[str, str], dest: Path, opener) -> None:
    expected = int(case["bytes"])
    expected_sha = case["sha256"]
    if dest.is_symlink():
        raise ValueError(f"destination is a symlink: {dest}")
    if dest.is_file() and dest.stat().st_size == expected:
        if hashlib.sha256(dest.read_bytes()).hexdigest() == expected_sha:
            return
    req = urllib.request.Request(case["url"], headers={"User-Agent": "Akula-image-bench/1.0"})
    temp_path = None
    try:
        with opener.open(req, timeout=35) as response:
            if response.status != 200:
                raise ValueError(f"unexpected HTTP status: {response.status}")
            content_length = response.headers.get("Content-Length")
            if content_length and int(content_length) > expected:
                raise ValueError("HTTP payload exceeds manifest limit")
            digest = hashlib.sha256()
            received = 0
            with tempfile.NamedTemporaryFile(dir=dest.parent, prefix=".case-", delete=False) as tmp:
                temp_path = Path(tmp.name)
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    received += len(chunk)
                    if received > expected or received > MAX_IMAGE_BYTES:
                        raise ValueError("image bytes exceed manifest")
                    digest.update(chunk)
                    tmp.write(chunk)
        if received != expected or digest.hexdigest() != expected_sha:
            raise ValueError(f"length/SHA mismatch for {case['id']}")
        os.replace(temp_path, dest)
    finally:
        if temp_path and temp_path.exists():
            temp_path.unlink()


def prepare(tier: str, output: Path, opener=None):
    opener = opener or urllib.request.build_opener(NoRedirect)
    output.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(MANIFEST_URL, headers={"User-Agent": "Akula-image-bench/1.0"})
    with opener.open(req, timeout=35) as response:
        if response.status != 200:
            raise ValueError(f"manifest status {response.status}")
        manifest_bytes = read_limited(response, MAX_MANIFEST_BYTES)
    ids = SMOKE if tier == "smoke" else EXTENDED
    cases = validate_manifest(manifest_bytes.decode("utf-8-sig"), ids)
    # Dataset listing is not needed to run the image decoders themselves.
    with (output / "manifest-provenance.txt").open("w", encoding="utf-8") as f:
        f.write(f"manifest_url={MANIFEST_URL}\n")
        f.write(f"manifest_sha256={hashlib.sha256(manifest_bytes).hexdigest()}\n")
        f.write(f"selection={','.join(ids)}\n")
        f.write("no_corpus_bytes_committed=true\n")
    with (output / "cases.tsv").open("w", encoding="utf-8", newline="") as dst:
        writer = csv.writer(dst, delimiter="\t", lineterminator="\n")
        writer.writerow(("id", "format", "width", "height", "sha256", "file", "license", "bytes"))
        for case in cases:
            path = output / f"{case['id']}.{case['format']}"
            verified_download(case, path, opener)
            writer.writerow((
                case["id"], case["format"], case["width"], case["height"],
                case["sha256"], str(path), case["license"], case["bytes"]
            ))
            print(f"VERIFIED corpus id={case['id']} format={case['format']} "
                  f"size={case['bytes']} license={case['license']}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tier", choices=("smoke", "extended"), default="smoke")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.tier, args.output)
