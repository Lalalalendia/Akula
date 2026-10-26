#!/usr/bin/env python3
import csv, json, pathlib, subprocess, time
root=pathlib.Path(__file__).resolve().parent
cases=json.loads((root/"manifest.json").read_text())["files"]
out=root/"results";out.mkdir(exist_ok=True)
names={"dicom-rs":"akula-dicom-rs-probe","dcmfx":"akula-dcmfx-probe","toolkit":"akula-toolkit-probe"}
rows=[]
for name,bname in names.items():
    exe=root/name/"target"/"release"/bname
    for case in cases:
        if not exe.is_file():
            rows.append([name,case["file"],"NOT_COMPILED","", "","binary missing"])
            continue
        start=time.monotonic()
        try:
            p=subprocess.run([str(exe),str(root/"fixtures"/case["file"])],capture_output=True,text=True,timeout=30)
            status="DECODED" if p.returncode==0 and "status=decoded" in p.stdout else "REJECTED"
            if case["expect"]=="reject" and status=="REJECTED":status="EXPECTED_REJECTION"
            rows.append([name,case["file"],status,round((time.monotonic()-start)*1000,3),p.returncode,(p.stdout+p.stderr).strip().replace("\n"," | ")[:1100]])
        except Exception as e:
            rows.append([name,case["file"],"ERROR","","",str(e)])
with (out/"results.csv").open("w",newline="") as f:
    w=csv.writer(f);w.writerow(["library","file","status","runtime_ms","exit_code","detail"]);w.writerows(rows)
for r in rows:print("PROBE",*r[:4])
(out/"limitations.txt").write_text("Smoke only; no raw-pixel SHA parity, peak RSS, DICOMDIR, real device or clinical qualification. Different adapters return different pixel representations.\n")
