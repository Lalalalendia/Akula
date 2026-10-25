"""Run under pinned public Ararra's PYTHONPATH, not a locally reimplemented parser."""
import hashlib
import json
import sys
from pathlib import Path

from chaptera_server.book_import import parse_book_import


def main(manifest_path: str):
    manifest_file = Path(manifest_path)
    source = json.loads(manifest_file.read_text())
    assert source["schema"] == "erida-import-batches-v1"
    all_numbers = []
    for batch in source["batches"]:
        filename = batch["filename"]
        raw = (manifest_file.parent / filename).read_bytes()
        assert len(raw) == batch["bytes"]
        assert hashlib.sha256(raw).hexdigest() == batch["sha256"]
        preview, candidates, prefix = parse_book_import(filename, raw)
        expected = list(range(batch["first_ordinal"], batch["last_ordinal"] + 1))
        numbers = [c.number for c in candidates]
        assert numbers == expected, (numbers, expected)
        assert preview["kind"] == "book" and preview["stored"] is False
        assert preview["chapter_count"] == batch["chapter_count"]
        assert preview["warnings"] == [], preview["warnings"]
        assert not prefix.strip()
        assert all(c.html and c.html.strip() for c in candidates)
        all_numbers.extend(numbers)
    assert all_numbers == list(range(1, source["expected_chapters"]+1))
    print(f"ERIDA_REAL_PARSER_PASS chapters={len(all_numbers)} batches={len(source['batches'])}")

if __name__ == "__main__":
    main(sys.argv[1])

