"""One place for the suite's `openssl` calls (#2478).

Certificates for the SC tests come from the `openssl` CLI. Starting it is the
slow part on a cold Windows runner (the first call once passed ten seconds),
so the suite makes each certificate once per test session and gives every class
its own copy, and every command is timed.

* `site_authority().provide(dest, *names)` copies named certificates and keys
  into `dest`, making any that do not exist yet. The site CA, the foreign CA and
  every leaf are made at most once per process. A class still gets a private
  directory of its own copies, so a test may rewrite or delete files in its
  root without touching another class.
* A test that starts a child Python (`test_sc_npdu` does) leaves the session
  directory's path in `SESSION_ENV`, which the child adopts instead of making
  its own; the parent only waits while the child runs, so the two never write
  the directory together. The child never deletes it.
* `run(root, *args)` runs one command for fixtures that need their own
  material (a different lifetime or key type, or a per-test file); it is timed
  and reported the same way.
* A failed or timed-out command raises `OpenSslError` listing every command
  the session has run so far with its duration and stderr, so the next
  occurrence shows where the time went.

Nothing here is POSIX-only: paths come from `pathlib` and `tempfile`, files
are copied with `shutil`, and `openssl` is found on PATH as the suite always
did (on Windows, `openssl.exe` through the same lookup).
"""
import atexit
import os
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path

# A cold `openssl.exe` on a hosted Windows runner has been seen to pass ten
# seconds on its first start (#2478), and starts are cheap everywhere else, so
# this only bounds a command that has hung. The suite makes about two dozen
# calls, so even a slow runner stays well inside the job's time limit.
COMMAND_TIMEOUT = 60

SESSION_ENV = "BACNET_TEST_OPENSSL_SESSION"

LEAF_EXTENSIONS = (
    "[leaf]\nbasicConstraints=critical,CA:FALSE\n"
    "keyUsage=critical,digitalSignature\n"
    "extendedKeyUsage=serverAuth,clientAuth\n"
    "subjectAltName=DNS:localhost,IP:127.0.0.1\n"
)
EC = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"]

_lock = threading.RLock()
_commands = []  # (argv, seconds, outcome, stderr) for every command, in order


class OpenSslError(RuntimeError):
    """An `openssl` command failed or timed out; the message has the whole session's timings."""


def _text(data):
    if data is None:
        return ""
    if isinstance(data, bytes):
        data = data.decode("utf-8", "replace")
    return data.strip()


def session_report():
    """Every command run so far: number, seconds, outcome and stderr."""
    lines = [f"{len(_commands)} openssl command(s) this session, "
             f"{sum(seconds for _, seconds, _, _ in _commands):.2f} s in total:"]
    for number, (argv, seconds, outcome, stderr) in enumerate(_commands, 1):
        lines.append(f"  #{number} {seconds:7.2f} s  {outcome}  openssl {' '.join(argv[:4])}")
        if stderr:
            lines.append("      stderr: " + stderr[-400:].replace("\n", "\n              "))
    return "\n".join(lines)


def run(root, *args, timeout=COMMAND_TIMEOUT):
    """Run `openssl` in `root` and return its stdout bytes."""
    argv = [str(arg) for arg in args]
    started = time.monotonic()
    outcome, stderr = "ok", ""
    try:
        done = subprocess.run(["openssl", *argv], cwd=root, check=True,
                              capture_output=True, timeout=timeout)
        return done.stdout
    except subprocess.TimeoutExpired as exc:
        outcome, stderr = f"TIMEOUT after {timeout} s", _text(exc.stderr)
        failure = exc
    except subprocess.CalledProcessError as exc:
        outcome, stderr = f"exit {exc.returncode}", _text(exc.stderr)
        failure = exc
    except OSError as exc:  # not installed, or not startable
        outcome, stderr = f"{type(exc).__name__}: {exc}", ""
        failure = exc
    finally:
        _commands.append((argv, time.monotonic() - started, outcome, stderr))
    raise OpenSslError(f"openssl {' '.join(argv)} failed: {outcome}\n{session_report()}") from failure


