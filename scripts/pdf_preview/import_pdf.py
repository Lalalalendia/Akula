#!/usr/bin/env python3
"""Import one explicitly trusted text PDF into Akula's existing Markdown search.

PREVIEW ONLY: Shiva CLI / Poppler run WITHOUT an OS sandbox. Do not process
untrusted PDFs. This script does not modify Akula's production File Gate.
"""
import argparse
import hashlib
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import quote

MAX_PDF = 2 * 1024 * 1024
MAX_TEXT = 2 * 1024 * 1024
PREVIEW_DIR = 'PDF Search Preview'


def read_pdf(path):
    if path.is_symlink():
        raise ValueError('PDF symlinks are not accepted')
    meta = path.stat()
    if not stat.S_ISREG(meta.st_mode) or not (16 <= meta.st_size <= MAX_PDF):
        raise ValueError('PDF must be a regular file, at most 2 MiB')
    with path.open('rb') as stream:
        raw = stream.read(MAX_PDF + 1)
    if len(raw) != meta.st_size or not raw.startswith(b'%PDF-'):
        raise ValueError('PDF header is invalid or source changed')
    return raw


def write_once(path, content):
    if path.is_symlink():
        raise ValueError('Output symlink is prohibited')
    try:
        with path.open('xb') as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError:
        if path.is_symlink() or not path.is_file() or path.stat().st_size != len(content):
            raise ValueError('Existing output cannot be replaced')
        if path.read_bytes() != content:
            raise ValueError('Existing output has different content; refusing overwrite')


def run(args):
    if not args.trusted_local_only:
        raise ValueError('Use --trusted-local-only: this preview converter is not sandboxed')
    source = Path(args.pdf).expanduser()
    raw = read_pdf(source)
    vault = Path(args.vault).expanduser().resolve(strict=True)
    if not vault.is_dir():
        raise ValueError('Vault must exist and be a directory')
    digest = hashlib.sha256(raw).hexdigest()
    command_name = 'shiva' if args.engine == 'shiva' else 'pdftotext'
    binary = args.converter or shutil.which(command_name)
    if not binary:
        raise ValueError(f'{command_name} not found; set --converter to its executable')
    binary = Path(binary).resolve(strict=True)
    if not binary.is_file():
        raise ValueError('Converter must be an existing executable')

    with tempfile.TemporaryDirectory(prefix='akula-pdf-preview-') as temporary:
        staging = Path(temporary)
        input_pdf, output_md = staging / 'input.pdf', staging / 'text.md'
        input_pdf.write_bytes(raw)
        cmd = ([str(binary), str(input_pdf), str(output_md)] if args.engine == 'shiva'
               else [str(binary), '-layout', '-enc', 'UTF-8', str(input_pdf), str(output_md)])
        try:
            result = subprocess.run(cmd, cwd=staging, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, timeout=45, check=False)
        except subprocess.TimeoutExpired as exc:
            raise ValueError('PDF extraction exceeded 45 seconds') from exc
        if result.returncode or not output_md.is_file():
            raise ValueError('PDF conversion failed; vault was not changed')
        if not 0 < output_md.stat().st_size <= MAX_TEXT:
            raise ValueError('Extracted text is empty or over 2 MiB')
        body = output_md.read_text(encoding='utf-8')
        if not body.strip():
            raise ValueError('PDF has no extractable text; OCR is not enabled')

    if hashlib.sha256(read_pdf(source)).hexdigest() != digest:
        raise ValueError('Original PDF changed during conversion')
    target = vault / PREVIEW_DIR
    target.mkdir(exist_ok=True)
    if target.is_symlink() or not target.is_dir() or target.resolve() != target:
        raise ValueError('Preview directory cannot be a symlink')
    stem = re.sub(r'[^\w .-]', '-', source.stem).strip(' .-_')[:60] or 'document'
    base = f'PDF-{stem}-{digest[:12]}'
    original_path, note_path = target / (base + '.pdf'), target / (base + '.md')
    # The original is a byte-identical copy. Neither file is ever overwritten.
    write_once(original_path, raw)
    # Extracted Markdown is inert: its content remains within a code fence.
    max_ticks = max((len(m.group()) for m in re.finditer(chr(96) + '+', body)), default=0)
    fence = chr(96) * max(3, max_ticks + 1)
    link = quote(original_path.name, safe='.-_')
    note = (f'# PDF preview: {stem}\n\n'
            '> **Needs review.** Search projection, not verified PDF text. '
            'Only for explicitly trusted local documents.\n\n'
            f'Original: [{original_path.name}](./{link})\n\n'
            f'Source SHA-256: {digest}\n\n'
            f'## Extracted text (unverified)\n\n{fence}text\n{body.rstrip()}\n{fence}\n')
    write_once(note_path, note.encode('utf-8'))
    print(f'AKULA_PDF_PREVIEW=IMPORTED;note={note_path};original={original_path}')
    return note_path


def main(argv=None):
    parser = argparse.ArgumentParser(description='One trusted PDF -> searchable Akula Markdown preview')
    parser.add_argument('--pdf', required=True)
    parser.add_argument('--vault', required=True)
    parser.add_argument('--engine', choices=('shiva', 'poppler'), default='shiva')
    parser.add_argument('--converter', help='Full path to trusted shiva or pdftotext executable')
    parser.add_argument('--trusted-local-only', action='store_true')
    args = parser.parse_args(argv)
    try:
        run(args)
    except (OSError, ValueError, UnicodeError) as exc:
        print(f'AKULA_PDF_PREVIEW=HOLD;reason={exc}', file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())