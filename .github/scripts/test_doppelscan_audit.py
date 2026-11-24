import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("doppelscan-audit.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditTests(unittest.TestCase):
    def test_scope(self):
        for path in ("aquilum-app/core/src/lib.rs", "aquilum-app/native/src/ui/card.rs",
                     "modules/text-delta-codec/src/lib.rs"):
            self.assertTrue(audit.included(path), path)
        for path in ("aquilum-app/native/vendor/masonry/src/lib.rs",
                     "aquilum-app/native/src/app/tests/cards.rs",
                     "aquilum-app/core/src/sync_tests.rs",
                     "aquilum-app/core/src/generated/schema.rs",
                     "aquilum-app/core/src/fixtures/example.rs",
                     "aquilum-app/core/src/test_helper.rs",
                     "labs/video-lite/src/lib.rs", "modules/game-session/tests/test.rs",
                     "aquilum-app/native/src/../outside.rs", "/etc/test.rs",
                     "aquilum-app/core/src/lib.rs\nbad.rs", "aquilum-app/core/src/a\\b.rs"):
            self.assertFalse(audit.included(path), path)

    def test_snapshot_ignores_dirty_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, dest = Path(tmp) / "repo", Path(tmp) / "snapshot"
            repo.mkdir()
            dest.mkdir()
            def git(*args):
                return subprocess.check_output(["git", "-C", str(repo), *args],
                                               stderr=subprocess.DEVNULL).decode().strip()
            git("init")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            files = {"aquilum-app/core/src/lib.rs": b"fn keep() {}\n",
                     "aquilum-app/native/vendor/a/src/lib.rs": b"fn vendor() {}\n",
                     "aquilum-app/core/src/feature_tests.rs": b"fn test() {}\n",
                     "modules/example/src/lib.rs": b"fn module() {}\n"}
            for name, data in files.items():
                p = repo / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(data)
            (repo / "aquilum-app/core/src/link.rs").symlink_to("/etc/passwd")
            git("add", ".")
            git("commit", "-m", "fixture")
            sha = git("rev-parse", "HEAD")
            (repo / "aquilum-app/core/src/lib.rs").write_text("dirty")
            manifest = audit.snapshot(repo, sha, dest)
            wanted = {name: hashlib.sha256(files[name]).hexdigest()
                      for name in ("aquilum-app/core/src/lib.rs", "modules/example/src/lib.rs")}
            self.assertEqual(manifest, wanted)
            self.assertEqual((dest / "aquilum-app/core/src/lib.rs").read_bytes(),
                             files["aquilum-app/core/src/lib.rs"])
            self.assertFalse((dest / "aquilum-app/core/src/link.rs").exists())
            with self.assertRaises(ValueError):
                audit.snapshot(repo, "HEAD", dest)

    def test_validated_locations_and_coverage(self):
        path = "aquilum-app/core/src/lib.rs"
        other = "modules/example/src/lib.rs"
        manifest = {path: "a", other: "b"}
        def group(location):
            return {"id": 1, "tokens": 50, "score": 50,
                    "occurrences": [{"file": location, "start_line": 1, "end_line": 4},
                                    {"file": path, "start_line": 10, "end_line": 14}]}
        result = audit.validate_report({"summary": {"files": 1, "groups": 1}, "groups": [
            group("./" + path)]}, manifest)
        self.assertEqual(result["unscanned_count"], 1)
        self.assertEqual(result["top"][0]["locations"][0]["file"], path)
        for bad in ("../../etc/passwd", "/etc/passwd", "aquilum-app/native/vendor/x.rs"):
            with self.assertRaises(ValueError):
                audit.validate_report({"summary": {"files": 1, "groups": 1},
                                       "groups": [group(bad)]}, manifest)
        for count in (0, 3):
            with self.assertRaises(ValueError):
                audit.validate_report({"summary": {"files": count, "groups": 0},
                                       "groups": []}, manifest)
        with self.assertRaises(ValueError):
            audit.validate_report({"summary": {"files": 1, "groups": 2},
                                   "groups": [group(path)]}, manifest)

    def test_three_advisory_modes(self):
        self.assertEqual(set(audit.MODES), {"exact", "renamed", "near"})
        self.assertIn("--strict", audit.MODES["exact"])
        self.assertNotIn("--strict", audit.MODES["renamed"])
        self.assertEqual(audit.MODES["near"][-2:], ["--similarity", "0.8"])
        self.assertTrue(all("check" not in flags and "--limit" in flags
                            for flags in audit.MODES.values()))


if __name__ == "__main__":
    unittest.main()
