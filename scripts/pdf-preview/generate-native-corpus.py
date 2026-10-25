#!/usr/bin/env python3
"""Deterministic PDFium acceptance corpus, built with the Python standard library.

These synthetic fixtures do not prove that Akula's installed GUI passes.
"""
import argparse
import hashlib
import json
import zlib
from pathlib import Path


class Pdf:
    def __init__(self):
        self.objects = [None]

    def add(self, data=b""):
        self.objects.append(data)
        return len(self.objects) - 1

    def set(self, number, data):
        self.objects[number] = data

    def stream(self, data, dictionary=b""):
        return self.add(b"<< /Length %d %s >>\nstream\n" % (len(data), dictionary) + data + b"\nendstream")

    def build(self, *, version='1.4'):
        out = bytearray(b"%PDF-" + version.encode('ascii') + b"\n%\xe2\xe3\xcf\xd3\n")
        offsets = [0]
        for index, obj in enumerate(self.objects[1:], 1):
            assert isinstance(obj, bytes), f"unassigned PDF object {index}"
            offsets.append(len(out))
            out.extend(b"%d 0 obj\n" % index + obj + b"\nendobj\n")
        xref = len(out)
        out.extend(b"xref\n0 %d\n0000000000 65535 f \n" % len(self.objects))
        for pos in offsets[1:]:
            out.extend(b"%010d 00000 n \n" % pos)
        out.extend(b"trailer\n<< /Root 1 0 R /Size %d >>\nstartxref\n%d\n%%%%EOF\n" % (len(self.objects), xref))
        return bytes(out)


def base():
    pdf = Pdf()
    catalog = pdf.add()
    tree = pdf.add()
    font = pdf.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    assert (catalog, tree, font) == (1, 2, 3)
    return pdf


def finish(pdf, pages):
    pdf.set(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    kids = b" ".join(b"%d 0 R" % number for number in pages)
    pdf.set(2, b"<< /Type /Pages /Kids [" + kids + b"] /Count %d >>" % len(pages))
    return pdf.build()


def labelled(count, *, rotations=()):
    pdf = base()
    pages = []
    for number in range(1, count + 1):
        rgb = b"1 0 0 rg" if number % 2 else b"0 0.7 0 rg"
        canvas = rgb + b" 0 0 612 792 re f\n0 0 0 rg BT /F1 30 Tf 48 730 Td (AKULA PAGE %d) Tj ET\n" % number
        content = pdf.stream(canvas)
        rotate = (b"/Rotate %d " % rotations[number-1]) if rotations else b""
        page = pdf.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] " +
                       rotate + b"/Resources << /Font << /F1 3 0 R >> >> " +
                       b"/Contents %d 0 R >>" % content)
        pages.append(page)
    return finish(pdf, pages)


def scan_only():
    pdf = base()
    width = height = 64
    rgb = bytearray()
    for y in range(height):
        for x in range(width):
            rgb.extend((240 if (x+y)//8 % 2 else 22, (x*4) % 256, (y*4) % 256))
    bitmap = zlib.compress(bytes(rgb), level=9)
    image = pdf.stream(bitmap, b"/Type /XObject /Subtype /Image /Width 64 /Height 64 "
                       b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode")
    draw = pdf.stream(b"q 500 0 0 700 56 40 cm /Im0 Do Q\n")
    page = pdf.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] " +
                   b"/Resources << /XObject << /Im0 %d 0 R >> >> /Contents %d 0 R >>" % (image, draw))
    return finish(pdf, [page])


def generate(out):
    out.mkdir(parents=True, exist_ok=True)
    samples = {
        'native_two_page.pdf': labelled(2),
        'native_1000_page.pdf': labelled(1000),
        'native_rotated.pdf': labelled(2, rotations=(90, 270)),
        'native_scan_no_text.pdf': scan_only(),
        'native_truncated.pdf': b'%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n',
        'native_invalid_magic.pdf': b'INVALID MAGIC NOT PDF\n',
    }
    manifest = {}
    for name, contents in samples.items():
        (out/name).write_bytes(contents)
        manifest[name] = {'sha256': hashlib.sha256(contents).hexdigest(), 'bytes': len(contents)}
    (out/'native_manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print('AKULA_NATIVE_CORPUS=PASS;files=6;synthetic=1')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--out-dir', type=Path, required=True)
    generate(parser.parse_args().out_dir)
