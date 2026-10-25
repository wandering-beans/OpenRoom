#!/usr/bin/env python3
"""Run the 30k search benchmark while three packaged workers render 2MP assets.
Usage: script APP_EXECUTABLE DESKTOP_TEST_BINARY AE_FORMAT_ASSETS
"""
import concurrent.futures, pathlib, subprocess, sys, tempfile, threading, time
exe, test, assets = (pathlib.Path(p).resolve() for p in sys.argv[1:4])
stop = threading.Event()

def render_loop(index, directory):
    source = assets / ('large.' + 'tiff')
    destination = pathlib.Path(directory) / f'{index}.png'
    count = 0
    while not stop.is_set():
        result = subprocess.run([str(exe), '--render-media', str(source), str(destination), '384'],
                                capture_output=True, timeout=65)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors='replace'))
        destination.unlink()
        count += 1
    return count

with tempfile.TemporaryDirectory() as directory, concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
    started = time.perf_counter()
    futures = [pool.submit(render_loop, i, directory) for i in range(3)]
    try:
        result = subprocess.run([str(test), 'desktop_queries_at_30000', '--ignored', '--nocapture'],
                                capture_output=True, text=True, timeout=180)
        print(result.stdout, end='')
        print(result.stderr, end='', file=sys.stderr)
        assert result.returncode == 0, 'Search performance target failed under rendering load'
    finally:
        stop.set()
    counts = [future.result() for future in futures]
    print(f'Background renders: {counts}; elapsed_s={time.perf_counter()-started:.2f}')
