#!/usr/bin/env python3
"""Verify multi-tab observation convergence under independent socket arrivals."""
import argparse
import json
from pathlib import Path
import shutil
import threading
import time
import uuid

from eval_activity import snapshot_tabs
from eval_arrivals import sample
from eval_coordination import Agent
from eval_support import Fixture, encode, save_report


PROGRAM = r'''
import json, os, pathlib, sys, time, uuid
root = pathlib.Path.cwd()
session = int(os.environ['FLERE_SESSION'])
files = [root / ('rollout-%s-%s.jsonl' % (session, phase)) for phase in (0, 1)]
for phase, file in enumerate(files):
    file.write_text(json.dumps({'type':'session_meta', 'payload':{
        'id':str(uuid.UUID(int=(session << 32) + phase)), 'cwd':str(root), 'source':'cli'}}) + '\n')
fillers = [open('/dev/null') for _ in range(int(sys.argv[1]))]
handles, previous = [], None
while True:
    phase = int((root / 'phase').read_text())
    if phase != previous:
        for handle in handles:
            handle.close()
        wanted = {0:[0], 1:[1], 2:[0,1], 3:[0], 4:[]}[phase]
        handles = [files[index].open() for index in wanted]
        previous = phase
    print('\x1b[2J\x1b[HFIXTURE_READY OBSERVATION_READY_%s\n• Working (1s • esc to interrupt)\n\n› ' % phase, end='', flush=True)
    time.sleep(.05)
'''


def run(binary, agents, descriptors):
    with Fixture(binary) as fixture:
        executable = fixture.root / 'codex'
        shutil.copyfile('/usr/bin/python3', executable)
        executable.chmod(0o700)
        script = fixture.root / 'observation.py'
        script.write_text(PROGRAM)
        phase_file = fixture.root / 'phase'
        phase_file.write_text('0')
        (fixture.state / 'harnesses.json').write_bytes(encode([{'name':'codex', 'command':[
            str(executable), str(script), str(descriptors)]}]))
        actors = [Agent(fixture, f'observation fixture {i}') for i in range(agents)]
        original_epoch, _ = snapshot_tabs(fixture)
        expected = {a.tab['id']: a for a in actors}
        store = fixture.state / 'workspaces.v2.json'

        def verify(identity_phase, working):
            epoch, tabs = snapshot_tabs(fixture)
            assert epoch == original_epoch and set(tabs) == set(expected)
            saved = json.loads(store.read_bytes())
            by_workspace = {w['id']: w for w in saved['workspaces']}
            converged = True
            for session, actor in expected.items():
                tab = tabs[session]
                assert tab['alive'] and tab['run'] == actor.tab['run'] and tab['pid'] == actor.tab['pid']
                assert tab['workspace'] == actor.workspace
                wanted = str(uuid.UUID(int=(session << 32) + identity_phase))
                card = by_workspace[actor.workspace]
                converged &= tab['working'] == working
                converged &= card['tabs'][0]['conversation'] == wanted
                converged &= card['meta'].get('last_conversation', {}).get('uuid') == wanted
            assert saved['coordination']['messages'] == [], 'probe created unexpected mail'
            return converged

        def wait(identity_phase, working):
            start = time.monotonic()
            checks = 0
            while True:
                checks += 1
                if verify(identity_phase, working):
                    return {'convergence_ms': (time.monotonic() - start) * 1000,
                            'snapshot_checks': checks}
                if time.monotonic() - start > 5:
                    raise AssertionError('observation did not converge under concurrent traffic')
                time.sleep(.02)

        wait(0, True)
        cases = []
        for phase, identity_phase, working in [(1,1,True), (2,1,False), (3,0,True), (4,0,False)]:
            reports, errors = [], []
            expected_ping = fixture.request('ping')
            def traffic():
                try:
                    reports.append(sample(fixture.state / 'control.sock', expected_ping,
                                          seconds=3, rate=100, seed=19, max_inflight=16))
                except BaseException as error:
                    errors.append(error)
            thread = threading.Thread(target=traffic)
            thread.start()
            try:
                changed = fixture.root / 'phase-next'
                changed.write_text(str(phase))
                phase_started_wall_time = time.time()
                changed.replace(phase_file)
                started = time.monotonic()
                # Require every stand-in to have changed its actual open handles.
                while True:
                    ready = all(f'OBSERVATION_READY_{phase}' in fixture.json(
                        'capture', a.tab['id'], a.tab['run'], 4)['text'] for a in actors)
                    if ready:
                        break
                    if time.monotonic() - started > 5:
                        raise AssertionError('fixture producer did not switch phase')
                    time.sleep(.02)
                producer_ms = (time.monotonic() - started) * 1000
                result = wait(identity_phase, working)
                result['total_phase_ms'] = (time.monotonic() - started) * 1000
                result['producer_ready_ms'] = producer_ms
            finally:
                thread.join(timeout=6)
                if thread.is_alive():
                    raise AssertionError('arrival probe did not stop')
            if errors:
                raise errors[0]
            assert verify(identity_phase, working)
            cases.append({'phase':phase, 'expected_working':working, **result,
                          'started_wall_time':phase_started_wall_time, 'finished_wall_time':time.time(),
                          'traffic':reports[0]})
        saved = json.loads(store.read_bytes())
        for actor in actors:
            card = next(w for w in saved['workspaces'] if w['id'] == actor.workspace)
            assert {c['uuid'] for c in card['meta']['conversations']} == {
                str(uuid.UUID(int=(actor.tab['id'] << 32) + p)) for p in (0,1)}
        return {'schema':1, 'provenance':fixture.provenance(), 'fixture':{
            'agents':agents, 'extra_descriptors':descriptors}, 'cases':cases,
            'method':'Owned copied-Python terminals switch their actual open rollout files. '
                     'Verify exact identities, persisted conversations, ambiguous/missing metadata '
                     'and continued observation under 100 independent ping arrivals/sec plus output/snapshot traffic. '
                     'Phase convergence is observed at snapshot-read granularity; not an absolute scheduling bound. '
                     'Inspect every traffic failure/miss count. No model, native user input, network or live state.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--agents',type=int,default=16)
    parser.add_argument('--descriptors',type=int,default=128)
    args = parser.parse_args()
    if not (1 <= args.agents <= 64 and 0 <= args.descriptors <= 128):
        parser.error('use agents 1..64 and descriptors 0..128')
    result = run(args.binary,args.agents,args.descriptors)
    save_report(args.output,result)
    print(json.dumps([{'phase':c['phase'],'total_phase_ms':c['total_phase_ms'],
                      'counts':c['traffic']['counts']} for c in result['cases']]))
    if not all(c['traffic']['all_arrivals_succeeded'] for c in result['cases']):
        raise SystemExit('traffic lost arrivals; inspect the saved report')
