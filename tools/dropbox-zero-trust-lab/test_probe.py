"""Offline-only Zero Trust contract tests. No Dropbox credentials or network calls."""

import contextlib
import io
import os
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe


class FakeDropbox:
    """In-memory provider controlled by tests; intentionally not a Dropbox emulator."""

    def __init__(self, policy="create_only"):
        self.policy = policy
        self.tickets = {}
        self.content = None
        self.file_id = "id:fake-object"
        self.revision_number = 0
        self.name = None
        self.names = []
        self.posts = 0
        self.reads = 0

    def issue(self, token, path):
        url = "test-ticket-%d" % (len(self.tickets) + 1)
        self.tickets[url] = [path, False]
        return url

    def post_object(self, ticket, body):
        self.posts += 1
        if ticket not in self.tickets or self.tickets[ticket][1]:
            return probe.Reply(410, b"", {})
        path, _ = self.tickets[ticket]
        self.tickets[ticket][1] = True
        name = path.rsplit("/", 1)[-1]
        if self.policy == "no_remote_object":
            return probe.Reply(409, b"", {})
        if self.policy == "reject_oversize" and len(body) > probe.LIMIT:
            return probe.Reply(413, b"", {})
        if self.content is not None:
            if self.policy == "create_only":
                return probe.Reply(409, b"", {})
            if self.policy == "sibling":
                self.names.append(name.removesuffix(".bin") + " (1).bin")
                return probe.Reply(200, b"", {})
        self.content = bytes(body)
        self.name = name
        self.names = [name] if not self.names else self.names
        self.revision_number += 1
        return probe.Reply(200, b"", {})

    def metadata(self, token, path):
        if self.content is None:
            raise RuntimeError("no remote object")
        return {
            ".tag": "file", "size": len(self.content), "id": self.file_id,
            "rev": "%04x" % self.revision_number,
            "content_hash": probe.dropbox_content_hash(self.content),
            "path_lower": self.name and "/" + self.name.lower(),
        }

    def download_revision(self, token, rev, expected_max):
        self.reads += 1
        if self.content is None or rev != "%04x" % self.revision_number:
            raise RuntimeError("requested remote revision is gone")
        if len(self.content) > expected_max:
            raise RuntimeError("readback exceeds agreed size")
        return self.content

    def sibling_names(self, token, path):
        return list(self.names)

    def patches(self):
        return patch.multiple(
            probe, issue=self.issue, post_object=self.post_object,
            metadata=self.metadata, download_revision=self.download_revision,
            sibling_names=self.sibling_names,
        )


