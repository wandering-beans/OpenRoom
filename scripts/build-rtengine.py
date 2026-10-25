#!/usr/bin/env python3
"""Build the pinned engine and the small OpenRoom C ABI, without RawTherapee's UI."""
import json
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys


def run(*args, **kwargs):
    subprocess.run(args, check=True, stdout=sys.stderr, **kwargs)


def main():
    root = Path(__file__).resolve().parents[1]
    bridge = root / "native/rtengine"
    pin = json.loads((bridge / "source.json").read_text())
    cache = root / ".tools/rtengine"
    source = cache / "source"
    build = cache / "build"
    cache.mkdir(parents=True, exist_ok=True)
    # Cargo manifests can share this native build directory concurrently.
    lock = (cache / "build.lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    if not source.exists():
        local = os.environ.get("OPENROOM_RTENGINE_SOURCE")
        if local:
            shutil.copytree(local, source)
        else:
            run("git", "clone", "--no-checkout", pin["repository"], str(source))
        run("git", "-C", str(source), "checkout", "--detach", pin["commit"])
    actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if actual != pin["commit"]:
        raise RuntimeError("rtengine source does not match native/rtengine/source.json")
    patch = bridge / "float-preview.patch"
    applied = subprocess.run(["git", "-C", str(source), "apply", "--reverse", "--check", str(patch)], capture_output=True)
    if applied.returncode:
        run("git", "-C", str(source), "apply", "--check", str(patch))
        run("git", "-C", str(source), "apply", str(patch))
    actual_patch = subprocess.check_output([
        # Match the recorded patch even when a full clone chooses longer object IDs.
        "git", "-C", str(source), "diff", "--abbrev=7", "--no-ext-diff", "--no-color", "--binary", "HEAD", "--"])
    if actual_patch != patch.read_bytes():
        raise RuntimeError("rtengine source has changes outside the recorded patch")
    prefix = subprocess.check_output(["brew", "--prefix"], text=True).strip()
    omp = Path(prefix) / "opt/libomp"
    run("cmake", "-S", str(source), "-B", str(build), "-G", "Ninja",
        "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        "-DOPENROOM_ENGINE_ONLY=ON", f"-DOPENROOM_BRIDGE_DIR={bridge}",
        "-DWITH_SYSTEM_FMT=ON", "-DWITH_SYSTEM_LIBRAW=ON", "-DWITH_JXL=OFF",
        "-DWITH_LTO=OFF", "-DOSX_DEV_BUILD=ON", f"-DLOCAL_PREFIX={prefix}",
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=14.0", "-DPROC_TARGET_NUMBER=0",
        "-DOpenMP_C_FLAGS=-Xpreprocessor -fopenmp", "-DOpenMP_CXX_FLAGS=-Xpreprocessor -fopenmp",
        "-DOpenMP_C_LIB_NAMES=omp", "-DOpenMP_CXX_LIB_NAMES=omp",
        f"-DOpenMP_omp_LIBRARY={omp}/lib/libomp.dylib",
        f"-DOpenMP_C_INCLUDE_DIR={omp}/include", f"-DOpenMP_CXX_INCLUDE_DIR={omp}/include")
    run("cmake", "--build", str(build), "--target", "openroom_rtengine", "--parallel", "4")
    data = cache / "data"
    data.mkdir(exist_ok=True)
    shutil.copy2(source / "rtengine/camconst.json", data)
    for name in ["iccprofiles", "dcpprofiles"]:
        shutil.copytree(source / "rtdata" / name, data / name, dirs_exist_ok=True)
    print(json.dumps({"library": str(build / "libopenroom_rtengine.dylib"), "data": str(data)}))


if __name__ == "__main__":
    main()