_available = None


def require_openssl():
    """Skip the calling class when `openssl` cannot start; checked once per session."""
    global _available
    with _lock:
        if _available is None:
            try:
                run(None, "version")
                _available = ""
            except OpenSslError as exc:
                _available = str(exc).splitlines()[0]
    if _available:
        raise unittest.SkipTest(f"openssl unavailable: {_available}")


class SiteAuthority:
    """A site CA, a foreign CA and leaves under them, made on first request.

    Leaves are signed by the site CA (two days for the CA, one for a leaf)
    except `wrong`, which the foreign CA signs, and `expired` and `future`,
    which the site CA signs with a validity window in the past and the far
    future. Any other name is an ordinary valid leaf for localhost and
    127.0.0.1.
    """

    def __init__(self):
        inherited = os.environ.get(SESSION_ENV)
        if inherited and (Path(inherited) / "ca.cnf").is_file():
            self.root = Path(inherited)  # a parent's session: reuse, never delete
            return
        self.temp = tempfile.TemporaryDirectory(prefix="bacnet-openssl-session-",
                                                ignore_cleanup_errors=True)
        atexit.register(self.temp.cleanup)
        self.root = Path(self.temp.name)
        os.environ[SESSION_ENV] = str(self.root)
        (self.root / "index").write_text("")
        (self.root / "serial").write_text("1000\n")
        (self.root / "ca.cnf").write_text(
            "[ca]\ndefault_ca=issuer\n[issuer]\ndatabase=index\n"
            "serial=serial\nnew_certs_dir=.\ncertificate=site.pem\n"
            "private_key=site.key\ndefault_md=sha256\npolicy=policy\n"
            "unique_subject=no\n[policy]\ncommonName=supplied\n" + LEAF_EXTENSIONS)

    def _ensure(self, name):
        if (self.root / f"{name}.pem").is_file() and (self.root / f"{name}.key").is_file():
            return
        if name in ("site", "foreign"):
            # Explicit CA constraints; leaves are never substituted for trust anchors.
            run(self.root, "req", "-x509", *EC, "-days", "2", "-subj", f"/CN={name}",
                "-keyout", f"{name}.key", "-out", f"{name}.pem",
                "-addext", "basicConstraints=critical,CA:TRUE",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        else:
            self._ensure("site")
            run(self.root, "req", *EC, "-subj", f"/CN={name}",
                "-keyout", f"{name}.key", "-out", f"{name}.csr")
            if name in ("expired", "future"):
                start, end = (("20000101000000Z", "20010101000000Z") if name == "expired"
                              else ("20990101000000Z", "21000101000000Z"))
                run(self.root, "ca", "-batch", "-config", "ca.cnf", "-extensions", "leaf",
                    "-in", f"{name}.csr", "-out", f"{name}.pem", "-notext",
                    "-startdate", start, "-enddate", end)
            else:
                ca = "site"
                if name == "wrong":
                    self._ensure("foreign")
                    ca = "foreign"
                (self.root / "leaf.cnf").write_text(LEAF_EXTENSIONS)
                run(self.root, "x509", "-req", "-in", f"{name}.csr", "-CA", f"{ca}.pem",
                    "-CAkey", f"{ca}.key", "-CAcreateserial", "-days", "1",
                    "-extfile", "leaf.cnf", "-extensions", "leaf", "-out", f"{name}.pem")

    def provide(self, dest, *names):
        """Copy `site` and each named certificate and key into `dest`, a directory the caller owns."""
        with _lock:
            for name in ("site", *names):
                self._ensure(name)
        for name in dict.fromkeys(("site", *names)):
            for suffix in ("pem", "key"):
                shutil.copyfile(self.root / f"{name}.{suffix}", Path(dest) / f"{name}.{suffix}")


_authority = None


def site_authority():
    """The session's one `SiteAuthority`."""
    global _authority
    with _lock:
        if _authority is None:
            _authority = SiteAuthority()
        return _authority
