#!/usr/bin/env python3
"""Prepare a target-channel promotion from verified published assets; never publish."""
import argparse
import copy
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


release = module('channel_release', 'release-automation.py')
installer = module('channel_installer', 'install.py')
LINUX = 'x86_64-unknown-linux-gnu'
WINDOWS = 'x86_64-pc-windows-msvc'
MACOS = 'aarch64-apple-darwin'


def promote(previous, updates):
    """Preserve other targets; refuse rollback and same-version byte replacement."""
    previous = installer.parse_channel(release.json_bytes(previous))
    installer.parse_channel(release.json_bytes({'schema_version': 1, 'targets': updates}))
    if not updates or any(value['policy'] != 'current' for value in updates.values()):
        raise ValueError('promotion requires verified current target records')
    result = copy.deepcopy(previous)
    changed = []
    for target, new in sorted(updates.items()):
        old = result['targets'].get(target, {'policy': 'unavailable'})
        if old['policy'] != 'unavailable':
            before, after = installer.version_tuple(old['version']), installer.version_tuple(new['version'])
            if after < before:
                raise ValueError('channel promotion cannot downgrade ' + target)
            if after == before and old != new:
                raise ValueError('same-version channel bytes/policy differ for ' + target)
        if old != new:
            result['targets'][target] = copy.deepcopy(new)
            changed.append(target)
    installer.parse_channel(release.json_bytes(result))
    return result, changed


def target_records(directory, descriptor):
    """Called only after full sealed-asset validation and public verification."""
    schema = descriptor['schema_version']
    if schema not in (3, 4):
        raise ValueError('channel promotion needs a current Linux/Windows release profile')
    targets = {LINUX: ('flere', 'flere-connect'), WINDOWS: ('flere-connect',)}
    if schema == 4:
        targets[MACOS] = ('flere', 'flere-connect')
    result = {}
    for target, components in targets.items():
        pins = {}
        for component in components:
            name = f'{component}-{target}.manifest.json'
            raw = release.read(directory / name, 65536)
            pin = {'bytes': len(raw), 'sha256': release.sha(raw)}
            if descriptor['assets'].get(name) != pin:
                raise ValueError('channel manifest differs from sealed asset')
            manifest = json.loads(raw)
            build, source = manifest['build'], manifest['source']
            if (build['component'] != component or build['target'] != target
                    or build['package_version'] != descriptor['version']
                    or source['git_commit'] != descriptor['commit']
                    or source['source_sha256'] != descriptor['source_sha256']):
                raise ValueError('channel manifest identity differs from selected release')
            pins[component] = pin
        result[target] = {'policy': 'current', 'version': descriptor['version'], 'manifests': pins}
    installer.parse_channel(release.json_bytes({'schema_version': 1, 'targets': result}))
    return result


def prepare(directory, previous, previous_sha256, output, version, commit, run_id,
            workflow_sha, descriptor_sha256):
    for value in (previous_sha256, descriptor_sha256):
        if not isinstance(value, str) or not re.fullmatch('[0-9a-f]{64}', value):
            raise ValueError('exact previous-channel and release-descriptor hashes are required')
    raw_previous = release.read(previous, 8192)
    if release.sha(raw_previous) != previous_sha256:
        raise ValueError('previous channel changed; reconcile before preparing promotion')
    old = installer.parse_channel(raw_previous)
    output = output.absolute()
    cache = Path.home() / '.cache/flere/tmp'
    if (not output.is_relative_to(cache) or output == cache or output.resolve() != output
            or output.is_relative_to(directory.absolute()) or output.exists() or output.is_symlink()):
        raise ValueError('promotion output must be a fresh separate private home-cache directory')
    release.validate(directory, version, commit, run_id, workflow_sha, descriptor_sha256)
    # Anonymous verification checks all final published bytes before creating any output.
    release.verify_public(directory, version, commit, run_id, workflow_sha, descriptor_sha256)
    final_descriptor = release.read(directory / 'release.json', 65536)
    if release.sha(final_descriptor) != descriptor_sha256:
        raise ValueError('release descriptor changed during public verification')
    descriptor = json.loads(final_descriptor)
    selected = target_records(directory, descriptor)
    promoted, changed = promote(old, selected)
    raw = release.json_bytes(promoted)
    if release.read(previous, 8192) != raw_previous:
        raise ValueError('previous channel changed during public verification')
    receipt = {'schema_version': 1, 'status': 'prepared_not_published',
               'version': version, 'commit': commit, 'run_id': run_id, 'workflow_sha': workflow_sha,
               'release_descriptor_sha256': descriptor_sha256,
               'previous_sha256': previous_sha256, 'channel_sha256': release.sha(raw),
               'changed_targets': changed, 'selected': selected,
               'publication_precondition': 'Compare the current channel to previous_sha256 before a signed commit.'}
    output.mkdir(mode=0o700, parents=True)
    (output / 'stable.json').write_bytes(raw)
    (output / 'promotion.json').write_bytes(release.json_bytes(receipt))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('directory', 'previous', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('previous-sha256', 'version', 'commit', 'run-id', 'workflow-sha', 'descriptor-sha256'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    result = prepare(args.directory, args.previous, args.previous_sha256, args.output,
                     args.version, args.commit, args.run_id, args.workflow_sha, args.descriptor_sha256)
    print(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print('Channel preparation stopped: ' + str(error), file=sys.stderr)
        sys.exit(1)
