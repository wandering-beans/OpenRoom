#!/usr/bin/env python3
"""Compare local point-curve TIFF exports after checking LRC's embedded settings.

Usage: python compare-lightroom-curves.py FIXTURE_ROOT OUTPUT.json
FIXTURE_ROOT contains cases.json, input/, reference/CASE/, before/CASE/, after/CASE/.
"""
from pathlib import Path
import hashlib
import json
import re
import sys
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image

CURVES = ['ToneCurvePV2012' + suffix for suffix in ['', 'Red', 'Green', 'Blue']]
CRS = '{http://ns.adobe.com/camera-raw-settings/1.0/}'
RDF = '{http://www.w3.org/1999/02/22-rdf-syntax-ns#}'


def reference_pixels(path, case):
    data = path.read_bytes()
    xmp = re.search(rb'<x:xmpmeta.*?</x:xmpmeta>', data, re.S)
    assert xmp, f'Missing XMP: {path}'
    root = ET.fromstring(xmp[0])
    description = root.find('.//' + RDF + 'Description')
    expected = {'ProcessVersion': 15.4, 'Exposure2012': 0, 'Contrast2012': 0,
                'IncrementalTemperature': 0, 'IncrementalTint': 0,
                'Highlights2012': 0, 'Shadows2012': 0, 'Whites2012': 0, 'Blacks2012': 0,
                'Sharpness': 0, 'Saturation': 0, 'Vibrance': 0}
    if 'ToneCurvePV2012' in case:
        expected['CurveRefineSaturation'] = 100
    for key, value in expected.items():
        assert float(description.attrib[CRS + key]) == value, (path, key, value)
    assert description.attrib[CRS + 'CameraProfile'] == 'Embedded'
    for key in CURVES:
        curve = description.find(CRS + key)
        values = [int(n.strip()) for point in curve.iter(RDF + 'li') for n in point.text.split(',')]
        assert values == case.get(key, [0, 0, 255, 255]), (path, key, values)
    with Image.open(path) as image:
        tags = image.tag_v2
        assert image.info.get('icc_profile'), f'Missing output ICC: {path}'
        assert tags[259] == 1 and tags[258] == (16, 16, 16) and tags.get(284, 1) == 1
        strips = b''.join(data[o:o + n] for o, n in zip(tags[273], tags[279]))
        width, height = image.size
    dtype = '<u2' if data[:2] == b'II' else '>u2'
    return np.frombuffer(strips, dtype=dtype).reshape(height, width, 3).astype(np.float64) / 257


def compare(root):
    cases = json.loads((root / 'cases.json').read_text())
    rows = []
    for name, case in cases.items():
        references = sorted((root / 'reference' / name).glob('*.tif'))
        assert references, f'No references: {name}'
        for path in references:
            target = reference_pixels(path, case)
            row = {'case': name, 'photo': path.stem, 'dimensions': list(target.shape[1::-1])}
            for side in ['before', 'after']:
                with Image.open(root / side / name / path.name) as image:
                    assert image.info.get('icc_profile'), (side, path)
                    actual = np.asarray(image.convert('RGB'), dtype=np.float64)
                assert actual.shape == target.shape, (side, path, actual.shape, target.shape)
                difference = np.abs(actual - target)
                row[side] = {'mean': float(difference.mean()), 'p95': float(np.percentile(difference, 95))}
            rows.append(row)
    def aggregate(selected):
        return {side: float(np.mean([r[side]['mean'] for r in selected])) for side in ['before', 'after']}
    photos = [row for row in rows if row['photo'].startswith('IMGP')]
    return {'metric': 'Mean absolute sRGB channel error, 0..255; not perceptual Delta E. LRC 16-bit vs OpenRoom 8-bit output.',
            'reference': 'LRC 15.6 / PV 15.4; embedded sRGB; curve refinement 100; long edge 1024; no output sharpening.',
            'pairs': rows, 'photos': aggregate(photos),
            'photos_by_case': {name: aggregate([r for r in photos if r['case'] == name]) for name in cases},
            'synthetic': aggregate([r for r in rows if not r['photo'].startswith('IMGP')]),
            'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((root / 'input').iterdir())}}


if __name__ == '__main__':
    assert len(sys.argv) == 3, __doc__
    result = compare(Path(sys.argv[1]))
    Path(sys.argv[2]).write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: result[key] for key in ['photos', 'photos_by_case', 'synthetic']}, indent=2))
