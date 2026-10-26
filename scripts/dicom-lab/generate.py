#!/usr/bin/env python3
"""Deterministic synthetic DICOM Part 10 samples; no actual patient data."""
import hashlib
import io
import json
import pathlib
import struct
import sys
from PIL import Image

ROOT = pathlib.Path(__file__).parent
FIXTURES = ROOT / "fixtures"
SC_UID = "1.2.840.10008.5.1.4.1.1.7"
MF_UID = "1.2.840.10008.5.1.4.1.1.7.3"
IMPL_UID = "2.25.326388818347560521005678978231154333002"
STUDY_UID = "2.25.326388818347560521005678978231154333100"
SERIES_UID = "2.25.326388818347560521005678978231154333101"
LONG_VR = {"OB", "OD", "OF", "OL", "OV", "OW", "SQ", "UC", "UN", "UR", "UT", "SV", "UV"}
manifest = []

def pad(data, vr):
    data = bytes(data)
    if len(data) % 2:
        data += b"\x00" if vr in ("OB", "OW", "UI") else b" "
    return data

def value(vr, v):
    if vr == "US":
        return struct.pack("<H", v)
    if vr == "UL":
        return struct.pack("<I", v)
    if isinstance(v, str):
        return pad(v.encode("ascii"), vr)
    return pad(v, vr)

def element(tag, vr, v, explicit=True):
    data = value(vr, v)
    start = struct.pack("<HH", *tag)
    if not explicit:
        return start + struct.pack("<I", len(data)) + data
    if vr in LONG_VR:
        return start + vr.encode() + b"\x00\x00" + struct.pack("<I", len(data)) + data
    return start + vr.encode() + struct.pack("<H", len(data)) + data

def encode_entries(entries, explicit=True):
    return b"".join(element(tag, vr, v, explicit) for tag, vr, v in sorted(entries))

def encapsulate(frame):
    frame = pad(frame, "OB")
    return (
        struct.pack("<HH", 0x7FE0, 0x0010) + b"OB\x00\x00" + struct.pack("<I", 0xffffffff)
        + struct.pack("<HHI", 0xfffe, 0xe000, 0)
        + struct.pack("<HHI", 0xfffe, 0xe000, len(frame)) + frame
        + struct.pack("<HHI", 0xfffe, 0xe0dd, 0)
    )

def rle(mono):
    encoded = bytearray()
    for i in range(0, len(mono), 128):
        segment = mono[i:i+128]
        encoded.append(len(segment)-1)
        encoded.extend(segment)
    return struct.pack("<II", 1, 64) + bytes(56) + bytes(encoded)

