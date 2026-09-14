#!/usr/bin/env python3
"""Only open KVM, check its API, create one empty VM fd, and close owned fds."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import platform
import stat


def probe(scope, run_identity, host_network_namespace):
    result = {
        'schema': 'flere-nixos-kvm-probe-v1', 'status': 'blocked',
        'scope': scope, 'run_identity': run_identity,
        'uid': os.getuid(), 'gid': os.getgid(), 'groups': os.getgroups(),
        'machine': platform.machine(), 'kernel': platform.release(),
        'network_namespace': os.readlink('/proc/self/ns/net'),
        'vm_created': False, 'owned_fds_closed': False,
        'guest_memory_or_vcpus_created': False,
    }
    device = vm = None
    stage = 'identity'
    try:
        assert result['machine'] == 'x86_64', 'x86_64 required'
        assert result['uid'] != 0, 'ordinary runner/build uid required'
        if scope == 'sandbox':
            assert host_network_namespace
            assert result['network_namespace'] != host_network_namespace, 'network sandbox absent'
            assert stat.S_ISCHR(Path('/dev/ptmx').stat().st_mode), 'sandbox PTY absent'
        stage = 'device'
        info = Path('/dev/kvm').stat()
        result['device'] = {'mode': stat.S_IMODE(info.st_mode), 'uid': info.st_uid,
                            'gid': info.st_gid, 'major': os.major(info.st_rdev),
                            'minor': os.minor(info.st_rdev)}
        assert stat.S_ISCHR(info.st_mode), 'KVM path is not a character device'
        stage = 'open'
        device = os.open('/dev/kvm', os.O_RDWR | os.O_CLOEXEC)
        # Linux uapi/linux/kvm.h: _IO(KVMIO=0xae, 0x00/0x01), x86_64.
        stage = 'api'
        result['api_version'] = fcntl.ioctl(device, 0xAE00, 0)
        assert result['api_version'] == 12, 'unexpected KVM API version'
        stage = 'create_vm'
        vm = fcntl.ioctl(device, 0xAE01, 0)
        assert isinstance(vm, int) and vm >= 0, 'CREATE_VM did not return a file descriptor'
        result['vm_created'] = True
        result['status'] = 'passed'
    except OSError as error:
        result['blocker'] = {'stage': stage, 'errno': error.errno, 'message': error.strerror}
    except AssertionError as error:
        result['status'] = 'failed'
        result['error'] = {'stage': stage, 'message': str(error)}
    finally:
        for fd in (vm, device):
            if fd is not None:
                os.close(fd)
        result['owned_fds_closed'] = True
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scope', choices=('runner', 'sandbox'), required=True)
    parser.add_argument('--run-identity', required=True)
    parser.add_argument('--host-network-namespace', default='')
    args = parser.parse_args()
    # A blocked capability is data, so the surrounding job can retain both probes.
    # The controller exits nonzero unless both probes pass; no failed VM is accepted.
    print(json.dumps(probe(args.scope, args.run_identity, args.host_network_namespace), sort_keys=True))
