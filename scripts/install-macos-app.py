#!/usr/bin/env python3
"""Install a completed bundle without rewriting executables used by a running app."""
from pathlib import Path
import os
import shutil
import subprocess
import sys


def install_app(source, destination):
    source, destination = source.resolve(), destination.resolve()
    commands = subprocess.check_output(["ps", "-axo", "command="], text=True)
    prefix = str(destination / "Contents") + os.sep
    if any(command.strip().startswith(prefix) for command in commands.splitlines()):
        raise RuntimeError(f"Close {destination.name} and its workers before installing")

    previous = source.parent / "previous.app"
    if previous.exists():
        raise RuntimeError(f"A previous bundle needs recovery: {previous}")
    if destination.exists():
        destination.rename(previous)
    try:
        source.rename(destination)
    except BaseException:
        if previous.exists():
            previous.rename(destination)
        raise
    if previous.exists():
        shutil.rmtree(previous)


if __name__ == "__main__":
    install_app(Path(sys.argv[1]), Path(sys.argv[2]))
