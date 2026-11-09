#!/usr/bin/env python3
"""Package a built macOS app or Windows NSIS installer for its signed update feed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path, help='OpenRoom.app, its prepared macOS archive, or the Windows installer')
    parser.add_argument('--notes', type=Path)
    parser.add_argument('--prepare-only', action='store_true', help='Create the verified macOS archive without a signing key')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    app = args.app.resolve(strict=True)
    config = json.loads((root / 'desktop/tauri.conf.json').read_text(encoding='utf-8'))
    version = config['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Expected a three-part app version')
    if args.prepare_only and app.name != 'OpenRoom.app':
        parser.error('--prepare-only requires OpenRoom.app')
    if not args.prepare_only and not args.notes:
        parser.error('--notes is required when signing')
    if app.suffix.lower() == '.exe':
        package_windows(app, args, root, config)
        return
    if args.output.exists():
        raise ValueError('Output already exists; preserve existing releases')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent) as temporary:
        staging = Path(temporary) / 'release'
        staging.mkdir()
        archive = staging / f'OpenRoom-{version}-macos-arm64.app.tar.gz'
        if app.name == 'OpenRoom.app':
            prepare_macos_archive(app, archive, root)
        elif app.name == archive.name:
            # Use only the exact archive plus checksum from the macOS release job.
            shutil.copyfile(app, archive)
            expected = (app.parent / 'SHA256SUMS.txt').read_text().strip()
            if expected != checksum_line(archive).strip():
                raise ValueError('Prepared macOS archive checksum mismatch')
        else:
            raise ValueError(f'Expected OpenRoom.app or {archive.name}')
        with tarfile.open(archive, 'r:gz') as bundle:
            plist = plistlib.loads(bundle.extractfile('OpenRoom.app/Contents/Info.plist').read())
        if plist['CFBundleShortVersionString'] != version or plist['CFBundleIdentifier'] != config['identifier']:
            raise ValueError('App does not match the configured release')
        if not args.prepare_only:
            key = Path(os.environ['TAURI_SIGNING_PRIVATE_KEY_PATH']).resolve(strict=True)
            if Path(str(key) + '.pub').read_text().strip() != config['plugins']['updater']['pubkey']:
                raise ValueError('Signing key does not match the Mac app public key')
            subprocess.run([
                'node', str(root / 'desktop/node_modules/@tauri-apps/cli/tauri.js'),
                'signer', 'sign', '--app-version', version, str(archive),
            ], check=True, stdout=subprocess.DEVNULL)
            signature = Path(str(archive) + '.sig').read_text().strip()
            manifest = {
                'version': version,
                'notes': args.notes.read_text(encoding='utf-8').strip(),
                'minimum_macos': plist['LSMinimumSystemVersion'],
                'platforms': {'darwin-aarch64': {
                    'url': f'https://github.com/wandering-beans/OpenRoom/releases/download/v{version}/{archive.name}',
                    'signature': signature,
                }},
            }
            (staging / 'macos-aarch64.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        (staging / 'SHA256SUMS.txt').write_text(checksum_line(archive), encoding='utf-8')
        staging.rename(args.output)
    print(f'Prepared macOS {"archive" if args.prepare_only else "signed update"} {version}: {args.output}')


def checksum_line(path):
    with path.open('rb') as file:
        return f'{hashlib.file_digest(file, "sha256").hexdigest()}  {path.name}\n'


def prepare_macos_archive(app, archive, root):
    for path in app.rglob('*'):
        if path.is_symlink() and not path.resolve().is_relative_to(app):
            raise ValueError('Bundle contains an external symbolic link')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    executable = app / 'Contents/MacOS/OpenRoom'
    if subprocess.check_output(['lipo', '-archs', str(executable)], text=True).strip() != 'arm64':
        raise ValueError('Only arm64 update packages are supported')
    subprocess.run(['python3', str(root / 'scripts/privacy-check.py'), str(app)], check=True)

    def anonymous_metadata(info):
        info.uid = info.gid = 0
        info.uname = info.gname = ''
        info.pax_headers = {}
        return info

    with tarfile.open(archive, 'w:gz', format=tarfile.PAX_FORMAT) as output:
        output.add(app, arcname='OpenRoom.app', filter=anonymous_metadata)


def package_windows(installer, args, root, config):
    version = config['version']
    expected = f'OpenRoom-{version}-windows-x64-setup.exe'
    if installer.name != expected or installer.read_bytes()[:2] != b'MZ':
        raise ValueError('Expected the versioned Windows x64 NSIS installer from build-windows.py')
    key = Path(os.environ['TAURI_SIGNING_PRIVATE_KEY_PATH']).resolve(strict=True)
    windows = json.loads((root / 'desktop/tauri.windows.conf.json').read_text(encoding='utf-8'))
    public_key = windows['plugins']['updater'].get('pubkey', config['plugins']['updater']['pubkey'])
    if Path(str(key) + '.pub').read_text().strip() != public_key:
        raise ValueError('Signing key does not match the app public key')
    if args.output.exists():
        raise ValueError('Output already exists; preserve existing releases')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent) as temporary:
        staging = Path(temporary) / 'release'
        staging.mkdir()
        artifact = staging / expected
        shutil.copy2(installer, artifact)
        subprocess.run(['node', str(root / 'desktop/node_modules/@tauri-apps/cli/tauri.js'),
                        'signer', 'sign', '--app-version', version, str(artifact)],
                       check=True, stdout=subprocess.DEVNULL)
        signature = Path(str(artifact) + '.sig').read_text().strip()
        manifest = {'version': version, 'notes': args.notes.read_text(encoding='utf-8').strip(),
                    'platforms': {'windows-x86_64': {
                        'url': f'https://github.com/wandering-beans/OpenRoom/releases/download/v{version}/{expected}',
                        'signature': signature}}}
        (staging / 'windows-x86_64.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        checksum = hashlib.sha256(artifact.read_bytes()).hexdigest()
        (staging / 'SHA256SUMS.txt').write_text(f'{checksum}  {expected}\n', encoding='utf-8')
        staging.rename(args.output)
    print(f'Prepared signed Windows update {version}: {args.output}')
    print('Publish assets before promoting windows-x86_64.json; never replace the macOS feed.')


if __name__ == '__main__':
    main()
