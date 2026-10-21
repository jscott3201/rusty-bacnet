"""The stack's tracing events reach Python's ``logging`` (#1724).

Events go to ``rusty_bacnet.<target>``, the Rust module path with dots. The
bridge reads the logging levels at import, when a client, server, hub or
endpoint starts, and on ``refresh_logging()``; a lower level set after that
takes effect only at the next of them. Records arrive from a forwarder
thread, so each test waits for them.

The datagrams are sent to a running server's B/IP port:

- an NPDU with protocol version 2 is dropped: the first drop warns
  (``Dropping an inbound NPDU``), and later ones, until the warning
  interval has passed, log at debug;
- an NPDU for a remote network is dropped at debug by a node that doesn't
  route (``Discarding routed message``).
"""
import asyncio
import logging
import os
import socket
import subprocess
import sys
import textwrap
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetServer

# Original-Unicast-NPDU, then NPDU version 2 with no control bits.
BAD_VERSION = bytes([0x81, 0x0A, 0x00, 0x06, 0x02, 0x00])
# Original-Unicast-NPDU, then an NPDU for station 5 on network 2 (DNET 2,
# DLEN 1, DADR 5, hop count 255) carrying a Who-Is.
ROUTED = bytes([0x81, 0x0A, 0x00, 0x0D, 0x01, 0x20, 0x00, 0x02, 0x01, 0x05,
                0xFF, 0x10, 0x08])
LOGGER = "rusty_bacnet.bacnet_network"
WARNING_TEXT = "Dropping an inbound NPDU"
DEBUG_TEXT = "Discarding routed message"

# A child process: a server on loopback, sent the given datagrams, after
# the given logging setup. Shared by the subprocess tests.
CHILD = textwrap.dedent("""
    import asyncio, logging, socket, sys, threading, time
    import rusty_bacnet
    from rusty_bacnet import BACnetServer

    def make_server():
        return BACnetServer(device_instance=1725, device_name="child",
                            interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1")

    async def send(server, datagram):
        host, port = (await server.local_address()).rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.sendto(datagram, (host, int(port)))
""")


def run_child(body: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-c", CHILD + textwrap.dedent(body)],
        capture_output=True, text=True, timeout=60,
        env={**os.environ, "PYTHONWARNINGS": "ignore"},
    )


