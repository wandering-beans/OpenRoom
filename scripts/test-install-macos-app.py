#!/usr/bin/env python3
"""A running app must stay untouched, and a failed replacement must restore it."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-macos-app.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.source = root / "staging" / "OpenRoom.app"
        self.destination = root / "OpenRoom.app"
        for path, value in [(self.source, "new"), (self.destination, "old")]:
            path.mkdir(parents=True)
            (path / "binary").write_text(value)

    def test_running_app_or_worker_blocks_replacement(self):
        for suffix in ["", " --rtengine-resident"]:
            command = str(self.destination.resolve() / "Contents/MacOS/OpenRoom") + suffix
            with self.subTest(suffix=suffix), patch.object(installer.subprocess, "check_output", return_value=command):
                with self.assertRaisesRegex(RuntimeError, "Close"):
                    installer.install_app(self.source, self.destination)
            self.assertEqual((self.destination / "binary").read_text(), "old")
            self.assertTrue(self.source.exists())

    def test_completed_bundle_replaces_old_bundle(self):
        with patch.object(installer.subprocess, "check_output", return_value="/Applications/Other.app/Contents/MacOS/Other"):
            installer.install_app(self.source, self.destination)
        self.assertEqual((self.destination / "binary").read_text(), "new")
        self.assertFalse(self.source.exists())
        self.assertFalse((self.source.parent / "previous.app").exists())

    def test_failed_move_restores_previous_bundle(self):
        rename = Path.rename
        def fail_new_bundle(path, target):
            if path == self.source.resolve():
                raise OSError("simulated installation failure")
            return rename(path, target)
        with patch.object(installer.subprocess, "check_output", return_value=""), patch.object(Path, "rename", fail_new_bundle):
            with self.assertRaises(OSError):
                installer.install_app(self.source, self.destination)
        self.assertEqual((self.destination / "binary").read_text(), "old")
        self.assertTrue(self.source.exists())

    def test_process_inspection_failure_leaves_old_bundle(self):
        with patch.object(installer.subprocess, "check_output", side_effect=OSError("unavailable")):
            with self.assertRaises(OSError):
                installer.install_app(self.source, self.destination)
        self.assertEqual((self.destination / "binary").read_text(), "old")


if __name__ == "__main__":
    unittest.main()
