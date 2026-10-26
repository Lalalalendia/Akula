#!/usr/bin/env python3
"""Only container integrity / checksum verification, NOT DICOM conformance."""
import hashlib, json, pathlib, struct
root=pathlib.Path(__file__).parent
data=json.loads((root/"manifest.json").read_text())
for row in data["files"]:
    blob=(root/"fixtures"/row["file"]).read_bytes()
    assert hashlib.sha256(blob).hexdigest()==row["whole_file_sha256"], row["file"]
    assert blob[128:132]==b"DICM", row["file"]
    assert struct.unpack_from("<HH",blob,132)==(2,0), row["file"]
    assert blob[136:138]==b"UL", row["file"]
    assert struct.unpack_from("<H",blob,138)[0]==4, row["file"]
    assert struct.unpack_from("<I",blob,140)[0] > 0, row["file"]
    if row["expect"]=="decode":
        assert blob.find(bytes.fromhex("e07f1000"))>144, row["file"]
    print("FIXTURE_SHA_OK",row["file"])
print("STRUCTURE_ONLY_OK",len(data["files"]),"DICOM conformance and decoder correctness NOT tested here.")
