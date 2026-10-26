#!/usr/bin/env python3
"""CI-only deterministic PDF fixture for the *installed* Akula/Shiva beta test."""
import argparse
from pathlib import Path


def pdf_with_text():
    instructions = b"BT /F1 16 Tf 72 720 Td (AKULANATIVEPDFNEEDLE) Tj ET\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]"
        b" /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Length %d >>\nstream\n" % len(instructions)
        + instructions + b"endstream",
    ]
    result = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    positions = [0]
    for index, data in enumerate(objects, start=1):
        positions.append(len(result))
        result.extend(str(index).encode() + b" 0 obj\n" + data + b"\nendobj\n")
    offset = len(result)
    result.extend(b"xref\n0 6\n0000000000 65535 f \n")
    for at in positions[1:]:
        result.extend(b"%010d 00000 n \n" % at)
    result.extend(b"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n")
    result.extend(str(offset).encode() + b"\n%%EOF\n")
    return bytes(result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    path = Path(args.out)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(pdf_with_text())
    print("AKULA_PDF_FIXTURE=PASS;bytes=%d" % path.stat().st_size)
