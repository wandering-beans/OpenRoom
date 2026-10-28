#!/usr/bin/env python3
"""Reject stale dependency notices before packaging the macOS app."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    root = Path(__file__).resolve().parents[1]
    licenses = root / 'licenses'
    manifest = json.loads((licenses / 'manifest.json').read_text())
    for name, expected in manifest['inputs'].items():
        if hashlib.sha256((root / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f'License inventory is stale: {name}; update licenses/manifest.json and notices')

    libraries = {}
    for package in manifest['packages']:
        for name in package['notices']:
            path = (licenses / name).resolve()
            if not path.is_relative_to(licenses.resolve()) or not path.is_file():
                raise ValueError(f'Missing license notice: {name}')
        for library in package.get('libraries', []):
            libraries[library['name']] = library['uuid']

    if len(sys.argv) > 1:
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
    for issue in manifest['unresolved']:
        print(f"Unresolved license notice: {issue['component']}")
    print('This checks inventory consistency, not distribution readiness; see DISTRIBUTION.md.')


if __name__ == '__main__':
    main()
