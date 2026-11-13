#!/usr/bin/env python3
"""Build the pinned engine and the small OpenRoom C ABI, without RawTherapee's UI."""
import json
from contextlib import contextmanager
import platform
import re
import time
import os
from pathlib import Path
import shutil
import subprocess
import sys


def run(*args, **kwargs):
    subprocess.run(args, check=True, stdout=sys.stderr, **kwargs)


@contextmanager
def build_lock(path):
    # Lock one byte without truncating a file another Cargo process has locked.
    with path.open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            while True:
                lock.seek(0)
                try:
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError as error:
                    if error.errno not in (13, 36):
                        raise
                    time.sleep(.1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            if os.name == "nt":
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)


def windows_prefix():
    prefix = os.environ.get("OPENROOM_MSYS2_PREFIX")
    if not prefix:
        raise RuntimeError("Set OPENROOM_MSYS2_PREFIX to the MSYS2 UCRT64 directory")
    prefix = Path(prefix).resolve()
    if not (prefix / "bin/g++.exe").is_file():
        raise RuntimeError("MSYS2 UCRT64 GCC is missing")
    if platform.machine().lower() not in ("amd64", "x86_64"):
        raise RuntimeError("The Windows native build currently requires x86_64")
    os.environ["PATH"] = os.pathsep.join([str(prefix / "bin"), os.environ["PATH"], str(prefix.parent / "usr/bin")])
    return prefix


def dll_dependencies(library, prefix):
    # Copy only the transitive runtime dependencies, never the whole compiler bin directory.
    pending, found = [library], {}
    system = Path(os.environ["SystemRoot"]) / "System32"
    while pending:
        current = pending.pop()
        output = subprocess.check_output([str(prefix / "bin/objdump.exe"), "-p", str(current)], text=True)
        for name in re.findall(r"DLL Name: (\S+)", output):
            key = name.lower()
            if key in found or key.startswith(("api-ms-win-", "ext-ms-win-")):
                continue
            dependency = prefix / "bin" / name
            if dependency.is_file():
                found[key] = dependency
                pending.append(dependency)
            elif not (system / name).is_file():
                raise RuntimeError(f"Missing runtime DLL: {name}")
    return list(found.values())


def main():
    root = Path(__file__).resolve().parents[1]
    bridge = root / "native/rtengine"
    pin = json.loads((bridge / "source.json").read_text())
    cache = root / ".tools/rtengine"
    source = cache / "source"
    build = cache / "build"
    cache.mkdir(parents=True, exist_ok=True)
    with build_lock(cache / "build.lock"):
        build_engine(root, bridge, pin, cache, source, build)


def build_engine(root, bridge, pin, cache, source, build):
    if not source.exists():
        local = os.environ.get("OPENROOM_RTENGINE_SOURCE")
        if local:
            shutil.copytree(local, source)
        else:
            run("git", "-c", "core.autocrlf=false", "clone", "--no-checkout", pin["repository"], str(source))
        run("git", "-C", str(source), "config", "core.autocrlf", "false")
        run("git", "-C", str(source), "checkout", "--detach", pin["commit"])
    actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if actual != pin["commit"]:
        raise RuntimeError("rtengine source does not match native/rtengine/source.json")
    patches = [bridge / "float-preview.patch", bridge / "metadata-time.patch"]
    for patch in patches:
        applied = subprocess.run(["git", "-C", str(source), "apply", "--reverse", "--check", str(patch)], capture_output=True)
        if applied.returncode:
            run("git", "-C", str(source), "apply", "--check", str(patch))
            run("git", "-C", str(source), "apply", str(patch))
    actual_patch = subprocess.check_output([
        "git", "-C", str(source), "diff", "--abbrev=7", "--no-ext-diff", "--no-color", "--binary", "HEAD", "--"])
    expected = b"".join(patch.read_bytes().replace(b"\r\n", b"\n") for patch in patches)
    # The patches touch disjoint files; Git sorts the combined diff by filename.
    if sorted(actual_patch.split(b"diff --git ")) != sorted(expected.split(b"diff --git ")):
        raise RuntimeError("rtengine source has changes outside the recorded patches")
    common = ["-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        "-DOPENROOM_ENGINE_ONLY=ON", f"-DOPENROOM_BRIDGE_DIR={bridge.as_posix()}",
        "-DWITH_SYSTEM_FMT=ON", "-DWITH_SYSTEM_LIBRAW=ON", "-DWITH_JXL=OFF",
        "-DWITH_LTO=OFF", "-DPROC_TARGET_NUMBER=0"]
    if os.name == "nt":
        prefix = windows_prefix()
        build = cache / "build-windows-x86_64"
        common += [f"-DCMAKE_PREFIX_PATH={prefix.as_posix()}",
            f"-DCMAKE_C_COMPILER={prefix.as_posix()}/bin/gcc.exe",
            f"-DCMAKE_CXX_COMPILER={prefix.as_posix()}/bin/g++.exe"]
        library = build / "openroom_rtengine.dll"
    elif sys.platform == "darwin":
        prefix = subprocess.check_output(["brew", "--prefix"], text=True).strip()
        omp = Path(prefix) / "opt/libomp"
        common += ["-DOSX_DEV_BUILD=ON", f"-DLOCAL_PREFIX={prefix}",
            "-DCMAKE_OSX_DEPLOYMENT_TARGET=14.0",
            "-DOpenMP_C_FLAGS=-Xpreprocessor -fopenmp", "-DOpenMP_CXX_FLAGS=-Xpreprocessor -fopenmp",
            "-DOpenMP_C_LIB_NAMES=omp", "-DOpenMP_CXX_LIB_NAMES=omp",
            f"-DOpenMP_omp_LIBRARY={omp}/lib/libomp.dylib",
            f"-DOpenMP_C_INCLUDE_DIR={omp}/include", f"-DOpenMP_CXX_INCLUDE_DIR={omp}/include"]
        library = build / "libopenroom_rtengine.dylib"
    elif sys.platform == "linux":
        build = cache / "build-linux"
        common += ["-DUSE_LIBCANBERRA=OFF"]
        if subprocess.run(["pkg-config", "--atleast-version=0.21", "libraw_r"]).returncode:
            common.append("-DWITH_SYSTEM_LIBRAW=OFF")
        library = build / "libopenroom_rtengine.so"
    else:
        raise RuntimeError(f"Unsupported rtengine platform: {sys.platform}")
    run("cmake", "-S", str(source), "-B", str(build), "-G", "Ninja", *common)
    run("cmake", "--build", str(build), "--target", "openroom_rtengine", "--parallel", "4")
    data = cache / "data"
    data.mkdir(exist_ok=True)
    shutil.copy2(source / "rtengine/camconst.json", data)
    for name in ["iccprofiles", "dcpprofiles"]:
        shutil.copytree(source / "rtdata" / name, data / name, dirs_exist_ok=True)
    dependencies = dll_dependencies(library, prefix) if os.name == "nt" else []
    for dependency in dependencies:
        shutil.copy2(dependency, library.parent / dependency.name)
    print(json.dumps({"library": str(library), "data": str(data),
                      "runtime": [str(library.parent / dll.name) for dll in dependencies]}))


if __name__ == "__main__":
    main()
