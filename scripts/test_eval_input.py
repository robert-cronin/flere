#!/usr/bin/env python3
"""Run input-evaluator negative controls against an explicitly selected local core."""
import argparse
from pathlib import Path
from unittest.mock import patch

import eval_input as probe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    binary = str(Path(parser.parse_args().binary).resolve(strict=True))
    result = probe.run(binary, 2, 1, 64, 10, 1, echo=True)
    assert result['all_checks_passed'] and len(result['cases']) == 3
    assert all(c['echo_observed'] for c in result['cases'] if c['operation'] != 'ping')
    print('Valid fixture preserves bytes, echo, metadata and identities', flush=True)

    original = probe.read_frame
    calls = 0

    def wrong_once(stream):
        nonlocal calls
        value = original(stream)
        calls += 1
        return b'wrong synthetic acknowledgement' if calls == 1 else value

    with patch.object(probe, 'read_frame', wrong_once):
        result = probe.run(binary, 2, 1, 64, 10, 1)
    assert not result['all_checks_passed'] and len(result['cases']) == 1
    assert result['cases'][0]['errors'] == 1 and len(result['cases'][0]['samples']) == 10
    print('One wrong acknowledgement remains a measured failure', flush=True)

    original_frame = probe.frame

    def omit_payload(data):
        fields = data.split(b'\t')
        if fields[0] == b'input':
            fields[-1] = b''
            data = b'\t'.join(fields)
        return original_frame(data)

    with patch.object(probe, 'frame', omit_payload):
        result = probe.run(binary, 2, 1, 64, 10, 1)
    assert not result['all_checks_passed'] and len(result['cases']) == 2
    assert result['cases'][1]['errors'] == 0 and not result['cases'][1]['exact_child_input']
    print('Successful replies cannot hide missing child bytes', flush=True)

    original_audit_check = probe.audit_records_match

    def omit_last_record(before, after, epoch, tab, lengths):
        if lengths:
            after = before + b''.join(after[len(before):].splitlines(keepends=True)[:-1])
        return original_audit_check(before, after, epoch, tab, lengths)

    with patch.object(probe, 'audit_records_match', omit_last_record):
        result = probe.run(binary, 2, 1, 64, 10, 1)
    assert not result['all_checks_passed'] and len(result['cases']) == 2
    case = result['cases'][1]
    assert case['errors'] == 0 and case['exact_child_input'] and not case['audit_records_exact']
    print('Missing audit record fails despite successful replies and exact child bytes', flush=True)


if __name__ == '__main__':
    main()
