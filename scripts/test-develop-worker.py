#!/usr/bin/env python3
"""Exercise the shipped Rust workers against public RAW/HEIF and generated raster inputs."""
import argparse
import array
import math
import os
import sys
import zlib
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tiff_tags(path):
    with path.open('rb') as stream:
        header = stream.read(8)
        endian = '<' if header[:2] == b'II' else '>'
        assert struct.unpack_from(endian + 'H', header, 2)[0] == 42
        stream.seek(struct.unpack_from(endian + 'I', header, 4)[0])
        count = struct.unpack(endian + 'H', stream.read(2))[0]
        return {tag: (kind, length, value) for tag, kind, length, value in
                (struct.unpack(endian + 'HHII', stream.read(12)) for _ in range(count))}


def float_cache_difference(reference, actual):
    if digest(reference) == digest(actual):
        return 0.0
    def samples(path):
        tags = tiff_tags(path)
        assert tags[259][2] == 1, 'Expected uncompressed TIFF'
        data = path.read_bytes()
        endian = '<' if data[:2] == b'II' else '>'
        def values(tag):
            kind, count, value = tags[tag]
            assert kind in (3, 4)
            code, size = ('H', 2) if kind == 3 else ('I', 4)
            packed = struct.pack(endian + 'I', value) if count * size <= 4 else data[value:value + count * size]
            return struct.unpack_from(endian + code * count, packed)
        assert all(v == 32 for v in values(258)) and all(v == 3 for v in values(339))
        pixels = array.array('f')
        for offset, count in zip(values(273), values(279)):
            pixels.frombytes(data[offset:offset + count])
        if (endian == '<') != (sys.byteorder == 'little'):
            pixels.byteswap()
        return pixels
    expected, found = samples(reference), samples(actual)
    assert len(expected) == len(found)
    assert all(math.isfinite(v) for v in found)
    return max(abs(a - b) for a, b in zip(expected, found))


def export_content(path, format_name):
    data = bytearray(path.read_bytes())
    if format_name == 'png':
        # ICC profiles carry their creation time; compare encoded pixels separately.
        output = data[:8]
        offset = 8
        while offset < len(data):
            length = struct.unpack_from('>I', data, offset)[0]
            end = offset + length + 12
            if data[offset + 4:offset + 8] == b'iCCP':
                chunk = data[offset + 8:end - 4]
                name_end = chunk.index(0)
                assert chunk[name_end + 1] == 0
                profile = bytearray(zlib.decompress(chunk[name_end + 2:]))
                profile[24:36] = bytes(12)
                output.extend(b'iCCP' + profile)
            else:
                output.extend(data[offset:end])
            offset = end
        return output
    profile = data.index(b'acsp') - 36
    data[profile + 24:profile + 36] = bytes(12)
    return data


def run(binary, *args):
    result = subprocess.run([str(binary), *map(str, args)], capture_output=True, timeout=90,
                            env={**os.environ, 'OMP_NUM_THREADS': '4', 'OMP_MAX_ACTIVE_LEVELS': '1'})
    assert result.returncode == 0, result.stderr.decode(errors='replace')
    return json.loads(result.stdout)


