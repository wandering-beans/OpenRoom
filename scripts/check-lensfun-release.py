"""Release preparation only. Refresh the reviewed pin before creating the release tag."""
import argparse
import hashlib
import urllib.request
from lensfun_data import ARCHIVE, PIN, profiles

URL = 'https://lensfun.github.io/db/version_1.tar.bz2'

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--update', action='store_true', help='stage the latest database for review')
    args = parser.parse_args()
    with urllib.request.urlopen(URL, timeout=30) as response:
        data = response.read(4_000_001)
    if len(data) > 4_000_000:
        raise ValueError('Unexpected Lensfun download size')
    digest = hashlib.sha256(data).hexdigest()
    files = profiles(data)
    if args.update:
        ARCHIVE.write_bytes(data)
        PIN.write_text(digest + '\n', encoding='ascii')
    if data != ARCHIVE.read_bytes() or digest != PIN.read_text().strip():
        raise SystemExit('New Lensfun data: run python scripts/check-lensfun-release.py --update, review, test and commit before tagging.')
    print(f'Lensfun: {len(files)} XML files, SHA-256 {digest}')

if __name__ == '__main__':
    main()
