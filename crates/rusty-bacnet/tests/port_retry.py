"""Rerunning a test whose closing rebind loses the freed port (#1419).

A test that stops a node and then binds the node's old port, to prove the node
let it go, leaves a gap in which another process can take the port. The bind
then fails although the node released it. ``rerun_on_lost_port`` runs such a
test again from the start; the node binds a fresh port each run, so one that
really keeps its port fails every run, and the last run reports it. This is the
Python twin of ``bacnet-endpoint/tests/support/port_retry.rs``.
"""
import errno
import sys

ATTEMPTS = 8

# Windows can refuse a bind to a port another socket holds with WSAEACCES
# instead of WSAEADDRINUSE.
ADDR_IN_USE = (
    {errno.EADDRINUSE, errno.WSAEACCES} if sys.platform == "win32"
    else {errno.EADDRINUSE}
)


def lost_to_another_socket(error: BaseException) -> bool:
    """Whether ``error`` is a bind that another socket beat to the port."""
    return isinstance(error, OSError) and error.errno in ADDR_IN_USE


async def rerun_on_lost_port(run):
    """Await ``run()`` until it returns, rerunning it on a lost port.

    Any other failure, or a lost port on the last run, propagates.
    """
    for attempt in range(1, ATTEMPTS + 1):
        try:
            return await run()
        except OSError as error:
            if attempt < ATTEMPTS and lost_to_another_socket(error):
                continue
            raise
