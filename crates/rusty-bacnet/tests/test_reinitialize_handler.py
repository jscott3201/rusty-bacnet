"""ReinitializeDevice handled from Python (#1566): the on_reinitialize callable
accepts or refuses under the server's lock, and after_reinitialize runs on the
event loop once the SimpleACK has been handed to the link."""
import asyncio
import gc
import socket
import sys
import unittest
import warnings

from rusty_bacnet import (
    BACnetClient, BACnetServer, Bbmd, BipEndpoint, ErrorClass, ErrorCode,
    ForeignDevice, ObjectIdentifier, ObjectType, PropertyIdentifier as P,
    PropertyValue as V, ReinitializedState as S, ReinitializeRequest,
)

SERVICE = 20  # ReinitializeDevice


def ack(invoke):
    return bytes([0x20, invoke, SERVICE])


def error(invoke, error_class, error_code):
    return bytes([0x50, invoke, SERVICE, 0x91, error_class, 0x91, error_code])


SERVICES_OTHER = (5, 0)
SERVICE_REQUEST_DENIED = (5, 29)
PASSWORD_FAILURE = (4, 26)
CONFIGURATION_IN_PROGRESS = (0, 2)
# What an after_reinitialize hook sends the peer; not a B/IP frame.
MARKER = b"after_reinitialize ran"


def request(invoke, state, password=None):
    """A B/IP unicast ReinitializeDevice request, framed for the wire."""
    body = bytes([0x09, state.to_raw()])
    if password is not None:
        content = b"\0" + password.encode()
        body += bytes([0x1D, len(content)]) + content
    npdu = b"\x01\x04" + bytes([0x00, 0x05, invoke, SERVICE]) + body
    return b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu


def reply_apdu(datagram):
    """The APDU of a B/IP unicast reply."""
    assert datagram[:2] == b"\x81\x0a"
    offset = 6
    if datagram[5] & 0x20:
        offset += 4 + datagram[8]
    return datagram[offset:]


class Peer:
    """A raw UDP peer on loopback, so replies are checked as raw PDUs."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.setblocking(False)

    def close(self):
        self.sock.close()

    @property
    def mac(self):
        ip, port = self.sock.getsockname()
        return socket.inet_aton(ip) + port.to_bytes(2, "big")

    def mark(self):
        """Send this peer a MARKER from another socket. Loopback delivers
        datagrams in the order they were sent, so the marker arrives after
        every datagram sent to the peer before the call."""
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as marker:
            marker.sendto(MARKER, self.sock.getsockname())

    async def take(self):
        """The next datagram: a reply's APDU, or the MARKER."""
        # Poll rather than use a loop reader: Windows' default event loop
        # has no add_reader.
        async with asyncio.timeout(2):
            while True:
                try:
                    datagram = self.sock.recv(2048)
                except BlockingIOError:
                    await asyncio.sleep(0.002)
                    continue
                return datagram if datagram == MARKER else reply_apdu(datagram)

    async def ask(self, address, invoke, state, password=None):
        ip, port = address.rsplit(":", 1)
        self.sock.sendto(request(invoke, state, password), (ip, int(port)))
        return await self.take()


def server(**keywords):
    return BACnetServer(4100, interface="127.0.0.1", port=0,
                        broadcast_address="127.0.0.1", **keywords)


async def eventually(check, seconds=5.0):
    async with asyncio.timeout(seconds):
        while not check():
            await asyncio.sleep(0.01)


class ConstructorTests(unittest.TestCase):
    def test_callables_are_checked(self):
        with self.assertRaisesRegex(TypeError, "on_reinitialize must be callable"):
            server(on_reinitialize=1)
        with self.assertRaisesRegex(TypeError, "after_reinitialize must be callable"):
            server(on_reinitialize=lambda request: None, after_reinitialize="later")
        with self.assertRaisesRegex(ValueError, "after_reinitialize needs on_reinitialize"):
            server(after_reinitialize=lambda request: None)
        with self.assertRaisesRegex(ValueError, "reinit_password needs on_reinitialize"):
            BipEndpoint(4101, reinit_password="secret")
        with self.assertRaisesRegex(ValueError, "after_reinitialize needs on_reinitialize"):
            BipEndpoint(4101, after_reinitialize=lambda request: None)

    def test_password_over_twenty_characters_is_refused(self):
        # The limit counts characters, not octets (#2060).
        handler = lambda request: None
        with self.assertRaisesRegex(ValueError, "reinit_password exceeds the 20-character limit"):
            server(on_reinitialize=handler, reinit_password="é" * 21)
        with self.assertRaisesRegex(ValueError, "reinit_password exceeds the 20-character limit"):
            BipEndpoint(4101, on_reinitialize=handler, reinit_password="é" * 21)
        # An empty one could never match (#2094).
        with self.assertRaisesRegex(ValueError, "empty reinit_password"):
            server(on_reinitialize=handler, reinit_password="")
        with self.assertRaisesRegex(ValueError, "empty reinit_password"):
            BipEndpoint(4101, on_reinitialize=handler, reinit_password="")
        server(on_reinitialize=handler, reinit_password="é" * 20)
        BipEndpoint(4101, on_reinitialize=handler, reinit_password="é" * 20)


class ServerTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.peer = Peer()

    async def asyncTearDown(self):
        self.peer.close()

    async def run_server(self, owner, exchange):
        await owner.start()
        try:
            await exchange(await owner.local_address())
        finally:
            await owner.stop()

    async def test_accept_exposes_the_request(self):
        seen = []
        owner = server(on_reinitialize=seen.append)

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 7, S.WARMSTART), ack(7))
        await self.run_server(owner, exchange)
        [request] = seen
        self.assertIsInstance(request, ReinitializeRequest)
        self.assertEqual(request.state, S.WARMSTART)
        self.assertEqual(request.invoke_id, 7)
        self.assertEqual(request.source_mac, self.peer.mac)
        self.assertIsNone(request.source_network)
        self.assertIsNone(request.source_address)

    async def test_refusals_answer_with_their_class_and_code(self):
        answers = iter([
            (ErrorClass.DEVICE, ErrorCode.CONFIGURATION_IN_PROGRESS),
            False,
            True,
            None,
        ])
        owner = server(on_reinitialize=lambda request: next(answers))

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 1, S.COLDSTART),
                             error(1, *CONFIGURATION_IN_PROGRESS))
            self.assertEqual(await self.peer.ask(address, 2, S.COLDSTART),
                             error(2, *SERVICE_REQUEST_DENIED))
            self.assertEqual(await self.peer.ask(address, 3, S.COLDSTART), ack(3))
            self.assertEqual(await self.peer.ask(address, 4, S.COLDSTART), ack(4))
        await self.run_server(owner, exchange)

    async def test_an_exception_answers_services_other_and_the_server_keeps_serving(self):
        def handler(request):
            if request.invoke_id == 1:
                raise ValueError("refused by a bug")
            if request.invoke_id == 2:
                return 42  # not an answer
            return None
        owner = server(on_reinitialize=handler)

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 1, S.WARMSTART),
                             error(1, *SERVICES_OTHER))
            self.assertEqual(await self.peer.ask(address, 2, S.WARMSTART),
                             error(2, *SERVICES_OTHER))
            self.assertEqual(await self.peer.ask(address, 3, S.WARMSTART), ack(3))
        await self.run_server(owner, exchange)

    async def test_an_async_handler_is_refused_without_a_never_awaited_warning(self):
        ran = []

        async def handler(request):
            ran.append(request.invoke_id)
        unraisable = []
        previous = sys.unraisablehook
        sys.unraisablehook = lambda hook_args: unraisable.append(hook_args.exc_value)
        try:
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                owner = server(on_reinitialize=handler)

                async def exchange(address):
                    self.assertEqual(await self.peer.ask(address, 1, S.WARMSTART),
                                     error(1, *SERVICES_OTHER))
                await self.run_server(owner, exchange)
                gc.collect()
        finally:
            sys.unraisablehook = previous
        self.assertEqual(ran, [])
        self.assertTrue(any("plain function" in str(exc) for exc in unraisable), unraisable)
        self.assertFalse([w for w in caught if "never awaited" in str(w.message)])

    async def test_a_password_failure_never_reaches_the_callable(self):
        seen = []
        owner = server(reinit_password="secret", on_reinitialize=seen.append)

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 1, S.COLDSTART),
                             error(1, *PASSWORD_FAILURE))
            self.assertEqual(await self.peer.ask(address, 2, S.COLDSTART, "wrong"),
                             error(2, *PASSWORD_FAILURE))
            self.assertEqual(seen, [])
            self.assertEqual(await self.peer.ask(address, 3, S.COLDSTART, "secret"), ack(3))
        await self.run_server(owner, exchange)
        self.assertEqual([request.invoke_id for request in seen], [3])

    async def test_the_callable_cannot_reenter_the_bindings(self):
        """A binding operation started under the server's lock could wait for
        that lock, and a callable blocked on it would never return; the
        bindings refuse it instead."""
        failures = []
        owner = None

        async def probe():
            return await owner.comm_state()

        def handler(request):
            try:
                asyncio.run(probe())
            except RuntimeError as failure:
                failures.append(str(failure))
                raise
        owner = server(on_reinitialize=handler)

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 1, S.WARMSTART),
                             error(1, *SERVICES_OTHER))
        await self.run_server(owner, exchange)
        self.assertEqual(len(failures), 1)
        self.assertIn("cannot start a binding operation", failures[0])

    async def test_after_reinitialize_runs_once_the_reply_is_on_the_link(self):
        """The hook runs only after the server sent the reply. Whether the
        reply already sits in the peer's socket then is up to the platform:
        macOS loopback delivers a datagram on another thread, after sendto
        returns. Loopback does keep sends in order, though, so the hook
        sends the peer a marker: the reply must arrive before it. (The
        ordering itself is pinned without a socket in bacnet-server's
        reinitialize_reply_tests.)"""
        for coroutine in [False, True]:
            with self.subTest(coroutine=coroutine):
                notified = []

                def record(request):
                    notified.append(request.invoke_id)
                    self.peer.mark()

                async def record_later(request):
                    record(request)
                    await asyncio.sleep(0)

                owner = server(on_reinitialize=lambda request: None,
                               after_reinitialize=record_later if coroutine else record)

                async def exchange(address):
                    self.assertEqual(await self.peer.ask(address, 5, S.COLDSTART), ack(5))
                    self.assertEqual(await self.peer.take(), MARKER)
                await self.run_server(owner, exchange)
                self.assertEqual(notified, [5])

    async def test_after_reinitialize_never_runs_for_a_refusal(self):
        notified = []

        def handler(request):
            if request.invoke_id == 1:
                return (ErrorClass.DEVICE, ErrorCode.CONFIGURATION_IN_PROGRESS)
            if request.invoke_id == 2:
                raise RuntimeError("handler failure")
            return None
        owner = server(reinit_password="secret", on_reinitialize=handler,
                       after_reinitialize=lambda request: notified.append(request.invoke_id))

        async def exchange(address):
            self.assertEqual(await self.peer.ask(address, 1, S.WARMSTART, "secret"),
                             error(1, *CONFIGURATION_IN_PROGRESS))
            self.assertEqual(await self.peer.ask(address, 2, S.WARMSTART, "secret"),
                             error(2, *SERVICES_OTHER))
            self.assertEqual(await self.peer.ask(address, 3, S.WARMSTART, "wrong"),
                             error(3, *PASSWORD_FAILURE))
            # The accepted request's notification follows every earlier one.
            self.assertEqual(await self.peer.ask(address, 4, S.WARMSTART, "secret"), ack(4))
            await eventually(lambda: notified)
            await asyncio.sleep(0.05)
        await self.run_server(owner, exchange)
        self.assertEqual(notified, [4])

    async def test_a_handler_makes_bbmd_rows_writable_and_activate_changes_applies_them(self):
        port = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
        owner = BACnetServer(4102, interface="127.0.0.1", port=0,
                             broadcast_address="127.255.255.255", registered_network_port=1,
                             bbmd=Bbmd(), on_reinitialize=lambda request: None)
        owner.add_bip_network_port(1, "bbmd", ip_address="127.0.0.1", udp_port=0)
        await owner.start()
        try:
            address = await owner.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.write_property(address, port, P.BBMD_ACCEPT_FD_REGISTRATIONS,
                                            V.boolean(True))
                self.assertEqual(await client.read_property(address, port, P.CHANGES_PENDING),
                                 V.boolean(True))
                await client.reinitialize_device(address, S.ACTIVATE_CHANGES)
                self.assertEqual(await client.read_property(address, port, P.CHANGES_PENDING),
                                 V.boolean(False))
            # The BBMD now takes the foreign device it refused before.
            foreign = ForeignDevice(address, ttl=60)
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    foreign_device=foreign) as client:
                async def accepted():
                    while True:
                        status = await client.foreign_registration()
                        if status["last_outcome"] == "accepted":
                            return status
                        self.assertNotEqual(status["last_outcome"], "refused")
                        await asyncio.sleep(0.02)
                await asyncio.wait_for(accepted(), 20)
        finally:
            await owner.stop()

    async def test_a_handler_makes_foreign_rows_writable_and_activate_changes_moves_them(self):
        port = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
        first, second = Peer(), Peer()

        async def registration(peer):
            """The time-to-live of the next Register-Foreign-Device `peer`
            receives, polled since a test may not use add_reader."""
            async with asyncio.timeout(10):
                while True:
                    try:
                        datagram = peer.sock.recv(2048)
                    except BlockingIOError:
                        await asyncio.sleep(0.01)
                        continue
                    if datagram[:4] == b"\x81\x05\x00\x06":
                        return int.from_bytes(datagram[4:6], "big")

        def host_n_port(peer):
            ip, number = peer.sock.getsockname()
            # [0] { ip-address [1] } port [1]
            return (b"\x0e\x1c" + socket.inet_aton(ip) + b"\x0f\x1a"
                    + number.to_bytes(2, "big"))

        bbmd = "127.0.0.1:%d" % first.sock.getsockname()[1]
        owner = BACnetServer(4105, interface="127.0.0.1", port=0,
                             broadcast_address="127.255.255.255", registered_network_port=1,
                             foreign_device=ForeignDevice(bbmd, ttl=60),
                             on_reinitialize=lambda request: None)
        owner.add_bip_network_port(1, "foreign", ip_address="127.0.0.1", udp_port=0)
        await owner.start()
        try:
            self.assertEqual(await registration(first), 60)
            address = await owner.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.write_property(address, port, P.FD_BBMD_ADDRESS,
                                            V.application_data(host_n_port(second)))
                await client.write_property(address, port, P.FD_SUBSCRIPTION_LIFETIME,
                                            V.unsigned(120))
                self.assertEqual(await client.read_property(address, port, P.CHANGES_PENDING),
                                 V.boolean(True))
                await client.reinitialize_device(address, S.ACTIVATE_CHANGES)
                self.assertEqual(await client.read_property(address, port, P.CHANGES_PENDING),
                                 V.boolean(False))
                self.assertEqual(
                    await client.read_property(address, port, P.FD_SUBSCRIPTION_LIFETIME),
                    V.unsigned(120))
            # The next registration goes to the new BBMD, for the new lifetime.
            self.assertEqual(await registration(second), 120)
        finally:
            await owner.stop()
            first.close()
            second.close()


class LocalResetTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_local_reset_report_needs_a_running_server(self):
        owner = server()
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await owner.report_device_reset_local()
        await owner.start()
        try:
            # No Audit Reporter is configured: nothing to record, no error.
            self.assertIsNone(await owner.report_device_reset_local())
        finally:
            await owner.stop()


class EndpointTests(unittest.IsolatedAsyncioTestCase):
    async def test_an_endpoint_executes_reinitialize_device(self):
        seen, notified = [], []

        def handler(request):
            seen.append(request.invoke_id)
            if request.invoke_id == 1:
                return (ErrorClass.DEVICE, ErrorCode.CONFIGURATION_IN_PROGRESS)
            if request.invoke_id == 2:
                raise ValueError("handler failure")
            return None
        peer = Peer()
        endpoint = BipEndpoint(4103, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1", reinit_password="secret",
                               on_reinitialize=handler,
                               after_reinitialize=lambda request: (
                                   notified.append(request.invoke_id), peer.mark()))
        try:
            async with endpoint:
                scope = (await endpoint.server()).service_scope()
                self.assertEqual(scope["executes"], ["read_property", "reinitialize_device"])
                address = await endpoint.local_address()
                self.assertEqual(await peer.ask(address, 1, S.WARMSTART, "secret"),
                                 error(1, *CONFIGURATION_IN_PROGRESS))
                self.assertEqual(await peer.ask(address, 2, S.WARMSTART, "secret"),
                                 error(2, *SERVICES_OTHER))
                self.assertEqual(await peer.ask(address, 3, S.WARMSTART),
                                 error(3, *PASSWORD_FAILURE))
                self.assertEqual(await peer.ask(address, 4, S.START_BACKUP, "secret"), ack(4))
                # The hook's marker follows the reply (see the server test).
                self.assertEqual(await peer.take(), MARKER)
                await asyncio.sleep(0.05)
        finally:
            peer.close()
        self.assertEqual(seen, [1, 2, 4])
        self.assertEqual(notified, [4])

    async def test_without_a_handler_the_endpoint_executes_read_property_only(self):
        async with BipEndpoint(4104, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1") as endpoint:
            scope = (await endpoint.server()).service_scope()
            self.assertEqual(scope["executes"], ["read_property"])


if __name__ == "__main__":
    unittest.main()
