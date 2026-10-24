#!/usr/bin/env python3
"""Deterministic two-page PDF for installed native PDFium acceptance.

Distinct page colours and text make wrong-page caching visibly measurable.
No third-party Python packages are required.
"""
import argparse
from pathlib import Path


def make_pdf():
    def stream(contents):
        return b"<< /Length %d >>\nstream\n" % len(contents) + contents + b"endstream"

    def page(content_id):
        return (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << /F1 5 0 R >> >> /Contents "
            + str(content_id).encode("ascii") + b" 0 R >>"
        )

    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        page(6),
        page(7),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(b"1 0 0 rg 0 0 612 792 re f\n0 0 0 rg BT /F1 20 Tf 72 720 Td (AKULA PAGE ONE) Tj ET\n"),
        stream(b"0 0.8 0 rg 0 0 612 792 re f\n0 0 0 rg BT /F1 20 Tf 72 720 Td (AKULA PAGE TWO) Tj ET\n"),
    ]
    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    positions = [0]
    for index, obj in enumerate(objects, 1):
        positions.append(len(output))
        output.extend(str(index).encode("ascii") + b" 0 obj\n" + obj + b"\nendobj\n")
    startxref = len(output)
    output.extend(b"xref\n0 8\n0000000000 65535 f \n")
    for position in positions[1:]:
        output.extend(b"%010d 00000 n \n" % position)
    output.extend(b"trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n")
    output.extend(str(startxref).encode("ascii") + b"\n%%EOF\n")
    return bytes(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    path = Path(args.out)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(make_pdf())
    print("AKULA_TWO_PAGE_PDF=PASS;bytes=%d;pages=2" % path.stat().st_size)
