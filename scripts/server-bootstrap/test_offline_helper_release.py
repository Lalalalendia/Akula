"""No real signer is provisioned. Keys are ephemeral test-only fixtures."""
import contextlib
import hashlib
import io
import os
import pathlib
import subprocess
import tempfile
import time
import unittest

import offline_helper_release as release
import root_stage_v1 as stage


def openssl(*argv):
    return subprocess.run(
        ["/usr/bin/openssl", *map(str, argv)], check=True,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, timeout=20).stdout


def fake_elf():
    data = bytearray(256)
    data[:7] = b"\x7fELF\x02\x01\x01"
    data[18:20] = b"\x3e\x00"
    return bytes(data)


class OfflineReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary_key_dir = tempfile.TemporaryDirectory(
            prefix="akula-ephemeral-test-ed25519-")
        cls.key = pathlib.Path(cls.temporary_key_dir.name) / "test-only.pem"
        openssl("genpkey", "-algorithm", "Ed25519", "-out", cls.key)
        cls.public_der = openssl(
            "pkey", "-in", cls.key, "-pubout", "-outform", "DER")
        assert cls.public_der.startswith(stage.PUB_SPKI_PREFIX)
        cls.public = cls.public_der[-32:]

    @classmethod
    def tearDownClass(cls):
        cls.temporary_key_dir.cleanup()

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="akula-offline-fixture-")
        self.addCleanup(self.dir.cleanup)
        self.path = pathlib.Path(self.dir.name)
        self.binary = self.path / "helper.elf"
        self.binary.write_bytes(fake_elf())
        self.now = int(time.time())

    def sign(self, manifest):
        path = self.path / "signed-manifest"
        path.write_bytes(manifest)
        return openssl("pkeyutl", "-sign", "-inkey", self.key,
                       "-rawin", "-in", path)

    def manifest(self):
        return release.prepare_manifest(
            self.binary.read_bytes(), "0.3.13", 1,
            self.now - 10, self.now + 3600)

    def frame(self):
        manifest = self.manifest()
        return release.assemble_verified_frame(
            self.binary.read_bytes(), manifest, self.sign(manifest),
            self.public, self.now)

    def test_real_external_ed25519_signature_matches_root_stage_wire(self):
        frame = self.frame()
        self.assertTrue(frame.startswith(stage.WIRE))
        with tempfile.TemporaryDirectory(prefix="akula-stage-review-") as temp:
            m, signature, digest = stage.receive_verified(
                io.BytesIO(frame), self.public, self.now,
                os.path.join(temp, "scratch"))
        self.assertEqual(m, self.manifest())
        self.assertEqual(len(signature), 64)
        self.assertEqual(digest, hashlib.sha256(fake_elf()).digest())

    def test_prepare_and_frame_cli_never_sign_or_overwrite_existing_outputs(self):
        manifest = self.path / "signable.manifest"
        self.assertEqual(
            release.main(["prepare", "--elf", str(self.binary),
                          "--version", "0.3.13", "--sequence", "1",
                          "--not-before", str(self.now - 10),
                          "--expires", str(self.now + 3600),
                          "--out-manifest", str(manifest)]),
            0)
        self.assertEqual(manifest.read_bytes(), self.manifest())
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(
                release.main(["prepare", "--elf", str(self.binary),
                              "--version", "0.3.13", "--sequence", "1",
                              "--not-before", str(self.now - 10),
                              "--expires", str(self.now + 3600),
                              "--out-manifest", str(manifest)]),
                2)
        self.assertEqual(manifest.read_bytes(), self.manifest())
        signature = self.path / "external-signature"
        signature.write_bytes(self.sign(manifest.read_bytes()))
        destination = self.path / "verified.frame"
        self.assertEqual(
            release.main(["frame", "--elf", str(self.binary),
                          "--manifest", str(manifest),
                          "--signature", str(signature),
                          "--trusted-pubkey-hex", self.public.hex(),
                          "--now-unix", str(self.now),
                          "--out-frame", str(destination)]),
            0)
        self.assertEqual(destination.read_bytes(), self.frame())
        self.assertEqual(destination.stat().st_mode & 0o777, 0o600)

    def test_external_public_key_is_required_and_substitution_refuses(self):
        m = self.manifest()
        sig = self.sign(m)
        for key in [bytes(32), bytes([0x33]) * 32]:
            with self.assertRaises(release.ReleaseHold):
                release.assemble_verified_frame(fake_elf(), m, sig, key, self.now)
        for value in ["", "X" * 64, "0" * 64, "f" * 65]:
            with self.assertRaises(release.ReleaseHold):
                release.independent_public_hex(value)
        self.assertEqual(release.independent_public_hex(self.public.hex()), self.public)

    def test_changed_signature_manifest_binary_and_time_refuse(self):
        m, sig = self.manifest(), self.sign(self.manifest())
        damaged_sig = bytearray(sig)
        damaged_sig[0] ^= 1
        damaged_manifest = bytearray(m)
        damaged_manifest[-1] ^= 1
        changed_elf = bytearray(fake_elf())
        changed_elf[-1] ^= 1
        for elf, manifest, signature, now in [
            (fake_elf(), m, bytes(damaged_sig), self.now),
            (fake_elf(), bytes(damaged_manifest), sig, self.now),
            (bytes(changed_elf), m, sig, self.now),
            (fake_elf() + b"X", m, sig, self.now),
            (fake_elf(), m, sig, self.now + 3600),
            (fake_elf(), m, sig, self.now - 11),
        ]:
            with self.subTest(now=now, size=len(elf), manifest_len=len(manifest)):
                with self.assertRaises(release.ReleaseHold):
                    release.assemble_verified_frame(
                        elf, manifest, signature, self.public, now)

    def test_bad_version_epoch_window_or_elf_cannot_produce_signable_manifest(self):
        for version in ["0.3", "0.3.13.4", "01.3.13", "../bin/evil", "1.x.3",
                        "4294967296.0.0"]:
            with self.subTest(version=version):
                with self.assertRaises(release.ReleaseHold):
                    release.prepare_manifest(fake_elf(), version, 1,
                                             self.now, self.now + 120)
        for seq, start, end in [
            (0, self.now, self.now + 100),
            (-1, self.now, self.now + 100),
            (1 << 64, self.now, self.now + 100),
            (1, self.now, self.now),
            (1, self.now + 3, self.now),
            (1, self.now, self.now + stage.MAX_VALIDITY_SECONDS + 1),
        ]:
            with self.subTest(sequence=seq, start=start, end=end):
                with self.assertRaises(release.ReleaseHold):
                    release.prepare_manifest(fake_elf(), "0.3.13", seq, start, end)
        for elf in [b"short", bytes(256), b"\x7fELF" + bytes(90)]:
            with self.assertRaises(release.ReleaseHold):
                release.prepare_manifest(elf, "0.3.13", 1,
                                         self.now, self.now + 100)

    def test_no_authority_or_file_on_invalid_external_signature(self):
        m = self.manifest()
        manifest = self.path / "manifest"
        signature = self.path / "signature"
        manifest.write_bytes(m)
        signature.write_bytes(bytes(64))
        result = self.path / "never-created"
        with contextlib.redirect_stderr(io.StringIO()):
            status = release.main(
                ["frame", "--elf", str(self.binary),
                 "--manifest", str(manifest), "--signature", str(signature),
                 "--trusted-pubkey-hex", self.public.hex(),
                 "--now-unix", str(self.now), "--out-frame", str(result)])
        self.assertEqual(status, 2)
        self.assertFalse(result.exists())

    def test_symlink_and_changed_local_package_refuse(self):
        alias = self.path / "symlink.elf"
        alias.symlink_to(self.binary)
        # A local unsigned helper is not allowed to be a hardlinked inode.
        second = self.path / "hardlink.elf"
        os.link(self.binary, second)
        with self.assertRaises(release.ReleaseHold):
            release.read_elf(alias)
        with self.assertRaises(release.ReleaseHold):
            release.read_elf(self.binary)


if __name__ == "__main__":
    unittest.main()
