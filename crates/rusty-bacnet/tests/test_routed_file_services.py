"""AtomicReadFile and AtomicWriteFile to a station behind a router (#2613).

`atomic_read_file` and `atomic_write_file` take a `RoutedTarget` as the five
calls of #2612/#2617 do, with the same routing rules (a named router, the
route the client learns, a lookup that misses, a group router refused). A
raw peer plays the router so each request's NPDU can be read off the wire
and each answer scripted: short, empty and segmented blocks, empty records,
and reject, error and abort answers. Every case runs on a standalone client
and on the client from a `BipEndpoint`. One class goes through a real
`BACnetRouter` to a real server.

The raw peer (bvll_peer) runs in a thread, since Windows' default event
loop has no add_reader.
"""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll
from test_bip_modes import accepted_registration
from test_endpoint_routed import finish, i_am, request_of

O = rb.ObjectType
FILE_1 = rb.ObjectIdentifier(O.FILE, 1)
NEAR = 2613
FAR = 2614
STATION = b"\x0a"
READ_FILE = 6
WRITE_FILE = 7


def octets(data):
    """An application-tagged octet string."""
    if len(data) <= 4:
        return bytes([0x60 + len(data)]) + data
    if len(data) < 254:
        return bytes([0x65, len(data)]) + data
    return b"\x65\xfe" + len(data).to_bytes(2, "big") + data


def stream_ack(start, data, eof):
    return bytes([0x11 if eof else 0x10, 0x0E, 0x31, start]) + octets(data) + b"\x0f"


def record_ack(start, records, eof):
    return (bytes([0x11 if eof else 0x10, 0x1E, 0x31, start, 0x21, len(records)])
            + b"".join(octets(record) for record in records) + b"\x1f")


def parse_read(body):
    """(access method, start, count) of an AtomicReadFile request body."""
    assert body[0] == 0xC4, body
    choice = body[5]
    at = 6
    values = []
    for _ in range(2):
        length = body[at] & 7
        values.append(int.from_bytes(body[at + 1:at + 1 + length], "big", signed=body[at] >> 4 == 3))
        at += 1 + length
    return ("stream" if choice == 0x0E else "record", *values)


def relay(network, station, apdu):
    """`apdu` relayed from `station` on `network` as a router relays it, or
    sent direct when `network` is None."""
    if network is None:
        return bvll(0x0A, b"\x01\x00" + apdu)
    source = network.to_bytes(2, "big") + bytes([len(station)]) + station
    return bvll(0x0A, bytes([0x01, 0x08]) + source + apdu)


def segments(invoke, service_data, size, window):
    """A ComplexACK's service data as segmented APDUs of `size` octets."""
    chunks = [service_data[i:i + size] for i in range(0, len(service_data), size)]
    return [bytes([0x38 | (0x04 if n + 1 < len(chunks) else 0), invoke, n, window, READ_FILE])
            + chunk for n, chunk in enumerate(chunks)]


class FileWire:
    """The raw router's side: remembers each request and answers it with
    what `reply` returns for it (a list of APDUs after the invoke ID is
    known), or with silence."""

    def __init__(self):
        self.requests = []
        self.segment_acks = []
        self.script = None

    def handle(self, datagram, source):
        if datagram[:2] != b"\x81\x0a":
            return None
        request = request_of(datagram)
        apdu = request["apdu"]
        if apdu[0] >> 4 == 4:  # a SegmentACK from the client
            self.segment_acks.append(request)
            return None
        self.requests.append(request)
        apdus = self.script(request, apdu[2], apdu[3], apdu[4:]) if self.script else None
        return [relay(request["dnet"], request["dadr"], apdu) for apdu in apdus or ()]

    def assert_routed(self, service):
        self.test.assertTrue(self.requests)
        for request in self.requests:
            # DNET present, expecting a reply, normal priority, no SNET.
            self.test.assertEqual(request["control"], 0x24)
            self.test.assertEqual((request["dnet"], request["dadr"]), (5, STATION))
            self.test.assertEqual(request["hop_count"], 255)
            self.test.assertEqual(request["apdu"][0] >> 4, 0)  # Confirmed-Request
            self.test.assertEqual(request["apdu"][3], service)


class RoutedFileWireTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.wire = FileWire()
        self.wire.test = self
        self.router = RawPeer(self.wire.handle)
        self.addCleanup(self.router.close)
        self.target = rb.RoutedTarget(5, STATION, router=self.router.address)

    async def run_both(self, scenario, timeout_ms=2000):
        """`scenario` on a standalone client, then on an endpoint's."""
        for kind in ("standalone", "endpoint"):
            with self.subTest(client=kind):
                self.wire.requests.clear()
                self.wire.segment_acks.clear()
                if kind == "standalone":
                    async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                               broadcast_address="127.0.0.1",
                                               apdu_timeout_ms=timeout_ms) as client:
                        await scenario(client)
                else:
                    endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                              broadcast_address="127.0.0.1",
                                              apdu_timeout_ms=timeout_ms, apdu_retries=0)
                    await asyncio.wait_for(endpoint.start(), 10)
                    try:
                        await scenario(await endpoint.client())
                        self.assertEqual((await endpoint.status())["active_leases"], 0)
                    finally:
                        await finish(endpoint.close)

    async def test_stream_blocks_keep_a_short_and_an_empty_final_block(self):
        content = b"0123456789"

        def script(request, invoke, service, body):
            _, start, count = parse_read(body)
            data = content[start:start + count]
            return [bytes([0x30, invoke, service]) + stream_ack(start, data, start + count >= 10)]

        self.wire.script = script

        async def scenario(client):
            first = await asyncio.wait_for(client.atomic_read_file(
                self.target, FILE_1, "stream", start_position=0, requested_octet_count=8), 5)
            self.assertEqual(first, {"access_method": "stream", "end_of_file": False,
                                     "start_position": 0, "data": b"01234567"})
            short = await asyncio.wait_for(client.atomic_read_file(
                self.target, FILE_1, "stream", start_position=8, requested_octet_count=8), 5)
            self.assertEqual(short, {"access_method": "stream", "end_of_file": True,
                                     "start_position": 8, "data": b"89"})
            empty = await asyncio.wait_for(client.atomic_read_file(
                self.target, FILE_1, "stream", start_position=10, requested_octet_count=8), 5)
            self.assertEqual(empty, {"access_method": "stream", "end_of_file": True,
                                     "start_position": 10, "data": b""})
            self.wire.assert_routed(READ_FILE)
            self.assertEqual([parse_read(r["apdu"][4:]) for r in self.wire.requests],
                             [("stream", 0, 8), ("stream", 8, 8), ("stream", 10, 8)])

        await self.run_both(scenario)

    async def test_record_blocks_keep_empty_records(self):
        def script(request, invoke, service, body):
            kind, start, count = parse_read(body)
            self.assertEqual(kind, "record")
            return [bytes([0x30, invoke, service]) + record_ack(start, [b"one", b"", b"three"], True)]

        self.wire.script = script

        async def scenario(client):
            records = await asyncio.wait_for(client.atomic_read_file(
                self.target, FILE_1, "record", start_record=0, requested_record_count=10), 5)
            self.assertEqual(records, {"access_method": "record", "end_of_file": True,
                                       "start_record": 0, "record_count": 3,
                                       "records": [b"one", b"", b"three"]})
            self.wire.assert_routed(READ_FILE)
            self.assertEqual(parse_read(self.wire.requests[0]["apdu"][4:]), ("record", 0, 10))

        await self.run_both(scenario)

    async def test_a_segmented_answer_is_put_together(self):
        data = bytes(range(256)) * 4  # 1024 octets: four segments of 300 or so
        self.wire.script = lambda request, invoke, service, body: segments(
            invoke, stream_ack(0, data, True), 300, 4)

        async def scenario(client):
            reply = await asyncio.wait_for(client.atomic_read_file(
                self.target, FILE_1, "stream", start_position=0, requested_octet_count=1024), 5)
            self.assertEqual((reply["data"], reply["end_of_file"]), (data, True))
            # The request said it accepts a segmented answer (bit 1 of the
            # first octet is "segmented response accepted"), and the client's
            # SegmentACK went back through the router to the station.
            self.assertEqual(self.wire.requests[0]["apdu"][0] & 0x02, 0x02)
            self.assertTrue(self.wire.segment_acks)
            for ack in self.wire.segment_acks:
                self.assertEqual((ack["dnet"], ack["dadr"]), (5, STATION))

        await self.run_both(scenario)

    async def test_reject_error_and_abort_keep_their_detail(self):
        answers = {
            # Reject reason 9 is "unrecognized service"; abort reason 4 is
            # "segmentation not supported"; the error is class 2 (object),
            # code 0x1F (unknown object).
            "reject": lambda invoke: bytes([0x60, invoke, 9]),
            "error": lambda invoke: bytes([0x50, invoke, READ_FILE, 0x91, 0x02, 0x91, 0x1F]),
            "abort": lambda invoke: bytes([0x71, invoke, 4]),
        }
        mode = {}
        self.wire.script = lambda request, invoke, service, body: [answers[mode["now"]](invoke)]

        async def scenario(client):
            mode["now"] = "reject"
            with self.assertRaises(rb.BacnetRejectError) as rejected:
                await asyncio.wait_for(client.atomic_read_file(
                    self.target, FILE_1, "stream", requested_octet_count=8), 5)
            self.assertEqual(int(rejected.exception.reason), 9)
            mode["now"] = "error"
            with self.assertRaises(rb.BacnetProtocolError) as refused:
                await asyncio.wait_for(client.atomic_read_file(
                    self.target, FILE_1, "stream", requested_octet_count=8), 5)
            self.assertEqual((int(refused.exception.error_class), int(refused.exception.error_code)),
                             (2, 0x1F))
            mode["now"] = "abort"
            with self.assertRaises(rb.BacnetAbortError) as aborted:
                await asyncio.wait_for(client.atomic_read_file(
                    self.target, FILE_1, "stream", requested_octet_count=8), 5)
            self.assertEqual(int(aborted.exception.reason), 4)

        await self.run_both(scenario)

    async def test_an_answer_the_request_doesnt_fit_raises(self):
        # Each is refused as a direct read refuses it: more octets than
        # asked for, more records than asked for, and the other access method
        # in each direction.
        cases = [
            ("stream", dict(requested_octet_count=4), stream_ack(0, b"0123456789", False)),
            ("record", dict(requested_record_count=2),
             record_ack(0, [b"a", b"", b"c"], False)),
            ("stream", dict(requested_octet_count=4), record_ack(0, [b"a"], False)),
            ("record", dict(requested_record_count=2), stream_ack(0, b"ab", False)),
        ]
        current = []
        self.wire.script = lambda request, invoke, service, body: [
            bytes([0x30, invoke, service]) + current[0]]

        async def scenario(client):
            for method, window, ack in cases:
                with self.subTest(method=method, ack=ack):
                    current[:] = [ack]
                    with self.assertRaises(rb.BacnetError):
                        await asyncio.wait_for(client.atomic_read_file(
                            self.target, FILE_1, method, **window), 5)

        await self.run_both(scenario)

    async def test_truncated_file_reply_releases_the_request_for_a_valid_read(self):
        valid = stream_ack(3, b"ok", True)

        async def scenario(client):
            for address in (self.router.address, self.target):
                for body, expect_error in ((valid[:-1], True), (valid, False)):
                    self.wire.script = lambda request, invoke, service, request_body: [
                        bytes([0x30, invoke, service]) + body]
                    read = client.atomic_read_file(address, FILE_1, "stream",
                                                   start_position=3, requested_octet_count=2)
                    if expect_error:
                        with self.assertRaisesRegex(rb.BacnetError, "missing closing tag"):
                            await asyncio.wait_for(read, 5)
                    else:
                        reply = await asyncio.wait_for(read, 5)
                        self.assertEqual((reply["data"], reply["start_position"], reply["end_of_file"]),
                                         (b"ok", 3, True))

        await self.run_both(scenario)

    async def test_endpoint_serves_while_its_file_read_is_pending_then_reuses_the_socket(self):
        loop = asyncio.get_running_loop()
        admitted = loop.create_future()

        def silent(request, invoke, service, body):
            if service == READ_FILE:
                loop.call_soon_threadsafe(admitted.set_result, invoke)
            return None

        self.wire.script = silent
        async with rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0, device_name="File test",
                                  apdu_retries=0, apdu_timeout_ms=30000) as endpoint:
            local = await endpoint.local_address()
            source = ("127.0.0.1", int(local.rsplit(":", 1)[1]))
            client = await endpoint.client()
            pending = asyncio.ensure_future(client.atomic_read_file(self.target, FILE_1, "stream",
                                                                   requested_octet_count=2))
            try:
                await asyncio.wait_for(admitted, 5)
                self.assertEqual(self.router.received[-1][1], source)
                self.assertEqual((await endpoint.status())["active_leases"], 1)
                self.assertFalse(pending.done())
                async with rb.BACnetClient(interface="127.0.0.1", port=0) as observer:
                    name = await observer.read_property(local, rb.ObjectIdentifier(O.DEVICE, NEAR),
                                                        rb.PropertyIdentifier.OBJECT_NAME)
                    self.assertEqual(name, rb.PropertyValue.character_string("File test"))
                self.assertEqual((await endpoint.status())["active_leases"], 1)
                self.assertFalse(pending.done())
            finally:
                pending.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await pending
            async with asyncio.timeout(5):
                while (await endpoint.status())["active_leases"]:
                    await asyncio.sleep(0)
            self.wire.script = lambda request, invoke, service, body: [
                bytes([0x30, invoke, service]) + stream_ack(0, b"ok", True)]
            # A cancelled routed request holds its route for the APDU window.
            # Address the same peer directly to check immediate socket reuse.
            result = await asyncio.wait_for(
                client.atomic_read_file(self.router.address, FILE_1, "stream", requested_octet_count=2), 5)
            self.assertEqual(result["data"], b"ok")
            self.assertEqual((await endpoint.status())["active_leases"], 0)
            self.assertEqual(self.router.received[-1][1], source)

    async def test_a_routed_write_names_the_station(self):
        def script(request, invoke, service, body):
            self.assertEqual(service, WRITE_FILE)
            return [bytes([0x30, invoke, service, 0x09, 0x08])]  # ACK: file start position 8

        self.wire.script = script

        async def scenario(client):
            result = await asyncio.wait_for(client.atomic_write_file(
                self.target, FILE_1, "stream", start_position=8, file_data=b"tail"), 5)
            self.assertEqual(result, {"access_method": "stream", "start_position": 8})
            self.wire.assert_routed(WRITE_FILE)
            # The request carries the data to the station, not to the router.
            self.assertIn(b"tail", self.wire.requests[0]["apdu"])
            # A str address is still a direct station.
            self.wire.requests.clear()
            await asyncio.wait_for(client.atomic_write_file(
                self.router.address, FILE_1, "stream", start_position=8, file_data=b"tail"), 5)
            [direct] = self.wire.requests
            self.assertEqual((direct["control"], direct["dnet"]), (0x04, None))

        await self.run_both(scenario)

    async def test_a_str_read_is_direct_and_a_wrong_type_raises_before_sending(self):
        self.wire.script = lambda request, invoke, service, body: [
            bytes([0x30, invoke, service]) + stream_ack(0, b"x", True)]

        async def scenario(client):
            reply = await asyncio.wait_for(client.atomic_read_file(
                self.router.address, FILE_1, "stream", requested_octet_count=4), 5)
            self.assertEqual(reply["data"], b"x")
            [request] = self.wire.requests
            self.assertEqual((request["control"], request["dnet"]), (0x04, None))
            self.wire.requests.clear()
            for method in (client.atomic_read_file, client.atomic_write_file):
                with self.assertRaisesRegex(TypeError, "str or a RoutedTarget, not int"):
                    method(7, FILE_1, "stream")
            with self.assertRaises(ValueError):
                await client.atomic_read_file("5:0a", FILE_1, "stream", requested_octet_count=4)
            self.assertEqual(self.wire.requests, [])

        await self.run_both(scenario)

    async def test_a_group_router_is_refused_before_anything_is_sent(self):
        async def scenario(client):
            for router in ("255.255.255.255:47808", "239.255.1.1:47808"):
                target = rb.RoutedTarget(5, STATION, router=router)
                for call in (
                    lambda: client.atomic_read_file(target, FILE_1, "stream",
                                                    requested_octet_count=4),
                    lambda: client.atomic_write_file(target, FILE_1, "stream", file_data=b"x"),
                ):
                    with self.subTest(router=router), \
                            self.assertRaisesRegex(rb.BacnetError, "broadcast or group"):
                        await asyncio.wait_for(call(), 5)
            self.assertEqual(self.wire.requests, [])
            self.assertEqual(self.router.received, [])

        await self.run_both(scenario)

    async def test_an_unanswered_read_aborts_as_a_typed_call_does(self):
        self.wire.script = None

        async def scenario(client):
            with self.assertRaises(rb.BacnetAbortError):
                await asyncio.wait_for(client.atomic_read_file(
                    self.target, FILE_1, "stream", requested_octet_count=4), 5)

        await self.run_both(scenario, timeout_ms=300)


