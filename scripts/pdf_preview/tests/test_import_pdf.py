import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'import_pdf.py'


class PreviewTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.vault = self.dir / 'vault'
        self.vault.mkdir()
        self.pdf = self.dir / 'trusted.pdf'
        self.pdf.write_bytes(b'%PDF-1.4\n% trusted test fixture\n' + b'a' * 20)
        self.shiva = self.dir / 'shiva-stub'
        self.shiva.write_text(
            '#!/usr/bin/env python3\n'
            'import sys\nfrom pathlib import Path\n'
            'Path(sys.argv[2]).write_text("AKULAPDFNEEDLE\\nContent", encoding="utf-8")\n'
        )
        self.shiva.chmod(0o700)

    def call(self, *extra):
        return subprocess.run([
            sys.executable, str(SCRIPT), '--pdf', str(self.pdf),
            '--vault', str(self.vault), '--engine', 'shiva',
            '--converter', str(self.shiva), *extra,
        ], text=True, capture_output=True)

    def test_import_and_idempotence(self):
        for _ in range(2):
            result = self.call('--trusted-local-only')
            self.assertEqual(result.returncode, 0, result.stderr)
        root = self.vault / 'PDF Search Preview'
        pdfs, notes = list(root.glob('*.pdf')), list(root.glob('*.md'))
        self.assertEqual(len(pdfs), 1)
        self.assertEqual(len(notes), 1)
        self.assertEqual(pdfs[0].read_bytes(), self.pdf.read_bytes())
        text = notes[0].read_text()
        self.assertIn('AKULAPDFNEEDLE', text)
        self.assertIn('Needs review', text)
        self.assertIn(pdfs[0].name, text)

    def test_explicit_trust_required(self):
        result = self.call()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.vault / 'PDF Search Preview').exists())

    def test_invalid_pdf_rejected(self):
        self.pdf.write_bytes(b'ordinary plaintext')
        self.assertNotEqual(self.call('--trusted-local-only').returncode, 0)
        self.assertFalse((self.vault / 'PDF Search Preview').exists())

    def test_oversized_pdf_rejected(self):
        self.pdf.write_bytes(b'%PDF-1.4\n' + b'a' * (2 * 1024 * 1024))
        self.assertNotEqual(self.call('--trusted-local-only').returncode, 0)

    def test_never_clobbers_user_changes(self):
        self.assertEqual(self.call('--trusted-local-only').returncode, 0)
        note = next((self.vault / 'PDF Search Preview').glob('*.md'))
        note.write_text('USER EDIT')
        self.assertNotEqual(self.call('--trusted-local-only').returncode, 0)
        self.assertEqual(note.read_text(), 'USER EDIT')

    def test_pdf_symlink_denied(self):
        symlink = self.dir / 'alias.pdf'
        try:
            symlink.symlink_to(self.pdf)
        except OSError:
            self.skipTest('symlinks unsupported')
        self.pdf = symlink
        self.assertNotEqual(self.call('--trusted-local-only').returncode, 0)

    def test_failed_converter_does_not_touch_vault(self):
        self.shiva.write_text('#!/bin/sh\nexit 12\n')
        self.assertNotEqual(self.call('--trusted-local-only').returncode, 0)
        self.assertFalse((self.vault / 'PDF Search Preview').exists())

    def test_extracted_markdown_is_fenced(self):
        self.shiva.write_text('#!/usr/bin/env python3\nimport sys\nfrom pathlib import Path\nPath(sys.argv[2]).write_text("SAFEWORD\\n'+String.raw```\\n![x](https://example.com)\\n")\n')
        self.assertEqual(self.call('--trusted-local-only').returncode, 0)
        note = next((self.vault / 'PDF Search Preview').glob('*.md'))
        self.assertIn('\x60\x60\x60\x60text', note.read_text())


if __name__ == '__main__':
    unittest.main()
