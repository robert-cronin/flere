#!/usr/bin/env python3
"""Reject false native capacity results using independent provenance/retry/ACK faults."""
import argparse
from pathlib import Path
from unittest.mock import patch

from eval_coordination import Agent
import eval_native_storage as probe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    binary = str(Path(parser.parse_args().binary).resolve(strict=True))
    result = probe.run_case(binary, 4096, False)
    assert all(c['accepted'] for c in result['cases'])
    assert result['exact_records_identities_and_retry_ids_preserved']
    assert result['cold_restart_preserved_without_launch']
    print('Valid native fixture preserves read/ACK/retry/history and cold restart', flush=True)
    original = Agent.call
    for fault, expected in [
        ('provenance', 'native body/provenance differs from exact oracle'),
        ('retry', 'request retry created a different message'),
        ('ack', 'native ACK receipt lacks durable acknowledgement'),
    ]:
        changed = False

        def call(agent, name, arguments=None):
            nonlocal changed
            args = arguments or {}
            armed = (agent.fixture.state / 'workspaces.v2.json').stat().st_size >= probe.STORE_LIMIT - 4096
            if armed and fault == 'ack' and name == 'inbox' and args.get('ack_ids'):
                changed = True
                return {'acknowledged': args['ack_ids'], 'pending': 0}
            result = original(agent, name, args)
            if armed and not changed:
                if fault == 'provenance' and name == 'inbox' and not args:
                    result['messages'][0]['chat']['sender']['conversation'] = '00000000-0000-4000-8000-000000000000'
                    changed = True
                elif fault == 'retry' and name == 'send_chat_message':
                    result['message']['id'] = '0' * 32
                    changed = True
            return result

        with patch.object(Agent, 'call', call):
            try:
                probe.run_case(binary, 4096, False)
            except AssertionError as error:
                assert str(error) == expected, (fault, str(error))
            else:
                raise AssertionError('native pressure probe accepted false ' + fault)
        assert changed, 'native fault was not exercised'
        print('Rejected false ' + fault + ' with the real supervisor still running', flush=True)


if __name__ == '__main__':
    main()
