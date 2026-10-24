"""Disposable local and CI tests; never enroll a real VPS or start an ELF."""
import importlib.util
import io
import os
import pathlib
import stat
import struct
import subprocess
import sys
import tempfile
import time
import unittest
import hashlib

SOURCE = pathlib.Path(__file__).with_name("root_stage_v1.py")
spec = importlib.util.spec_from_file_location("akula_root_stage_v1", SOURCE)
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


def openssl(*args):
    return subprocess.run(["/usr/bin/openssl", *args], check=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout


def fake_elf():
    elf = bytearray(128)
    elf[:7] = b"\x7fELF\x02\x01\x01"
    elf[16:20] = b"\x03\x00\x3e\x00"
    return bytes(elf)


class RootStageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.keys = tempfile.TemporaryDirectory(prefix="akula-key-fixture-")
        cls.private = os.path.join(cls.keys.name, "signer.pem")
        openssl("genpkey", "-algorithm", "Ed25519", "-out", cls.private)
        der = openssl("pkey", "-in", cls.private, "-pubout", "-outform", "DER")
        assert der.startswith(stage.PUB_SPKI_PREFIX) and len(der) == 44
        cls.public = der[-32:]

    @classmethod
    def tearDownClass(cls):
        cls.keys.cleanup()

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="akula-stage-fixture-")
        self.addCleanup(self.tmp.cleanup)

    def bundle(self, payload=None, signer=None, valid=True):
        payload = fake_elf() if payload is None else payload
        now = int(time.time())
        name = (stage.EXPECTED_SIGNER if signer is None else signer).encode()
        manifest = (stage.B1A + b"\x01\x01" + struct.pack(">IIIQQQQ", 0, 3, 13,
                    1, len(payload), now - 10, now + (600 if valid else -1))
                    + hashlib.sha256(payload).digest() + bytes([len(name)]) + name)
        manifest_path = os.path.join(self.tmp.name, "manifest")
        signature_path = os.path.join(self.tmp.name, "signature")
        with open(manifest_path, "wb") as file:
            file.write(manifest)
        openssl("pkeyutl", "-sign", "-inkey", self.private, "-rawin",
                "-in", manifest_path, "-out", signature_path)
        with open(signature_path, "rb") as file:
            signature = file.read()
        return stage.WIRE + struct.pack(">H", len(manifest)) + manifest + signature + payload

    def receive(self, content, public=None):
        self.scratch = os.path.join(tempfile.mkdtemp(dir=self.tmp.name), "scratch")
        return stage.receive_verified(io.BytesIO(content),
                                      self.public if public is None else public,
                                      int(time.time()), self.scratch)

    @unittest.skipUnless(os.geteuid() == 0, "root-owned publish under disposable root CI")
    def test_success_is_nonexecutable_root_owned_hold_only(self):
        manifest, signature, digest = self.receive(self.bundle())
        parent = os.path.join(self.tmp.name, "parent")
        os.mkdir(parent, 0o700)
        fd = os.open(parent, stage.O_DIR)
        try:
            stage.publish_hold(fd, self.scratch,
                               manifest, signature, digest, self.public)
            with self.assertRaises(FileExistsError):
                stage.publish_hold(fd, self.scratch,
                                   manifest, signature, digest, self.public)
        finally:
            os.close(fd)
        root = os.path.join(parent, "bootstrap-v1")
        self.assertEqual(set(os.listdir(root)),
                         {"manifest", "signature", "helper.staged", "release.ed25519", "HOLD"})
        with open(os.path.join(root, "helper.staged"), "rb") as stream:
            self.assertEqual(stream.read(), fake_elf())
        st = os.stat(os.path.join(root, "helper.staged"))
        self.assertEqual(st.st_uid, os.geteuid())
        self.assertEqual(stat.S_IMODE(st.st_mode), 0o400)
        self.assertFalse(os.access(os.path.join(root, "helper.staged"), os.X_OK))

    @unittest.skipIf(os.geteuid() == 0, "requires unprivileged test process")
    def test_unprivileged_publish_refuses_without_creating_root_state(self):
        manifest, signature, digest = self.receive(self.bundle())
        parent = os.path.join(self.tmp.name, "parent")
        os.mkdir(parent, 0o700)
        fd = os.open(parent, stage.O_DIR)
        try:
            with self.assertRaises(stage.Hold):
                stage.publish_hold(fd, self.scratch,
                                   manifest, signature, digest, self.public)
        finally:
            os.close(fd)
        self.assertEqual(os.listdir(parent), [])

    def test_bad_signature_key_or_elf_fail_before_root_publication(self):
        bundle = self.bundle()
        with self.assertRaises(stage.Hold):
            self.receive(bundle, bytes([8]) * 32)
        with self.assertRaises(stage.Hold):
            self.receive(self.bundle(payload=b"Z" * 128))
        damaged = bytearray(bundle)
        damaged[-1] ^= 1
        with self.assertRaises(stage.Hold):
            self.receive(bytes(damaged))
        self.assertFalse(os.path.exists(os.path.join(self.tmp.name, "parent")))

    def test_wrong_signer_time_trailing_or_truncation_hold(self):
        for modified in [self.bundle(signer="other-signer"), self.bundle(valid=False),
                         self.bundle() + b"x", self.bundle()[:-1], b"bad"]:
            with self.subTest(size=len(modified)):
                with tempfile.TemporaryDirectory(dir=self.tmp.name) as tmp:
                    scratch = os.path.join(tmp, "scratch")
                    with self.assertRaises(stage.Hold):
                        stage.receive_verified(io.BytesIO(modified), self.public,
                                               int(time.time()), scratch)

    @unittest.skipUnless(os.geteuid() == 0, "disposable root CI only")
    def test_actual_root_entrypoint_publishes_only_protected_hold(self):
        from contextlib import redirect_stdout
        from unittest.mock import patch
        parent = os.path.join(self.tmp.name, "disposable-parent")
        os.mkdir(parent, 0o700)
        self.assertEqual(os.stat(parent).st_uid, 0)
        with patch.object(stage, "PINNED_BOOTSTRAP_ED25519_HEX", self.public.hex()):
            with patch.object(stage, "open_parent_root", lambda: os.open(parent, stage.O_DIR)):
                with redirect_stdout(io.StringIO()) as out:
                    stage.root_stage(io.BytesIO(self.bundle()))
                self.assertEqual(out.getvalue(), "AKULA_ROOT_STAGE_VERIFIED_HOLD\n")
        fixed = os.path.join(parent, "bootstrap-v1", "helper.staged")
        self.assertEqual(os.stat(fixed).st_uid, 0)
        self.assertEqual(stat.S_IMODE(os.stat(fixed).st_mode), 0o400)
        self.assertFalse(os.access(fixed, os.X_OK))

    def test_production_entrypoint_is_locked_without_issued_key(self):
        run = subprocess.run([sys.executable, "-I", str(SOURCE)],
                             input=self.bundle(), stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, timeout=10)
        self.assertEqual(run.returncode, 2)
        self.assertNotIn(b"VERIFIED_HOLD", run.stdout)


if __name__ == "__main__":
    unittest.main()
