#!/usr/bin/env python3
"""Build the Windows x64 NSIS installer with the exact native runtime closure."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    if os.name != 'nt':
        raise SystemExit('Build this installer on Windows with scripts/windows-dev.ps1 loaded.')
    root = Path(__file__).resolve().parents[1]
    result = subprocess.check_output([sys.executable, str(root / 'scripts/build-rtengine.py')], cwd=root)
    native = json.loads(result)
    target = root / '.tools/windows-bundle'
    target.parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='windows-bundle-', dir=target.parent) as temporary:
        staging = Path(temporary) / 'resources'
        staging.mkdir()
        for dll in [native['library'], *native['runtime']]:
            shutil.copy2(dll, staging / Path(dll).name)
        shutil.copytree(native['data'], staging / 'rtengine')
        notices = staging / 'licenses'
        shutil.copytree(root / 'licenses', notices / 'third-party')
        for name in ['LICENSE', 'COPYRIGHT.md', 'THIRD_PARTY.md']:
            shutil.copy2(root / name, notices / name)
        for name in ['source.json', 'float-preview.patch', 'metadata-time.patch', 'COPYING.GPL-3.0']:
            shutil.copy2(root / 'native/rtengine' / name, notices / name)
        prefix = Path(os.environ['OPENROOM_MSYS2_PREFIX'])
        shutil.copytree(prefix / 'share/licenses', notices / 'msys2')
        vcpkg = Path(os.environ['VCPKG_ROOT']) / 'installed/x64-windows-static-md/share'
        for copyright in vcpkg.glob('*/copyright'):
            destination = notices / 'vcpkg' / copyright.parent.name
            destination.mkdir(parents=True)
            shutil.copy2(copyright, destination / 'copyright')
        (staging / 'openroom-install-target').write_text(
            'local.advanced-explorer.desktop/windows-x86_64/nsis\n', encoding='utf-8')
        # Replace only this generated resource directory, never the installed app or user data.
        if target.exists():
            if target.is_symlink() or target.resolve().parent != (root / '.tools').resolve():
                raise RuntimeError('Unexpected bundle staging path')
            shutil.rmtree(target)
        staging.rename(target)
    env = os.environ.copy()
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join([
        env.get('CARGO_ENCODED_RUSTFLAGS', ''),
        f'--remap-path-prefix={Path.home()}=/build/home',
        f'--remap-path-prefix={root}=/source/OpenRoom',
    ]).lstrip('\x1f')
    env.pop('RUSTFLAGS', None)
    resources = json.dumps({'bundle': {'resources': {'../.tools/windows-bundle/': ''}}})
    subprocess.run(['node', 'node_modules/@tauri-apps/cli/tauri.js', 'build', '--bundles', 'nsis',
                    '--config', resources],
                   cwd=root / 'desktop', env=env, check=True)
    installers = list((root / 'target/release/bundle/nsis').glob('*-setup.exe'))
    version = json.loads((root / 'desktop/tauri.conf.json').read_text(encoding='utf-8'))['version']
    installer = root / f'target/release/bundle/nsis/OpenRoom_{version}_x64-setup.exe'
    if installer not in installers:
        raise RuntimeError('The expected x64 NSIS installer was not produced')
    output = root / 'dist' / f'OpenRoom-{version}-windows-x64-setup.exe'
    output.parent.mkdir(exist_ok=True)
    shutil.copy2(installer, output)
    checksum = hashlib.sha256(output.read_bytes()).hexdigest()
    Path(str(output) + '.sha256').write_text(f'{checksum}  {output.name}\n', encoding='utf-8')
    print(json.dumps({'installer': str(output), 'sha256': checksum}))


if __name__ == '__main__':
    main()
