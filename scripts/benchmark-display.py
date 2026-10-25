#!/usr/bin/env python3
"""Measure packaged photo workers. Usage: benchmark-display.py APP_EXECUTABLE RAW_FIXTURE_DIR.
JSON includes format/timing/dimensions only, never EXIF values or source paths.
Each render uses a fresh destination; OS filesystem caches are not flushed.
"""
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time

exe = str(Path(sys.argv[1]).resolve())
folder = Path(sys.argv[2])
results = []
with tempfile.TemporaryDirectory() as directory:
    for source in sorted(folder.iterdir()):
        extension = source.suffix.lower().lstrip('.')
        if extension not in {'dng', 'cr2', 'cr3', 'crw', 'arw', 'sr2', 'srf', 'raf', 'nef', 'nrw', 'rw2', 'orf', 'pef', 'rwl', 'srw', '3fr', 'fff', 'iiq', 'heic', 'heif', 'hif'}:
            continue
        before = hashlib.sha256(source.read_bytes()).digest()
        for bound in (384, 1600, 4096):
            timings = []
            for iteration in range(3):
                output = Path(directory) / 'preview.png'
                output.unlink(missing_ok=True)
                started = time.perf_counter()
                run = subprocess.run([exe, '--render-media', str(source), str(output), str(bound)], capture_output=True, text=True, timeout=65)
                assert run.returncode == 0, (extension, run.stderr)
                timings.append(round((time.perf_counter() - started) * 1000, 3))
                info = json.loads(run.stdout)
                assert 0 < max(info['render_width'], info['render_height']) <= bound
                assert output.read_bytes().startswith(b'\x89PNG\r\n\x1a\n')
            results.append(dict(extension=extension, bound=bound, timings_ms=timings,
                median_ms=statistics.median(timings), source_kind=info['source_kind'],
                render_width=info['render_width'], render_height=info['render_height']))
        assert hashlib.sha256(source.read_bytes()).digest() == before
assert results, 'No photo fixtures found'
print(json.dumps(results, indent=2))
