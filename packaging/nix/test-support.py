#!/usr/bin/env python3
"""Adapt only the pinned release's test fixtures to declared Nix tools."""
import hashlib
import json
from pathlib import Path
import re
import sys

MODULES = {
    'src/git.rs': '#[cfg(test)]\nmod tests {',
    'src/server/panes.rs': '#[cfg(test)]\nmod tests {',
    'src/server/transfers.rs': '#[cfg(test)]\nmod tests {',
    'src/ui/github.rs': '#[cfg(test)]\nmod tests {',
    'src/install/ownership.rs': '#[cfg(test)]\nmod tests {',
    'companion/src/ports.rs': '#[cfg(all(test, unix))]\nmod tests {',
    'companion/src/update.rs': '#[cfg(all(test, unix))]\nmod tests {',
}
TOOLS = {'/bin/sh', '/bin/bash', '/bin/zsh', '/bin/sleep',
         '/usr/bin/env', '/usr/bin/python3', '/usr/bin/vim', '/usr/bin/git',
         '/usr/bin/false', '/usr/bin/true', '/usr/bin/yes', '/usr/bin/printf',
         '/usr/bin/stat', '/usr/bin/sha256sum', '/usr/bin:/bin'}


def adapt(root, replacements):
    if set(replacements) != TOOLS:
        raise ValueError('fixture tool map changed')
    if any(not re.fullmatch(r'/nix/store/[a-z0-9-]+[^\s\"\'\\]*', value)
           for key, value in replacements.items() if key != '/usr/bin:/bin'):
        raise ValueError('fixture tool must be an absolute immutable store path')
    pattern = re.compile('|'.join(re.escape(key) for key in sorted(TOOLS, key=len, reverse=True)))
    paths = {p.relative_to(root).as_posix(): None for p in (root / 'tests').rglob('*')
             if p.suffix in {'.rs', '.py'}}
    paths.update(MODULES)
    paths['companion/src/bootstrap/tests.rs'] = None
    result = {}
    for name, marker in sorted(paths.items()):
        path = root / name
        old = path.read_text()
        if marker:
            if old.count(marker) != 1:
                raise ValueError(f'test module changed: {name}')
            prefix, text = old.split(marker)
            prefix += marker
        else:
            prefix, text = '', old
        new, count = pattern.subn(lambda m: replacements[m.group()], text)
        if name == 'companion/src/bootstrap/tests.rs':
            # Execute the simulated remote receiver with test tools. Embedded
            # production SSH scripts remain byte-for-byte unchanged.
            needle = '.arg(script)'
            if new.count(needle) != 1:
                raise ValueError('bootstrap shell fixture changed')
            new = new.replace(needle, '.arg(script.replace("/usr/bin/stat", ' +
                json.dumps(replacements['/usr/bin/stat']) + ').replace("/usr/bin/sha256sum", ' +
                json.dumps(replacements['/usr/bin/sha256sum']) + '))')
            count += 1
        if count:
            new = prefix + new
            path.write_text(new)
            result[name] = {'substitutions': count, 'before': hashlib.sha256(old.encode()).hexdigest(),
                            'after': hashlib.sha256(new.encode()).hexdigest()}
    # Three Linux clipboard tests are nested before unrelated production code.
    path = root / 'companion/src/os.rs'
    old = path.read_text()
    new = old
    for name in ('/bin/sleep', '/usr/bin/yes', '/usr/bin/printf'):
        needle = f'Command::new("{name}")'
        if new.count(needle) != 1:
            raise ValueError('clipboard test fixture changed')
        new = new.replace(needle, f'Command::new({json.dumps(replacements[name])})')
    path.write_text(new)
    result['companion/src/os.rs'] = {'substitutions': 3, 'before': hashlib.sha256(old.encode()).hexdigest(),
                                   'after': hashlib.sha256(new.encode()).hexdigest()}
    return result


if __name__ == '__main__':
    print(json.dumps(adapt(Path(sys.argv[1]), json.loads(Path(sys.argv[2]).read_text())), indent=2, sort_keys=True))
