#!/usr/bin/env python3
"""Offline promotion regressions; no publication, payload execution or network."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('release_channel', Path(__file__).with_name('release-channel.py'))
channel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(channel)


def record(version='0.3.6', components=('flere', 'flere-connect')):
    return {'policy': 'current', 'version': version,
            'manifests': {name: {'bytes': 123, 'sha256': 'a' * 64} for name in components}}


class Promotion(unittest.TestCase):
    def previous(self):
        legacy = record('0.3.0')
        legacy['policy'] = 'legacy_unsigned'
        return {'schema_version': 1, 'targets': {
            channel.LINUX: record('0.3.5'), channel.MACOS: legacy,
            'x86_64-apple-darwin': {'policy': 'unavailable'}}}

    def test_upgrade_preserves_other_platforms_and_input(self):
        old = self.previous()
        saved = copy.deepcopy(old)
        result, changed = channel.promote(old, {channel.LINUX: record(),
            channel.WINDOWS: record(components=('flere-connect',))})
        self.assertEqual(old, saved)
        self.assertEqual(result['targets'][channel.MACOS], saved['targets'][channel.MACOS])
        self.assertEqual(result['targets']['x86_64-apple-darwin'], {'policy': 'unavailable'})
        self.assertEqual(set(changed), {channel.LINUX, channel.WINDOWS})

    def test_same_version_same_bytes_is_idempotent(self):
        old = self.previous()
        result, changed = channel.promote(old, {channel.LINUX: record('0.3.5')})
        self.assertEqual(result, old)
        self.assertEqual(changed, [])

    def test_no_downgrade_or_same_version_replacement(self):
        replacement = record('0.3.5')
        replacement['manifests']['flere']['sha256'] = 'b' * 64
        for entry in (record('0.3.4'), replacement):
            with self.assertRaises(ValueError):
                channel.promote(self.previous(), {channel.LINUX: entry})

    def test_invalid_inventory_and_policy_cannot_be_promoted(self):
        for updates in ({}, {channel.LINUX: {'policy': 'unavailable'}},
                        {channel.LINUX: record(components=('flere',))}, {'other': record()}):
            with self.assertRaises(ValueError):
                channel.promote(self.previous(), updates)

    def directory(self):
        parent = Path.home() / '.cache/flere/tmp'
        parent.mkdir(parents=True, exist_ok=True)
        temp = tempfile.TemporaryDirectory(prefix='channel-test-', dir=parent)
        self.addCleanup(temp.cleanup)
        return Path(temp.name)

    def test_descriptor_digest_failure_creates_no_output_or_network(self):
        root = self.directory()
        assets = root / 'assets'; assets.mkdir()
        (assets / 'release.json').write_bytes(b'{}')
        previous = root / 'previous.json'
        previous.write_bytes(channel.release.json_bytes(self.previous()))
        output = root / 'prepared'
        with mock.patch.object(channel.release, 'verify_public') as network:
            with self.assertRaisesRegex(ValueError, 'descriptor digest differs'):
                channel.prepare(assets, previous, channel.release.sha(previous.read_bytes()), output,
                    '0.3.6', 'a' * 40, '123', 'b' * 40, 'c' * 64)
            network.assert_not_called()
        self.assertFalse(output.exists())

    def test_descriptor_change_during_public_verification_is_rejected(self):
        root = self.directory()
        assets = root / 'assets'; assets.mkdir()
        descriptor = assets / 'release.json'; descriptor.write_bytes(b'{"schema_version":3}')
        original_digest = channel.release.sha(descriptor.read_bytes())
        previous = root / 'previous.json'; previous.write_bytes(channel.release.json_bytes(self.previous()))
        output = root / 'prepared'
        with mock.patch.object(channel.release, 'validate'), \
             mock.patch.object(channel.release, 'verify_public', side_effect=lambda *args: descriptor.write_bytes(b'{"schema_version":4}')):
            with self.assertRaisesRegex(ValueError, 'descriptor changed during public verification'):
                channel.prepare(assets, previous, channel.release.sha(previous.read_bytes()), output,
                    '0.3.6', 'a' * 40, '123', 'b' * 40, original_digest)
        self.assertFalse(output.exists())

    def test_changed_previous_channel_stops_before_asset_verification(self):
        root = self.directory()
        previous = root / 'previous.json'; previous.write_bytes(channel.release.json_bytes(self.previous()))
        with mock.patch.object(channel.release, 'validate') as assets:
            with self.assertRaisesRegex(ValueError, 'previous channel changed'):
                channel.prepare(root / 'assets', previous, '0' * 64, root / 'out',
                    '0.3.6', 'a' * 40, '123', 'b' * 40, 'c' * 64)
            assets.assert_not_called()

    def test_channel_pins_come_from_exact_manifest_bytes_and_identity(self):
        root = self.directory()
        descriptor = {'schema_version': 3, 'version': '0.3.6', 'commit': 'a' * 40,
                      'source_sha256': 'b' * 64, 'assets': {}}
        for target, components in ((channel.LINUX, ('flere', 'flere-connect')),
                                   (channel.WINDOWS, ('flere-connect',))):
            for component in components:
                name = f'{component}-{target}.manifest.json'
                raw = channel.release.json_bytes({'build': {'component': component, 'target': target,
                    'package_version': '0.3.6'}, 'source': {'git_commit': 'a' * 40, 'source_sha256': 'b' * 64}})
                (root / name).write_bytes(raw)
                descriptor['assets'][name] = {'bytes': len(raw), 'sha256': channel.release.sha(raw)}
        result = channel.target_records(root, descriptor)
        self.assertEqual(result[channel.LINUX]['manifests']['flere'], descriptor['assets'][f'flere-{channel.LINUX}.manifest.json'])
        self.assertEqual(set(result), {channel.LINUX, channel.WINDOWS})
        name = f'flere-connect-{channel.WINDOWS}.manifest.json'
        altered = json.loads((root / name).read_bytes()); altered['build']['package_version'] = '0.3.5'
        raw = channel.release.json_bytes(altered); (root / name).write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'differs from sealed asset'):
            channel.target_records(root, descriptor)
        descriptor['assets'][name] = {'bytes': len(raw), 'sha256': channel.release.sha(raw)}
        with self.assertRaisesRegex(ValueError, 'identity differs'):
            channel.target_records(root, descriptor)


if __name__ == '__main__':
    unittest.main()
