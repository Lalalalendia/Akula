#!/usr/bin/env python3
"""01t fail-closed tests: source-git identity, hashed pins, and NONRELEASE archive."""
import gzip
import importlib.util
import io
import json
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE=Path(__file__).resolve().parent
SPEC=importlib.util.spec_from_file_location("source_starter",HERE/"video-tools-01t-source-starter.py")
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
ROOT=HERE.parent.parent

class StarterTest(unittest.TestCase):
 def test_pinned_hashes_and_version_input(self):
  sample=("curl-cffi==0.17.0 ; implementation_name == 'cpython' \\\n"
          "    --hash=sha256:"+"a"*64+" \\\n"
          "websockets==17.3 \\\n"
          "    --hash=sha256:"+"b"*64+"\n")
  x=M.parse_requirements(sample)
  self.assertEqual(x["curl-cffi"]["version"],"0.17.0")
  self.assertEqual(x["websockets"]["hash_count"],1)
  custom="pyinstaller @ "+M.CUSTOM_WHEEL_URL+" \\\n    --hash=sha256:"+M.CUSTOM_WHEEL_SHA+"\n"
  p=M.parse_requirements(custom)["pyinstaller"]
  self.assertEqual(p["version"],"6.22.0")
  self.assertEqual(p["wheel_origin"],"CUSTOM_DIRECT_URL")
  self.assertEqual(p["custom_wheel_sha256"],M.CUSTOM_WHEEL_SHA)
  self.assertEqual(p["builder_git_commit"],M.PYBUILD)
  for badurl in (
    custom.replace(M.CUSTOM_WHEEL_SHA,"0"*64),
    custom.replace("Pyinstaller-Builds","unknown-repository"),
    custom.replace("pyinstaller-6.22.0","pyinstaller-6.23.0"),
  ):
   with self.assertRaises(M.Hold):
    M.parse_requirements(badurl)
  for bad in (
   "curl-cffi>=0.17.0 \\\n --hash=sha256:"+"a"*64,
   "curl-cffi==0.17.0 \\\n",
   "curl-cffi==0.17.0 \\\n --hash=sha256:INVALID",
   "curl-cffi==0.17.0 \\\n --hash=sha256:"+"a"*64+
       "\ncurl-cffi==0.17.0 \\\n --hash=sha256:"+"a"*64,
   "git+https://example.com/pkg.git",
  ):
   with self.subTest(bad=bad[:35]):
    with self.assertRaises(M.Hold):
     M.parse_requirements(bad)
  self.assertEqual(M.git_blob(b"hello"),"b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0")

 def test_nonrelease_allowlist_mutations_are_blocked(self):
  j=json.loads((ROOT/"docs/release/video18b04-five-exe-allowlist.json").read_text("utf8"))
  M.check_release_hold(j)
  for k in ("release_admitted","installer_signing_authorized",
            "stage_binary_distribution_authorized"):
   bad=json.loads(json.dumps(j))
   bad[k]=True
   with self.subTest(k=k),self.assertRaisesRegex(M.Hold,"release gate"):
    M.check_release_hold(bad)
  bad=json.loads(json.dumps(j))
  bad["components"][3]["license"]="Unlicense"
  with self.assertRaisesRegex(M.Hold,"binary/source license"):
   M.check_release_hold(bad)

 def test_exact_git_snapshot_is_deterministic_and_excludes_untracked(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td)
   subprocess.run(["git","init","-q",str(root)],check=True)
   (root/"LICENSE").write_text("source LICENSE\n","utf8")
   (root/"README.md").write_text("source README\n","utf8")
   (root/"src").mkdir()
   for i in range(5):
    (root/"src"/(str(i)+".py")).write_text("print('source')\n","utf8")
   subprocess.run(["git","-C",str(root),"add","."],check=True)
   subprocess.run(["git","-C",str(root),"-c","user.name=CI",
                   "-c","user.email=ci@example.org","commit","-qm","test"],check=True)
   sha=M.git_head(root)
   (root/"untracked.exe").write_bytes(b"MZ")
   first,count=M.archive_tracked(root,sha,"yt-dlp",[],["LICENSE","README.md"])
   second,n=M.archive_tracked(root,sha,"yt-dlp",[],["LICENSE","README.md"])
   self.assertEqual(first,second)
   self.assertEqual(count,n)
   with tarfile.open(fileobj=io.BytesIO(gzip.decompress(first)),mode="r:") as tf:
    names=tf.getnames()
   self.assertNotIn("yt-dlp/untracked.exe",names)
   self.assertIn("yt-dlp/LICENSE",names)
   with self.assertRaisesRegex(M.Hold,"checkout SHA changed"):
    M.archive_tracked(root,"0"*40,"yt-dlp",[],["LICENSE"])

 def test_workflow_source_artifact_and_incomplete_status_mandatory(self):
  wf=(ROOT/".github/workflows/akula-video18b09-windows-license-triage.yml").read_text("utf8")
  doc=(ROOT/"docs/release/video-tools-01t-source-starter.md").read_text("utf8")
  for term in (
   ".github/scripts/video-tools-01t-source-starter.py",
   ".github/scripts/video-tools-01t-source-starter.test.py",
   "docs/release/video-tools-01t-source-starter.md",
   "python3 .github/scripts/video-tools-01t-source-starter.test.py",
   "python3 .github/scripts/video-tools-01t-source-starter.py",
   "video-tools-01t-source-starter-HOLD",
   "repository: yt-dlp/Pyinstaller-Builds",
   "ref: 2026.08.19.215425",
   "source_trees=4",
   "VIDEO_TOOLS_01T_SOURCE_STARTER_PASS",
   "retention-days: 1","persist-credentials: false",
   "permissions:\n  contents: read",
  ):
   self.assertIn(term,wf,"source kit workflow must include "+term)
  for forbidden in ("workflow_dispatch:","\n  push:","contents: write","gh release create"):
   self.assertNotIn(forbidden,wf)
  for term in ("VIDEO-18 RELEASE HOLD","not complete corresponding source",
               "QuickJS","yt-dlp-ejs","PyInstaller","AGPL","GPLv3+"):
   self.assertIn(term,doc)
  self.assertIs(M.build.__doc__,None)
  self.assertIn("NOT complete corresponding source",M.README)
  self.assertIn("Pyinstaller-Builds",M.README)
  self.assertIn("VIDEO-18 RELEASE HOLD",M.README)

if __name__=="__main__":
 unittest.main(verbosity=2)
