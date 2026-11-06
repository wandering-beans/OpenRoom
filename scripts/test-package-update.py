#!/usr/bin/env python3
"""Exercise Windows release signing with a disposable key, never the release key."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('package_update', ROOT / 'scripts/package-update.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
CLI = str(ROOT / 'desktop/node_modules/@tauri-apps/cli/tauri.js')


class WindowsUpdates(unittest.TestCase):
    def test_signing_and_platform_isolation(self):
        original = (ROOT / 'desktop/tauri.conf.json').read_bytes()
        config = json.loads(original)
        windows = json.loads((ROOT / 'desktop/tauri.windows.conf.json').read_text())
        mac_feed = config['plugins']['updater']['endpoints']
        win_feed = windows['plugins']['updater']['endpoints']
        self.assertTrue(mac_feed[0].endswith('/updates/latest.json'))
        self.assertTrue(win_feed[0].endswith('/updates/windows-x86_64.json'))
        self.assertTrue(config['plugins']['updater']['requireSignedVersion'])
        self.assertNotIn('pubkey', windows['plugins']['updater'])
        self.assertEqual(windows['bundle']['targets'], ['nsis'])
        self.assertEqual(windows['bundle']['windows']['nsis']['installMode'], 'currentUser')
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            key = work / 'disposable.key'
            env = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY_PATH=str(key),
                       TAURI_SIGNING_PRIVATE_KEY_PASSWORD='')
            # Do not read or use any release signing credentials inherited from the shell.
            env.pop('TAURI_SIGNING_PRIVATE_KEY', None)
            subprocess.run(['node', CLI, 'signer', 'generate', '--ci', '--password', '',
                            '--write-keys', str(key)], env=env, check=True, stdout=subprocess.DEVNULL)
            config['plugins']['updater']['pubkey'] = Path(str(key) + '.pub').read_text().strip()
            version = config['version']
            installer = work / f'OpenRoom-{version}-windows-x64-setup.exe'
            # Synthetic non-executable data: the test signs it but never installs or runs it.
            installer.write_bytes(b'MZ' + bytes(126))
            notes = work / 'notes.txt'
            notes.write_text('Windows update fixture', encoding='utf-8')
            args = SimpleNamespace(notes=notes, output=work / 'release')
            with patch.dict(os.environ, env, clear=True):
                package.package_windows(installer, args, ROOT, config)
                result = json.loads((args.output / 'windows-x86_64.json').read_text())
                self.assertEqual(list(result['platforms']), ['windows-x86_64'])
                platform = result['platforms']['windows-x86_64']
                self.assertEqual(platform['url'], f'https://github.com/wandering-beans/OpenRoom/releases/download/v{version}/{installer.name}')
                comment = base64.b64decode(platform['signature']).decode()
                self.assertIn(f'file:{installer.name}\tversion:{version}', comment)
                self.assertEqual((args.output / installer.name).read_bytes(), installer.read_bytes())
                self.assertFalse((args.output / 'latest.json').exists())
                self.assertFalse(list(args.output.glob('*.key*')))
                with self.assertRaisesRegex(ValueError, 'Output already exists'):
                    package.package_windows(installer, args, ROOT, config)
                args.output = work / 'invalid'
                installer.write_bytes(b'\x1f\x8brenamed macOS archive')
                with self.assertRaisesRegex(ValueError, 'Expected.*Windows'):
                    package.package_windows(installer, args, ROOT, config)
                self.assertFalse(args.output.exists())
                installer.write_bytes(b'MZ' + bytes(126))
                config['plugins']['updater']['pubkey'] = 'wrong-key'
                with self.assertRaisesRegex(ValueError, 'Signing key does not match'):
                    package.package_windows(installer, args, ROOT, config)
                self.assertFalse(args.output.exists())
        self.assertEqual((ROOT / 'desktop/tauri.conf.json').read_bytes(), original)


if __name__ == '__main__':
    unittest.main()
