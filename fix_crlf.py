#!/usr/bin/env python3
"""
fix_crlf.py — strip \\r (CR) from all text files in the repo tree.
Skips binary files and .git directory.
"""

import os
import sys

# Extensions that are definitely binary — skip entirely
BINARY_EXTENSIONS = {
    '.img', '.bin', '.so', '.ko', '.a', '.o', '.gz', '.xz', '.bz2', '.lz4',
    '.zip', '.jar', '.apk', '.dex', '.elf', '.exe', '.dll',
    '.png', '.jpg', '.jpeg', '.gif', '.ico', '.webp', '.bmp',
    '.ttf', '.otf', '.woff', '.woff2',
    '.pyc', '.pyo',
    '.cpio', '.tar',
    '.lgz',
}

MAX_CHECK_BYTES = 8192  # read this much to sniff for binary content


def is_binary(path: str) -> bool:
    """Heuristic: file contains a NUL byte in the first MAX_CHECK_BYTES."""
    try:
        with open(path, 'rb') as f:
            chunk = f.read(MAX_CHECK_BYTES)
        return b'\x00' in chunk
    except OSError:
        return True


def fix_file(path: str) -> bool:
    """
    Remove all \\r characters from a text file.
    Returns True if the file was modified.
    """
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError as e:
        print(f'  SKIP (read error): {path}: {e}')
        return False

    if b'\r' not in data:
        return False

    fixed = data.replace(b'\r\n', b'\n').replace(b'\r', b'\n')
    try:
        with open(path, 'wb') as f:
            f.write(fixed)
    except OSError as e:
        print(f'  ERROR (write error): {path}: {e}')
        return False

    return True


def main():
    root = os.path.dirname(os.path.abspath(__file__))
    print(f'Scanning: {root}')

    fixed_count = 0
    skip_count = 0

    for dirpath, dirnames, filenames in os.walk(root):
        # Skip .git and out directories
        dirnames[:] = [d for d in dirnames if d not in ('.git', 'out', '__pycache__')]

        for name in filenames:
            filepath = os.path.join(dirpath, name)
            relpath = os.path.relpath(filepath, root)

            _, ext = os.path.splitext(name.lower())
            if ext in BINARY_EXTENSIONS:
                skip_count += 1
                continue

            if is_binary(filepath):
                skip_count += 1
                continue

            if fix_file(filepath):
                print(f'  FIXED: {relpath}')
                fixed_count += 1

    print(f'\nDone. Fixed: {fixed_count} files, skipped binary: {skip_count} files.')


if __name__ == '__main__':
    main()
