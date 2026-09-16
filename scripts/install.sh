#!/bin/sh
# Fetch the reviewed release installer; keep its package verification/ownership rules.
set -eu
if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'Flere needs Python 3.9 or newer. Install Python 3, then rerun this command.' >&2
    exit 1
fi
exec python3 -I - "$@" <<'PYTHON'
import sys
if sys.version_info < (3, 9):
    sys.exit("Flere needs Python 3.9 or newer; this bootstrap does not install system tools.")
import hashlib
import os
from pathlib import Path
import runpy
import signal
import tempfile
import urllib.parse
import urllib.request

URL = "https://raw.githubusercontent.com/robert-cronin/flere/48e1cf96e2bea721f7f305dae81f4c28b2daa385/scripts/install.py"
SHA256 = "f059237f2dab05aa7f1d27e541a278997500b4a62bebd64b2ea84effa64120db"
SIZE = 19659

class HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        parsed = urllib.parse.urlsplit(url)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            raise ValueError("installer redirected outside authenticated HTTPS")
        return super().redirect_request(request, fp, code, message, headers, url)

def timeout(signum, frame):
    raise TimeoutError("installer download exceeded 60 seconds")

def main():
    if os.geteuid() == 0:
        raise ValueError("run as your normal user, without sudo")
    home = Path(os.environ.get("HOME", ""))
    if not home.is_absolute() or not home.is_dir():
        raise ValueError("HOME must name your existing absolute home directory")
    home = home.resolve()
    cache = home / ".cache/flere/bootstrap"
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    if (cache.is_symlink() or not cache.resolve().is_relative_to(home)
            or cache.stat().st_uid != os.getuid() or cache.stat().st_mode & 0o077):
        raise ValueError("bootstrap cache must be private, user-owned, and inside HOME")
    with tempfile.TemporaryDirectory(prefix="install-", dir=cache) as directory:
        installer = Path(directory) / "install.py"
        previous = signal.signal(signal.SIGALRM, timeout)
        signal.alarm(60)
        try:
            opener = urllib.request.build_opener(HTTPSRedirect)
            with opener.open(URL, timeout=30) as response:
                data = response.read(SIZE + 1)
        finally:
            signal.alarm(0)
            signal.signal(signal.SIGALRM, previous)
        if len(data) != SIZE or hashlib.sha256(data).hexdigest() != SHA256:
            raise ValueError("published installer size/SHA-256 mismatch; nothing was executed")
        with installer.open("xb") as output:
            output.write(data)
        sys.argv = [str(installer), *sys.argv[1:]]
        runpy.run_path(str(installer), run_name="__main__")

try:
    main()
except (OSError, ValueError) as error:
    print(f"Flere bootstrap stopped: {error}", file=sys.stderr)
    sys.exit(1)
PYTHON
