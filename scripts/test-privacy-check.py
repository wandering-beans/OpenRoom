#!/usr/bin/env python3
"""Small regression check for public-artifact filtering and credential matching."""
import importlib.util
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile
import subprocess

spec = importlib.util.spec_from_file_location('privacy', Path(__file__).with_name('privacy-check.py'))
privacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(privacy)

home = b'/Users/' + b'example/project/src/lib.rs'
assert list(privacy.findings(home))
assert list(privacy.findings(home.decode().encode('utf-16le')))
assert list(privacy.findings(home.decode().encode('utf-16be')))
assert list(privacy.findings(b'C:' + b'\\Users\\' + b'Example Name\\project\\main.rs'))
assert list(privacy.findings(b'ghp_' + b'A' * 36))
assert list(privacy.findings(b'-----BEGIN ' + b'PRIVATE KEY-----'))
assert list(privacy.findings(b'https://' + b'user:password@example.invalid'))
assert list(privacy.findings(b'/Volumes/' + b'personal-share/photos/example.nef'))
assert not list(privacy.findings(b'mask-type-mask-image-mask-border-mask-position'))
assert not list(privacy.findings(b'/source/AdvancedExplorer/src/lib.rs\0/build/home/.cargo/lib.rs'))
with tempfile.TemporaryDirectory() as temp:
    root = Path(temp)
    (root/'src').mkdir(); (root/'src'/'main.rs').write_text('fn main() {}')
    (root/'target').mkdir(); (root/'target'/'private').write_bytes(home)
    for directory in ('node_modules', 'ui-dist'):
        output = root/'desktop'/directory
        output.mkdir(parents=True)
        (output/'vendor-example').write_bytes(home)
    (root/'.env').write_text('local-only')
    files = {p.relative_to(root).as_posix() for p in privacy.source_files(root)}
    assert files == {'src/main.rs', '.env'}  # Ignoring .env in Git must not bypass the check.
print('Privacy regression checks passed')

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as temp:
    repo = Path(temp)
    subprocess.run(['git', 'init', '-q', str(repo)], check=True)
    (repo / 'scripts').mkdir()
    checker = repo / 'scripts' / 'privacy-check.py'
    shutil.copyfile(root / 'scripts' / checker.name, checker)
    shutil.copyfile(root / '.gitignore', repo / '.gitignore')
    files = ['Personal.openroom', 'Archive.OPENROOM', 'Personal.openroom-wal', 'catalog.db-journal',
             'archive.SQLITE-journal', 'library.sqlite3-journal', 'renamed.bin']
    for name in files[:-1]:
        (repo / name).write_bytes(b'test-only')
        assert subprocess.run(['git', '-c', 'core.ignoreCase=false', 'check-ignore', '-q', '--', name], cwd=repo).returncode == 0, name
    db = sqlite3.connect(repo / files[-1])
    db.execute('CREATE TABLE photos (id INTEGER)')
    db.close()
    safe = repo / 'catalog-notes.txt'
    safe.write_text('SQLite format 3 is documented here. A catalog.db-journal is temporary.')
    subprocess.run(['git', 'add', '--', safe.name], cwd=repo, check=True)
    subprocess.run([sys.executable, str(checker), '--tracked'], cwd=repo, check=True)
    subprocess.run(['git', 'add', '-f', '--', *files], cwd=repo, check=True)
    for args in [['--tracked'], files]:
        result = subprocess.run([sys.executable, str(checker), *args], cwd=repo, capture_output=True, text=True)
        assert result.returncode == 1, result
        for name in files:
            assert name + ':' in result.stdout, name
        assert 'test-only' not in result.stdout
    print('Forced catalog additions, rollback journals and renamed databases detected')

if (root / '.git').exists():
    for path in ['private/history.bundle', 'reports/local-audit.html', 'reports/tools/read.py',
                 'holiday.NEF', 'holiday.RW2', 'holiday.JPG', 'holiday.png', 'edit.xmp',
                 'personal.openroom', '.env', 'photos/camera.heic']:
        assert subprocess.run(['git', 'check-ignore', '--no-index', '-q', '--', path], cwd=root).returncode == 0, path
    for path in ['reports/raw-heif-samples.json', 'desktop/icons/icon.png',
                 'desktop/ui/public/branding/openroom-icon.png',
                 'crates/media-core/tests/fixtures/layers.psd',
                 'crates/media-core/tests/fixtures/layers-psd.png']:
        assert subprocess.run(['git', 'check-ignore', '--no-index', '-q', '--', path], cwd=root).returncode == 1, path
    print('Distribution exclusions and required fixtures verified')
