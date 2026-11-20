#!/usr/bin/env python3
"""01t: exact Git source STARTER, explicitly not complete corresponding source."""
import gzip
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path

YT="f264a6cbd1f630894da0f368df00f8aae50140e0"
QJS="d7215b0ad3fc2c562e8317fd9bca058fd4863804"
PYBUILD="5eb86b862d61adc3df0f44e6e0b815b7d74fa3f"
UPSTREAM_PYINSTALLER="70fc17210920bce17f4ab09bbf8104b0dbd45338"
CUSTOM_WHEEL_URL="https://github.com/yt-dlp/Pyinstaller-Builds/releases/download/2026.08.19.215425/pyinstaller-6.22.0-py3-none-win_amd64.whl"
CUSTOM_WHEEL_SHA="294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a"
YT_EXE="f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d"
QJS_EXE="ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7"
BLOBS={
 "yt":{
  ".github/workflows/build.yml":"7cf8ad361deb6b455c16aa28e8e2bcca0c3256a8",
  "bundle/pyinstaller.py":"bcd03c89570f8f2489b861cd552e7181bf6b0a7a",
  "bundle/requirements/win-x64-pyinstaller.txt":"7b43607508aea6b7a4031bc05529a258c48bd1ec",
  "bundle/requirements/curl-cffi.txt":"e2e0ca6d87f090d30c53f0d514f8f36e8142bbb6",
  "bundle/requirements/pip.txt":"14bcf5c7f283125473711c88b280be63e31cd771",
  "pyproject.toml":"6d79ecbdae7d04a6c245393339643f01f7c97026",
  "uv.lock":"0745ea2435d6e6a86fb020942340cc2673f2a455",
  "THIRD_PARTY_LICENSES.txt":"88f9872c66b456703e91ca32db4041e1ec47258d",
  "LICENSE":"68a49daad8ff7e35068f2b7a97d643aab440eaec",
 },
 "quickjs":{"LICENSE":"fae657320cdf957cab10654ac39990fdb20e97ef"},
 "pybuild":{
  "README.md":"46fe51986985f5d9f3a7523097254670917f7425",
  ".github/workflows/build.yml":"413323417c48d73dc226f8a1f053fbd63f1a0322",
  "pyproject.toml":"ea0a9160bd5559528b162f999b2bacefc3bff6c5",
  "requirements/build.txt":"6af552ee058c12cf4ba1febdd2da05e8375edea7",
  "requirements/pip.txt":"92a1e34f1fbc6141bc9e59abfc96ae8b21d47f39",
  "LICENSE":"68a49daad8ff7e35068f2b7a97d643aab440eaec",
 },
 "akula":{
  "LICENSE":"fe6b9036ba1818c79c15f802a586a9711ece2b97",
  "labs/video-lite/Cargo.toml":"4adb6f13f345749200e21c05c9b94f1626c4cebd",
 }
}
OUTPUT={
 "yt":"yt-dlp-2026.10.26-GIT-ONLY.tar.gz",
 "quickjs":"quickjs-v0.18.0-GIT-ONLY.tar.gz",
 "pybuild":"pyinstaller-build-recipe-GIT-ONLY.tar.gz",
 "akula":"akula-video-mux-GIT-ONLY.tar.gz",
}
REQ={
 "win-x64-pyinstaller":"bundle/requirements/win-x64-pyinstaller.txt",
 "curl-cffi":"bundle/requirements/curl-cffi.txt",
 "pip":"bundle/requirements/pip.txt",
}

class Hold(Exception):
 pass

def need(ok,why):
 if not ok:
  raise Hold("VIDEO_TOOLS_01T_HOLD: "+why)

def git_head(p):
 return subprocess.check_output(["git","-C",str(p),"rev-parse","HEAD"],text=True,timeout=15).strip()

def git_blob(b):
 return hashlib.sha1(b"blob "+str(len(b)).encode()+b"\0"+b).hexdigest()

def verified_blobs(root,kind):
 result=[]
 for name,sha in sorted(BLOBS[kind].items()):
  p=root/name
  need(p.is_file() and not p.is_symlink(),"missing exact upstream source file "+name)
  data=p.read_bytes()
  need(git_blob(data)==sha,"source Git blob mismatch: "+name)
  result.append({"path":name,"git_blob_sha1":sha,"bytes":len(data)})
 return result

