#!/usr/bin/env python3
"""Check DCP exposure ownership and RAW detail scale through the shipped C ABI."""
import ctypes as c
from importlib import import_module
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


class Adjustments(c.Structure):
    _fields_ = [(name, c.c_float) for name in (
        'exposure', 'temperature', 'tint', 'raw_temperature', 'raw_tint',
        'sharpen', 'sharpen_radius', 'sharpen_detail', 'sharpen_masking',
        'noise_luma', 'noise_color', 'scale')] + [
        ('ca_auto', c.c_uint32), ('defringe', c.c_uint32)] + [
        (name, c.c_float) for name in ('ca_red', 'ca_blue', 'defringe_radius', 'defringe_threshold')]


class Frame(c.Structure):
    _fields_ = [(name, c.c_uint32) for name in
                ('width', 'height', 'source_width', 'source_height')] + [
        ('temperature', c.c_double), ('green', c.c_double)]


def profile(path, offset):
    matrix = [1.620227, -.768569, -.249266, -.969266, 1.876011, .041556,
              .03617, -.132616, .687696]
    entries = [
        (50721, 10, 9, b''.join(struct.pack('<ii', round(v * 1000000), 1000000) for v in matrix)),
        (50778, 3, 1, struct.pack('<H', 21)),
        (51109, 10, 1, struct.pack('<ii', offset, 1)),
    ]
    start = 8 + 2 + len(entries) * 12 + 4
    directory, extra = bytearray(struct.pack('<H', len(entries))), bytearray()
    for tag, kind, count, data in entries:
        value = data.ljust(4, b'\0') if len(data) <= 4 else struct.pack('<I', start + len(extra))
        directory.extend(struct.pack('<HHI', tag, kind, count) + value)
        if len(data) > 4:
            extra.extend(data)
    path.write_bytes(b'II*\0' + struct.pack('<I', 8) + directory + bytes(4) + extra)


def measure(library, offset):
    engine = c.CDLL(str(library.resolve()))
    signatures = {
        'init': ([c.c_char_p, c.c_char_p], c.c_int),
        'open': ([c.c_char_p, c.POINTER(c.c_float), c.c_uint32, c.c_uint32, c.c_uint32], c.c_void_p),
        'render': ([c.c_void_p, c.POINTER(Adjustments), c.POINTER(Frame)], c.c_int),
        'copy': ([c.c_void_p, c.POINTER(c.c_float), c.c_size_t], c.c_int),
        'close': ([c.c_void_p], None),
        'camera': ([c.c_void_p], c.c_char_p),
        'auto_ca_supported': ([c.c_void_p], c.c_int),
        'error': ([], c.c_char_p),
    }
    for name, (args, result) in signatures.items():
        function = getattr(engine, 'openroom_rt_' + name)
        function.argtypes, function.restype = args, result
    with tempfile.TemporaryDirectory(prefix='rtengine-contract-') as temporary:
        directory = Path(temporary)
        profiles = directory / 'dcpprofiles'
        profiles.mkdir()
        source = directory / 'neutral.dng'
        import_module('test-highlight-recovery').neutral_dng(source)
        assert engine.openroom_rt_init(bytes(directory), bytes(directory)) == 0, engine.openroom_rt_error()
        session = engine.openroom_rt_open(bytes(source), None, 0, 0, 128)
        assert session, engine.openroom_rt_error()
        try:
            camera = engine.openroom_rt_camera(session).decode()
            profile(profiles / (camera + '.dcp'), offset)
            settings = Adjustments(sharpen_radius=3, sharpen_detail=25, scale=1, defringe_radius=2, defringe_threshold=13)
            def render():
                frame = Frame()
                assert engine.openroom_rt_render(session, c.byref(settings), c.byref(frame)) == 0, engine.openroom_rt_error()
                pixels = (c.c_float * (frame.width * frame.height * 4))()
                assert engine.openroom_rt_copy(session, pixels, len(pixels)) == 0
                return list(pixels)
            original = render()
            assert original == render(), 'Repeated rendering applied the profile twice'
            assert engine.openroom_rt_auto_ca_supported(session) == 1
            settings.ca_auto = 1
            corrected = render()
            assert corrected == render(), 'Automatic CA correction must not accumulate'
            settings.ca_auto = 0
            assert original == render(), 'Disabling automatic CA must restore the original RAW'
            settings.exposure = -1
            exposed = render()
            assert all(abs(exposed[i] * 2 - original[i]) < 1e-5 for i in range(len(original)) if i % 4 != 3)
            settings.exposure, settings.sharpen = 0, 100
            detailed = render()
            settings.scale = .3
            rescaled = render()
            scale_error = max(abs(a - b) for a, b in zip(detailed, rescaled))
            return {'camera': camera, 'samples': original[::16], 'detail_scale_error': scale_error}
        finally:
            engine.openroom_rt_close(session)


def main():
    library = Path(sys.argv[1])
    if len(sys.argv) == 3:
        print(json.dumps(measure(library, int(sys.argv[2]))))
        return
    samples = [json.loads(subprocess.check_output(
        [sys.executable, __file__, str(library), str(offset)], text=True, timeout=120)) for offset in (0, 1)]
    error = max(abs(b - a * 2) for a, b in zip(samples[0]['samples'], samples[1]['samples']))
    assert error < 1e-5, ('DCP +1 EV must apply exactly once', error)
    assert all(result['detail_scale_error'] == 0 for result in samples), 'RAW sharpening used the requested output scale instead of its processing dimensions'
    print(json.dumps({'dcp_gain_error': error, 'detail_scale_error': 0, 'repeated_exposure': 'passed'}))


if __name__ == '__main__':
    main()
