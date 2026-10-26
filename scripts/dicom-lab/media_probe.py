#!/usr/bin/env python3
"""Exercise a valid pydicom-produced DICOMDIR, extensionless paths and copied media."""
import hashlib
import json
import pathlib
import shutil
from pydicom import dcmread
from pydicom.fileset import FileSet

root = pathlib.Path(__file__).resolve().parent
media = root / "fixtures" / "medical_media"
if media.exists():
    shutil.rmtree(media)
fs = FileSet()
fs.ID = "AKULALAB"
source_names = ["mono8_explicit.dcm", "mono8_no_extension", "mono16_signed.dcm"]
for name in source_names:
    ds = dcmread(root / "fixtures" / name)
    # Preserve source bytes; add only metadata needed for FileSet directory records.
    ds.StudyID = "LABSTUDY"
    ds.StudyDescription = "SYNTHETIC_NOT_PATIENT"
    ds.SeriesDescription = "AKULA_TEST"
    fs.add(ds)
fs.write(str(media))
assert (media / "DICOMDIR").is_file()
index = dcmread(media / "DICOMDIR")
records = list(index.DirectoryRecordSequence)
assert sum(r.DirectoryRecordType == "IMAGE" for r in records) == 3
report = {"root": str(media.relative_to(root)),
          "records": len(records),
          "images": [], "extensionless": True,
          "copied_media_readable": False,
          "missing_file_detected": False}
loaded = FileSet(str(media / "DICOMDIR"))
assert len(loaded) == 3, len(loaded)
for instance in loaded:
    p = pathlib.Path(instance.path)
    assert p.is_file()
    assert p.resolve().is_relative_to(media.resolve())
    assert not p.suffix, f"Expected extensionless media file: {p}"
    ds = instance.load()
    assert int(ds.Rows) == 8 and int(ds.Columns) == 8
    report["images"].append({"path": str(p.relative_to(media)),
                              "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                              "uid": str(ds.SOPInstanceUID)})
assert len({x["uid"] for x in report["images"]}) == 3
copy = root / "fixtures" / "medical_media_copied"
if copy.exists():
    shutil.rmtree(copy)
shutil.copytree(media, copy)
loaded_copy = FileSet(str(copy / "DICOMDIR"))
assert len(loaded_copy) == 3
assert all(pathlib.Path(inst.path).is_file() for inst in loaded_copy)
report["copied_media_readable"] = True
first = pathlib.Path(next(iter(loaded_copy)).path)
first.unlink()
report["missing_file_detected"] = sum(not pathlib.Path(i.path).is_file() for i in loaded_copy) == 1
assert report["missing_file_detected"], "DICOMDIR dangling reference was not detected"
out = root / "results"
out.mkdir(exist_ok=True)
(out / "media_results.json").write_text(json.dumps(report, indent=2))
print("DICOMDIR_OK images=3 extensionless=3 copy_reopen=PASS missing_reference=DETECTED")