def parse_requirements(text):
 """Pin all build inputs, including non-PyPI custom PyInstaller wheel."""
 pins={}
 key=None
 for original in text.splitlines():
  line=original.strip()
  if not line or line.startswith("#"):
   continue
  if line.startswith("--hash="):
   need(key is not None and
        re.fullmatch(r"--hash=sha256:[0-9a-f]{64}\s*\\?",line) is not None,
        "malformed or orphan SHA-256 wheel hash")
   digest=line.split(":")[1].split()[0].rstrip("\\")
   pins[key]["hash_count"]+=1
   if key=="pyinstaller" and pins[key].get("wheel_origin")=="CUSTOM_DIRECT_URL":
    need(digest==CUSTOM_WHEEL_SHA,"custom PyInstaller wheel SHA-256 changed")
    pins[key]["custom_wheel_sha256"]=digest
   continue
  if key is not None:
   need(pins[key]["hash_count"]>=1,"unhashed dependency "+key)
  m=re.match(r"^([a-z][a-z0-9_.-]*)==([0-9][a-zA-Z0-9.+_-]*)\b",line,re.I)
  if m is None:
   need(line==("pyinstaller @ "+CUSTOM_WHEEL_URL+" \\"),
        "unknown or unpinned direct wheel URL; fail closed")
   key="pyinstaller"
   version="6.22.0"
   extra={"wheel_origin":"CUSTOM_DIRECT_URL","custom_wheel_url":CUSTOM_WHEEL_URL,
          "custom_wheel_sha256":None,"builder_git_commit":PYBUILD,
          "upstream_pyinstaller_tag_commit":UPSTREAM_PYINSTALLER,
          "status":"CUSTOM_BOOTLOADER_WHEEL_REQUIRES_CORRESPONDING_SOURCE_REVIEW"}
  else:
   need(line.endswith("\\"),"unversioned or unsupported requirement")
   key=m.group(1).lower().replace("_","-")
   version=m.group(2)
   extra={"status":"PINNED_BUILD_INSTALL_INPUT_NOT_VERIFIED_EMBEDDED_SOURCE"}
  need(key not in pins,"duplicate dependency "+key)
  pins[key]={"version":version,"hash_count":0,**extra}
 need(key is not None and pins[key]["hash_count"]>=1,"empty or unhashed requirements")
 if "pyinstaller" in pins and pins["pyinstaller"].get("wheel_origin")=="CUSTOM_DIRECT_URL":
  need(pins["pyinstaller"]["hash_count"]==1 and
       pins["pyinstaller"]["custom_wheel_sha256"]==CUSTOM_WHEEL_SHA,
       "custom wheel must have exactly one pinned SHA-256")
 return pins


def check_release_hold(allow):
 names=[x.get("name") for x in allow.get("components",[])]
 need(allow.get("schema")==1 and
      names==["akula-app.exe","shiva-pdf-preview.exe","akula-video-mux.exe",
              "akula-yt-dlp.exe","akula-quickjs.exe"] and
      allow.get("release_admitted") is False and
      allow.get("installer_signing_authorized") is False and
      allow.get("stage_binary_distribution_authorized") is False and
      allow.get("independent_owner_approval_required") is True,
      "EXE allowlist release gate or component list changed")
 yt,q=allow["components"][3:5]
 need(yt.get("tag_commit_sha")==YT and yt.get("sha256")==YT_EXE and
      yt.get("license")=="GPL-3.0-or-later" and
      yt.get("source_license")=="Unlicense" and
      yt.get("license_status")=="REVIEW_BUNDLED_DEPENDENCIES",
      "yt-dlp binary/source license or digest changed")
 need(q.get("tag_commit_sha")==QJS and q.get("sha256")==QJS_EXE and
      q.get("license")=="MIT" and
      q.get("license_status")=="REVIEW_NOTICE_PACKAGING",
      "QuickJS pinned release MIT review changed")

