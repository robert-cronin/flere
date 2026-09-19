#!/usr/bin/env python3
"""Check capacity evidence against corrupted history and false ACK receipts."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

import eval_storage as probe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    binary = str(Path(parser.parse_args().binary).resolve(strict=True))
    small_headroom = probe.STORE_LIMIT - 65536
    result = probe.run_case(binary, small_headroom)
    assert result['all_completion_operations_available']
    assert result['cold_restart_preserved'] and result['records_and_provenance_preserved']
    print('Below-cap fixture preserves exact records, progress, ACK and cold restart', flush=True)

    original = probe.human
    for fault, expected in [('history', 'retrieved history differs from exact oracle'),
                            ('ack', 'ACK receipt lacks durable acknowledgement'),
                            ('durable', 'durable evidence differs from exact oracle')]:
        changed = False

        def human(fixture, workspace, operation, args=None):
            nonlocal changed
            args = args or {}
            if fault == 'ack' and operation == 'inbox' and args.get('ack_ids'):
                # Fabricate a successful transport receipt while doing no write.
                changed = True
                return {'acknowledged': args['ack_ids'], 'pending': 0}
            result = original(fixture, workspace, operation, args)
            if not changed and operation == 'inbox' and args.get('include_acknowledged'):
                changed = True
                if fault == 'history':
                    result['messages'][0]['body'] += 'corrupted synthetic evidence'
                elif fault == 'durable':
                    # Keep JSON valid and memory intact; only the saved evidence changes.
                    path = fixture.state / 'workspaces.v2.json'
                    state = json.loads(path.read_bytes())
                    state['coordination']['messages'][0]['body'] += 'corrupted synthetic evidence'
                    path.write_bytes(probe.encode(state))
            return result

        with patch.object(probe, 'human', human):
            try:
                probe.run_case(binary, small_headroom)
            except AssertionError as error:
                assert str(error) == expected, (fault, str(error))
            else:
                raise AssertionError('capacity probe accepted corrupted ' + fault)
        assert changed, 'fault was not exercised'
        print('Rejected ' + fault + ' despite otherwise healthy responses', flush=True)

    # The diagnostic normally records pressure failures. Its optional acceptance
    # mode must retain that evidence and return failure when progress is blocked.
    root = Path.home() / '.cache/flere/evals'
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.TemporaryDirectory(prefix='capacity-control-', dir=root) as directory:
        output = Path(directory) / 'report.json'
        completed = subprocess.run([sys.executable, str(Path(probe.__file__).resolve()),
                                    binary, str(output), '--headroom', '0', '--require-completion'],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
        assert completed.returncode == 1, completed.stderr.decode()
        assert b'evidence saved' in completed.stderr
        report = json.loads(output.read_bytes())
        case, = report['cases']
        assert case['initial_store_bytes'] == probe.STORE_LIMIT
        assert not case['all_completion_operations_available']
        assert all(not op['accepted'] and op['store_growth_bytes'] == 0 for op in case['operations'])
        assert case['records_and_provenance_preserved'] and case['cold_restart_preserved']
        print('At-cap mode reports unavailable progress after retaining rollback/restart evidence', flush=True)


if __name__ == '__main__':
    main()
