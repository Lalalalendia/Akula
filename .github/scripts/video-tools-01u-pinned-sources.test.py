#!/usr/bin/env python3
"""VIDEO-TOOLS-01u fail-closed source identity, license and workflow tests."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE=Path(__file__).resolve().parent
SPEC=importlib.util.spec_from_file_location("source_addon",HERE/"video-tools-01u-pinned-sources.py")
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
ROOT=HERE.parent.parent


def base_manifest():
    return {
        "schema":1,"exact_akula_head":"a"*40,"release_admitted":False,
        "legal_approval":False,"complete_corresponding_source_available":False,
        "corresponding_source_rebuilt_and_verified":False,
        "windows_executables_included":False,
        "snapshots":{
            k:{"sha256":"b"*64,"tracked_file_count":20}
            for k in ("yt","quickjs","pybuild","akula")
        }
    }


class AddonTest(unittest.TestCase):
    def test_two_immutable_commits_and_no_prebuilt_bootloader_images(self):
        self.assertEqual(len(M.PY_SHA),40)
        self.assertEqual(len(M.EJS_SHA),40)
        self.assertEqual(M.EJS_SHA,"4fb477f4af56880cfd324c48bd4294a2d2294e50")
        self.assertNotIn("PyInstaller/bootloader",M.PY_FILES)
        self.assertIn("bootloader",M.PY_FILES)
        self.assertEqual(set(M.OUTPUT),{"pyinstaller","ejs"})
        self.assertTrue(all(x.endswith("-SOURCE-ONLY.tar.gz") for x in M.OUTPUT.values()))

    def test_base_four_snapshots_never_fake_complete_release(self):
        b=base_manifest()
        M.verify_base_starter(b,"a"*40)
        for key,value in [
            ("release_admitted",True),
            ("legal_approval",True),
            ("complete_corresponding_source_available",True),
            ("corresponding_source_rebuilt_and_verified",True),
            ("windows_executables_included",True),
            ("exact_akula_head","c"*40),
        ]:
            j=json.loads(json.dumps(b));j[key]=value
            with self.subTest(key=key),self.assertRaisesRegex(M.STARTER.Hold,"VIDEO_TOOLS_01U_HOLD"):
                M.verify_base_starter(j,"a"*40)
        j=base_manifest();j["snapshots"].pop("akula")
        with self.assertRaises(M.STARTER.Hold):
            M.verify_base_starter(j,"a"*40)

    def test_pinned_source_declares_multi_license_built_ejs_and_pyinstaller_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            py=Path(tmp)/"py";py.mkdir()
            ejs=Path(tmp)/"ejs";ejs.mkdir()
            (py/"COPYING.txt").write_text(
                "GNU General Public License\nBootloader Exception\nRun-time Hooks",encoding="utf8")
            (py/"pyproject.toml").write_text(
                "GPLv2-or-later with a special exception",encoding="utf8")
            (py/"bootloader").mkdir()
            (py/"bootloader/wscript").write_text(
                "GPL-2.0-or-later WITH Bootloader-exception",encoding="utf8")
            (ejs/"pyproject.toml").write_text(
                'license = "Unlicense AND MIT AND ISC"',encoding="utf8")
            (ejs/"README.md").write_text(
                "Unlicense meriyah ISC astring MIT",encoding="utf8")
            (ejs/"package.json").write_text(json.dumps({
                "dependencies":{"astring":"1.9.0","meriyah":"6.1.4"}
            }),encoding="utf8")
            (ejs/"package-lock.json").write_text(json.dumps({
                "packages":{
                    "node_modules/meriyah":{"version":"6.1.4","integrity":"sha512-QQ=="},
                    "node_modules/astring":{"version":"1.9.0","integrity":"sha512-QQ=="}
                }
            }),encoding="utf8")
            facts=M.verify_source_metadata(py,ejs)
            self.assertEqual(facts["ejs"]["built_wheel_declared_license"],
                             "Unlicense AND MIT AND ISC")
            self.assertFalse(facts["pyinstaller"]["bootloader_compiled_wheel_equivalence_verified"])
            (ejs/"pyproject.toml").write_text('license = "Unlicense"',encoding="utf8")
            with self.assertRaisesRegex(M.STARTER.Hold,"EJS bundled wheel"):
                M.verify_source_metadata(py,ejs)
            (ejs/"pyproject.toml").write_text(
                'license = "Unlicense AND MIT AND ISC"',encoding="utf8")
            (ejs/"package-lock.json").write_text(json.dumps({
                "packages":{
                    "node_modules/meriyah":{"version":"6.1.5","integrity":"sha512-QQ=="},
                    "node_modules/astring":{"version":"1.9.0","integrity":"sha512-QQ=="}
                }
            }),encoding="utf8")
            with self.assertRaisesRegex(M.STARTER.Hold,"package lock"):
                M.verify_source_metadata(py,ejs)

    def test_fail_closed_upstream_git_blobs(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            (root/"COPYING.txt").write_bytes(b"tampered")
            with self.assertRaisesRegex(M.STARTER.Hold,"Git blob"):
                M.verify_blobs(root,{"COPYING.txt":M.PY_BLOBS["COPYING.txt"]})

    def test_existing_ci_safeguards_are_reused_not_expanded_to_release(self):
        w=(ROOT/".github/workflows/akula-video18b09-windows-license-triage.yml").read_text("utf8")
        d=(ROOT/"docs/release/video-tools-01u-source-addon.md").read_text("utf8")
        for s in (
            ".github/scripts/video-tools-01u-pinned-sources.py",
            ".github/scripts/video-tools-01u-pinned-sources.test.py",
            "docs/release/video-tools-01u-source-addon.md",
            "python3 .github/scripts/video-tools-01u-pinned-sources.test.py",
            "repository: pyinstaller/pyinstaller",
            "ref: 70fc17210920bce17f4ab09bbf8104b0dbd45338",
            "repository: yt-dlp/ejs",
            "ref: 4fb477f4af56880cfd324c48bd4294a2d2294e50",
            "python3 .github/scripts/video-tools-01u-pinned-sources.py",
            "video-tools-01u-source-addon-HOLD",
            "VIDEO_TOOLS_01U_GIT_SOURCES_PASS",
            "retention-days: 1","persist-credentials: false",
            "permissions:\n  contents: read",
        ):
            self.assertIn(s,w,"01u source add-on workflow evidence missing "+s)
        for forbidden in ("workflow_dispatch:","\n  push:","contents: write",
                          "gh release create","upload-release-asset"):
            self.assertNotIn(forbidden,w)
        for token in ("VIDEO-18 RELEASE HOLD","Unlicense AND MIT AND ISC",
                      "GPL-2.0-or-later","NOT COMPLETE","corresponding source",
                      "meriyah","astring"):
            self.assertIn(token,d)
        self.assertIn("NOT complete corresponding source",M.README)
        self.assertIn("VIDEO-18 RELEASE HOLD",M.README)


if __name__=="__main__":
    unittest.main(verbosity=2)
