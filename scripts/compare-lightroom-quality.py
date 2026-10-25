#!/usr/bin/env python3
"""Render or compare a local 16-bit LRC quality fixture set; never align or resize measurements.

ROOT contains cases.json (name/lrc/openroom), input/, reference/CASE/*.tif.
Use --render before|after --executable APP_BINARY, then --compare REPORT.json.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image


def read_tiff(path):
    data = path.read_bytes()
    with Image.open(path) as image:
        tags = image.tag_v2
        channels = tags[277]
        assert image.info.get('icc_profile'), f'Missing ICC: {path}'
        assert tags[259] == 1 and tags[258] == (16,) * channels and tags.get(284, 1) == 1, path
        strips = b''.join(data[offset:offset + count] for offset, count in zip(tags[273], tags[279]))
        width, height = image.size
    dtype = '<u2' if data[:2] == b'II' else '>u2'
    return np.frombuffer(strips, dtype=dtype).reshape(height, width, channels)[..., :3]


def validate_reference(path, case, source):
    data = path.read_bytes()
    expected = {'ProcessVersion': 15.4, 'Exposure2012': 0, 'Contrast2012': 0,
                'Highlights2012': 0, 'Shadows2012': 0, 'Whites2012': 0, 'Blacks2012': 0,
                'Sharpness': 0, 'LuminanceSmoothing': 0, 'ColorNoiseReduction': 0,
                'Saturation': 0, 'Vibrance': 0, 'Clarity2012': 0, 'Dehaze': 0}
    expected.update({key: value for key, value in case['lrc'].items() if not isinstance(value, bool)})
    for key, value in expected.items():
        match = re.search(rb'crs:' + key.encode() + rb'="([^"]*)"', data)
        assert match and float(match[1]) == value, (path, key, value)
    match = re.search(rb'crs:ConvertToGrayscale="([^"]*)"', data)
    assert match and (match[1].lower() == b'true') == case['lrc'].get('ConvertToGrayscale', False), path
    profile = b'Embedded' if source.suffix.lower() in ['.png', '.jpg', '.tif', '.tiff'] else b'Adobe Standard'
    assert re.search(rb'crs:CameraProfile="([^"]*)"', data)[1] == profile, path
    assert re.search(rb'crs:WhiteBalance="([^"]*)"', data)[1] == b'As Shot', path
    xmp = re.search(rb'<x:xmpmeta.*?</x:xmpmeta>', data, re.S)
    assert xmp, path
    document = ET.fromstring(xmp[0])
    crs = '{http://ns.adobe.com/camera-raw-settings/1.0/}'
    rdf = '{http://www.w3.org/1999/02/22-rdf-syntax-ns#}'
    for suffix in ['', 'Red', 'Green', 'Blue']:
        curve = document.find('.//' + crs + 'ToneCurvePV2012' + suffix)
        assert curve is not None, path
        points = [int(value.strip()) for point in curve.iter(rdf + 'li') for value in point.text.split(',')]
        assert points == [0, 0, 255, 255], (path, suffix, points)


def stats(pixels):
    return {'black_pixels': int(np.all(pixels == 0, axis=2).sum()),
            'white_pixels': int(np.all(pixels == 65535, axis=2).sum()),
            'distinct_red_levels': int(np.unique(pixels[..., 0]).size)}


def render(root, side, executable, cases):
    sources = sorted((root / 'input').iterdir())
    hashes = {source.name: hashlib.sha256(source.read_bytes()).hexdigest() for source in sources}
    timings = []
    for case in cases:
        folder = root / side / case['name']
        folder.mkdir(parents=True, exist_ok=True)
        recipe = folder / 'recipe.json'
        recipe.write_text(json.dumps(case['openroom']))
        for source in sources:
            destination = folder / (source.stem + '.tif')
            assert not destination.exists(), f'Output already exists: {destination}'
            start = time.monotonic()
            subprocess.run([str(executable), '--develop-export', str(source), str(destination),
                            str(recipe), 'tiff', json.dumps({'long_edge': 1024})],
                           check=True, stdout=subprocess.DEVNULL, timeout=120)
            timings.append({'case': case['name'], 'source': source.name, 'seconds': time.monotonic() - start})
        print(side, case['name'], 'complete', flush=True)
    assert all(hashlib.sha256(source.read_bytes()).hexdigest() == hashes[source.name] for source in sources)
    (root / (side + '-run.json')).write_text(json.dumps({'source_sha256': hashes, 'timings': timings}, indent=2))


def compare(root, cases):
    pairs, excluded = [], []
    sources = {path.stem: path for path in (root / 'input').iterdir()}
    for case in cases:
        for reference in sorted((root / 'reference' / case['name']).glob('*.tif')):
            if case.get('comparison_skip_reason'):
                excluded.append({'case': case['name'], 'photo': reference.stem, 'reason': case['comparison_skip_reason']})
                continue
            validate_reference(reference, case, sources[reference.stem])
            target = read_tiff(reference)
            row = {'case': case['name'], 'photo': reference.stem, 'dimensions': list(target.shape[1::-1]),
                   'reference': stats(target)}
            for side in ['before', 'after']:
                actual = read_tiff(root / side / case['name'] / reference.name)
                if actual.shape != target.shape:
                    excluded.append({'case': case['name'], 'photo': reference.stem, 'side': side,
                                     'reference_dimensions': list(target.shape[1::-1]),
                                     'actual_dimensions': list(actual.shape[1::-1])})
                    break
                error = np.abs(actual.astype(np.float64) - target) / 257.
                row[side] = {'mean': float(error.mean()), 'p95': float(np.percentile(error, 95)), **stats(actual)}
            else:
                pairs.append(row)
    assert pairs, 'No valid comparison pairs'
    runs = {side: json.loads((root / (side + '-run.json')).read_text()) for side in ['before', 'after']}
    assert runs['before']['source_sha256'] == runs['after']['source_sha256']
    groups = {'JPEG': lambda row: row['photo'].startswith('IMGP'),
              'RAW': lambda row: row['photo'].startswith(('Canon', 'Fujifilm', 'Sony')),
              'synthetic': lambda row: not row['photo'].startswith(('IMGP', 'Canon', 'Fujifilm', 'Sony'))}
    return {'metric': 'Mean absolute sRGB channel error, 0..255, 16-bit TIFF on both sides. Not perceptual Delta E.',
            'reference': 'LRC 15.6 / PV15.4, Adobe Standard for RAW or Embedded, as-shot WB, long edge 1024, no output sharpening.',
            'pairs': pairs, 'excluded': excluded, 'source_sha256': runs['after']['source_sha256'],
            'by_source': {name: {'pairs': len([row for row in pairs if predicate(row)]), **{
                side: float(np.mean([row[side]['mean'] for row in pairs if predicate(row)])) for side in ['before', 'after']}}
                          for name, predicate in groups.items()},
            'by_case': {case['name']: {side: float(np.mean([row[side]['mean'] for row in pairs if row['case'] == case['name']]))
                                     for side in ['before', 'after']} for case in cases if not case.get('comparison_skip_reason')}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--render', choices=['before', 'after'])
    parser.add_argument('--executable', type=Path)
    parser.add_argument('--compare', type=Path)
    args = parser.parse_args()
    cases = json.loads((args.root / 'cases.json').read_text())
    if args.render:
        assert args.executable, '--executable is required for --render'
        render(args.root, args.render, args.executable.resolve(), cases)
    if args.compare:
        report = compare(args.root, cases)
        args.compare.write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'pairs': len(report['pairs']), 'excluded': report['excluded'], 'by_case': report['by_case']}, indent=2))
