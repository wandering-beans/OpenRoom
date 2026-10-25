#!/usr/bin/env python3
"""Fail on local account paths or recognizable credentials in public files.
Default: source tree excluding build/local data. Pass files/directories for artifacts.
Only filenames, offsets and categories are reported; never matching values.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess

EXCLUDED = {'target', 'dist', 'ui-dist', 'node_modules', '.git', '.tools', '.agents', '.codex', '__pycache__'}
PATTERNS = {
    'local-account-path': rb'(?<![\w/.-])(?:/(?:Users|home)/[A-Za-z0-9_.-]+/|[A-Za-z]:[\\/]+Users[\\/]+[A-Za-z0-9_. -]+[\\/])',
    'mac-private-temp-path': rb'/(?:private/)?var/folders/[a-zA-Z0-9_-]+/[a-zA-Z0-9_-]+/',
    'mounted-photo-path': rb'/Volumes/[A-Za-z0-9_\x80-\xff][^\s"<>`\x00]*/',
    'private-key': rb'-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----',
    'access-token': rb'(?<![\w-])(?:AKIA[0-9A-Z]{16}|(?:ghp_|github_pat_|gho_|ghs_)[A-Za-z0-9_]{25,}|sk-(?:proj-)?[A-Za-z0-9_-]{24,}|xox[baprs]-[A-Za-z0-9-]{20,}|AIza[0-9A-Za-z_-]{30,})(?![\w-])',
    'credential-url': rb'https?://[^\s/:<>"\x00]+:[^\s/@<>"\x00]+@',
}

def source_files(root):
    for base, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = [d for d in dirs if d not in EXCLUDED and not Path(base, d).is_symlink()]
        for name in sorted(names):
            path = Path(base, name)
            if not path.is_symlink() and name != ".DS_Store" and not name.startswith("._"):
                yield path

def findings(data):
    for category, pattern in PATTERNS.items():
        for match in re.finditer(pattern, data):
            yield category, match.start()
    # UTF-16 is common in Windows executables and image metadata.
    for encoding in ('utf-16le', 'utf-16be'):
        if b'\x00' not in data:
            break
        decoded = data.decode(encoding, errors='ignore').encode('utf-8')
        for category in ('local-account-path', 'private-key', 'access-token'):
            if re.search(PATTERNS[category], decoded):
                yield category + '-' + encoding, None

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tracked', action='store_true', help='Check only Git-tracked distribution files')
    parser.add_argument('paths', nargs='*', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if args.tracked and args.paths:
        parser.error('--tracked cannot be combined with paths')
    if args.tracked:
        names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
        files = [root / name for name in names if name]
    else:
        files = list(source_files(root)) if not args.paths else [
            p for path in args.paths for p in (path.rglob('*') if path.is_dir() else [path])
            if p.is_file() and not p.is_symlink()]
    errors = 0
    for path in files:
        if path.name.startswith('.env') and path.name != '.env.example' or path.suffix.lower() in {'.p12', '.pfx', '.pem', '.key', '.db', '.sqlite', '.sqlite3'}:
            print(f'{path.name}: sensitive-file-type'); errors += 1
        for category, offset in findings(path.read_bytes()):
            print(f'{path.name}: {category} offset={offset}'); errors += 1
    print(f'Privacy check: {len(files)} files, {errors} findings')
    return bool(errors)

if __name__ == '__main__':
    raise SystemExit(main())
