#!/usr/bin/env python3
"""Verify that resource measurements cannot hide changed fixture sessions/state."""
import argparse
import json
from pathlib import Path
from unittest.mock import patch

import eval_runtime as probe
from eval_support import Fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    binary = str(Path(parser.parse_args().binary).resolve(strict=True))
    result = probe.run(binary, 1, 64, 1, 1)
    assert len(result['cases']) == 5
    assert all(c['session_identities_and_metadata_unchanged']
               and c['saved_layout_and_store_unchanged'] for c in result['cases'])
    print('Valid fixture preserves all five watcher scenarios and exact state', flush=True)

    result = probe.run(binary, 1, 64, 1, 1, arrival_rate=50, arrival_seed=7)
    assert len(result['cases']) == 5
    assert all(c['arrival_probe']['offered'] == 50
               and c['arrival_probe']['all_arrivals_succeeded']
               and c['session_identities_and_metadata_unchanged']
               and c['saved_layout_and_store_unchanged'] for c in result['cases'])
    print('Independent arrivals preserve every offered request and exact state', flush=True)

    original_request = Fixture.request
    for fault, expected_error in [('metadata', 'fixture identity or metadata changed'),
                                  ('store', 'saved layout or store changed')]:
        armed = changed = False

        def request(fixture, *fields):
            nonlocal armed, changed
            result = original_request(fixture, *fields)
            if fields[0] == 'save-tabs':
                armed = True
            elif fields[0] == 'ping' and armed and not changed:
                changed = True
                if fault == 'metadata':
                    inventory = json.loads(original_request(fixture, 'list'))
                    target = inventory['workspaces'][0]['id']
                    original_request(fixture, 'rename', target, b'changed fixture'.hex())
                else:
                    # Valid JSON with unchanged metadata still changes the exact store.
                    path = fixture.state / 'workspaces.v2.json'
                    path.write_bytes(b'\n' + path.read_bytes())
            return result

        with patch.object(Fixture, 'request', request):
            try:
                probe.run(binary, 1, 64, 1, 1)
            except RuntimeError as error:
                assert str(error) == expected_error, str(error)
            else:
                raise AssertionError('resource probe accepted changed ' + fault)
        assert changed, 'fault was not exercised'
        print('Changed ' + fault + ' rejected despite healthy output', flush=True)


if __name__ == '__main__':
    main()
