#!/usr/bin/env python3
"""Independent reference check with pydicom on synthetic data, not clinical validation."""
import csv
import hashlib
import json
from pathlib import Path
import pydicom
root = Path(__file__).resolve().parent
cases = json.loads((root / 'manifest.json').read_text())['files']
rows = []
failures = []
lossy = {'1.2.840.10008.1.2.4.50'}
for case in cases:
    path = root / 'fixtures' / case['file']
    actual = ''
    try:
        ds = pydicom.dcmread(path, force=False)
        pixels = ds.pixel_array
        actual = hashlib.sha256(pixels.tobytes(order='C')).hexdigest()
        shape_ok = (int(ds.Rows) == case.get('height') and
                    int(ds.Columns) == case.get('width') and
                    int(getattr(ds, 'NumberOfFrames', 1)) == case.get('frames', 1))
        if case['expect'] == 'reject':
            status = 'UNEXPECTED_ACCEPTANCE'
        elif not shape_ok:
            status = 'SHAPE_MISMATCH'
        elif case.get('transfer_syntax') in lossy:
            status = 'LOSSY_DECODE_ONLY'
        elif actual == case.get('pixel_sha256'):
            status = 'RAW_HASH_MATCH'
        else:
            status = 'RAW_HASH_MISMATCH'
        detail = f'shape={pixels.shape} dtype={pixels.dtype}'
    except Exception as e:
        status = 'EXPECTED_REJECTION' if case['expect'] == 'reject' else 'DECODE_ERROR'
        detail = repr(e)[:600]
    rows.append(dict(file=case['file'],status=status,actual_sha256=actual,
                     expected_sha256=case.get('pixel_sha256',''),detail=detail))
    print('PYDICOM_REFERENCE',case['file'],status)
    if status not in {'RAW_HASH_MATCH','LOSSY_DECODE_ONLY','EXPECTED_REJECTION'}:
        failures.append(f"{case['file']}:{status}")
try:
    pydicom.dcmread(root / 'fixtures' / 'not_dicom.txt', force=False)
except Exception:
    print('PYDICOM_REFERENCE not_dicom.txt EXPECTED_REJECTION')
else:
    failures.append('not_dicom.txt:UNEXPECTED_ACCEPTANCE')
out = root / 'results'
out.mkdir(exist_ok=True)
with (out / 'reference.csv').open('w',newline='') as f:
    w=csv.DictWriter(f,fieldnames=['file','status','actual_sha256','expected_sha256','detail'])
    w.writeheader();w.writerows(rows)
if failures:
    raise SystemExit('PYDICOM_REFERENCE_FAIL '+', '.join(failures))
print('PYDICOM_REFERENCE_PASS',len(rows))
