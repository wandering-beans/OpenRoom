#!/usr/bin/env python3
"""Reject stale dependency notices before packaging the macOS app."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def check_engine_data(directory, inventory):
    entries = inventory['profiles'] + inventory['data_files']
    expected = {entry['file']: entry['sha256'] for entry in entries}
    if len(expected) != len(entries):
        raise ValueError('Duplicate engine data in the license inventory')
    actual = {path.relative_to(directory).as_posix(): path
              for path in directory.rglob('*') if path.is_file()}
    if set(actual) != set(expected):
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        raise ValueError(f'Engine data inventory changed; missing={missing}, extra={extra}')
    for name, path in actual.items():
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected[name]:
            raise ValueError(f'Engine data changed: {name}; review its license evidence')


def main():
    root = Path(__file__).resolve().parents[1]
    licenses = root / 'licenses'
    manifest = json.loads((licenses / 'manifest.json').read_text())
    profiles = json.loads((licenses / 'profile-notices.json').read_text())
    engine = json.loads((root / 'native/rtengine/source.json').read_text())
    if profiles['source_commit'] != engine['commit']:
        raise ValueError('Profile license inventory does not match the pinned engine')
    for name, expected in manifest['inputs'].items():
        if hashlib.sha256((root / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f'License inventory is stale: {name}; update licenses/manifest.json and notices')

    libraries = {}
    for name in manifest.get('supporting_notices', []):
        path = (licenses / name).resolve()
        if not path.is_relative_to(licenses.resolve()) or not path.is_file():
            raise ValueError(f'Missing supporting notice: {name}')
    for package in manifest['packages']:
        for name in package['notices']:
            path = (licenses / name).resolve()
            if not path.is_relative_to(licenses.resolve()) or not path.is_file():
                raise ValueError(f'Missing license notice: {name}')
        for patch in package.get('build_patches', []):
            path = (licenses / patch['file']).resolve()
            if not path.is_relative_to(licenses.resolve()) or not path.is_file():
                raise ValueError(f"Missing build patch: {patch['file']}")
            if hashlib.sha256(path.read_bytes()).hexdigest() != patch['sha256']:
                raise ValueError(f"Build patch changed: {patch['file']}")
        for library in package.get('libraries', []):
            libraries[library['name']] = library['uuid']

    if len(sys.argv) > 1:
        check_engine_data(Path(sys.argv[1]) / 'Contents/Resources/rtengine', profiles)
        frameworks = Path(sys.argv[1]) / 'Contents/Frameworks'
        actual = {path.name: path for path in frameworks.glob('*.dylib')}
        if set(actual) != set(libraries):
            raise ValueError('Bundled libraries changed; update the license inventory')
        for name, path in actual.items():
            # The bridge is rebuilt from our tracked source; third-party builds need an exact match.
            if name == 'libopenroom_rtengine.dylib':
                continue
            uuid = subprocess.check_output(['dwarfdump', '--uuid', str(path)], text=True).split()[1]
            if uuid != libraries[name]:
                raise ValueError(f'Bundled library changed: {name}; update its license and source record')

    print(f"License inventory: {len(manifest['packages'])} packages, notices and input hashes match")
    print(f"Engine data inventory: {len(profiles['profiles'])} profiles and {len(profiles['data_files'])} data files")
    for issue in manifest['unresolved']:
        print(f"Unresolved license notice: {issue['component']}")
    print('This checks inventory consistency, not distribution readiness.')


if __name__ == '__main__':
    main()
