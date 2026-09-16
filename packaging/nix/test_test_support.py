#!/usr/bin/env python3
"""Offline regressions; accepts an untouched extracted v0.3.5 source directory."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
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
            self.assertIn('.env_remove("RUSTFLAGS")\n            .env_remove("CARGO_BUILD_TARGET")', nested)
            self.assertIn('root.join("target/debug/flere-connect")', nested)
            # The literal-control regression remains exact; declared Dash
            # supplies its original non-bracketed Linux /bin/sh semantics.
            live = (root / 'tests/live.rs').read_text()
            begin = 'fn literal_cli_text_rejects_embedded_enter_or_escape() {'
            old_test = original['tests/live.rs'].decode().split(begin, 1)[1].split('\n#[test]', 1)[0]
            self.assertEqual(live.split(begin, 1)[1].split('\n#[test]', 1)[0], old_test)
            self.assertEqual(live.count(json.dumps(support.shell_return_marker(mapping['/bin/sh']))), 2)
            adapted = (root / 'companion/src/bootstrap/tests.rs').read_text()
            self.assertIn('.arg(script.replace("/usr/bin/stat", ', adapted)
            self.assertNotIn('"/usr/bin:/bin"', adapted)

    def test_shell_return_keeps_full_path_and_only_exact_wrap_boundaries(self):
        # Actual Nix failure: an 80-column capture split this immutable path.
        shell = '/nix/store/2ndah67h0z5m31v2wkdmg2md4380ggr5-bash-interactive-5.3p15/bin/sh'
        marker = support.shell_return_marker(shell)
        for status in (0, 1):
            text = (f'codex exited (exit status: {status}). Returning to /nix/store/'
                    '2ndah67h0z5m31v2wkdmg2md4\n380ggr5-bash-interactive-5.3p15/bin/sh.')
            capture = json.dumps({'text': text})
            self.assertIn(marker, capture)
            self.assertNotIn(marker, json.dumps({'text': text.replace('380ggr5', 'WRONG')}))
            self.assertNotIn(marker, json.dumps({'text': text.replace('md4\n3', 'md\n43')}))
        self.assertEqual(marker.replace('\\n', ''), 'Returning to ' + shell)
        self.assertEqual(support.shell_return_marker('/bin/sh'), 'Returning to /bin/sh')

    def test_published_source_already_contains_the_retained_historical_fix(self):
        patch = HERE / 'patches/wrapped-path-delimiter.patch'
        self.assertEqual(hashlib.sha256(patch.read_bytes()).hexdigest(),
                         '3e1bcd7b259d047974a85f2ff0cd8aae965e90207ceba1253e9637e8ef701275')
        name = 'src/ui/selection/paths.rs'
        with tempfile.TemporaryDirectory(dir=os.environ['TEST_TMPDIR']) as tmp:
            root = Path(tmp)
            target = root / name
            target.parent.mkdir(parents=True)
            target.write_bytes((SOURCE / name).read_bytes())
            self.assertEqual(hashlib.sha256(target.read_bytes()).hexdigest(),
                             '9d23e7f80cae847f2ef0fd93d85856613d80bbd98aa45a64d736866e1e6bf9c4')
            result = subprocess.run(['patch', '--batch', '--fuzz=0', '--reverse', '-p1', '-i', str(patch)],
                                    cwd=root, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(hashlib.sha256(target.read_bytes()).hexdigest(),
                             '5369d60d5d10db75a541a5861ec545bc72ce9e7d1cddf398d9c557808a691a6b')

    def test_current_locks_match_archive_and_historical_pair_stays_original(self):
        for component, name, archive, expected in (
            ('flere', 'core-Cargo.lock', 'Cargo.lock',
             '7a6a0ec936b4cd0a7fd82b85b08c5b2dbd52356fb6e8b8b8f0b8b9eebaa7c789'),
            ('flere-connect', 'companion-Cargo.lock', 'companion/Cargo.lock',
             '659e826726ecaadc8ea1a5b19fa95770e1ac286f4e4a3353b58416004003a0cb'),
        ):
            current = (HERE / 'locks' / name).read_bytes()
            previous = (HERE / 'locks' / ('previous-' + name)).read_bytes()
            self.assertEqual(current, (SOURCE / archive).read_bytes())
            self.assertEqual(hashlib.sha256(previous).hexdigest(), expected)
            old = tomllib.loads(previous.decode())['package']
            new = tomllib.loads(current.decode())['package']
            for packages, version in ((old, '0.3.3'), (new, '0.3.5')):
                own = [p for p in packages if p['name'] == component and 'source' not in p]
                self.assertEqual(len(own), 1)
                self.assertEqual(own[0]['version'], version)
            dependencies = lambda packages: {(p['name'], p['version'], p.get('source'), p.get('checksum'))
                                              for p in packages if 'source' in p}
            self.assertEqual(dependencies(old), dependencies(new))

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