class DropboxDropOfflineContractTests(unittest.TestCase):
    PATH = "/akula_zt_lab_fake_p01.bin"

    def test_p01_create_only_conflict_is_rejected_with_original_version(self):
        fake = FakeDropbox()
        with fake.patches():
            result = probe.p01_two_tickets("fake-token", self.PATH)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["second_http"], 409)
        self.assertTrue(result["first_rev_stable"])
        self.assertTrue(result["no_renamed_sibling"])
        self.assertEqual(fake.posts, 2)

    def test_p01_silent_replacement_fails_closed(self):
        fake = FakeDropbox("overwrite")
        with fake.patches():
            result = probe.p01_two_tickets("fake-token", self.PATH)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(fake.posts, 2)

    def test_p01_unexpected_autorename_is_failure(self):
        fake = FakeDropbox("sibling")
        with fake.patches():
            result = probe.p01_two_tickets("fake-token", self.PATH)
        self.assertEqual(result["status"], "FAIL")
        self.assertFalse(result["no_renamed_sibling"])

    def test_p02_accepting_8m_plus_1_is_negative_security_finding(self):
        fake = FakeDropbox()
        with fake.patches():
            result = probe.p02_size_cap("fake-token", "/akula_zt_lab_fake_p02.bin")
        self.assertEqual(result["status"], "NO_8M_SERVER_CAP_OBSERVED")
        self.assertEqual(result["remote_size"], probe.LIMIT + 1)
        self.assertEqual(fake.posts, 1)

    def test_p02_rejecting_8m_plus_1_does_not_prove_per_grant_quota(self):
        fake = FakeDropbox("reject_oversize")
        with fake.patches():
            result = probe.p02_size_cap("fake-token", "/akula_zt_lab_fake_p02.bin")
        self.assertEqual(result["status"], "HOLD")
        self.assertEqual(result["upload_http"], 413)

    def test_p03_discarded_app_ack_recovers_without_second_post(self):
        fake = FakeDropbox()
        with fake.patches():
            result = probe.p03_lost_ack("fake-token", "/akula_zt_lab_fake_p03.bin")
        self.assertEqual(result["status"], "PASS_RECONCILED_CLOUD_ONLY")
        self.assertFalse(result["durable_inbox_receipt"])
        self.assertTrue(result["app_layer_ack_discarded"])
        self.assertFalse(result["blind_retry"])
        self.assertEqual(fake.posts, 1)
        self.assertEqual(fake.reads, 1)

    def test_p03_absent_remote_object_holds_instead_of_ack(self):
        fake = FakeDropbox("no_remote_object")
        with fake.patches():
            with self.assertRaisesRegex(RuntimeError, "no remote object"):
                probe.p03_lost_ack("fake-token", "/akula_zt_lab_fake_p03.bin")
        self.assertEqual(fake.posts, 1)

    def test_readback_rejects_provider_hash_mismatch_without_download(self):
        fake = FakeDropbox()
        blob = b"a secret synthetic byte string"
        fake.content = blob
        fake.revision_number = 1
        fake.name = self.PATH.rsplit("/", 1)[-1]
        bad = fake.metadata("fake-token", self.PATH)
        bad["content_hash"] = "0" * 64
        with patch.object(probe, "metadata", return_value=bad), patch.object(
            probe, "download_revision", side_effect=AssertionError("must not download")
        ):
            with self.assertRaisesRegex(RuntimeError, "tree hash mismatch"):
                probe.readback("fake-token", self.PATH, blob)

    def test_provider_host_and_redirect_targets_are_rejected(self):
        for url in (
            "http://content.dropboxapi.com/apitul/test",
            "https://evil.example/apitul/test",
            "https://content.dropboxapi.com.evil.example/apitul/test",
            "https://attacker@content.dropboxapi.com/apitul/test",
            "https://content.dropboxapi.com:444/apitul/test",
            "https://content.dropboxapi.com/other/test",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                probe.checked_https(url, probe.CONTENT_HOST, "/apitul/")

    def test_no_network_without_explicit_case_and_disposable_account(self):
        for argv in (("probe.py",), ("probe.py", "--run"),
                     ("probe.py", "--run", "--case", "P01")):
            with self.subTest(argv=argv), patch.object(sys, "argv", list(argv)), patch.dict(
                os.environ, {}, clear=True
            ), patch.object(probe, "api", side_effect=AssertionError("network forbidden")):
                with contextlib.redirect_stdout(io.StringIO()) as stdout:
                    result = probe.main()
                self.assertNotIn("fake-token", stdout.getvalue())
                self.assertEqual(result, 0 if len(argv) == 1 else 2)

    def test_p03_real_transport_failure_after_commit_reconciles(self):
        from urllib.error import URLError
        fake = FakeDropbox()
        real_post = fake.post_object
        def dropped_response(ticket, body):
            self.assertEqual(real_post(ticket, body).code, 200)
            raise URLError("simulated TCP response loss after server commit")
        with fake.patches(), patch.object(probe, "post_object", side_effect=dropped_response):
            result = probe.p03_lost_ack("fake-token", "/akula_zt_lab_fake_p03.bin")
        self.assertTrue(result["network_ack_lost"])
        self.assertFalse(result["durable_inbox_receipt"])
        self.assertEqual(fake.posts, 1)
        self.assertEqual(fake.reads, 1)

    def test_p02_metadata_claim_without_matching_bytes_is_not_a_security_result(self):
        fake = FakeDropbox()
        real_download = fake.download_revision
        def tampered_download(token, rev, maximum):
            return b"bad" + real_download(token, rev, maximum)[3:]
        with fake.patches(), patch.object(probe, "download_revision", side_effect=tampered_download):
            result = probe.p02_size_cap("fake-token", "/akula_zt_lab_fake_p02.bin")
        self.assertEqual(result["status"], "HOLD")
        self.assertEqual(fake.posts, 1)

    def test_readback_refuses_wrong_account_path_even_if_bytes_match(self):
        fake = FakeDropbox()
        blob = b"a public synthetic fixture"
        fake.content = blob
        fake.revision_number = 1
        fake.name = self.PATH.rsplit("/", 1)[-1]
        bad = fake.metadata("fake-token", self.PATH)
        bad["path_lower"] = "/different_scope.bin"
        with patch.object(probe, "metadata", return_value=bad), patch.object(
            probe, "download_revision", side_effect=AssertionError("must not download")
        ):
            with self.assertRaisesRegex(RuntimeError, "identity/scope mismatch"):
                probe.readback("fake-token", self.PATH, blob)

    def test_revision_download_requires_matching_response_header(self):
        blob = b"test data"
        rev = "a1c10ce0dd78"
        meta = {"rev": rev, "size": len(blob),
                "content_hash": probe.dropbox_content_hash(blob)}
        with patch.object(probe, "request", return_value=probe.Reply(
            200, blob, {"Dropbox-API-Result": __import__("json").dumps(meta)}
        )):
            self.assertEqual(probe.download_revision("fake", rev, len(blob)), blob)
        meta["rev"] = "different"
        with patch.object(probe, "request", return_value=probe.Reply(
            200, blob, {"Dropbox-API-Result": __import__("json").dumps(meta)}
        )):
            with self.assertRaisesRegex(RuntimeError, "metadata disagrees"):
                probe.download_revision("fake", rev, len(blob))
        with patch.object(probe, "request", return_value=probe.Reply(200, blob, {})):
            with self.assertRaisesRegex(RuntimeError, "missing revision"):
                probe.download_revision("fake", rev, len(blob))

    def test_request_enforces_exact_bound_for_disposable_oversize_readback(self):
        import io
        class Response:
            status = 200
            headers = {}
            def __init__(self, data):
                self.stream = io.BytesIO(data)
            def __enter__(self): return self
            def __exit__(self, *args): return None
            def read(self, n): return self.stream.read(n)
        for max_allowed, data, fail in ((probe.LIMIT, b"x" * (probe.LIMIT + 1), True),
                                       (probe.LIMIT + 1, b"x" * (probe.LIMIT + 1), False)):
            with self.subTest(max_allowed=max_allowed), patch.object(
                probe.OPENER, "open", return_value=Response(data)
            ):
                if fail:
                    with self.assertRaisesRegex(RuntimeError, "bounded response"):
                        probe.request("https://content.dropboxapi.com/2/files/download", b"", {},
                                      probe.CONTENT_HOST, "/2/files/download",
                                      max_download_bytes=max_allowed)
                else:
                    r = probe.request("https://content.dropboxapi.com/2/files/download", b"", {},
                                      probe.CONTENT_HOST, "/2/files/download",
                                      max_download_bytes=max_allowed)
                    self.assertEqual(r.data, data)

    def test_dropbox_hash_official_empty_and_block_boundaries(self):
        import hashlib
        self.assertEqual(probe.dropbox_content_hash(b""), hashlib.sha256(b"").hexdigest())
        chunk = b"x" * (4 * 1024 * 1024)
        separate = hashlib.sha256(hashlib.sha256(chunk).digest() + hashlib.sha256(b"y").digest()).hexdigest()
        self.assertEqual(probe.dropbox_content_hash(chunk + b"y"), separate)

    def test_main_cannot_return_success_for_observed_oversize(self):
        expected = "dbid:disposable-testing"
        with patch.object(sys, "argv", ["probe.py", "--run", "--case", "P02"]), patch.dict(
            os.environ, {
                "AKULA_DROPBOX_TEST_TOKEN": "fake-token",
                "AKULA_DROPBOX_TEST_ACCOUNT_ID": expected,
                "AKULA_DROPBOX_DISPOSABLE_CONFIRM": "YES_DISPOSABLE_APP_FOLDER",
            }, clear=True,
        ), patch.object(
            probe, "api", return_value=probe.Reply(200, (
                '{"account_id":"%s"}' % expected).encode(), {})
        ), patch.object(probe, "p02_size_cap", return_value={
            "status": "NO_8M_SERVER_CAP_OBSERVED", "agreed_max": probe.LIMIT,
            "remote_size": probe.LIMIT + 1,
        }):
            with contextlib.redirect_stdout(io.StringIO()) as stdout:
                result = probe.main()
            self.assertEqual(result, 1)
            self.assertIn("NO_8M_SERVER_CAP_OBSERVED", stdout.getvalue())
            self.assertNotIn("fake-token", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
