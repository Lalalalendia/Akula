import io
import os
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import verify_mac_tar_parity as parity
from verify_mac_tar_parity import compare, InvalidBundle


class ParityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'Aquilum.app'
        (self.root / 'Contents' / 'MacOS').mkdir(parents=True)
        (self.root / 'Contents' / 'MacOS' / 'aquilum-app').write_bytes(b'old-binary')
        (self.root / 'Contents' / 'Info.plist').write_bytes(b'plist')
        (self.root / 'Contents' / 'MacOS' / 'aquilum-app').chmod(0o755)
        self.tar = Path(self.tmp.name) / 'Aquilum.app.tar.gz'
        self.pack()

    def pack(self, mutate=None):
        with tarfile.open(self.tar, 'w:gz') as archive:
            archive.add(self.root, arcname='Aquilum.app', recursive=True)
            if mutate:
                mutate(archive)

    def rejected(self, match=None):
        with self.assertRaises(InvalidBundle) as caught:
            compare(self.root, self.tar)
        if match:
            self.assertIn(match, str(caught.exception))

    def test_equal_bundle(self):
        self.assertGreater(compare(self.root, self.tar), 2)

    def test_archive_root_mode_must_match_mounted_app(self):
        with tarfile.open(self.tar, 'w:gz') as archive:
            root = archive.gettarinfo(self.root, arcname='Aquilum.app')
            mounted_mode = self.root.stat().st_mode & 0o777
            root.mode = 0o700 if mounted_mode != 0o700 else 0o755
            archive.addfile(root)
            archive.add(self.root / 'Contents', arcname='Aquilum.app/Contents')
        self.rejected("changed=['']")

    def test_duplicate_archive_root_is_refused(self):
        def extra(archive):
            root = tarfile.TarInfo('Aquilum.app/')
            root.type = tarfile.DIRTYPE
            root.mode = self.root.stat().st_mode & 0o777
            archive.addfile(root)
        self.pack(extra)
        self.rejected('duplicate archive root')

    def test_privileged_archive_root_is_refused(self):
        with tarfile.open(self.tar, 'w:gz') as archive:
            root = archive.gettarinfo(self.root, arcname='Aquilum.app')
            root.mode = 0o4755
            archive.addfile(root)
            archive.add(self.root / 'Contents', arcname='Aquilum.app/Contents')
        self.rejected('privileged permission bits in archive app root')

    def test_root_entry_is_required_before_children(self):
        with tarfile.open(self.tar, 'w:gz') as archive:
            archive.add(self.root / 'Contents', arcname='Aquilum.app/Contents')
        self.rejected('archive app root directory missing or not first')

    def test_modified_file(self):
        (self.root / 'Contents' / 'Info.plist').write_bytes(b'changed')
        self.rejected('changed=')

    def test_extra_file(self):
        def extra(tar):
            data = b'extra'
            info = tarfile.TarInfo('Aquilum.app/extra.txt')
            info.size = len(data)
            info.mode = 0o644
            tar.addfile(info, io.BytesIO(data))
        self.pack(extra)
        self.rejected('extra=')

    def test_duplicate(self):
        def duplicate(tar):
            info = tarfile.TarInfo('Aquilum.app/Contents/Info.plist')
            info.size = 5
            tar.addfile(info, io.BytesIO(b'plist'))
        self.pack(duplicate)
        self.rejected('duplicate')

    def test_casefold_alias(self):
        def extra(tar):
            info = tarfile.TarInfo('Aquilum.app/Contents/INFO.PLIST')
            info.size = 5
            tar.addfile(info, io.BytesIO(b'plist'))
        self.pack(extra)
        self.rejected('colliding')

    def test_path_traversal(self):
        def extra(tar):
            info = tarfile.TarInfo('Aquilum.app/../../outside')
            info.size = 1
            tar.addfile(info, io.BytesIO(b'!'))
        self.pack(extra)
        self.rejected('unsafe')

    def test_other_root(self):
        def extra(tar):
            info = tarfile.TarInfo('Other.app/danger')
            info.size = 1
            tar.addfile(info, io.BytesIO(b'x'))
        self.pack(extra)
        self.rejected('out-of-bundle')

    def test_unsafe_link(self):
        def extra(tar):
            info = tarfile.TarInfo('Aquilum.app/escape')
            info.type = tarfile.SYMTYPE
            info.linkname = '../../outside'
            tar.addfile(info)
        self.pack(extra)
        self.rejected('symlink escapes')

    def test_hardlink(self):
        def extra(tar):
            info = tarfile.TarInfo('Aquilum.app/hard')
            info.type = tarfile.LNKTYPE
            info.linkname = 'Aquilum.app/Contents/Info.plist'
            tar.addfile(info)
        self.pack(extra)
        self.rejected('unsupported')

    def test_symlinks_inside_app(self):
        os.symlink('Info.plist', self.root / 'Contents' / 'Info-copy')
        self.pack()
        compare(self.root, self.tar)

    def test_symlink_outside_app_even_if_matching(self):
        os.symlink('../../external', self.root / 'Contents' / 'malicious')
        self.pack()
        self.rejected('symlink escapes')

    def test_tar_size_budget(self):
        with patch.object(parity, 'MAX_FILE', 4):
            with self.assertRaises(InvalidBundle) as caught:
                parity.tar_manifest(self.tar)
            self.assertIn('oversized TAR', str(caught.exception))

    def test_mount_size_budget(self):
        with patch.object(parity, 'MAX_TOTAL', 2):
            self.rejected('mounted app exceeds size budget')


if __name__ == '__main__':
    unittest.main()