def validate_windows_inputs(ytroot):
 groups={k:parse_requirements((ytroot/v).read_text("utf8")) for k,v in REQ.items()}
 for group,name,version in [
  ("win-x64-pyinstaller","pyinstaller","6.22.0"),
  ("curl-cffi","curl-cffi","0.17.0"),
  ("curl-cffi","websockets","17.3"),
  ("curl-cffi","yt-dlp-ejs","0.8.0"),
  ("curl-cffi","pycryptodomex","3.24.1"),
  ("curl-cffi","mutagen","1.48.1"),
 ]:
  need(groups[group].get(name,{}).get("version")==version,
       "pinned Windows build dependency drift: "+name)
 builder=groups["win-x64-pyinstaller"]["pyinstaller"]
 need(builder.get("wheel_origin")=="CUSTOM_DIRECT_URL" and
      builder.get("custom_wheel_url")==CUSTOM_WHEEL_URL and
      builder.get("custom_wheel_sha256")==CUSTOM_WHEEL_SHA and
      builder.get("builder_git_commit")==PYBUILD,
      "actual Windows PyInstaller wheel provenance not verified")
 py=tomllib.loads((ytroot/"pyproject.toml").read_text("utf8"))
 need(py["project"]["license"]=="Unlicense" and
      "yt-dlp-ejs==0.8.0" in py["project"]["optional-dependencies"]["pin"],
      "exact tag project pin metadata mismatch")
 build=(ytroot/".github/workflows/build.yml").read_text("utf8")
 for token in ("runner: windows-2025","python-version: '3.14'",
               "bundle/requirements/curl-cffi.txt",
               "python -m bundle.pyinstaller",
               "python devscripts/make_lazy_extractors.py"):
  need(token in build,"Windows upstream build recipe changed: "+token)
 spec=(ytroot/"bundle/pyinstaller.py").read_text("utf8")
 need("--add-data=THIRD_PARTY_LICENSES.txt:." in spec,"bundled notice build step absent")
 return groups

def archive_tracked(root,commit,prefix,selectors,must_have):
 need(git_head(root)==commit,"archive checkout SHA changed")
 cmd=["git","-C",str(root),"archive","--format=tar",
      "--prefix="+prefix+"/","HEAD",*selectors]
 raw=subprocess.check_output(cmd,timeout=90)
 need(1024<len(raw)<80_000_000,"source archive missing or unbounded")
 names=set()
 with tarfile.open(fileobj=io.BytesIO(raw),mode="r:") as tar:
  for m in tar.getmembers():
   path=m.name
   at_source_root=(m.isdir() and path==prefix)
   need((at_source_root or path.startswith(prefix+"/")) and
        ".." not in path.split("/") and not path.startswith("/") and
        not path.lower().endswith((".exe",".dll")) and
        (m.isfile() or m.isdir()),"unsafe or compiled Windows file inside source snapshot")
   if m.isfile():
    names.add(path)
 need(len(names)>5 and all(prefix+"/"+x in names for x in must_have),
      "required tracked source/build/license file missing from source archive")
 return gzip.compress(raw,compresslevel=9,mtime=0),len(names)

def build(allow,ytroot,qroot,pyroot,akula,head):
 check_release_hold(allow)
 need(git_head(ytroot)==YT and git_head(qroot)==QJS and git_head(pyroot)==PYBUILD and git_head(akula)==head,
      "source checkout commits do not match pinned identity")
 evidence={k:verified_blobs(root,k)
           for k,root in (("yt",ytroot),("quickjs",qroot),("pybuild",pyroot),("akula",akula))}
 pins=validate_windows_inputs(ytroot)
 data={}
 snapshots={}
 specs=[
  ("yt",ytroot,YT,[],list(BLOBS["yt"]),"yt-dlp"),
  ("quickjs",qroot,QJS,[],["LICENSE"],"quickjs-ng"),
  ("pybuild",pyroot,PYBUILD,[],["LICENSE","README.md",
    ".github/workflows/build.yml","requirements/build.txt",
    "requirements/pip.txt"],"pyinstaller-builds"),
  ("akula",akula,head,["LICENSE","labs/video-lite"],
   ["LICENSE","labs/video-lite/Cargo.toml","labs/video-lite/Cargo.lock",
    "labs/video-lite/src/bin/dash.rs","labs/video-lite/src/version_01o.rs"],"akula"),
 ]
 for kind,root,sha,selected,required,prefix in specs:
  archive,count=archive_tracked(root,sha,prefix,selected,required)
  data[kind]=archive
  snapshots[kind]={"filename":OUTPUT[kind],"git_commit":sha,
                   "tracked_file_count":count,"sha256":hashlib.sha256(archive).hexdigest(),
                   "bytes":len(archive),"contents":"Git-tracked sources only; not dependencies"}
 manifest={
  "schema":1,"stage":"VIDEO-TOOLS-01t four-source starter / NOT legally complete corresponding source",
  "exact_akula_head":head,"release_admitted":False,
  "legal_approval":False,"complete_corresponding_source_available":False,
  "corresponding_source_rebuilt_and_verified":False,
  "windows_executables_included":False,"notices_distribution_approved":False,
  "publisher_signing_authorized":False,
  "snapshots":snapshots,"pinned_git_blob_evidence":evidence,
  "custom_pyinstaller_wheel":{
   "url":CUSTOM_WHEEL_URL,"sha256":CUSTOM_WHEEL_SHA,
   "recipe_git_commit":PYBUILD,
   "upstream_pyinstaller_tag_commit":UPSTREAM_PYINSTALLER,
   "wheel_bytes_included":False,
   "exact_wheel_to_source_match_verified":False,
  },
  "windows_x64_pinned_install_requirements":pins,
  "binary_evidence":{"yt_dlp_exe_sha256":YT_EXE,"quickjs_exe_sha256":QJS_EXE,
                     "actual_01s_module_names":1574,"unmapped_module_names":249},
  "unresolved_for_release":[
   "Acquire exact matching third-party Python wheel source distributions (not just hashed wheel pins)",
   "Reconcile special yt-dlp/Pyinstaller-Builds MinGW wheel SHA with PyInstaller v6.22.0 upstream code and bootloader source: this starter has build recipe only",
   "Establish exact provenance of Python 3.14, PyInstaller bootloader, DLL/PYD and curl-cffi native libraries",
   "Obtain and verify yt-dlp-ejs 0.8.0 generated JS corresponding sources",
   "Rebuild exact yt-dlp Windows EXE and prove binary/source correspondence; no rebuild is done here",
   "Acquire Rust Cargo transitive corresponding sources and build environment beyond Cargo.lock",
   "Verify QuickJS native dependencies and MIT notice inclusion in eventual distribution",
   "Independent full GPLv3+/AGPL corresponding-source, attribution, packaging and owner approval",
  ]
 }
 return manifest,data

