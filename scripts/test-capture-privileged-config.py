#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('inventory', Path(__file__).with_name('capture-privileged-config.py'))
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class InventoryTests(unittest.TestCase):
    def test_installer_is_source_of_truth(self):
        sources = inventory.installer_sources("config_links=(\n 'nix/x|/etc/nix/x'\n)\nconfig_copies=(\n 'bin/tool|/usr/local/bin/tool|0755'\n)")
        self.assertEqual(sources['/usr/local/bin/tool'], {'source': 'system-config/bin/tool', 'installation': 'copy', 'expected_mode': '0755'})
        self.assertEqual(sources['/etc/nix/x']['installation'], 'symlink')
        with self.assertRaises(ValueError):
            inventory.installer_sources('unsupported installer layout')

    def test_metadata_never_contains_contents_or_private_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for path in ['etc/local.conf', 'etc/owned.conf', 'etc/changed.conf', 'etc/NetworkManager/system-connections/PrivateSSID', 'etc/shadow', 'usr/local/bin/tool']:
                actual = root / path
                actual.parent.mkdir(parents=True, exist_ok=True)
                actual.write_text('TOP-SECRET-CONTENTS')
            (root / 'etc/external').symlink_to('/tmp', target_is_directory=True)
            result = inventory.capture(root, {'/usr/local/bin/tool': {'source': 'system-config/bin/tool', 'installation': 'copy'}},
                                       {'/etc/owned.conf': 'package', '/etc/changed.conf': 'package'}, {'/etc/changed.conf': 'modified'})
            entries = {entry['path']: entry for entry in result['files']}
            self.assertEqual(entries['/etc/local.conf']['classification'], 'unowned-local')
            self.assertEqual(entries['/etc/changed.conf']['classification'], 'package-backup')
            self.assertEqual(entries['/usr/local/bin/tool']['classification'], 'repo-managed')
            self.assertNotIn('/etc/owned.conf', entries)
            self.assertEqual(entries['/etc/external']['type'], 'symlink')
            serialized = json.dumps(result)
            self.assertNotIn('TOP-SECRET-CONTENTS', serialized)
            self.assertNotIn('PrivateSSID', serialized)
            self.assertNotIn('/etc/shadow', serialized)

    def test_pacman_status_parser(self):
        outputs = {'-Ql': 'foo /etc/test.conf\nfoo /etc/\n', '-Qii': 'Backup Files    : /etc/test.conf [modified]\n                  /etc/gone.conf [missing]\n'}
        owners, backups = inventory.pacman_metadata(outputs.__getitem__)
        self.assertEqual(owners['/etc/test.conf'], 'foo')
        self.assertEqual(backups['/etc/gone.conf'], 'missing')

    def test_missing_managed_file_reports_gap(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'etc').mkdir()
            (root / 'usr/local').mkdir(parents=True)
            result = inventory.capture(root, {'/etc/missing': {'source': 'system-config/missing'}}, {}, {})
            self.assertEqual(result['inspection_gaps'], ['/etc/missing'])


if __name__ == '__main__':
    unittest.main()
