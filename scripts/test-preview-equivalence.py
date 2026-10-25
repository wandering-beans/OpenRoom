#!/usr/bin/env python3
"""Compare resident preview pixels before/after an optimization, including cache invalidation."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess


@contextmanager
def worker(binary):
    process = subprocess.Popen(
        [str(binary.resolve()), '--rtengine-resident'], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        env={**os.environ, 'OMP_NUM_THREADS': '4', 'OMP_MAX_ACTIVE_LEVELS': '1'},
    )
    try:
        yield process
    finally:
        process.terminate()
        process.wait(timeout=10)


def timeout(_signal, _frame):
    raise TimeoutError('Resident preview exceeded 120 seconds')


def render(process, source, bound, settings):
    signal.alarm(120)
    try:
        request = {'source': str(source.resolve()), 'bound': bound, 'settings': settings, 'bytes': 0}
        process.stdin.write(json.dumps(request).encode() + b'\n')
        process.stdin.flush()
        header = json.loads(process.stdout.readline())
        assert not header.get('error'), header
        info = header['info']
        assert 0 < info['width'] * info['height'] <= 64000000
        assert header['bytes'] == info['width'] * info['height'] * 4
        pixels = process.stdout.read(header['bytes'])
        assert len(pixels) == header['bytes'], 'Truncated resident preview'
        return info, pixels
    finally:
        signal.alarm(0)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('photos', type=Path, nargs='+')
    args = parser.parse_args()
    signal.signal(signal.SIGALRM, timeout)
    detail = {'version': 1, 'exposure': .5, 'contrast': 12, 'highlights': -25,
              'shadows': 20, 'vibrance': 15, 'sharpen': 40, 'noise_luma': 15,
              'noise_color': 10, 'texture': 15, 'clarity': 10}
    recipes = [
        {'version': 1}, detail, {**detail, 'exposure': -.25},
        {**detail, 'sharpen': 65}, {**detail, 'sharpen_radius': 2, 'sharpen_masking': 35},
        {**detail, 'noise_luma': 25}, {**detail, 'noise_color': 25},
        {**detail, 'raw_temperature': 5500, 'raw_tint': 15},
        {**detail, 'sharpen': 0}, {'version': 1}, detail,
        {**detail, 'crop': [.1, .1, .8, .8], 'angle': 3},
    ]
    with worker(args.before) as before, worker(args.after) as after:
        for source in args.photos:
            original = hashlib.sha256(source.read_bytes()).digest()
            for bound in [1600, 0]:
                changed_samples = 0
                maximum_difference = 0
                for index, recipe in enumerate(recipes):
                    expected_info, expected = render(before, source, bound, recipe)
                    actual_info, actual = render(after, source, bound, recipe)
                    for key in ['width', 'height']:
                        assert expected_info[key] == actual_info[key], (source.name, bound, index, key)
                    if expected != actual:
                        differences = [abs(a - b) for a, b in zip(expected, actual) if a != b]
                        # The reference engine itself varies at rounding boundaries with denoising.
                        assert recipe.get('noise_luma', 0) > 0
                        assert max(differences) <= 1, (source.name, bound, index, max(differences))
                        assert len(differences) <= len(expected) / 100000, (source.name, bound, index, len(differences))
                        changed_samples += len(differences)
                        maximum_difference = max(maximum_difference, max(differences))
                    else:
                        for key in ['histogram', 'clipped_shadows', 'clipped_highlights']:
                            assert expected_info[key] == actual_info[key], (source.name, bound, index, key)
                print(json.dumps({'source': source.name, 'bound': bound, 'recipes': len(recipes),
                                  'changed_channel_samples': changed_samples,
                                  'maximum_8bit_difference': maximum_difference}), flush=True)
            assert hashlib.sha256(source.read_bytes()).digest() == original, 'Source changed'


if __name__ == '__main__':
    main()
