#!/usr/bin/env python3
"""Offline regressions; accepts an untouched extracted v0.3.3 source directory."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('support', HERE / 'test-support.py')
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)
SOURCE = Path(sys.argv.pop(1)).resolve()


class SupportTests(unittest.TestCase):
    def test_actual_release_fixture_scope_and_assertions(self):
        with tempfile.TemporaryDirectory(dir=os.environ['TEST_TMPDIR']) as tmp:
            root = Path(tmp) / 'source'
            shutil.copytree(SOURCE, root)
            original = {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            mapping = {key: '/nix/store/00000000000000000000000000000000-fixture/bin/' + key.rsplit('/', 1)[-1]
                       for key in support.TOOLS}
            mapping['/usr/bin:/bin'] = '/nix/store/00000000000000000000000000000000-fixture/bin'
            changed = support.adapt(root, mapping)
            self.assertGreater(len(changed), 20)
            for name, old in original.items():
                new = (root / name).read_bytes()
                if name not in changed:
                    self.assertEqual(new, old, name)
                else:
                    self.assertEqual(old.count(b'#[test]'), new.count(b'#[test]'), name)
                    self.assertEqual(old.count(b'assert'), new.count(b'assert'), name)
                    self.assertEqual(old.count(b'Duration::'), new.count(b'Duration::'), name)
                    if name in support.MODULES:
                        marker = support.MODULES[name].encode()
                        self.assertEqual(old.split(marker)[0], new.split(marker)[0], name)
            for script in ('probe.sh', 'receive.sh', 'cleanup.sh'):
                name = 'companion/src/bootstrap/' + script
                self.assertEqual(original[name], (root / name).read_bytes())
            nested = (root / 'tests/remote/mod.rs').read_text()
            self.assertIn('.arg("--target-dir").arg(root.join("target"))', nested.replace('\n            ', ''))
            self.assertIn('args(["build", "--offline", "--manifest-path"])', nested)
            adapted = (root / 'companion/src/bootstrap/tests.rs').read_text()
            self.assertIn('.arg(script.replace("/usr/bin/stat", ', adapted)
            self.assertNotIn('"/usr/bin:/bin"', adapted)

    def test_rejects_missing_or_unsafe_fixture_tool(self):
        with self.assertRaises(ValueError):
            support.adapt(SOURCE, {})
        mapping = dict.fromkeys(support.TOOLS, '/nix/store/good/bin/tool')
        mapping['/bin/sh'] = '/usr/bin/sh'
        with self.assertRaises(ValueError):
            support.adapt(SOURCE, mapping)

    def vendor(self, root, name, crates, lock):
        directory = root / name
        (directory / '.cargo').mkdir(parents=True)
        (directory / '.cargo/config.toml').write_text('same configuration\n')
        (directory / 'Cargo.lock').write_text(lock)
        for name, target in crates.items():
            (directory / name).symlink_to(target, target_is_directory=True)
        return directory

    def test_vendor_union_retains_exact_lock_and_rejects_conflict(self):
        with tempfile.TemporaryDirectory(dir=os.environ['TEST_TMPDIR']) as tmp:
            root = Path(tmp)
            for name in ('shared', 'core-only', 'companion-only', 'conflict'):
                (root / name).mkdir()
            core = self.vendor(root, 'core', {'shared-1': root / 'shared', 'core-1': root / 'core-only'}, 'CORE EXACT\n')
            companion = self.vendor(root, 'companion', {'shared-1': root / 'shared', 'companion-1': root / 'companion-only'}, 'COMPANION EXACT\n')
            for selected in (core, companion):
                output = root / ('out-' + selected.name)
                env = {**os.environ, 'core_deps': str(core), 'companion_deps': str(companion),
                       'selected_lock': str(selected / 'Cargo.lock'), 'out': str(output)}
                result = subprocess.run(['bash', str(HERE / 'merge-vendors.sh')], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual((output / 'Cargo.lock').read_bytes(), (selected / 'Cargo.lock').read_bytes())
                self.assertEqual({p.name for p in output.iterdir()}, {'.cargo', 'Cargo.lock', 'shared-1', 'core-1', 'companion-1'})
            (companion / 'shared-1').unlink()
            (companion / 'shared-1').symlink_to(root / 'conflict', target_is_directory=True)
            env['out'] = str(root / 'bad-output')
            self.assertNotEqual(subprocess.run(['bash', str(HERE / 'merge-vendors.sh')], env=env, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