def verify_exports(binary, source, directory, reference_binary):
    original = digest(source)
    recipe = directory / 'recipe.json'
    recipe.write_text(json.dumps({
        'version': 1, 'exposure': 0.5, 'contrast': 12, 'crop': [0.1, 0.1, 0.8, 0.8],
        'point_curves': [[[0, 0], [0.4, 0.6], [1, 1]], [], [], []],
        'sharpen': 40, 'noise_luma': 15, 'texture': 15, 'clarity': 10,
    }))
    for format_name, extension in [('jpeg', 'jpg'), ('png', 'png'), ('tiff', 'tif')]:
        destination = directory / f'output.{extension}'
        run(binary, '--develop-export', source, destination, recipe, format_name)
        assert destination.stat().st_size > 500
        if reference_binary:
            reference = directory / f'reference.{extension}'
            run(reference_binary, '--develop-export', source, reference, recipe, format_name)
            assert export_content(destination, format_name) == export_content(reference, format_name), f'Export changed: {format_name}'
        before = digest(destination)
        failed = subprocess.run([str(binary), '--develop-export', str(source), str(destination), str(recipe), format_name], capture_output=True)
        assert failed.returncode != 0 and digest(destination) == before, 'Existing output was overwritten'
        if format_name == 'jpeg': assert b'ICC_PROFILE' in destination.read_bytes()
        if format_name == 'png':
            data = destination.read_bytes()
            assert data[24] == 16 and b'iCCP' in data, 'PNG must be 16 bit with ICC'
        if format_name == 'tiff': assert 34675 in tiff_tags(destination), 'TIFF must contain ICC'
    assert digest(source) == original, 'Export modified its source'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('assets', type=Path)
    parser.add_argument('--reference-binary', type=Path, help='Compare float caches and exported files with an earlier build')
    parser.add_argument("--all-exports", action="store_true", help="Check every input through JPEG, 16-bit PNG and TIFF export")
    args = parser.parse_args()
    results = []
    with tempfile.TemporaryDirectory(prefix='photo-develop-') as temporary:
        directory = Path(temporary)
        sources = sorted(path for path in args.assets.iterdir() if path.suffix.lower() in
                         ['.dng', '.cr2', '.cr3', '.crw', '.arw', '.sr2', '.srf', '.raf', '.nef', '.nrw', '.rw2', '.orf', '.pef', '.rwl', '.srw', '.3fr', '.fff', '.iiq', '.heic', '.heif', '.hif', '.jpg', '.jpeg', '.png', '.tif', '.tiff', '.avif', '.psd', '.psb'])
        assert sources, 'No test photos found'
        for index, source in enumerate(sources):
            original = digest(source)
            metadata = run(args.binary, "--photo-metadata", source)
            if source.suffix.lower() not in {".heic", ".heif", ".hif", ".jpg", ".jpeg", ".png", ".tif", ".tiff", ".avif", ".psd", ".psb"}:
                assert metadata.get("メーカー") and metadata.get("カメラ"), (source.name, metadata)
            base = directory / f'base-{index}.tif'
            started = time.perf_counter()
            info = run(args.binary, '--develop-base', source, base)
            elapsed = (time.perf_counter() - started) * 1000
            assert max(info['width'], info['height']) <= 4096
            assert info['source_width'] > 0 and info['source_height'] > 0
            if source.suffix.lower() in ['.dng', '.cr2', '.cr3', '.crw', '.arw', '.sr2', '.srf', '.raf', '.nef', '.nrw', '.rw2', '.orf', '.pef', '.rwl', '.srw', '.3fr', '.fff', '.iiq']:
                assert info['source_kind'] in ['raw-camera-linear', 'raw-sensor-linear', 'raw-rtengine-linear'], info
            tags = tiff_tags(base)
            assert 339 in tags, 'Working cache must contain floating point samples'
            assert digest(source) == original, 'Source was modified'
            results.append({'extension': source.suffix.lower(), 'source': [info['source_width'], info['source_height']],
                            'preview': [info['width'], info['height']], 'kind': info['source_kind'], 'cold_base_ms': round(elapsed, 2)})
            if args.reference_binary:
                reference = directory / 'reference.tif'
                reference_info = run(args.reference_binary, '--develop-base', source, reference)
                assert reference_info == info, 'Development dimensions or color interpretation changed'
                difference = float_cache_difference(reference, base)
                # Native parallel demosaicing can differ by a few float32 rounding bits.
                assert difference <= 1e-5, (source.name, difference)
                results[-1]['reference_max_float_difference'] = difference
                reference.unlink()
            base.unlink()
        export_sources = sources if args.all_exports else [next((path for path in sources if path.suffix.lower() == '.cr2'), sources[0])]
        for index, source in enumerate(export_sources):
            output_directory = directory / f'exports-{index}'
            output_directory.mkdir()
            verify_exports(args.binary, source, output_directory, args.reference_binary)
    print(json.dumps({'inputs': results, 'exports': ['JPEG with ICC', 'PNG 16 bit with ICC', 'TIFF 16 bit with ICC'],
                      'reference_float_tolerance': 1e-5 if args.reference_binary else None,
                      'reference_export_equality': bool(args.reference_binary),
                      'checks': ['sensor RAW development', '4096px linear float cache', 'unchanged source hashes', 'existing output refusal']}, indent=2))


if __name__ == '__main__':
    main()
