#!/usr/bin/env python3
"""Compare full-size local JPEG exports with checked LRC settings, without uploading photos.

Usage: python compare-lightroom.py FIXTURE_ROOT OUTPUT.json
FIXTURE_ROOT contains cases.json, input/, reference/CASE/, before/CASE/ and after/CASE/.
"""
from pathlib import Path
import hashlib
import json
import re
import sys

import numpy as np
from PIL import Image


def pixels(path):
    with Image.open(path) as image:
        assert image.info.get('icc_profile'), f'Missing output ICC: {path}'
        return np.array(image.convert('RGB'), dtype=np.float32)


def compare(root):
    cases = json.loads((root / 'cases.json').read_text())
    results = []
    for case, settings in cases.items():
        for reference in sorted((root / 'reference' / case).glob('IMGP*.jpg')):
            data = reference.read_bytes()
            expected = {'IncrementalTemperature': settings.get('temperature', 0),
                        'IncrementalTint': settings.get('tint', 0),
                        'Exposure2012': settings.get('exposure', 0),
                        'Contrast2012': settings.get('contrast', 0),
                        'ProcessVersion': 15.4, 'Sharpness': 0,
                        'Highlights2012': 0, 'Shadows2012': 0, 'Whites2012': 0, 'Blacks2012': 0}
            for key, value in expected.items():
                found = re.search(rb'crs:' + key.encode() + rb'="([^"]*)"', data)
                assert found and float(found[1]) == value, (reference, key, value)
            target = pixels(reference)
            record = {'case': case, 'photo': reference.stem, 'settings': settings,
                      'dimensions': [target.shape[1], target.shape[0]]}
            for label in ['before', 'after']:
                image = pixels(root / label / case / reference.name)
                assert image.shape == target.shape, (reference, label)
                record[label] = float(np.abs(image - target).mean(dtype=np.float64))
            results.append(record)
    assert results, 'No reference images'
    before = float(np.mean([row['before'] for row in results]))
    after = float(np.mean([row['after'] for row in results]))
    by_case = {}
    for case in cases:
        rows = [r for r in results if r['case'] == case]
        if rows:
            by_case[case] = {'count': len(rows), **{
                label: float(np.mean([r[label] for r in rows])) for label in ['before', 'after']}}
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
              for p in sorted((root / 'input').glob('IMGP*.JPG'))}
    return {'metric': 'Mean absolute sRGB channel error, 0..255; full-size, no alignment/resizing; not Delta E',
            'reference': 'Lightroom Classic PV 15.4, Embedded profile, linear curve, sRGB JPEG quality 100, no resize/output sharpening',
            'before_version': '0.7.6', 'after_version': '0.7.7',
            'photo_count': len(hashes), 'comparison_count': len(results),
            'before': before, 'after': after, 'reduction_percent': (1 - after / before) * 100,
            'by_case': by_case, 'pairs': results, 'source_sha256': hashes}


if __name__ == '__main__':
    assert len(sys.argv) == 3, __doc__
    report = compare(Path(sys.argv[1]))
    Path(sys.argv[2]).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ['photo_count', 'comparison_count', 'before', 'after', 'reduction_percent']}, indent=2))
