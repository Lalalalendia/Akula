"""Deterministic, synthetic, same-origin HTML corpus; generates no live web traffic."""
import hashlib
import json
from pathlib import Path


def generate(root: Path, pages: int = 160, size: int = 128 * 1024):
    root = Path(root)
    pdir = root / "pages"
    pdir.mkdir(parents=True, exist_ok=True)
    manifest = {}
    links = []
    for i in range(pages):
        name = f"{i:05d}.html"
        # Repeated SHAKE-256 pattern is deterministic and nontrivial to hash.
        text = hashlib.shake_256(f"trial-fixture:{i}:2026-10-25".encode()).hexdigest(size // 2)
        prefix = f"<!doctype html><html><head><meta charset='utf-8'><title>{i}</title></head><body><pre>"
        suffix = "</pre></body></html>"
        data = (prefix + text + suffix).encode("utf-8")
        (pdir / name).write_bytes(data)
        manifest[name] = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
        links.append(f'<a href="/pages/{name}">page {i}</a>')
    (root / "index.html").write_text("<!doctype html><html><body>" + "\n".join(links) + "</body></html>", encoding="utf-8")
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


if __name__ == "__main__":
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("directory")
    p.add_argument("--pages", type=int, default=160)
    p.add_argument("--size", type=int, default=128*1024)
    a = p.parse_args()
    print(json.dumps({"files": len(generate(Path(a.directory), a.pages, a.size))}))