def create(name, w, h, bits, samples, pixels, transfer, invert=False,
           signed=False, frames=1, implicit=False, compressed=None, sop=SC_UID):
    idx = len(manifest) + 1
    instance = f"2.25.326388818347560521005678978231154333{200+idx}"
    meta = [
        ((0x0002, 0x0001), "OB", b"\x00\x01"),
        ((0x0002, 0x0002), "UI", sop),
        ((0x0002, 0x0003), "UI", instance),
        ((0x0002, 0x0010), "UI", transfer),
        ((0x0002, 0x0012), "UI", IMPL_UID),
        ((0x0002, 0x0013), "SH", "AKULAPROBE01"),
    ]
    m = encode_entries(meta)
    preamble = bytes(128) + b"DICM" + element((0x0002,0x0000), "UL", len(m)) + m
    entries = [
        ((0x0008,0x0016), "UI", sop), ((0x0008,0x0018), "UI", instance),
        ((0x0008,0x0020), "DA", "20261026"), ((0x0008,0x0030), "TM", "120000"),
        ((0x0008,0x0060), "CS", "OT"), ((0x0008,0x0064), "CS", "WSD"),
        ((0x0010,0x0010), "PN", "SYNTHETIC^TEST"),
        ((0x0010,0x0020), "LO", "NO_PATIENT_DATA"),
        ((0x0020,0x000D), "UI", STUDY_UID), ((0x0020,0x000E), "UI", SERIES_UID),
        ((0x0020,0x0011), "IS", "1"), ((0x0020,0x0013), "IS", str(idx)),
        ((0x0028,0x0002), "US", samples),
        ((0x0028,0x0004), "CS", "MONOCHROME1" if invert else ("RGB" if samples==3 else "MONOCHROME2")),
        ((0x0028,0x0010), "US", h), ((0x0028,0x0011), "US", w),
        ((0x0028,0x0100), "US", bits), ((0x0028,0x0101), "US", bits),
        ((0x0028,0x0102), "US", bits-1), ((0x0028,0x0103), "US", int(signed)),
        ((0x0028,0x1050), "DS", "128" if bits==8 else "0"),
        ((0x0028,0x1051), "DS", "256" if bits==8 else "1024"),
        ((0x0028,0x1052), "DS", "-1024" if signed else "0"),
        ((0x0028,0x1053), "DS", "1"),
    ]
    if frames > 1:
        entries.append(((0x0028,0x0008), "IS", str(frames)))
    if samples == 3:
        entries.append(((0x0028,0x0006), "US", 0))
    dataset = encode_entries(entries, not implicit)
    if compressed is None:
        dataset += element((0x7FE0,0x0010), "OB" if bits==8 else "OW", pixels, not implicit)
    else:
        dataset += encapsulate(compressed)
    blob = preamble + dataset
    (FIXTURES / name).write_bytes(blob)
    manifest.append({
        "file": name, "size": len(blob), "width": w, "height": h,
        "samples": samples, "bits": bits, "signed": signed, "frames": frames,
        "transfer_syntax": transfer, "pixel_sha256": hashlib.sha256(pixels).hexdigest(),
        "whole_file_sha256": hashlib.sha256(blob).hexdigest(),
        "expect": "decode", "note": "synthetic, no patient data"
    })

def main():
    FIXTURES.mkdir(parents=True, exist_ok=True)
    explicit = "1.2.840.10008.1.2.1"
    mono = bytes((x*17+y*13)%256 for y in range(8) for x in range(8))
    create("mono8_explicit.dcm",8,8,8,1,mono,explicit)
    signed = b"".join(struct.pack("<h",x*151+y*47-1000) for y in range(8) for x in range(8))
    create("mono16_signed.dcm",8,8,16,1,signed,explicit,signed=True)
    create("mono8_inverted.dcm",8,8,8,1,mono,explicit,invert=True)
    create("mono8_no_extension",8,8,8,1,mono,explicit)
    create("mono8_implicit.dcm",8,8,8,1,mono,"1.2.840.10008.1.2",implicit=True)
    rgb=bytes((x*17+y*13+c*51)%256 for y in range(8) for x in range(8) for c in range(3))
    create("rgb8_explicit.dcm",8,8,8,3,rgb,explicit)
    multi=b"".join(struct.pack("<h",(f*47+i*9)%8000-4000) for f in range(40) for i in range(64*64))
    create("multi40_mono16.dcm",64,64,16,1,multi,explicit,signed=True,frames=40,sop=MF_UID)
    create("mono8_rle.dcm",8,8,8,1,mono,"1.2.840.10008.1.2.5",compressed=rle(mono))
    image=Image.frombytes("L",(8,8),mono)
    out=io.BytesIO()
    image.save(out,format="JPEG",quality=95)
    create("mono8_jpeg.dcm",8,8,8,1,mono,"1.2.840.10008.1.2.4.50",compressed=out.getvalue())
    corrupt=(FIXTURES/"mono16_signed.dcm").read_bytes()[:-43]
    (FIXTURES/"invalid_truncated.dcm").write_bytes(corrupt)
    manifest.append({"file":"invalid_truncated.dcm","size":len(corrupt),
                     "expect":"reject","whole_file_sha256":hashlib.sha256(corrupt).hexdigest(),
                     "note":"deliberately truncated"})
    (FIXTURES/"not_dicom.txt").write_text("This is not DICOM.\n",encoding="utf8")
    (ROOT/"manifest.json").write_text(json.dumps({"created":"2026-10-26","data_type":"synthetic",
                                                     "files":manifest},indent=2),encoding="utf8")
    print(f"DICOM_CASES={len(manifest)}")

if __name__=="__main__":
    main()
