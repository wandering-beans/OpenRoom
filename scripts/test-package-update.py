#!/usr/bin/env python3
"""Exercise Windows release signing with a disposable key, never the release key."""
import base64
import importlib.util
import io
import json
import plistlib
import sys
import tarfile
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
        self.assertTrue(mac_feed[0].endswith('/updates/macos-aarch64.json'))
        self.assertTrue(win_feed[0].endswith('/updates/windows-x86_64.json'))
        self.assertTrue(config['plugins']['updater']['requireSignedVersion'])
        self.assertNotEqual(windows['plugins']['updater']['pubkey'], config['plugins']['updater']['pubkey'])
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
            windows['plugins']['updater']['pubkey'] = Path(str(key) + '.pub').read_text().strip()
            fixture = work / 'fixture'
            (fixture / 'desktop').mkdir(parents=True)
            windows_config = fixture / 'desktop/tauri.windows.conf.json'
            windows_config.write_text(json.dumps(windows), encoding='utf-8')
            run = subprocess.run
            def sign(command, **kwargs):
                command[1] = CLI
                return run(command, **kwargs)
            version = config['version']
            installer = work / f'OpenRoom-{version}-windows-x64-setup.exe'
            # Synthetic non-executable data: the test signs it but never installs or runs it.
            installer.write_bytes(b'MZ' + bytes(126))
            notes = work / 'notes.txt'
            notes.write_text('Windows update fixture', encoding='utf-8')
            args = SimpleNamespace(notes=notes, output=work / 'release')
            with patch.dict(os.environ, env, clear=True), patch.object(package.subprocess, 'run', side_effect=sign):
                package.package_installer(installer, args, fixture, config)
                result = json.loads((args.output / 'windows-x86_64.json').read_text())
                self.assertEqual(list(result['platforms']), ['windows-x86_64'])
                platform = result['platforms']['windows-x86_64']
                self.assertEqual(platform['url'], f'https://github.com/wandering-beans/OpenRoom/releases/download/v{version}/{installer.name}')
                comment = base64.b64decode(platform['signature']).decode()
                self.assertIn(f'file:{installer.name}\tversion:{version}', comment)
                self.assertEqual((args.output / installer.name).read_bytes(), installer.read_bytes())
                self.assertFalse((args.output / 'macos-aarch64.json').exists())
                self.assertFalse(list(args.output.glob('*.key*')))
                with self.assertRaisesRegex(ValueError, 'Output already exists'):
                    package.package_installer(installer, args, fixture, config)
                args.output = work / 'invalid'
                installer.write_bytes(b'\x1f\x8brenamed macOS archive')
                with self.assertRaisesRegex(ValueError, 'Expected.*Windows'):
                    package.package_installer(installer, args, fixture, config)
                self.assertFalse(args.output.exists())
                installer.write_bytes(b'MZ' + bytes(126))
                windows['plugins']['updater']['pubkey'] = config['plugins']['updater']['pubkey']
                windows_config.write_text(json.dumps(windows), encoding='utf-8')
                with self.assertRaisesRegex(ValueError, 'Signing key does not match'):
                    package.package_installer(installer, args, fixture, config)
                self.assertFalse(args.output.exists())
        self.assertEqual((ROOT / 'desktop/tauri.conf.json').read_bytes(), original)



class LinuxUpdates(unittest.TestCase):
    def test_target_architecture_checksum_and_key_isolation(self):
        configs = [json.loads((ROOT / 'desktop' / file).read_bytes())['plugins']['updater']
                   for file in ['tauri.conf.json', 'tauri.windows.conf.json', 'tauri.linux.conf.json']]
        self.assertEqual(len({c['pubkey'] for c in configs}), 3)
        self.assertTrue(configs[2]['endpoints'][0].endswith('/updates/linux-x86_64.json'))
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            config = {'version': '1.0.0'}
            image = work / 'OpenRoom-1.0.0-linux-x86_64.AppImage'
            header = bytearray(64)
            header[:7] = b'\x7fELF\x02\x01\x01'
            header[8:11] = b'AI\x02'
            image.write_bytes(header)
            with self.assertRaisesRegex(ValueError, 'x86_64 Linux'):
                package.package_installer(image, None, ROOT, config)
            header[18:20] = b'\x3e\x00'
            image.write_bytes(header)
            (work / 'SHA256SUMS.txt').write_text('wrong')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                package.package_installer(image, None, ROOT, config)


class MacUpdates(unittest.TestCase):
    def test_offline_signing_rejects_modified_archive_and_wrong_key(self):
        config = json.loads((ROOT / 'desktop/tauri.conf.json').read_bytes())
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            archive = work / f"OpenRoom-{config['version']}-macos-arm64.app.tar.gz"
            plist = plistlib.dumps({'CFBundleIdentifier': config['identifier'],
                                    'CFBundleShortVersionString': config['version'],
                                    'LSMinimumSystemVersion': '26.0'})
            with tarfile.open(archive, 'w:gz') as output:
                info = tarfile.TarInfo('OpenRoom.app/Contents/Info.plist')
                info.size = len(plist)
                output.addfile(info, io.BytesIO(plist))
            notes = work / 'notes.txt'
            notes.write_text('Test update')
            args = ['package-update', str(archive), '--notes', str(notes), '--output', str(work / 'signed')]
            key = work / 'wrong.key'
            key.touch()
            Path(str(key) + '.pub').write_text('wrong public key')
            checksum = work / 'SHA256SUMS.txt'
            checksum.write_text('incorrect')
            with patch.object(sys, 'argv', args), patch.dict(os.environ, {'TAURI_SIGNING_PRIVATE_KEY_PATH': str(key)}):
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    package.main()
                checksum.write_text(package.checksum_line(archive))
                with self.assertRaisesRegex(ValueError, 'Signing key does not match'):
                    package.main()
            self.assertFalse((work / 'signed').exists())


if __name__ == '__main__':
    unittest.main()
