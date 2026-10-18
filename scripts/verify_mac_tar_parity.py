#!/usr/bin/env python3
"""Read-only app.tar.gz vs mounted, already-verified macOS DMG app parity.

No tar extraction is performed. This is NOT Apple signing or minisign verification.
"""
import argparse
import hashlib
import os
import posixpath
import stat
import sys
import tarfile
import unicodedata
from pathlib import Path

MAX_ENTRIES = 40000
MAX_TOTAL = 1024 * 1024 * 1024
MAX_FILE = 256 * 1024 * 1024
MAX_NAME = 4096


class InvalidBundle(ValueError):
    pass


def fail(reason):
    raise InvalidBundle(reason)


def valid_name(name):
    if not name or len(name.encode('utf-8')) > MAX_NAME or '\x00' in name or '\\' in name:
        fail('invalid TAR member name')
    parts = name.rstrip('/').split('/')
    if parts[0] != 'Aquilum.app' or any(p in ('', '.', '..') for p in parts):
        fail(f'unsafe or out-of-bundle TAR entry: {name!r}')
    if len(parts) > 64:
        fail('archive path too deep')
    return '/'.join(parts[1:])


def valid_link(relative, target):
    if not target or '\x00' in target or '\\' in target or target.startswith('/'):
        fail(f'unsafe symlink target at {relative}')
    normalized = posixpath.normpath(posixpath.join(posixpath.dirname(relative), target))
    if normalized == '..' or normalized.startswith('../'):
        fail(f'symlink escapes app bundle: {relative}')
    return target


def digest(reader):
    h = hashlib.sha256()
    while True:
        data = reader.read(1024 * 1024)
        if not data:
            break
        h.update(data)
    return h.hexdigest()


def add(manifest, folded, name, result):
    if not name:
        return
    key = unicodedata.normalize('NFC', name).casefold()
    if key in folded:
        fail(f'duplicate or case/Unicode-colliding app path: {name}')
    folded.add(key)
    manifest[name] = result
    if len(manifest) > MAX_ENTRIES:
        fail('too many bundle entries')


def mounted_manifest(root):
    if root.name != 'Aquilum.app' or not root.is_dir() or root.is_symlink():
        fail('expected an actual mounted Aquilum.app directory')
    # The bundle root itself is installed by the updater, so its mode is
    # part of parity just like the modes of all nested directories.
    root_mode = stat.S_IMODE(root.lstat().st_mode)
    if root_mode & 0o7000:
        fail('privileged permission bits in mounted app root')
    manifest, folded = {'': ('dir', root_mode & 0o777)}, set()
    pending = [(root, '')]
    total = 0
    while pending:
        parent, prefix = pending.pop()
        with os.scandir(parent) as entries:
            items = list(entries)
        for item in items:
            rel = f'{prefix}/{item.name}' if prefix else item.name
            if len(rel.encode('utf-8')) > MAX_NAME or len(rel.split('/')) > 64:
                fail('mounted app path exceeds allowed limits')
            info = item.stat(follow_symlinks=False)
            mode = stat.S_IMODE(info.st_mode)
            if mode & 0o7000:
                fail(f'privileged permission bits in mounted bundle: {rel}')
            if stat.S_ISLNK(info.st_mode):
                result = ('link', valid_link(rel, os.readlink(item.path)))
            elif stat.S_ISDIR(info.st_mode):
                result = ('dir', mode & 0o777)
                pending.append((Path(item.path), rel))
            elif stat.S_ISREG(info.st_mode):
                if info.st_size > MAX_FILE:
                    fail(f'oversized mounted bundle file: {rel}')
                total += info.st_size
                if total > MAX_TOTAL:
                    fail('mounted app exceeds size budget')
                with open(item.path, 'rb') as reader:
                    result = ('file', mode & 0o777, info.st_size, digest(reader))
            else:
                fail(f'unsupported mounted bundle entry: {rel}')
            add(manifest, folded, rel, result)
    return manifest


def tar_manifest(archive):
    manifest, folded = {}, set()
    total = 0
    root_seen = False
    with tarfile.open(archive, mode='r|gz') as tar:
        for member in tar:
            name = valid_name(member.name)
            if not name:
                if not member.isdir():
                    fail('archive root is not a directory')
                if root_seen:
                    fail('duplicate archive root directory')
                if member.mode & 0o7000:
                    fail('privileged permission bits in archive app root')
                manifest[''] = ('dir', member.mode & 0o777)
                root_seen = True
                continue
            # The release TAR must begin with exactly one app root. Otherwise
            # an extractor can implicitly create or later chmod this directory.
            if not root_seen:
                fail('archive app root directory missing or not first')
            if member.mode & 0o7000:
                fail(f'privileged file mode: {name}')
            mode = member.mode & 0o777
            if member.isdir():
                result = ('dir', mode)
            elif member.issym():
                result = ('link', valid_link(name, member.linkname))
            elif member.isfile():
                if member.size < 0 or member.size > MAX_FILE:
                    fail(f'oversized TAR entry: {name}')
                total += member.size
                if total > MAX_TOTAL:
                    fail('TAR payload exceeds size budget')
                reader = tar.extractfile(member)
                if reader is None:
                    fail(f'archive member lacks file body: {name}')
                with reader:
                    result = ('file', mode, member.size, digest(reader))
            else:
                fail(f'unsupported TAR member type: {name}')
            add(manifest, folded, name, result)
    if not root_seen:
        fail('archive app root directory missing')
    return manifest


def compare(app, archive):
    actual = mounted_manifest(app)
    packed = tar_manifest(archive)
    if actual != packed:
        missing = sorted(actual.keys() - packed.keys())[:5]
        extra = sorted(packed.keys() - actual.keys())[:5]
        changed = sorted(k for k in actual.keys() & packed.keys() if actual[k] != packed[k])[:5]
        fail(f'archive differs from signed DMG bundle; missing={missing}, extra={extra}, changed={changed}')
    for critical in ('Contents/MacOS/aquilum-app', 'Contents/Info.plist'):
        if critical not in actual or actual[critical][0] != 'file':
            fail(f'important signed app file is missing: {critical}')
    return len(actual)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mounted_app', type=Path)
    parser.add_argument('archive', type=Path)
    args = parser.parse_args()
    try:
        count = compare(args.mounted_app, args.archive)
    except (InvalidBundle, OSError, tarfile.TarError, EOFError) as exc:
        print(f'Akula updater archive HOLD: {exc}', file=sys.stderr)
        return 1
    print(f'Akula macOS updater archive matches signed DMG contents ({count} entries)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
