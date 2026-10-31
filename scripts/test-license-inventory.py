#!/usr/bin/env python3
"""Check that packaging rejects unrecorded or changed engine data."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'license_inventory', Path(__file__).with_name('check-license-inventory.py'))
inventory_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory_check)


class EngineDataInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.inventory = {'profiles': [], 'data_files': []}
        for name, group in [('camera.icm', 'profiles'), ('camera.dcp', 'profiles'),
                            ('camconst.json', 'data_files')]:
            contents = name.encode()
            (self.directory / name).write_bytes(contents)
            self.inventory[group].append({
                'file': name, 'sha256': hashlib.sha256(contents).hexdigest()})

    def check(self):
        inventory_check.check_engine_data(self.directory, self.inventory)

    def test_complete_inventory_passes(self):
        self.check()

    def test_unrecorded_icm_is_rejected(self):
        self.inventory['profiles'] = self.inventory['profiles'][1:]
        with self.assertRaisesRegex(ValueError, 'camera.icm'):
            self.check()

    def test_missing_profile_is_rejected(self):
        (self.directory / 'camera.dcp').unlink()
        with self.assertRaisesRegex(ValueError, 'camera.dcp'):
            self.check()

    def test_changed_camera_data_is_rejected(self):
        (self.directory / 'camconst.json').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'Engine data changed'):
            self.check()

    def test_duplicate_record_is_rejected(self):
        self.inventory['profiles'].append(self.inventory['profiles'][0])
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            self.check()

    def test_symlink_is_rejected_even_when_contents_match(self):
        path = self.directory / 'camera.icm'
        path.unlink()
        path.symlink_to(self.directory / 'camera.dcp')
        self.inventory['profiles'][0]['sha256'] = self.inventory['profiles'][1]['sha256']
        with self.assertRaisesRegex(ValueError, 'Engine data changed'):
            self.check()


if __name__ == '__main__':
    unittest.main()