class LearnedFileRouteTests(unittest.IsolatedAsyncioTestCase):
    """A RoutedTarget with no router: the endpoint is a foreign device of a
    raw BBMD that is also the router, so its Who-Is shows on the wire."""

    async def asyncSetUp(self):
        self.who_is = []
        self.requests = []

        def handle(datagram, source):
            if datagram[:2] == b"\x81\x05":  # Register-Foreign-Device: accept
                return [bvll(0x00, b"\x00\x00")]
            if datagram[:2] == b"\x81\x09":  # a broadcast: answer a Who-Is to 5
                request = request_of(datagram, 0x09)
                if request["dnet"] == 5 and request["apdu"][:2] == b"\x10\x08":
                    self.who_is.append(request)
                    return [i_am(5, STATION, FAR)]
                return None
            if datagram[:2] != b"\x81\x0a":
                return None
            request = request_of(datagram)
            self.requests.append(request)
            apdu = request["apdu"]
            return [relay(request["dnet"], request["dadr"],
                          bytes([0x30, apdu[2], apdu[3]]) + stream_ack(0, b"abc", True))]

        self.bbmd = RawPeer(handle)
        self.addCleanup(self.bbmd.close)
        self.endpoint = rb.BipEndpoint(
            NEAR + 30, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2000, foreign_device=rb.ForeignDevice(self.bbmd.address, ttl=60))
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        await accepted_registration(self.endpoint)
        self.role = await self.endpoint.client()

    async def test_the_first_read_learns_the_router_and_the_rest_reuse_it(self):
        target = rb.RoutedTarget(5, STATION)
        for _ in range(3):
            reply = await asyncio.wait_for(self.role.atomic_read_file(
                target, FILE_1, "stream", requested_octet_count=8), 10)
            self.assertEqual(reply["data"], b"abc")
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual({(r["control"], r["dnet"], r["dadr"]) for r in self.requests},
                         {(0x24, 5, STATION)})
        # Naming the router gives the same request, and no lookup.
        named = rb.RoutedTarget(5, STATION, router=self.bbmd.address)
        await asyncio.wait_for(self.role.atomic_read_file(
            named, FILE_1, "stream", requested_octet_count=8), 5)
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual(len(self.requests), 4)

    async def test_a_silent_station_says_to_name_its_router_and_isnt_asked_again(self):
        silent = rb.RoutedTarget(5, b"\x0b")
        with self.assertRaises(rb.BacnetLookupError) as caught:
            await asyncio.wait_for(self.role.atomic_read_file(
                silent, FILE_1, "stream", requested_octet_count=8), 10)
        text = str(caught.exception)
        for words in ("5:0x0B", "Who-Is on network 5", "router=", "discover_routers()"):
            self.assertIn(words, text)
        self.assertEqual(len(self.who_is), 1)
        # Again, from either file call: at once, with no second Who-Is.
        with self.assertRaisesRegex(rb.BacnetLookupError, "in the last 30 s"):
            await asyncio.wait_for(self.role.atomic_read_file(
                silent, FILE_1, "stream", requested_octet_count=8), 1)
        with self.assertRaisesRegex(rb.BacnetLookupError, "in the last 30 s"):
            await asyncio.wait_for(self.role.atomic_write_file(
                silent, FILE_1, "stream", file_data=b"x"), 1)
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual(self.requests, [])
        # Naming the router works at once.
        through = rb.RoutedTarget(5, b"\x0b", router=self.bbmd.address)
        reply = await asyncio.wait_for(self.role.atomic_read_file(
            through, FILE_1, "stream", requested_octet_count=8), 5)
        self.assertEqual(reply["data"], b"abc")


