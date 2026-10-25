#!/usr/bin/env python3
"""Pack local LRC 16-bit sRGB ramp exports. Requires Pillow and numpy, never photos.

Usage: python build-raster-calibration.py EXPORT_ROOT OUTPUT.bin
Layout: wb-fine/wb_{temperature}_{tint}/ramp.tif and ramps/{control}_{value}/ramp.tif.
The source is a 1024 x 32, 16-bit sRGB neutral ramp; export without resizing/sharpening.
"""
from pathlib import Path
import re
import sys

import numpy as np
from PIL import Image

KNOTS = 129
AXIS = np.linspace(0, 1, 1024)
SAMPLES = np.linspace(0, 1, KNOTS)
# D50-adapted linear sRGB and ProPhoto RGB colorants (ICC matrix/TRC model).
SRGB_XYZ = np.array([[.4360747, .3850649, .1430804],
                     [.2225045, .7168786, .0606169], [.0139322, .0971045, .7141733]])
PROPHOTO_XYZ = np.array([[.7976749, .1351917, .0313534],
                        [.2880402, .7118741, .0000857], [0., 0., .8252100]])
TO_WIDE = np.linalg.solve(PROPHOTO_XYZ, SRGB_XYZ)


def decode(values):
    return np.where(values <= .04045, values / 12.92, ((values + .055) / 1.055) ** 2.4)


def encode(values):
    return np.where(values <= .0031308, values * 12.92,
                    1.055 * np.maximum(values, 0) ** (1 / 2.4) - .055)


def read_ramp(path, settings):
    data = path.read_bytes()
    for key, expected in {"ProcessVersion": 15.4, **settings}.items():
        match = re.search(rb'crs:' + key.encode() + rb'="([^"]*)"', data)
        assert match and float(match[1]) == expected, (path, key, expected)
    with Image.open(path) as image:
        tags = image.tag_v2
        assert image.size == (1024, 32), path
        assert tags[259] == 1 and tags[258] == (16, 16, 16) and tags.get(284, 1) == 1, path
        strips = b''.join(data[offset:offset + count] for offset, count in zip(tags[273], tags[279]))
    dtype = '<u2' if data[:2] == b'II' else '>u2'
    rgb = np.frombuffer(strips, dtype=dtype).reshape(32, 1024, 3)[16] / 65535
    wide = np.clip(encode(decode(rgb) @ TO_WIDE.T), 0, 1)
    return np.stack([np.interp(SAMPLES, AXIS, wide[:, channel]) for channel in range(3)])


def build(root):
    curves = []
    for temperature in range(-100, 101, 10):
        for tint in range(-100, 101, 10):
            curve = read_ramp(root / f'wb-fine/wb_{temperature}_{tint}/ramp.tif', {
                'IncrementalTemperature': temperature, 'IncrementalTint': tint,
                'Exposure2012': 0, 'Contrast2012': 0,
            })
            curves.extend(curve if temperature or tint else [SAMPLES] * 3)
    for control, xmp, values in [('exposure', 'Exposure2012', range(-5, 6)),
                                 ('contrast', 'Contrast2012', range(-100, 101, 25))]:
        for value in values:
            settings = {'IncrementalTemperature': 0, 'IncrementalTint': 0,
                        'Exposure2012': 0, 'Contrast2012': 0, xmp: value}
            curve = read_ramp(root / f'ramps/{control}_{value}/ramp.tif', settings)
            curves.append(curve.mean(axis=0) if value else SAMPLES)
    packed = np.round(np.array(curves) * 65535).astype('<u2')
    assert packed.shape == (21 * 21 * 3 + 11 + 9, KNOTS)
    # Extreme WB can move a channel backwards during gamut clipping; tone ramps cannot.
    assert np.all(np.diff(packed[-20:].astype(int), axis=1) >= -2), 'Non-monotonic tone ramp'
    return packed.tobytes()


if __name__ == '__main__':
    assert len(sys.argv) == 3, __doc__
    output = build(Path(sys.argv[1]))
    Path(sys.argv[2]).write_bytes(output)
    print(f'Wrote {len(output)} bytes of measured curves')
