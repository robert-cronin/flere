"""Bounded arrival-driven Unix-socket latency sampling; no live-state discovery."""
import errno
import math
import random
import selectors
import socket
import struct
import time

from eval_support import distribution, frame


def sample(path, expected, *, seconds, rate=50, seed=0, max_inflight=16,
           timeout=2, max_lateness=.05):
    """One independent jittered arrival per time slot, with no waiting queue.

    The seed fixes the offered schedule, not the host's dispatch/completion times.
    Saturated and late arrivals remain in the report; they are never rescheduled.
    Each dispatched request has an absolute deadline including connect and write.
    """
    if not (math.isfinite(seconds) and .1 <= seconds <= 30
            and math.isfinite(rate) and 1 <= rate <= 500 and seconds * rate >= 1
            and isinstance(max_inflight, int) and 1 <= max_inflight <= 64
            and math.isfinite(timeout) and .01 <= timeout <= 5
            and math.isfinite(max_lateness) and .001 <= max_lateness <= 1):
        raise ValueError('unbounded arrival-probe configuration')
    rng = random.Random(seed)
    offsets = [(i + rng.random()) / rate for i in range(int(seconds * rate))]
    samples, active = [], {}
    selector = selectors.DefaultSelector()
    packet = frame(b'ping')
    wall_start = time.time()
    begin = time.monotonic()
    index = 0

    def finish(stream, status, error=None):
        state = active.pop(stream)
        selector.unregister(stream)
        stream.close()
        record = state['record']
        record['completion_ms'] = (time.monotonic() - begin) * 1000
        record['status'] = status
        if error is not None:
            record['error'] = error

    try:
        while index < len(offsets) or active or time.monotonic() < begin + seconds:
            deadlines = [state['deadline'] for state in active.values()]
            if index < len(offsets):
                deadlines.append(begin + offsets[index])
            elif time.monotonic() < begin + seconds:
                deadlines.append(begin + seconds)
            delay = max(0, min(deadlines) - time.monotonic()) if deadlines else 0
            for key, mask in selector.select(delay):
                stream = key.fileobj
                state = active[stream]
                try:
                    if time.monotonic() >= state['deadline']:
                        finish(stream, 'timeout')
                        continue
                    if mask & selectors.EVENT_WRITE:
                        error = stream.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)
                        if error:
                            raise OSError(error, 'connection failed')
                        state['written'] += stream.send(packet[state['written']:])
                        if state['written'] == len(packet):
                            selector.modify(stream, selectors.EVENT_READ)
                    if mask & selectors.EVENT_READ:
                        data = stream.recv(4096)
                        if not data:
                            finish(stream, 'error', 'eof')
                            continue
                        state['received'].extend(data)
                        received = state['received']
                        if len(received) >= 4:
                            size, = struct.unpack('>I', received[:4])
                            if size > 4092 or len(received) > size + 4:
                                finish(stream, 'error', 'invalid_frame')
                            elif len(received) == size + 4:
                                if received[4:] == expected:
                                    finish(stream, 'ok')
                                else:
                                    finish(stream, 'error', 'unexpected_response')
                except BlockingIOError:
                    pass
                except OSError as error:
                    finish(stream, 'error', f'os_error_{error.errno}')
            now = time.monotonic()
            for stream, state in list(active.items()):
                if now >= state['deadline']:
                    finish(stream, 'timeout')
            while index < len(offsets) and begin + offsets[index] <= time.monotonic():
                offset = offsets[index]
                now = time.monotonic()
                record = {'sequence': index, 'scheduled_ms': offset * 1000,
                          'observed_ms': (now - begin) * 1000, 'dispatch_ms': None,
                          'completion_ms': None, 'inflight_at_dispatch': None}
                index += 1
                samples.append(record)
                if now - begin - offset > max_lateness:
                    record['status'] = 'missed_late'
                    continue
                if len(active) >= max_inflight:
                    record['status'] = 'missed_inflight'
                    continue
                record['dispatch_ms'] = (now - begin) * 1000
                record['inflight_at_dispatch'] = len(active) + 1
                stream = None
                try:
                    stream = socket.socket(socket.AF_UNIX)
                    stream.setblocking(False)
                    error = stream.connect_ex(str(path))
                    if error not in (0, errno.EINPROGRESS, errno.EALREADY, errno.EINTR):
                        raise OSError(error, 'connect failed')
                    active[stream] = {'record': record, 'deadline': now + timeout,
                                      'written': 0, 'received': bytearray()}
                    selector.register(stream, selectors.EVENT_WRITE)
                except OSError as error:
                    active.pop(stream, None)
                    if stream is not None:
                        stream.close()
                    record.update(status='error', error=f'os_error_{error.errno}',
                                  completion_ms=(time.monotonic() - begin) * 1000)
    finally:
        for stream in active:
            stream.close()
        selector.close()
    elapsed = time.monotonic() - begin
    counts = {status: sum(s['status'] == status for s in samples)
              for status in ('ok', 'error', 'timeout', 'missed_late', 'missed_inflight')}
    good = [s for s in samples if s['status'] == 'ok']
    dispatched = [s for s in samples if s['dispatch_ms'] is not None]

    def stats(values):
        return distribution(values) if values else None

    assert len(samples) == len(offsets) == sum(counts.values())
    return {'schema': 1, 'seed': seed, 'configured_rate_per_second': rate,
            'arrival_seconds': seconds, 'observed_seconds_including_drain': elapsed,
            'started_wall_time': wall_start, 'started_monotonic': begin,
            'max_inflight': max_inflight, 'timeout_seconds': timeout,
            'max_dispatch_lateness_seconds': max_lateness,
            'offered': len(samples), 'dispatched': len(dispatched), 'counts': counts,
            'all_arrivals_succeeded': len(good) == len(samples),
            'offered_rate_per_second': len(samples) / seconds,
            'completion_rate_per_second_including_drain': len(good) / elapsed,
            'successful_arrival_to_response': stats([s['completion_ms'] - s['scheduled_ms'] for s in good]),
            'successful_dispatch_to_response': stats([s['completion_ms'] - s['dispatch_ms'] for s in good]),
            'dispatch_lateness': stats([s['dispatch_ms'] - s['scheduled_ms'] for s in dispatched]),
            'samples': samples,
            'method': 'Independent seeded jitter within each fixed arrival slot; no waiting queue. '
                      'Quantiles cover successful responses only; always inspect missed/error/timeout counts. '
                      'Dispatch lateness separates client scheduling from socket service delay. '
                      'This is local control-socket latency, not visible typing latency.'}
