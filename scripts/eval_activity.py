#!/usr/bin/env python3
"""Linux active-terminal scaling probe using copied Python, never native models."""
import argparse
import json
from pathlib import Path
import re
import shutil
import struct
import time
import uuid

from eval_arrivals import sample as sample_arrivals
from eval_coordination import Agent
from eval_runtime import Watcher, resources
from eval_support import Fixture, distribution, encode, save_report


STANDIN = r'''
import json, os, pathlib, sys, time, uuid
root = pathlib.Path.cwd()
run = os.environ['FLERE_RUN']
conversation = str(uuid.UUID(hex=run))
transcript = root / ('rollout-' + run + '.jsonl')
transcript.write_text(json.dumps({'type':'session_meta', 'payload':{
    'id':conversation, 'cwd':str(root), 'source':'cli'}}) + '\n')
held = transcript.open()
fillers = [open('/dev/null') for _ in range(int(sys.argv[1]))]
busy = sys.argv[2] == 'busy'
rate = float(sys.argv[3])
for number in range(60000):
    screen = '\x1b[2J\x1b[HFIXTURE_READY OUTPUT_%06d\r\n' % number
    if busy:
        screen += '• Working (1s • esc to interrupt)\r\n\r\n'
    screen += '› '
    sys.stdout.write(screen)
    sys.stdout.flush()
    time.sleep(1 / rate if busy else 600)
'''


def snapshot_tabs(fixture):
    """Read the version-4 metadata prefix; no terminal escapes are executed."""
    data = fixture.request('snapshot')
    offset = 0

    def read(size):
        nonlocal offset
        part = data[offset:offset + size]
        if len(part) != size:
            raise ValueError('short snapshot metadata')
        offset += size
        return part

    def number():
        return struct.unpack('>Q', read(8))[0]

    def string():
        return read(number()).decode()

    assert read(1) == b'\x04', 'unexpected snapshot version'
    epoch = string()
    number()  # generation
    number()  # focused workspace
    number()  # focused tab
    tabs = {}
    for _ in range(number()):
        workspace = number()
        string()  # name
        string()  # cwd
        meta = json.loads(string())
        for _ in range(number()):
            session, run, pid = number(), string(), number()
            alive = read(1) != b'\x00'
            string()  # title
            string()  # kind
            string()  # path
            working = read(1) != b'\x00'
            tabs[session] = {'workspace': workspace, 'run': run, 'pid': pid,
                             'alive': alive, 'working': working,
                             'conversations': meta['conversations']}
    return epoch, tabs


