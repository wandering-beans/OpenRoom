#!/usr/bin/env python3
"""Package an already built macOS app for the signed Tauri update feed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path)
    parser.add_argument('--notes', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    app = args.app.resolve(strict=True)
    config = json.loads((root / 'desktop/tauri.conf.json').read_text())
    version = config['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Expected a three-part app version')
    if app.name != 'OpenRoom.app':
        raise ValueError('Expected OpenRoom.app')
    plist = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    if plist['CFBundleShortVersionString'] != version or plist['CFBundleIdentifier'] != config['identifier']:
        raise ValueError('App does not match the configured release')
    key = Path(os.environ['TAURI_SIGNING_PRIVATE_KEY_PATH']).resolve(strict=True)
    public_key = Path(str(key) + '.pub').read_text().strip()
    if public_key != config['plugins']['updater']['pubkey']:
        raise ValueError('Signing key does not match the app public key')
    if args.output.exists():
        raise ValueError('Output already exists; preserve existing releases')
    for path in app.rglob('*'):
        if path.is_symlink() and not path.resolve().is_relative_to(app):
            raise ValueError('Bundle contains an external symbolic link')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    executable = app / 'Contents/MacOS/OpenRoom'
    if subprocess.check_output(['lipo', '-archs', str(executable)], text=True).strip() != 'arm64':
        raise ValueError('Only arm64 update packages are supported')
    subprocess.run(['python3', str(root / 'scripts/privacy-check.py'), str(app)], check=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent) as temporary:
        staging = Path(temporary) / 'release'
        staging.mkdir()
        archive = staging / f'OpenRoom-{version}-macos-arm64.app.tar.gz'

        def anonymous_metadata(info):
            info.uid = info.gid = 0
            info.uname = info.gname = ''
            info.pax_headers = {}
            return info

        with tarfile.open(archive, 'w:gz', format=tarfile.PAX_FORMAT) as output:
            output.add(app, arcname='OpenRoom.app', filter=anonymous_metadata)
        # The trusted comment binds the archive to this version, preventing manifest downgrades.
        subprocess.run([
            'node', str(root / 'desktop/node_modules/@tauri-apps/cli/tauri.js'),
            'signer', 'sign', '--app-version', version, str(archive),
        ], check=True, stdout=subprocess.DEVNULL)
        signature = Path(str(archive) + '.sig').read_text().strip()
        manifest = {
            'version': version,
            'notes': args.notes.read_text().strip(),
            'minimum_macos': plist['LSMinimumSystemVersion'],
            'platforms': {'darwin-aarch64': {
                'url': f'https://github.com/wandering-beans/OpenRoom/releases/download/v{version}/{archive.name}',
                'signature': signature,
            }},
        }
        (staging / 'latest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (staging / 'SHA256SUMS.txt').write_text(f'{checksum}  {archive.name}\n')
        staging.rename(args.output)
    print(f'Prepared signed update {version}: {args.output}')
    print('Publish the release assets before promoting latest.json to updates/latest.json.')


if __name__ == '__main__':
    main()
