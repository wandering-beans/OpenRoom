#!/usr/bin/env python3
"""Check dependency resolution without modifying or signing an application."""
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("bundle-macos-libs.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class ResolutionTests(unittest.TestCase):
    def test_rebuild_removes_libraries_no_longer_referenced(self):
        with tempfile.TemporaryDirectory() as folder:
            app = Path(folder) / "OpenRoom.app"
            (app / "Contents/MacOS").mkdir(parents=True)
            frameworks = app / "Contents/Frameworks"
            frameworks.mkdir()
            stale = frameworks / "libunused.dylib"
            stale.touch()
            with (app / "Contents/Info.plist").open("wb") as stream:
                plistlib.dump({"CFBundleExecutable": "OpenRoom"}, stream)
            with patch.object(bundle, "run", return_value=""):
                bundle.bundle_app(app)
            self.assertFalse(stale.exists())

    def test_loader_siblings_and_explicit_run_paths_resolve_and_unknowns_fail(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            original = root / "libdecoder.dylib"
            sibling = root / "libcommon.dylib"
            sibling.touch()
            executable = root / "MacOS/OpenRoom"
            with patch.object(bundle, "run", return_value=""):
                self.assertEqual(bundle.resolve_dependency("@rpath/libcommon.dylib", original, executable), sibling)
                self.assertEqual(bundle.resolve_dependency("@loader_path/libcommon.dylib", original, executable), sibling)
                with self.assertRaises(RuntimeError):
                    bundle.resolve_dependency("@rpath/missing.dylib", original, executable)
            framework = root / "Frameworks"
            framework.mkdir()
            library = framework / "libextra.dylib"
            library.touch()
            with patch.object(bundle, "run", return_value="cmd LC_RPATH\ncmdsize 32\npath @loader_path/Frameworks (offset 12)"):
                self.assertEqual(bundle.resolve_dependency("@rpath/libextra.dylib", original, executable), library)


if __name__ == "__main__":
    unittest.main()