def run(binary, agents, descriptors, messages, activity, rate, seconds, repeats, context_interval,
        arrival_rate=0, arrival_seed=0, max_inflight=16):
    if arrival_rate and context_interval:
        raise ValueError('arrival mode currently requires context_interval=0 to isolate the sampler')
    if not Path('/proc/self/stat').exists():
        raise RuntimeError('this resource probe requires Linux /proc')
    with Fixture(binary) as fixture:
        # The basename exercises native discovery. The contents are system Python;
        # the command runs only this synthetic program and never a model harness.
        executable = fixture.root / 'codex'
        shutil.copyfile('/usr/bin/python3', executable)
        executable.chmod(0o700)
        script = fixture.root / 'standin.py'
        script.write_text(STANDIN)
        (fixture.state / 'harnesses.json').write_bytes(encode([{'name': 'codex', 'command': [
            str(executable), str(script), str(descriptors), activity, str(rate)]}]))
        actors = [Agent(fixture, f'active fixture {i}') for i in range(agents)]
        owner = actors[0]
        fixture.request('focus', owner.workspace, owner.tab['id'])
        ids = []
        for i in range(messages):
            value = fixture.json('coordinate', owner.workspace, 'send_message', encode({
                'to': owner.workspace, 'body': f'retained synthetic {i}: ' + 'x' * 15000}).hex())
            ids.append(value['message']['id'])
        if ids:
            fixture.request('coordinate', owner.workspace, 'inbox', encode({'ack_ids': ids}).hex())
        expected = {a.tab['id']: a for a in actors}

        def verified():
            epoch, tabs = snapshot_tabs(fixture)
            assert set(tabs) == set(expected), 'unexpected or missing fixture terminals'
            for session, actor in expected.items():
                tab = tabs[session]
                assert tab['alive'] and tab['run'] == actor.tab['run'] and tab['pid'] == actor.tab['pid']
                assert tab['workspace'] == actor.workspace
                assert tab['working'] == (activity == 'busy'), 'native activity was not recognized'
                assert any(c['uuid'] == str(uuid.UUID(hex=actor.tab['run']))
                           for c in tab['conversations']), 'native conversation was not recognized'
            return epoch

        deadline = time.monotonic() + 8
        while True:
            try:
                epoch = verified()
                break
            except AssertionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.05)
        time.sleep(1.1)  # let persisted tab layouts settle after discovery
        store = fixture.state / 'workspaces.v2.json'
        original_records = json.loads(store.read_bytes())['coordination']['messages']

        def output_indices():
            values = []
            for actor in actors:
                screen = fixture.json('capture', actor.tab['id'], actor.tab['run'], 20)['text']
                found = re.findall(r'OUTPUT_(\d+)', screen)
                assert found, 'fixture output marker missing'
                values.append(int(found[-1]))
            return values

        cases = []
        for repeat in range(repeats):
            scenarios = [('detached', None), ('links', 'watch-links')]
            if repeat % 2:
                scenarios.reverse()
            for label, command in scenarios:
                watcher = Watcher(fixture, command) if command else None
                try:
                    time.sleep(.25)
                    first_output = output_indices()
                    before_store = store.stat()
                    expected_ping = fixture.request('ping') if arrival_rate else None
                    cpu_before, _ = resources(fixture.server.pid)
                    begin = time.monotonic()
                    started_wall_time = time.time()
                    next_context = begin
                    ping_ms, ping_offsets, context_ms, context_offsets, rss = [], [], [], [], []
                    context_bytes = calls = 0
                    arrivals = None
                    if arrival_rate:
                        rss.append(resources(fixture.server.pid)[1])
                        arrivals = sample_arrivals(fixture.state / 'control.sock', expected_ping,
                            seconds=seconds, rate=arrival_rate, seed=arrival_seed,
                            max_inflight=max_inflight)
                        rss.append(resources(fixture.server.pid)[1])
                        good = [s for s in arrivals['samples'] if s['status'] == 'ok']
                        ping_ms = [s['completion_ms'] - s['scheduled_ms'] for s in good]
                        ping_offsets = [s['scheduled_ms'] for s in good]
                    else:
                        while time.monotonic() - begin < seconds:
                            start = time.monotonic()
                            fixture.request('ping')
                            ping_ms.append((time.monotonic() - start) * 1000)
                            ping_offsets.append((start - begin) * 1000)
                            rss.append(resources(fixture.server.pid)[1])
                            if context_interval and time.monotonic() >= next_context:
                                actor = actors[calls % agents]
                                context_start = time.monotonic()
                                value = actor.call('get_context')
                                assert value['workspace']['id'] == actor.workspace
                                assert value['messaging']['run'] == actor.tab['run']
                                context_ms.append(actor.metrics[-1]['ms'])
                                context_offsets.append((context_start - begin) * 1000)
                                context_bytes += actor.metrics[-1]['mcp_response_bytes']
                                calls += 1
                                next_context = time.monotonic() + context_interval
                            time.sleep(.02)
                    elapsed = time.monotonic() - begin
                    cpu_after, _ = resources(fixture.server.pid)
                    after_store = store.stat()
                    advanced = [last - first for first, last in zip(first_output, output_indices())]
                    if activity == 'busy':
                        assert min(advanced) >= elapsed * rate * .5, 'a terminal stopped producing output'
                    assert verified() == epoch, 'fixture identity changed'
                    if watcher:
                        assert watcher.frames >= 1, 'watcher received no frames'
                        if activity == 'busy':
                            assert watcher.frames > 1, 'busy watcher stopped receiving output'
                    cases.append({'scenario': label, 'repeat': repeat, 'seconds': elapsed, 'started_wall_time': started_wall_time,
                        'supervisor_cpu_percent_one_core': (cpu_after - cpu_before) / elapsed * 100,
                        'max_sampled_supervisor_rss_bytes': max(rss), 'ping': distribution(ping_ms) if ping_ms else None,
                        'arrival_probe': arrivals,
                        'ping_samples': [{'offset_ms': offset, 'ms': duration}
                                         for offset, duration in zip(ping_offsets, ping_ms)],
                        'context': {'calls': calls, 'mcp_response_bytes': context_bytes,
                                    'latency': distribution(context_ms) if context_ms else None,
                                    'samples': [{'offset_ms': offset, 'ms': duration}
                                                for offset, duration in zip(context_offsets, context_ms)]},
                        'output_updates_per_terminal': advanced,
                        'store_changed': (before_store.st_ino, before_store.st_mtime_ns) !=
                                         (after_store.st_ino, after_store.st_mtime_ns),
                        'frames': watcher.frames if watcher else 0,
                        'wire_bytes': watcher.bytes if watcher else 0})
                    p95 = f'{cases[-1]["ping"]["p95_ms"]:.2f}' if ping_ms else 'none'
                    print(f'{activity} {agents} terminals {label}: CPU={cases[-1]["supervisor_cpu_percent_one_core"]:.2f}% '
                          f'ping_p95={p95}ms', flush=True)
                finally:
                    if watcher:
                        watcher.close()
        saved = json.loads(store.read_bytes())['coordination']['messages']
        assert saved == original_records, 'retained message evidence changed'
        assert len(saved) == messages and all(m['acknowledged'] is not None for m in saved)
        return {'schema': 2, 'provenance': fixture.provenance(), 'supervisor_pid': fixture.server.pid, 'fixture': {
            'agents': agents, 'extra_open_descriptors_per_process': descriptors,
            'retained_acknowledged_messages': messages, 'activity': activity, 'updates_per_second': rate,
            'context_interval_seconds': context_interval, 'arrival_rate_per_second': arrival_rate,
            'arrival_seed': arrival_seed, 'max_inflight': max_inflight}, 'cases': cases,
            'method': 'Owned Linux supervisor and copied Python native stand-ins with synthetic open rollout metadata. '
                      'Verify exact live runs, recognized conversations/activity and continued output. '
                      'CPU/RSS include only the supervisor; context call timing includes MCP startup. '
                      'Arrival-mode RSS is sampled at interval endpoints only; inspect all arrival failure counts. '
                      'No model, network, user data, UI renderer or visible typing latency. No durable history pruning.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--agents', type=int, default=16)
    parser.add_argument('--descriptors', type=int, default=32)
    parser.add_argument('--messages', type=int, default=64)
    parser.add_argument('--activity', choices=['idle', 'busy'], default='busy')
    parser.add_argument('--rate', type=float, default=8)
    parser.add_argument('--seconds', type=float, default=5)
    parser.add_argument('--repeats', type=int, default=2)
    parser.add_argument('--context-interval', type=float, default=.2)
    parser.add_argument('--arrival-rate', type=float, default=0)
    parser.add_argument('--arrival-seed', type=int, default=0)
    parser.add_argument('--max-inflight', type=int, default=16)
    args = parser.parse_args()
    if not ((args.arrival_rate == 0 or 1 <= args.arrival_rate <= 500)
            and 1 <= args.max_inflight <= 64):
        parser.error('arrival-rate must be 0 or 1..500; max-inflight must be 1..64')
    if args.arrival_rate and args.context_interval:
        parser.error('arrival mode requires --context-interval 0')
    if not (1 <= args.agents <= 64 and 0 <= args.descriptors <= 128 and 0 <= args.messages <= 300
            and .5 <= args.rate <= 30 and 2 <= args.seconds <= 30 and 1 <= args.repeats <= 5
            and (args.context_interval == 0 or .05 <= args.context_interval <= 10)):
        parser.error('use agents 1..64, descriptors 0..128, messages 0..300, rate .5..30, '
                     'seconds 2..30, repeats 1..5, context-interval 0 or .05..10')
    result = run(args.binary, args.agents, args.descriptors, args.messages,
                 args.activity, args.rate, args.seconds, args.repeats, args.context_interval,
                 args.arrival_rate, args.arrival_seed, args.max_inflight)
    save_report(args.output, result)
    if any(c['arrival_probe'] and not c['arrival_probe']['all_arrivals_succeeded'] for c in result['cases']):
        raise SystemExit('arrival probe recorded failures or missed arrivals; inspect the saved report')
