"""Generate deterministic, synthetic fixtures for the Akula one-PC plan.

All output is confined to the repository's ignored .artifacts directory. The
generator never deletes files and refuses to replace a fixture with different
contents, so it is safe to re-run after an interrupted generation.
"""

from __future__ import annotations

import hashlib
import json
import struct
import zlib
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_ROOT = REPO_ROOT / ".artifacts" / "one-pc-fixtures"
MARKER_NAME = ".akula-one-pc-fixtures"
MARKER_CONTENT = b"AKULA-ONE-PC-PLAN-01 synthetic fixtures v1\n"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def png_chunk(kind: bytes, data: bytes) -> bytes:
    payload = kind + data
    return (
        struct.pack(">I", len(data))
        + payload
        + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
    )


def one_pixel_png() -> bytes:
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    pixels = zlib.compress(b"\x00\xff\xff\xff\xff")
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", ihdr)
        + png_chunk(b"IDAT", pixels)
        + png_chunk(b"IEND", b"")
    )


def write_fixture(relative: str, data: bytes, entries: list[tuple[str, int, str]]) -> None:
    path = FIXTURE_ROOT / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    digest = sha256(data)

    if path.exists():
        existing = path.read_bytes()
        if existing != data:
            raise RuntimeError(f"Refusing to overwrite different fixture: {path}")
    else:
        path.write_bytes(data)

    entries.append((relative.replace("\\", "/"), len(data), digest))


def main() -> None:
    if FIXTURE_ROOT.exists() and not (FIXTURE_ROOT / MARKER_NAME).exists():
        raise RuntimeError(
            f"Refusing to use an unmarked existing directory: {FIXTURE_ROOT}"
        )
    FIXTURE_ROOT.mkdir(parents=True, exist_ok=True)
    marker = FIXTURE_ROOT / MARKER_NAME
    if marker.exists() and marker.read_bytes() != MARKER_CONTENT:
        raise RuntimeError(f"Fixture marker has unexpected contents: {marker}")
    if not marker.exists():
        marker.write_bytes(MARKER_CONTENT)

    entries: list[tuple[str, int, str]] = []
    empty_vault = FIXTURE_ROOT / "empty-vault"
    empty_vault.mkdir(parents=True, exist_ok=True)
    if any(empty_vault.iterdir()):
        raise RuntimeError(f"The empty-vault fixture must stay empty: {empty_vault}")

    group_counts = (1_000, 10_000, 50_000)
    for count in group_counts:
        group = f"files_{count}"
        for index in range(1, count + 1):
            content = (
                f"# Synthetic Akula fixture {index}\n"
                f"group={count}\n"
                f"marker=akula-fixture-{index % 997:03d}\n"
            ).encode("utf-8")
            write_fixture(f"{group}/note_{index:06d}.md", content, entries)

    line_counts = (30_000, 100_000)
    for count in line_counts:
        content = "".join(
            f"row={index:06d} marker={index % 101:03d} text=synthetic-akula\n"
            for index in range(1, count + 1)
        ).encode("utf-8")
        write_fixture(f"markdown/lines_{count}.md", content, entries)

    pixel = one_pixel_png()
    for count in (1, 10, 100):
        for index in range(1, count + 1):
            write_fixture(f"images_{count}/image_{index:03d}.png", pixel, entries)

    write_fixture("unicode/Привет-мир.md", "Русский текст\n".encode(), entries)
    write_fixture("unicode/日本語ノート.md", "日本語のテスト\n".encode(), entries)
    write_fixture("unicode/café-and-emoji-📚.md", "Unicode test\n".encode(), entries)

    write_fixture(
        "conflicts/source_a/same-name.md",
        b"source=A\ncontent=left\n",
        entries,
    )
    write_fixture(
        "conflicts/source_b/same-name.md",
        b"source=B\ncontent=right\n",
        entries,
    )
    write_fixture("corrupt/invalid-utf8.md", b"# invalid UTF-8\nvalue=\xff\n", entries)
    write_fixture("corrupt/truncated.png", b"\x89PNG\r\n\x1a\n\x00\x00", entries)
    write_fixture("corrupt/interrupted-write.tmp", b"partial synthetic write", entries)

    entries.sort(key=lambda entry: entry[0])
    manifest = "path\tbytes\tsha256\n" + "".join(
        f"{path}\t{size}\t{digest}\n" for path, size, digest in entries
    )
    manifest_bytes = manifest.encode("utf-8")
    write_fixture("SHA256SUMS.tsv", manifest_bytes, [])

    summary = {
        "format": 1,
        "synthetic_only": True,
        "generated_fixture_files": len(entries),
        "file_catalog_sizes": list(group_counts),
        "markdown_line_counts": list(line_counts),
        "image_set_sizes": [1, 10, 100],
        "manifest": "SHA256SUMS.tsv",
        "manifest_sha256": sha256(manifest_bytes),
    }
    summary_bytes = (json.dumps(summary, ensure_ascii=False, indent=2) + "\n").encode(
        "utf-8"
    )
    write_fixture("SUMMARY.json", summary_bytes, [])

    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    print(f"fixture_root={FIXTURE_ROOT}")


if __name__ == "__main__":
    main()
