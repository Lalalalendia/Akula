#!/usr/bin/env python3
"""Independent pydicom verification of large synthetic DICOM pixel data."""
import hashlib
import json
import pathlib
import pydicom

root = pathlib.Path(__file__).resolve().parent
meta = json.loads((root / "stress_manifest.json").read_text())
ds = pydicom.dcmread(root / "fixtures" / meta["file"])
pixels = ds.pixel_array
assert pixels.shape == (200, 512, 512), pixels.shape
assert pixels.dtype.kind == "i" and pixels.dtype.itemsize == 2, pixels.dtype
actual = hashlib.sha256(pixels.tobytes(order="C")).hexdigest()
assert actual == meta["pixel_sha256"], (actual, meta["pixel_sha256"])
print("STRESS_PYDICOM_REFERENCE_PASS pixels", pixels.size, "sha256", actual)