class RoutedFileRouterTests(unittest.IsolatedAsyncioTestCase):
    """Through a real BACnetRouter to a real server on network 5."""

    async def asyncSetUp(self):
        self.router = rb.BACnetRouter([
            rb.BipRouterPort(n, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                             bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True))
            for n in (1, 5)])
        await asyncio.wait_for(self.router.start(), 10)
        self.addAsyncCleanup(finish, self.router.close)
        ports = await self.router.ports()
        self.router_address = ports[0].address
        self.far = rb.BACnetServer(FAR, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1",
                                   segmentation_supported=rb.Segmentation.SEGMENTED_BOTH,
                                   foreign_device=rb.ForeignDevice(ports[1].address, ttl=60))
        self.far.add_file(4, "Large")
        self.far.set_file_data(4, bytes(range(256)) * 12)  # 3072 octets
        self.far.add_file(1, "Stream")
        self.far.set_file_data(1, b"0123456789")
        self.far.add_file(2, "Records")
        self.far.set_file_access_method(2, "record")
        self.far.set_file_records(2, [b"one", b"", b"three"])
        self.far.add_file(3, "Writable")
        self.far.add_file(5, "Large writable")
        await asyncio.wait_for(self.far.start(), 10)
        self.addAsyncCleanup(finish, self.far.stop)
        await accepted_registration(self.far)
        self.far_address = await self.far.local_address()

    async def read_matches_direct(self, client, target):
        for start, count in ((0, 8), (8, 8), (10, 8)):
            routed = await asyncio.wait_for(client.atomic_read_file(
                target, FILE_1, "stream", start_position=start, requested_octet_count=count), 10)
            direct = await client.atomic_read_file(
                self.far_address, FILE_1, "stream", start_position=start,
                requested_octet_count=count)
            self.assertEqual(routed, direct)
        self.assertEqual(routed["data"], b"")  # the block at the file's end
        records = await asyncio.wait_for(client.atomic_read_file(
            target, rb.ObjectIdentifier(O.FILE, 2), "record", start_record=0,
            requested_record_count=10), 10)
        self.assertEqual(records["records"], [b"one", b"", b"three"])
        # A device error comes through the router with its detail.
        with self.assertRaises(rb.BacnetProtocolError) as refused:
            await asyncio.wait_for(client.atomic_read_file(
                target, rb.ObjectIdentifier(O.FILE, 99), "stream",
                requested_octet_count=8), 10)
        with self.assertRaises(rb.BacnetProtocolError) as direct:
            await client.atomic_read_file(
                self.far_address, rb.ObjectIdentifier(O.FILE, 99), "stream",
                requested_octet_count=8)
        self.assertEqual((refused.exception.error_class, refused.exception.error_code),
                         (direct.exception.error_class, direct.exception.error_code))
        # A routed write lands on the far server.
        result = await asyncio.wait_for(client.atomic_write_file(
            target, rb.ObjectIdentifier(O.FILE, 3), "stream", start_position=0,
            file_data=b"through"), 10)
        self.assertEqual(result, {"access_method": "stream", "start_position": 0})
        back = await client.atomic_read_file(
            self.far_address, rb.ObjectIdentifier(O.FILE, 3), "stream",
            start_position=0, requested_octet_count=16)
        self.assertEqual(back["data"], b"through")

    async def read_segmented(self, client, target):
        large = rb.ObjectIdentifier(O.FILE, 4)
        routed = await asyncio.wait_for(client.atomic_read_file(
            target, large, "stream", requested_octet_count=3072), 15)
        self.assertEqual(routed, await client.atomic_read_file(
            self.far_address, large, "stream", requested_octet_count=3072))
        self.assertEqual((routed["data"], routed["end_of_file"]),
                         (bytes(range(256)) * 12, True))

    async def test_a_large_block_is_segmented_through_the_router(self):
        target = rb.RoutedTarget(5, self.far_address, router=self.router_address)
        endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        async with endpoint:
            await self.read_segmented(await endpoint.client(), target)
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.read_segmented(client, target)

    async def write_segmented(self, client, target):
        # A 3072-octet write is larger than one APDU: it is sent segmented
        # through the router, and the far file holds it.
        payload = bytes(range(255, -1, -1)) * 12
        file_5 = rb.ObjectIdentifier(O.FILE, 5)
        for to, name in ((target, "routed"), (self.far_address, "direct")):
            with self.subTest(write=name):
                result = await asyncio.wait_for(client.atomic_write_file(
                    to, file_5, "stream", start_position=0, file_data=payload), 15)
                self.assertEqual(result, {"access_method": "stream", "start_position": 0})
                back = await client.atomic_read_file(
                    self.far_address, file_5, "stream", start_position=0,
                    requested_octet_count=3072)
                self.assertEqual(back["data"], payload)
                await client.atomic_write_file(
                    self.far_address, file_5, "stream", start_position=0, file_data=b"")

    async def edge_cases(self, client, target):
        # A negative read start and a write to a missing file are the device's
        # errors, routed or direct;
        # an empty write is what the device says it is, routed or direct.
        for call in (
            lambda to: client.atomic_read_file(to, FILE_1, "stream", start_position=-1,
                                               requested_octet_count=4),
            # A start of -1 is a legal append for a write, so the write's
            # refusal is a file that isn't there.
            lambda to: client.atomic_write_file(to, rb.ObjectIdentifier(O.FILE, 99), "stream",
                                                start_position=0, file_data=b"x"),
        ):
            with self.assertRaises(rb.BacnetProtocolError) as routed:
                await asyncio.wait_for(call(target), 10)
            with self.assertRaises(rb.BacnetProtocolError) as direct:
                await call(self.far_address)
            self.assertEqual((routed.exception.error_class, routed.exception.error_code),
                             (direct.exception.error_class, direct.exception.error_code))
        empty = await asyncio.wait_for(client.atomic_write_file(
            target, rb.ObjectIdentifier(O.FILE, 3), "stream", start_position=0,
            file_data=b""), 10)
        self.assertEqual(empty, await client.atomic_write_file(
            self.far_address, rb.ObjectIdentifier(O.FILE, 3), "stream", start_position=0,
            file_data=b""))

    async def test_a_large_write_is_segmented_and_edge_cases_match_direct(self):
        target = rb.RoutedTarget(5, self.far_address, router=self.router_address)
        endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", apdu_timeout_ms=2000,
                                  apdu_retries=0)
        async with endpoint:
            client = await endpoint.client()
            await self.write_segmented(client, target)
            await self.edge_cases(client, target)
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.write_segmented(client, target)
            await self.edge_cases(client, target)

    async def test_routed_file_reads_match_direct_ones(self):
        target = rb.RoutedTarget(5, self.far_address, router=self.router_address)
        endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        async with endpoint:
            await self.read_matches_direct(await endpoint.client(), target)
            self.assertEqual((await endpoint.status())["active_leases"], 0)
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.read_matches_direct(client, target)

    async def test_with_no_router_the_client_learns_the_route_through_the_router(self):
        endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", apdu_timeout_ms=2000,
                                  foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        async with endpoint:
            await accepted_registration(endpoint)
            await self.read_matches_direct(await endpoint.client(),
                                           rb.RoutedTarget(5, self.far_address))


if __name__ == "__main__":
    unittest.main()