README="""VIDEO-TOOLS-01t: source starter -- NOT complete corresponding source.
Contains four Git snapshots: yt-dlp 2026.10.26, QuickJS v0.18.0,
Pyinstaller-Builds recipe at 2026.08.19.215425, and Akula video-lite at
exact PR HEAD. The custom PyInstaller wheel and upstream PyInstaller code
are NOT included.
No Python wheels/sdists, CPython/bootloader/DLL source, JavaScript EJS generated
sources, or Rust Cargo transitive source archives are included. These are
not yet proven to match all contents of the compiled Windows executables.
See manifest-NOT-COMPLETE.json for every unresolved obligation.
No Windows EXE, installer, publisher signing key or release authorization.
VIDEO-18 RELEASE HOLD remains mandatory.
"""

def cli(argv):
 need(len(argv)==7,"usage: script allowlist yt-source quickjs-source builder-source HEAD output-dir")
 _,apath,yroot,qroot,proot,head,output=argv
 need(os.environ.get("GITHUB_ACTIONS")=="true" and
      os.environ.get("GITHUB_EVENT_NAME")=="pull_request" and
      re.fullmatch(r"[0-9a-f]{40}",head) is not None and
      head==os.environ.get("EXPECTED_HEAD_SHA"),
      "not exact PR HEAD in hosted source-only workflow")
 akula=Path(os.environ.get("GITHUB_WORKSPACE","")).resolve(strict=True)
 temp=Path(os.environ.get("RUNNER_TEMP","")).resolve(strict=True)
 out=Path(output).resolve(strict=False)
 need(out.parent==temp and out.name=="video-tools-01t-source-starter-HOLD" and
      not out.exists(),"staging directory not fresh under disposable runner")
 ytroot=Path(yroot).resolve(strict=True)
 qjsroot=Path(qroot).resolve(strict=True)
 pyroot=Path(proot).resolve(strict=True)
 need(len({ytroot,qjsroot,pyroot,akula})==4,"source checkout paths overlap")
 manifest,archives=build(json.loads(Path(apath).read_text("utf8")),ytroot,qjsroot,pyroot,akula,head)
 out.mkdir(mode=0o700)
 for kind,data in archives.items():
  (out/OUTPUT[kind]).write_bytes(data)
 (out/"manifest-NOT-COMPLETE.json").write_text(json.dumps(manifest,sort_keys=True,indent=2)+"\n","utf8")
 (out/"README-NOT-COMPLETE.txt").write_text(README,"utf8")
 need(sorted(x.name for x in out.iterdir())==
      sorted([*OUTPUT.values(),"manifest-NOT-COMPLETE.json","README-NOT-COMPLETE.txt"]),
      "source starter contains unexpected files")
 print("VIDEO_TOOLS_01T_SOURCE_STARTER_PASS source_trees=4 wheels=0 "
       "complete_source=0 rebuilt_exe=0 legal_approval=0 release=0")

if __name__=="__main__":
 try:
  cli(sys.argv)
 except (Hold,OSError,KeyError,ValueError,tarfile.TarError,subprocess.SubprocessError) as exc:
  print(str(exc),file=sys.stderr)
  sys.exit(3)
