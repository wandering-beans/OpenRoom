#!/usr/bin/env python3
"""Pack a neutral linear-DNG ramp exported by LRC (not a photograph).

Usage: python scripts/build-raw-calibration.py RAMP.tif OUTPUT.bin
Input: 1024 x 256, 16-bit uncompressed sRGB, no resizing/sharpening,
PV 15.4, Embedded profile, WB 5000/10, linear point curve, basic controls 0.
The source DNG has three identical linear 0..65535 channels, a ProPhoto D50
ColorMatrix, neutral [1,1,1], black 0, white 65535 and baseline exposure 0.
"""
from pathlib import Path
import re
import sys

import numpy as np
from PIL import Image


def build(path):
    data = path.read_bytes()
    expected = {'ProcessVersion': 15.4, 'Temperature': 5000, 'Tint': 10,
                'Exposure2012': 0, 'Contrast2012': 0, 'Highlights2012': 0,
                'Shadows2012': 0, 'Whites2012': 0, 'Blacks2012': 0,
                'Sharpness': 0, 'LuminanceSmoothing': 0, 'ColorNoiseReduction': 0}
    for key, value in expected.items():
        match = re.search(rb'crs:' + key.encode() + rb'="([^"]*)"', data)
        assert match and float(match[1]) == value, (key, value)
    with Image.open(path) as image:
        tags = image.tag_v2
        assert image.size == (1024, 256)
        assert tags[259] == 1 and tags[258] == (16, 16, 16) and tags.get(284, 1) == 1
        strips = b''.join(data[offset:offset + count] for offset, count in zip(tags[273], tags[279]))
    dtype = '<u2' if data[:2] == b'II' else '>u2'
    rgb = np.frombuffer(strips, dtype=dtype).reshape(256, 1024, 3)[128] / 65535
    linear = np.where(rgb <= .04045, rgb / 12.92, ((rgb + .055) / 1.055) ** 2.4)
    curve = np.interp(np.linspace(0, 1, 4097), np.linspace(0, 1, 1024), linear.mean(axis=1))
    curve[0], curve[-1] = 0, 1
    assert np.isfinite(curve).all() and (np.diff(curve) >= 0).all()
    return curve.astype('<f4').tobytes()


if __name__ == '__main__':
    assert len(sys.argv) == 3, __doc__
    output = build(Path(sys.argv[1]))
    Path(sys.argv[2]).write_bytes(output)
    print(f'Wrote {len(output)} bytes of measured RAW tone data')
