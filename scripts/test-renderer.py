"""Render real fixtures; pass a compiled renderer and a fixture directory."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

renderer, folder = sys.argv[1], Path(sys.argv[2])
files = [folder / name for name in ['Leica-M8.dng', '風景-00.png', '風景-01.jpeg']]
results = []
with tempfile.TemporaryDirectory() as tmp:
    for source in files:
        original = hashlib.sha256(source.read_bytes()).hexdigest()
        for bound in (384, 4096):
            output = Path(tmp) / 'render.png'
            start = time.perf_counter()
            result = subprocess.run([renderer, str(source), str(output), str(bound)], capture_output=True, text=True, timeout=60)
            assert result.returncode == 0, (source, result.stderr)
            metadata = json.loads(result.stdout)
            assert output.read_bytes().startswith(b'\x89PNG\r\n\x1a\n')
            assert 0 < max(metadata['render_width'], metadata['render_height']) <= bound
            results.append({'file': source.name, 'bound': bound, 'ms': round((time.perf_counter()-start)*1000, 3), **metadata})
        assert hashlib.sha256(source.read_bytes()).hexdigest() == original
    broken = Path(tmp) / 'broken.dng'; broken.write_bytes(b'broken')
    assert subprocess.run([renderer, str(broken), output, '384'], capture_output=True).returncode != 0
print(json.dumps({'renders': results, 'checks': ['three photo formats', 'corrupt DNG', 'source bytes unchanged']}, ensure_ascii=False, indent=2))
