"""Measure a staged Windows import worker across repeated passes over distinct RAWs."""
import argparse
import ctypes
import json
import subprocess
from pathlib import Path
from ctypes import wintypes

parser = argparse.ArgumentParser()
parser.add_argument("executable", type=Path)
parser.add_argument("folder", type=Path)
parser.add_argument("--passes", type=int, default=6)
args = parser.parse_args()
samples = sorted(args.folder.glob("*.NEF"))
if len(samples) < 2:
    raise SystemExit("Pass a folder with at least two different NEFs")

class Memory(ctypes.Structure):
    _fields_ = [(name, wintypes.DWORD) for name in ("size", "faults")] + [
        (name, ctypes.c_size_t) for name in ("peak_rss", "rss", "peak_pool", "pool",
            "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")]

get_memory = ctypes.WinDLL("psapi").GetProcessMemoryInfo
get_memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
with subprocess.Popen([str(args.executable.resolve()), "--rtengine-import"],
                      stdin=subprocess.PIPE, stdout=subprocess.PIPE) as worker:
    try:
        for cycle in range(args.passes):
            for source in samples:
                request = dict(prepare=False, source=str(source.resolve()), bound=1600, settings={}, base=None, bytes=0)
                worker.stdin.write(json.dumps(request).encode() + b"\n")
                worker.stdin.flush()
                header = json.loads(worker.stdout.readline())
                if header["error"]:
                    raise RuntimeError(header["error"])
                assert 0 < header["bytes"] <= 1600*1600*4
                assert len(worker.stdout.read(header["bytes"])) == header["bytes"]
                memory = Memory()
                memory.size = ctypes.sizeof(memory)
                if not get_memory(worker._handle, ctypes.byref(memory), memory.size):
                    raise ctypes.WinError()
                print(json.dumps(dict(cycle=cycle,photo=source.name,private=memory.private,
                                      rss=memory.rss,peak_rss=memory.peak_rss)),flush=True)
    finally:
        worker.stdin.close()
        worker.wait(timeout=30)
