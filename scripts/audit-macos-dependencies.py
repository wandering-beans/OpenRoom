#!/usr/bin/env python3
"""Collect a candidate inventory only when installed Homebrew recipes are unchanged."""
import difflib
import hashlib
import json
from pathlib import Path
import re
import subprocess


def source_recipe(text):
    # Bottle hashes describe platform builds, not source or build instructions.
    return re.sub(r'^  bottle do\n.*?^  end\n\n?', '', text.replace('\r\n', '\n'),
                  flags=re.MULTILINE | re.DOTALL).strip()


def main():
    root = Path(__file__).resolve().parents[1]
    manifest = json.loads((root / 'licenses/manifest.json').read_text())
    evidence = []
    changes = []
    output = root / 'dist/macos-dependency-audit'
    output.mkdir(parents=True, exist_ok=True)
    for package in manifest['packages']:
        if not package.get('libraries') or package['name'] == 'RawTherapee':
            continue
        name = package['name']
        prefix = Path(subprocess.check_output(['brew', '--prefix', name], text=True).strip()).resolve()
        installed = (prefix / '.brew' / f'{name}.rb').read_text()
        recorded = next(p for p in package['notices'] if p.endswith('/build-formula.rb'))
        previous = (root / 'licenses' / recorded).read_text()
        if prefix.name != package['version'] or source_recipe(installed) != source_recipe(previous):
            changes.append(f'{name}: recorded {package["version"]}, installed {prefix.name}')
            (output / f'{name}.rb').write_text(installed)
            print(''.join(difflib.unified_diff(source_recipe(previous).splitlines(True),
                                             source_recipe(installed).splitlines(True))))
        for library in package['libraries']:
            binary = (prefix / 'lib' / library['name']).resolve()
            if not binary.is_file():
                changes.append(f'{name}: missing {library["name"]}')
                continue
            if not binary.is_relative_to(prefix):
                raise ValueError(f'Library escapes its package: {library["name"]}')
            library['uuid'] = subprocess.check_output(
                ['dwarfdump', '--uuid', str(binary)], text=True).split()[1]
        evidence.append({'name': name, 'version': prefix.name,
                         'source_recipe_sha256': hashlib.sha256(source_recipe(installed).encode()).hexdigest(),
                         'libraries': package['libraries']})
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    if changes:
        raise ValueError('Review changed dependencies:\n' + '\n'.join(changes))
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'Verified unchanged source recipes for {len(evidence)} Homebrew packages')


if __name__ == '__main__':
    assert source_recipe('class A\n  bottle do\n    sha256 "x"\n  end\n  url "source"\nend\n') == 'class A\n  url "source"\nend'
    assert source_recipe('  url "one"') != source_recipe('  url "two"')
    main()
