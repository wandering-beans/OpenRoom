#!/usr/bin/env python3
"""Reject false RAW highlight color using a generated, neutral Bayer exposure ramp."""
import array
import hashlib
import math
import signal
from pathlib import Path
import struct
import sys
import tempfile

from importlib import import_module

worker = import_module("test-develop-worker")
resident = import_module("test-preview-equivalence")


def neutral_dng(path):
    width, height = 256, 128
    entries = []

    def tag(code, kind, values):
        if kind == 2:
            data = values.encode() + b'\0'
            count = len(data)
        elif kind in (5, 10):
            pair = '<II' if kind == 5 else '<ii'
            data = b''.join(struct.pack(pair, round(value * 1000000), 1000000) for value in values)
            count = len(values)
        else:
            data = struct.pack('<' + {1: 'B', 3: 'H', 4: 'I'}[kind] * len(values), *values)
            count = len(values)
        entries.append((code, kind, count, data))

    for code, value in [(256, width), (257, height), (273, 0), (278, height), (279, width * height * 2)]:
        tag(code, 4, [value])
    for code, value in [(258, 16), (259, 1), (262, 32803), (277, 1), (50714, 0), (50717, 4095), (50778, 21)]:
        tag(code, 3, [value])
    tag(271, 2, 'OpenRoom')
    tag(272, 2, 'Synthetic Bayer')
    tag(50708, 2, 'OpenRoom Synthetic Bayer')
    tag(33421, 3, [2, 2])
    tag(33422, 1, [0, 1, 1, 2])
    tag(50706, 1, [1, 4, 0, 0])
    tag(50707, 1, [1, 1, 0, 0])
    tag(50728, 5, [.5, 1, .65])
    # XYZ-to-sRGB matrix scaled by sensor sensitivity; neutral samples clip G, then B, then R.
    tag(50721, 10, [1.620227, -.768569, -.249266, -.969266, 1.876011, .041556, .03617, -.132616, .687696])
    entries.sort()
    data_offset = 8 + 2 + 12 * len(entries) + 4
    extra = bytearray()
    directory = bytearray(struct.pack('<H', len(entries)))
    for code, kind, count, data in entries:
        if len(data) > 4:
            packed = struct.pack('<I', data_offset + len(extra))
            extra.extend(data)
        else:
            packed = data.ljust(4, b'\0')
        directory.extend(struct.pack('<HHI', code, kind, count) + packed)
    for index, entry in enumerate(entries):
        if entry[0] == 273:
            struct.pack_into('<I', directory, 2 + 12 * index + 8, data_offset + len(extra))
    pixels = bytearray()
    for y in range(height):
        for x in range(width):
            sensitivity = [[.5, 1], [1, .65]][y % 2][x % 2]
            exposure = .1 + 3.9 * (x // 2) / (width // 2 - 1)
            pixels.extend(struct.pack('<H', min(4095, round(exposure * sensitivity * 4095))))
    path.write_bytes(b'II*\0' + struct.pack('<I', 8) + directory + bytes(4) + extra + pixels)


def float_pixels(path):
    data = path.read_bytes()
    endian = '<' if data[:2] == b'II' else '>'
    tags = worker.tiff_tags(path)

    def values(tag):
        kind, count, value = tags[tag]
        code, size = {3: ('H', 2), 4: ('I', 4)}[kind]
        packed = struct.pack(endian + 'I', value) if count * size <= 4 else data[value:value + count * size]
        return struct.unpack_from(endian + code * count, packed)

    assert values(259) == (1,) and values(277) == (4,)
    assert all(value == 32 for value in values(258)) and all(value == 3 for value in values(339))
    samples = array.array('f')
    for offset, count in zip(values(273), values(279)):
        samples.frombytes(data[offset:offset + count])
    if (endian == '<') != (sys.byteorder == 'little'):
        samples.byteswap()
    return samples


def main():
    binary = Path(sys.argv[1]).resolve()
    signal.signal(signal.SIGALRM, resident.timeout)
    with tempfile.TemporaryDirectory(prefix='highlight-regression-') as temporary:
        directory = Path(temporary)
        source = directory / 'neutral.dng'
        neutral_dng(source)
        original = hashlib.sha256(source.read_bytes()).digest()
        for bound in [384, 1600, 0]:
            output = directory / f'{bound}.tif'
            info = worker.run(binary, '--develop-base', source, output, bound)
            pixels = float_pixels(output)
            assert all(math.isfinite(value) for value in pixels)
            width, height = info['width'], info['height']
            for x in [width // 5, width // 2, width * 4 // 5]:
                index = ((height // 2) * width + x) * 4
                rgb = pixels[index:index + 3]
                assert max(rgb) - min(rgb) < .02, (bound, x, list(rgb), 'neutral highlight acquired false color')
            assert max(pixels) > 1.1, 'highlight repair must retain floating-point headroom'
        with resident.worker(binary) as process:
            for bound in [384, 1600, 0]:
                for settings in [{}, {'exposure': -2, 'highlights': -80},
                                 {'exposure': -2, 'sharpen': 40, 'noise_luma': 15}]:
                    info, pixels = resident.render(process, source, bound, settings)
                    for x in [info['width'] // 5, info['width'] // 2, info['width'] * 4 // 5]:
                        index = (info['height'] // 2 * info['width'] + x) * 4
                        rgb = pixels[index:index + 3]
                        assert max(rgb) - min(rgb) <= 3, (bound, settings, list(rgb), 'display tone added false color')
        assert hashlib.sha256(source.read_bytes()).digest() == original
    print('Neutral RAW highlights: no false color at 384px, 1600px and full size; headroom and source retained.')


if __name__ == '__main__':
    main()
