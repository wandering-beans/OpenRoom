#!/usr/bin/env python3
"""Copy native dependencies into an app; reject unresolved non-system dylibs."""
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys


def run(*args):
    return subprocess.check_output(args, text=True)


def resolve_dependency(dependency, original, executable):
    if dependency.startswith("/"):
        return Path(dependency).resolve(strict=True)
    if dependency.startswith("@loader_path/"):
        return (original.parent / dependency.removeprefix("@loader_path/")).resolve(strict=True)
    if dependency.startswith("@executable_path/"):
        return (executable.parent / dependency.removeprefix("@executable_path/")).resolve(strict=True)
    if dependency.startswith("@rpath/"):
        name = dependency.removeprefix("@rpath/")
        load = run("otool", "-l", str(original))
        paths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", load)
        for path in paths:
            path = path.replace("@loader_path", str(original.parent))
            path = path.replace("@executable_path", str(executable.parent))
            candidate = (Path(path) / name).resolve()
            if candidate.is_file():
                return candidate
        # Homebrew's sibling dylibs can share a run path supplied by their caller.
        sibling = (original.parent / name).resolve()
        if sibling.is_file():
            return sibling
    raise RuntimeError(f"Unresolved native dependency {dependency} in {original}")


def bundle_app(app):
    app = app.resolve()
    plist = app / "Contents/Info.plist"
    with plist.open("rb") as stream:
        info = plistlib.load(stream)
    main = app / "Contents/MacOS" / info["CFBundleExecutable"]
    frameworks = app / "Contents/Frameworks"
    frameworks.mkdir(exist_ok=True)
    seen = {}
    minimum = (14, 0)

    def bundle(binary, original, is_main=False):
        nonlocal minimum
        load = run("otool", "-l", str(binary))
        for value in re.findall(r"\bminos ([0-9.]+)", load):
            minimum = max(minimum, tuple(map(int, value.split("."))))
        dependencies = [line.strip().split(" (")[0]
                        for line in run("otool", "-L", str(binary)).splitlines()[1:]]
        for dependency in dependencies:
            if dependency.startswith(("/usr/lib/", "/System/Library/")):
                continue
            name = Path(dependency).name
            if not is_main and name == binary.name:
                continue
            source = resolve_dependency(dependency, original, main)
            destination = frameworks / name
            if name not in seen:
                seen[name] = source
                shutil.copy2(source, destination)
                destination.chmod(0o755)
                subprocess.check_call(["install_name_tool", "-id", "@rpath/" + name, str(destination)])
                bundle(destination, source)
                subprocess.check_call(["codesign", "--force", "--sign", "-", str(destination)])
            elif seen[name] != source:
                raise RuntimeError(f"Conflicting library names: {name}")
            prefix = "@executable_path/../Frameworks/" if is_main else "@loader_path/"
            subprocess.check_call(["install_name_tool", "-change", dependency, prefix + name, str(binary)])

    bundle(main, main, True)
    for stale in frameworks.glob("*.dylib"):
        if stale.name not in seen:
            stale.unlink()
    info["LSMinimumSystemVersion"] = ".".join(map(str, minimum))
    with plist.open("wb") as stream:
        plistlib.dump(info, stream)
    print(f'Bundled {len(seen)} dylibs; package minimum macOS {info["LSMinimumSystemVersion"]}')


if __name__ == "__main__":
    bundle_app(Path(sys.argv[1]))
