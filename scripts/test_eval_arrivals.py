"""Offline sampler controls: owned sockets, deliberate stalls, no Flere/model launch."""
import errno
from pathlib import Path
import selectors
import shutil
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from eval_arrivals import sample
from eval_support import frame


class Responder:
    def __init__(self, *, pause=False, response=b'pong', fragment=False, truncate=False):
        parent = Path.home() / '.cache/flere/evals'
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.root = Path(tempfile.mkdtemp(prefix='arrival-control-', dir=parent))
        self.path = self.root / 'control.sock'
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(self.path))
        self.listener.listen(128)
        self.listener.setblocking(False)
        self.stop = threading.Event()
        self.ready = threading.Event()
        self.pause, self.response = pause, response
        self.fragment, self.truncate = fragment, truncate
        self.origin = self.pause_begin = self.pause_end = None
        self.error = None
        self.thread = threading.Thread(target=self.run)
        self.thread.start()
        assert self.ready.wait(2), 'control responder did not become ready'

    def run(self):
        selector = selectors.DefaultSelector()
        selector.register(self.listener, selectors.EVENT_READ)
        clients = {}
        self.ready.set()
        try:
            while not self.stop.is_set():
                now = time.monotonic()
                if self.pause and self.origin is not None and now >= self.origin + .1:
                    self.pause_begin = now
                    self.stop.wait(.25)
                    self.pause_end = time.monotonic()
                    self.pause = False
                for key, mask in selector.select(.002):
                    stream = key.fileobj
                    if stream is self.listener:
                        stream, _ = self.listener.accept()
                        stream.setblocking(False)
                        clients[stream] = [bytearray(), 0]
                        selector.register(stream, selectors.EVENT_READ)
                        if self.origin is None:
                            self.origin = time.monotonic()
                        continue
                    request, written = clients[stream]
                    close = False
                    try:
                        if mask & selectors.EVENT_READ:
                            data = stream.recv(1024)
                            request.extend(data)
                            close = not data
                            if request == frame(b'ping'):
                                selector.modify(stream, selectors.EVENT_WRITE)
                        else:
                            packet = frame(self.response)
                            if self.truncate:
                                packet = packet[:3]
                            part = packet[written:written + 2] if self.fragment else packet[written:]
                            clients[stream][1] += stream.send(part)
                            close = clients[stream][1] == len(packet)
                    except BlockingIOError:
                        pass
                    except (BrokenPipeError, ConnectionResetError):
                        close = True
                    if close:
                        selector.unregister(stream)
                        stream.close()
                        del clients[stream]
        except BaseException as error:
            self.error = error
        finally:
            for stream in clients:
                stream.close()
            selector.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.stop.set()
        self.thread.join(timeout=2)
        self.listener.close()
        shutil.rmtree(self.root)
        assert not self.thread.is_alive(), 'control responder did not stop'
        if self.error:
            raise self.error


class ArrivalControls(unittest.TestCase):
    def probe(self, responder, **kwargs):
        return sample(responder.path, b'pong', seconds=.65, rate=100, seed=19,
                      max_lateness=.1, **kwargs)

    def test_pause_retains_independent_arrivals_and_latency(self):
        with Responder(pause=True) as responder:
            report = self.probe(responder, max_inflight=32)
        self.assertTrue(report['all_arrivals_succeeded'], report['counts'])
        start = report['started_monotonic']
        during = [s for s in report['samples'] if responder.pause_begin <
                  start + s['dispatch_ms'] / 1000 < responder.pause_end]
        self.assertGreaterEqual(len(during), 15)
        self.assertTrue(all(start + s['completion_ms'] / 1000 >= responder.pause_end for s in during))
        self.assertGreater(report['successful_arrival_to_response']['p95_ms'], 180)
        self.assertGreater(max(s['inflight_at_dispatch'] for s in during), 10)

    def test_saturation_counts_missed_arrivals_without_queue_or_retry(self):
        with Responder(pause=True) as responder:
            report = self.probe(responder, max_inflight=2)
        self.assertFalse(report['all_arrivals_succeeded'])
        self.assertGreater(report['counts']['missed_inflight'], 10)
        self.assertEqual(report['offered'], sum(report['counts'].values()))
        self.assertLessEqual(max(s['inflight_at_dispatch'] or 0 for s in report['samples']), 2)
        self.assertTrue(all(s['dispatch_ms'] is None for s in report['samples']
                            if s['status'] == 'missed_inflight'))

    def test_timeout_includes_entire_request(self):
        with Responder(pause=True) as responder:
            report = self.probe(responder, max_inflight=32, timeout=.05)
        self.assertGreater(report['counts']['timeout'], 5)
        self.assertFalse(report['all_arrivals_succeeded'])

    def test_partial_response_frames_are_reassembled(self):
        with Responder(fragment=True) as responder:
            report = self.probe(responder)
        self.assertTrue(report['all_arrivals_succeeded'], report['counts'])

    def test_wrong_and_truncated_responses_are_errors(self):
        for options in ({'response': b'wrong'}, {'truncate': True}):
            with self.subTest(options=options), Responder(**options) as responder:
                report = self.probe(responder)
            self.assertEqual(report['counts']['error'], report['offered'])
            self.assertIsNone(report['successful_arrival_to_response'])

    def test_seed_fixes_schedule_even_when_all_connections_fail(self):
        with Responder() as responder:
            path = responder.root / 'absent.sock'
            reports = [sample(path, b'pong', seconds=.1, rate=100, seed=7) for _ in range(2)]
        self.assertEqual([s['scheduled_ms'] for s in reports[0]['samples']],
                         [s['scheduled_ms'] for s in reports[1]['samples']])
        self.assertTrue(all(r['counts']['error'] == r['offered'] for r in reports))

    def test_client_stall_counts_late_arrivals_instead_of_bursting(self):
        factory = selectors.DefaultSelector
        class SlowSelector:
            def __init__(self):
                self.inner = factory()
                self.first = True
            def __getattr__(self, name):
                return getattr(self.inner, name)
            def select(self, timeout=None):
                if self.first:
                    self.first = False
                    time.sleep(.15)
                return self.inner.select(0 if timeout is None else timeout)
        with Responder() as responder:
            with patch('eval_arrivals.selectors.DefaultSelector', SlowSelector):
                report = sample(responder.path, b'pong', seconds=.4, rate=100,
                                max_lateness=.02, seed=5)
        self.assertGreater(report['counts']['missed_late'], 8)
        self.assertGreater(report['counts']['ok'], 10)
        self.assertEqual(report['offered'], sum(report['counts'].values()))

    def test_socket_resource_failures_are_counted(self):
        with patch('eval_arrivals.socket.socket', side_effect=OSError(errno.EMFILE, 'synthetic')):
            report = sample('unused', b'pong', seconds=.1, rate=100)
        self.assertEqual(report['counts']['error'], report['offered'])
        self.assertTrue(all(s['error'] == 'os_error_24' for s in report['samples']))

    def test_rejects_unbounded_settings(self):
        for options in ({'seconds': float('nan')}, {'rate': float('inf')},
                        {'max_inflight': 100}, {'timeout': 0}, {'max_lateness': -1}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                sample('unused', b'pong', **({'seconds': .2} | options))


if __name__ == '__main__':
    unittest.main()