def make_server(instance: int = 1724) -> BACnetServer:
    return BACnetServer(
        device_instance=instance,
        device_name="Logging Bridge Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )


class LoggingBridgeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        # Starting the server reads the levels, whatever an earlier test set.
        self.server = make_server()
        await self.server.start()
        self.address = await self.address_of(self.server)
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(("127.0.0.1", 0))

    async def asyncTearDown(self) -> None:
        self.socket.close()
        await self.server.stop()
        rusty_bacnet.refresh_logging()

    @staticmethod
    async def address_of(server: BACnetServer) -> tuple[str, int]:
        host, port = (await server.local_address()).rsplit(":", 1)
        return host, int(port)

    def send(self, datagram: bytes, address: tuple[str, int] | None = None) -> None:
        self.socket.sendto(datagram, address or self.address)

    async def wait_for(self, records: list, text: str) -> logging.LogRecord:
        for _ in range(500):
            for record in records:
                if text in record.getMessage():
                    return record
            await asyncio.sleep(0.01)
        self.fail(f"no record with {text!r} in {[r.getMessage() for r in records]}")

    async def test_a_warning_from_the_stack_reaches_logging(self) -> None:
        with self.assertLogs(LOGGER, "WARNING") as captured:
            self.send(BAD_VERSION)
            record = await self.wait_for(captured.records, WARNING_TEXT)
        self.assertEqual(record.levelno, logging.WARNING)
        self.assertTrue(record.name.startswith(LOGGER + "."), record.name)
        self.assertEqual(record.bacnet_target, record.name.removeprefix(
            "rusty_bacnet.").replace(".", "::"))
        # The event's fields are in the message and on the record.
        self.assertIn("discard", record.bacnet_fields)
        self.assertEqual(record.bacnet_fields["dropped"], 1)
        self.assertIn(" discard=", record.getMessage())
        self.assertTrue(record.pathname.endswith(".rs"), record.pathname)
        self.assertIsNone(record.funcName)

    async def test_a_lower_level_takes_effect_after_refresh_logging(self) -> None:
        with self.assertLogs(LOGGER, "DEBUG") as captured:
            # DEBUG on the logger now, but the bridge still has the levels it
            # read before: the debug event isn't forwarded. The warning sent
            # after it comes through the same receive path, so once it's
            # here the debug event would be too.
            self.send(ROUTED)
            self.send(BAD_VERSION)
            await self.wait_for(captured.records, WARNING_TEXT)
            texts = [record.getMessage() for record in captured.records]
            self.assertFalse(any(DEBUG_TEXT in text for text in texts), texts)

            rusty_bacnet.refresh_logging()
            self.send(ROUTED)
            record = await self.wait_for(captured.records, DEBUG_TEXT)
        self.assertEqual(record.levelno, logging.DEBUG)

    async def test_a_start_reads_the_levels(self) -> None:
        with self.assertLogs(LOGGER, "DEBUG") as captured:
            # No refresh_logging(): starting a server reads the levels.
            server = make_server(1726)
            await server.start()
            try:
                self.send(ROUTED, await self.address_of(server))
                record = await self.wait_for(captured.records, DEBUG_TEXT)
            finally:
                await server.stop()
        self.assertEqual(record.levelno, logging.DEBUG)

    async def test_a_handler_may_call_back_into_rusty_bacnet(self) -> None:
        calls = []

        class CallBack(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                # On the forwarder thread, with the GIL: no lock of the
                # bridge is held, so this neither deadlocks nor stops it.
                rusty_bacnet.refresh_logging()
                calls.append(record.getMessage())

        # assertLogs puts the logger's own handlers back when it ends,
        # which removes this one.
        with self.assertLogs(LOGGER, "DEBUG") as captured:
            rusty_bacnet.refresh_logging()
            logging.getLogger(LOGGER).addHandler(CallBack())
            self.send(BAD_VERSION)
            await self.wait_for(captured.records, WARNING_TEXT)
            # Throttled: this one logs at debug.
            self.send(BAD_VERSION)
            for _ in range(500):
                if sum(WARNING_TEXT in call for call in calls) >= 2:
                    break
                await asyncio.sleep(0.01)
        self.assertEqual(sum(WARNING_TEXT in call for call in calls), 2, calls)

    def test_trace_has_a_level_name(self) -> None:
        self.assertEqual(logging.getLevelName(5), "TRACE")

    def test_the_package_logger_has_a_null_handler(self) -> None:
        handlers = logging.getLogger("rusty_bacnet").handlers
        self.assertTrue(any(isinstance(h, logging.NullHandler) for h in handlers))


class LoggingBridgeProcessTests(unittest.TestCase):
    def test_an_unconfigured_app_prints_nothing(self) -> None:
        # Without the NullHandler, logging's last resort prints the warning.
        child = run_child("""
            async def main():
                server = make_server()
                await server.start()
                await send(server, bytes([0x81, 0x0A, 0x00, 0x06, 0x02, 0x00]))
                await asyncio.sleep(0.5)
                await server.stop()
            asyncio.run(main())
            print("done")
        """)
        self.assertEqual(child.returncode, 0, child.stderr)
        self.assertEqual(child.stdout.strip(), "done")
        self.assertNotIn(WARNING_TEXT, child.stderr)

    def test_records_queued_before_exit_are_delivered(self) -> None:
        # The first record holds the forwarder in its handler; the second is
        # queued behind it as the program ends. The exit hook waits for it.
        child = run_child("""
            busy = threading.Event()

            class Slow(logging.Handler):
                def emit(self, record):
                    print(record.levelname, record.getMessage(), flush=True)
                    dropped = "Dropping an inbound NPDU" in record.getMessage()
                    if dropped and not busy.is_set():
                        busy.set()
                        time.sleep(1.0)

            # After import, before start, with no refresh_logging().
            logging.basicConfig(level=logging.DEBUG, handlers=[Slow()])

            async def main():
                server = make_server()
                await server.start()
                await send(server, bytes([0x81, 0x0A, 0x00, 0x06, 0x02, 0x00]))
                while not busy.is_set():
                    await asyncio.sleep(0.01)
                # Throttled, so at debug; queued while the handler sleeps.
                await send(server, bytes([0x81, 0x0A, 0x00, 0x06, 0x02, 0x00]))
                await asyncio.sleep(0.3)
            asyncio.run(main())
        """)
        self.assertEqual(child.returncode, 0, child.stderr)
        lines = [line for line in child.stdout.splitlines() if WARNING_TEXT in line]
        self.assertEqual(len(lines), 2, child.stdout)
        self.assertTrue(lines[0].startswith("WARNING"), lines)
        self.assertTrue(lines[1].startswith("DEBUG"), lines)


if __name__ == "__main__":
    unittest.main()
