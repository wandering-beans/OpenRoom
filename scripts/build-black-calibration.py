#!/usr/bin/env python3
"""Pack three LRC PV15.4 sRGB black-point ramps; photos are never calibration data.

Usage: build-black-calibration.py REFERENCE_ROOT OUTPUT.bin
ROOT contains neutral/ramp.tif and blacks_{50,100}/ramp.tif.
Input is the 1024 x 32 neutral 16-bit sRGB ramp used by build-raster-calibration.py.
"""
from pathlib import Path
import re
import sys

import numpy as np
from PIL import Image


def build(root):
    curves = []
    for value in [0, 50, 100]:
        path = root / ('neutral' if value == 0 else f'blacks_{value}') / 'ramp.tif'
        data = path.read_bytes()
        for key, expected in {'ProcessVersion': 15.4, 'Exposure2012': 0, 'Contrast2012': 0,
                              'Highlights2012': 0, 'Shadows2012': 0, 'Whites2012': 0,
                              'Blacks2012': value, 'Sharpness': 0, 'LuminanceSmoothing': 0,
                              'ColorNoiseReduction': 0}.items():
            match = re.search(rb'crs:' + key.encode() + rb'="([^"]*)"', data)
            assert match and float(match[1]) == expected, (path, key, expected)
        with Image.open(path) as image:
            tags = image.tag_v2
            assert image.size == (1024, 32) and tags[258] == (16, 16, 16) and tags[259] == 1
            assert image.info.get('icc_profile') and tags.get(284, 1) == 1
            strips = b''.join(data[o:o+c] for o,c in zip(tags[273], tags[279]))
        rgb = np.frombuffer(strips, dtype='<u2' if data[:2] == b'II' else '>u2').reshape(32,1024,3)[16] / 65535.
        assert np.max(np.ptp(rgb, axis=1)) < 32 / 65535., path
        curve = np.interp(np.linspace(0,1,129), np.linspace(0,1,1024), rgb.mean(axis=1))
        curve[0], curve[-1] = 0., 1.
        assert (np.diff(curve) >= 0).all(), path
        curves.append(np.linspace(0,1,129) if value == 0 else curve)
    return np.array(curves, dtype='<f4').tobytes()


if __name__ == '__main__':
    assert len(sys.argv) == 3, __doc__
    Path(sys.argv[2]).write_bytes(build(Path(sys.argv[1])))
