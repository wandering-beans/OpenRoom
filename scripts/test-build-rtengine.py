#!/usr/bin/env python3
"""Check shared build locking and fail-closed Windows DLL dependency discovery."""
from importlib import import_module
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

build = import_module("build-rtengine")


class BuildTests(unittest.TestCase):
    def test_lock_serializes_processes_and_releases_after_exception(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lock"
            code = ("import sys; from importlib import import_module; from pathlib import Path; "
                    "m=import_module('build-rtengine'); "
                    "lock=m.build_lock(Path(sys.argv[1])); lock.__enter__(); print('locked', flush=True)")
            try:
                with build.build_lock(path):
                    with subprocess.Popen([sys.executable, "-c", code, str(path)],
                                          cwd=Path(__file__).parent, stdout=subprocess.PIPE) as child:
                        with self.assertRaises(subprocess.TimeoutExpired):
                            child.wait(timeout=.3)
                        child.terminate()
                        child.wait(timeout=10)
                    raise ValueError("release")
            except ValueError:
                pass
            self.assertEqual(subprocess.check_output([sys.executable, "-c", code, str(path)],
                             cwd=Path(__file__).parent, timeout=10).strip(), b"locked")

    def test_dll_closure_handles_cycles_and_rejects_missing_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            (root / "System32").mkdir()
            for name in ["a.dll", "b.dll"]:
                (root / "bin" / name).touch()
            (root / "System32/kernel32.dll").touch()
            imports = {"engine.dll": "DLL Name: a.dll\nDLL Name: kernel32.dll",
                       "a.dll": "DLL Name: b.dll", "b.dll": "DLL Name: a.dll"}
            def inspect(args, **_):
                return imports[Path(args[-1]).name]
            with patch.dict(os.environ, {"SystemRoot": str(root)}), patch.object(build.subprocess, "check_output", inspect):
                self.assertEqual({p.name for p in build.dll_dependencies(root / "engine.dll", root)}, {"a.dll", "b.dll"})
                imports["b.dll"] = "DLL Name: missing.dll"
                with self.assertRaisesRegex(RuntimeError, "missing.dll"):
                    build.dll_dependencies(root / "engine.dll", root)


if __name__ == "__main__":
    unittest.main()
