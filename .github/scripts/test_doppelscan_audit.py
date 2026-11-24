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


    def test_pinned_comparable_scan_settings(self):
        # Do not accidentally compare clone counts across different tools,
        # thresholds, output limits or scopes.
        common = ["--format", "json", "--limit", "0", "--only", "rust",
                  "--min-tokens", "50"]
        self.assertEqual(audit.VERSION, "0.9.0")
        self.assertEqual(audit.TOOL_SHA256,
                         "e5d5abd63abfb74dddfc52d60d58144f402218aa49b3b6b6e610d1e3171f355f")
        self.assertEqual(audit.MODES, {
            "exact": common + ["--strict"],
            "renamed": common,
            "near": ["near"] + common + ["--similarity", "0.8"],
        })

    def test_clone_group_rejects_invalid_occurrence_evidence(self):
        path = "aquilum-app/core/src/lib.rs"
        manifest = {path: "digest"}
        valid = {"file": "/src/" + path, "start_line": 1, "end_line": 4}
        def check(occurrences):
            return audit.validate_report({
                "summary": {"files": 1, "groups": 1},
                "groups": [{"id": 1, "tokens": 50, "score": 50,
                            "occurrences": occurrences}]
            }, manifest)
        result = check([valid, {**valid, "file": "./" + path,
                                "start_line": 5, "end_line": 8}])
        self.assertEqual(result["unscanned_count"], 0)
        self.assertEqual(result["top"][0]["copies"], 2)
        self.assertEqual(result["top"][0]["locations"][0]["file"], path)
        with self.assertRaises(ValueError):
            check([valid])
        for bad in ({"start_line": 0}, {"end_line": 0},
                    {"start_line": 5, "end_line": 4},
                    {"start_line": True}, {"end_line": "4"},
                    {"file": "/src/../outside.rs"}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                check([valid, {**valid, **bad}])


    def test_manifest_delta_across_committed_source_rename_and_edits(self):
        # A comparable audit must distinguish moved paths, changed bytes, and
        # excluded test-only edits instead of attributing all deltas to reuse.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            def git(*args):
                return subprocess.check_output(["git", "-C", str(repo), *args],
                                               stderr=subprocess.DEVNULL).decode().strip()
            git("init")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            prefix = "aquilum-app/core/src/"
            source = repo / prefix
            source.mkdir(parents=True)
            files = {"old.rs": b"fn moved() {}\n",
                     "edit.rs": b"fn before() {}\n",
                     "removed.rs": b"fn removed() {}\n",
                     "ignored_tests.rs": b"fn test_before() {}\n"}
            for name, data in files.items():
                (source / name).write_bytes(data)
            git("add", ".")
            git("commit", "-m", "before")
            before = git("rev-parse", "HEAD")
            git("mv", prefix + "old.rs", prefix + "new.rs")
            (source / "edit.rs").write_bytes(b"fn after() {}\n")
            git("rm", prefix + "removed.rs")
            (source / "added.rs").write_bytes(b"fn added() {}\n")
            (source / "ignored_tests.rs").write_bytes(b"fn test_after() {}\n")
            git("add", ".")
            git("commit", "-m", "after")
            after = git("rev-parse", "HEAD")
            def manifest(sha, name):
                dest = root / name
                dest.mkdir()
                return audit.snapshot(repo, sha, dest)
            old = manifest(before, "old")
            new = manifest(after, "new")
            self.assertEqual(set(old) - set(new),
                             {prefix + "old.rs", prefix + "removed.rs"})
            self.assertEqual(set(new) - set(old),
                             {prefix + "new.rs", prefix + "added.rs"})
            self.assertEqual(old[prefix + "old.rs"], new[prefix + "new.rs"])
            self.assertNotEqual(old[prefix + "edit.rs"], new[prefix + "edit.rs"])
            self.assertNotIn(prefix + "ignored_tests.rs", old)
            self.assertNotIn(prefix + "ignored_tests.rs", new)
            self.assertEqual(old, manifest(before, "old_again"))
            self.assertEqual(new, manifest(after, "new_again"))

    def test_report_validation_checks_groups_beyond_log_preview(self):
        path = "aquilum-app/core/src/lib.rs"
        manifest = {path: "digest"}
        groups = []
        for index in range(6):
            start = index * 10 + 1
            groups.append({"id": index + 1, "tokens": 50, "score": 50,
                           "occurrences": [
                               {"file": path, "start_line": start, "end_line": start + 3},
                               {"file": path, "start_line": start + 4, "end_line": start + 7},
                           ]})
        data = {"summary": {"files": 1, "groups": 6}, "groups": groups}
        report = audit.validate_report(data, manifest)
        self.assertEqual(len(report["top"]), 5)
        self.assertEqual(report["stats"]["groups"], 6)
        groups[-1]["occurrences"][1]["file"] = "/src/outside.rs"
        with self.assertRaises(ValueError):
            audit.validate_report(data, manifest)


if __name__ == "__main__":
    unittest.main()
