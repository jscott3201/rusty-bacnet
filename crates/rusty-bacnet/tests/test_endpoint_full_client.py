"""The BACnetClient on an endpoint's socket (#2617 steps 2 and 3).

`await endpoint.client()` returns the standalone class running on the
endpoint session's one address, so a virtual device served by the endpoint
and a scanner's discovery, reads, file reads, BBMD table reads, COV and frame
observation all share one socket. Loopback only; broadcasts go to
127.0.0.1, since the macOS runner has no route to 255.255.255.255.

The raw peers (bvll_peer) run in a thread, since Windows' default event
loop has no add_reader.
"""
import asyncio
import struct
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll, mac

O = rb.ObjectType
P = rb.PropertyIdentifier
AI_1 = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
ENDPOINT = 2617
PEER = 2618
TWIN = 2619
SINK = 2620


async def finish(method):
    """Await a native close or stop: its future needs the running loop."""
    await asyncio.wait_for(method(), 10)


def endpoint(**keywords):
    options = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1",
               "apdu_timeout_ms": 2000, "apdu_retries": 0, **keywords}
    owner = rb.BipEndpoint(ENDPOINT, device_name="Scanner", **options)
    owner.add_analog_input(instance=1, name="Virtual temperature", units=62, present_value=21.5)
    return owner


def silent_peer(test):
    """A raw peer that keeps what it hears and answers nothing."""
    peer = RawPeer(lambda datagram, source: None)
    test.addCleanup(peer.close)
    return peer


def read_property_ack(datagram, value):
    """The answer to the ReadProperty in `datagram`: a REAL `value`."""
    npdu = datagram[4:]
    apdu = npdu[2:]
    invoke, body = apdu[2], apdu[4:]
    reply = bytes([0x30, invoke, 12]) + body + b"\x3e\x44" + struct.pack(">f", value) + b"\x3f"
    return bvll(0x0A, b"\x01\x00" + reply)


class FullClientTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.endpoint = endpoint()
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        self.local = await self.endpoint.local_address()

    async def server(self, instance, **options):
        server = rb.BACnetServer(instance, interface="127.0.0.1", port=0,
                                 broadcast_address="127.0.0.1", **options)
        server.add_analog_input(1, f"AI of {instance}", 62, float(instance))
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(finish, server.stop)
        return server, await server.local_address()

    async def test_one_client_object_per_session(self):
        client = await self.endpoint.client()
        self.assertIs(type(client), rb.BACnetClient)
        # The same live object, however often it is asked for.
        self.assertIs(await self.endpoint.client(), client)
        self.assertIs(await self.endpoint.client(), client)
        # async with only enters it: it is started already, and exit stops it.
        async with client as entered:
            self.assertIs(entered, client)
        with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
            await client.who_is()
        # Once stopped, it stays stopped; the endpoint hands out a fresh one.
        with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
            await client.__aenter__()
        fresh = await self.endpoint.client()
        self.assertIsNot(fresh, client)
        self.assertIs(await self.endpoint.client(), fresh)

    async def test_the_device_answers_while_its_client_asks_on_one_address(self):
        _, peer_address = await self.server(PEER)
        heard = []

        def raw(datagram, source):
            heard.append(source)
            return [read_property_ack(datagram, 4.5)] if datagram[:2] == b"\x81\x0a" else None

        raw_peer = RawPeer(raw)
        self.addCleanup(raw_peer.close)
        client = await self.endpoint.client()
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1") as other:
            served = [other.read_property(self.local, AI_1, P.PRESENT_VALUE) for _ in range(5)]
            asked = [client.read_property(peer_address, AI_1, P.PRESENT_VALUE) for _ in range(5)]
            raw_read = client.read_property(raw_peer.address, AI_1, P.PRESENT_VALUE)
            results = await asyncio.wait_for(asyncio.gather(*served, *asked, raw_read), 10)
        self.assertEqual(results[:5], [rb.PropertyValue.real(21.5)] * 5)
        self.assertEqual(results[5:10], [rb.PropertyValue.real(float(PEER))] * 5)
        self.assertEqual(results[10], rb.PropertyValue.real(4.5))
        # The client's request left from the endpoint's own socket.
        self.assertEqual({"%s:%d" % source for source in heard}, {self.local})
        status = await self.endpoint.status()
        self.assertEqual(status["active_leases"], 0)
        self.assertGreaterEqual(status["client_seat_apdus"], 6)
        self.assertEqual(status["client_seat_full_drops"], 0)
        self.assertEqual(status["client_seat_control_full_drops"], 0)

    async def test_discovery_directed_and_by_range_keeps_duplicate_instances(self):
        _, first = await self.server(TWIN)
        _, second = await self.server(TWIN)  # the same instance at another address
        client = await self.endpoint.client()
        [row] = await client.discover(scope=first, wait=0.5)
        self.assertEqual((row.address, row.instance, row.duplicate), (first, TWIN, False))
        [row] = await client.discover(scope=second, low=TWIN, high=TWIN, wait=0.5)
        self.assertEqual((row.address, row.instance, row.duplicate), (second, TWIN, True))
        # Outside the range nobody answers.
        self.assertEqual(await client.discover(scope=first, low=1, high=2, wait=0.3), [])
        # Both addresses stay in the device table.
        rows = [entry for entry in await client.discovered_devices()
                if entry.object_identifier.instance == TWIN]
        self.assertEqual(len(rows), 2)
        # A bare instance two addresses claim is ambiguous; an address isn't.
        with self.assertRaises(rb.BacnetAmbiguousDeviceError):
            await client.device(TWIN)
        device = await client.device(second)
        self.assertEqual((await device.read(AI_1, P.PRESENT_VALUE)).value,
                         rb.PropertyValue.real(float(TWIN)))
        # A local Who-Is goes out on the endpoint's link too.
        self.assertIsNone(await client.who_is(low=TWIN, high=TWIN))

    async def test_file_reads_keep_short_and_empty_blocks_and_empty_records(self):
        server = rb.BACnetServer(PEER, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        server.add_file(1, "Stream")
        server.set_file_data(1, b"0123456789")
        server.add_file(2, "Records")
        server.set_file_access_method(2, "record")
        server.set_file_records(2, [b"one", b"", b"three"])
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(finish, server.stop)
        address = await server.local_address()
        client = await self.endpoint.client()
        stream = rb.ObjectIdentifier(O.FILE, 1)
        first = await client.atomic_read_file(address, stream, "stream", start_position=0,
                                              requested_octet_count=8)
        self.assertEqual((first["data"], first["end_of_file"]), (b"01234567", False))
        last = await client.atomic_read_file(address, stream, "stream", start_position=8,
                                             requested_octet_count=8)
        self.assertEqual((last["start_position"], last["data"], last["end_of_file"]),
                         (8, b"89", True))
        records = await client.atomic_read_file(address, rb.ObjectIdentifier(O.FILE, 2), "record",
                                                start_record=0, requested_record_count=10)
        self.assertEqual(records, {"access_method": "record", "end_of_file": True,
                                   "start_record": 0, "record_count": 3,
                                   "records": [b"one", b"", b"three"]})

        # An empty final block, as a device sends one: end of file, no data.
        def empty_final(datagram, source):
            if datagram[:2] != b"\x81\x0a":
                return None
            invoke = datagram[4 + 2 + 2]
            body = bytes([0x30, invoke, 6]) + b"\x11\x0e\x31\x0a\x60\x0f"
            return [bvll(0x0A, b"\x01\x00" + body)]

        device = RawPeer(empty_final)
        self.addCleanup(device.close)
        empty = await client.atomic_read_file(device.address, stream, "stream",
                                              start_position=10, requested_octet_count=8)
        self.assertEqual(empty, {"access_method": "stream", "end_of_file": True,
                                 "start_position": 10, "data": b""})

    async def test_bbmd_tables_and_the_legacy_writes_on_the_endpoint_socket(self):
        _, bbmd = await self.server(PEER, bbmd=rb.Bbmd(
            bdt=[("127.0.0.2:47808", "255.255.255.0")], accept_foreign_devices=True,
            management_acl=["127.0.0.1"]))
        client = await self.endpoint.client()
        rows = await asyncio.wait_for(client.read_bdt(bbmd), 10)
        self.assertIn(("127.0.0.2:47808", "255.255.255.0"), rows)
        # The endpoint's own table reads stay, and see the same table.
        self.assertEqual(rows, await self.endpoint.read_bdt(bbmd))
        self.assertIsNone(await client.register_foreign_device_bvlc(bbmd, 60))
        [entry] = await client.read_fdt(bbmd)
        self.assertEqual(entry["address"], self.local)
        # A 2020 BBMD refuses Write-BDT with its NAK (Annex J.4.4.2).
        with self.assertRaises(rb.BacnetBvlcNakError) as refused:
            await asyncio.wait_for(client.write_bdt(bbmd, [bbmd]), 10)
        self.assertEqual(refused.exception.result_code, 0x0010)

        # An older BBMD still takes Write-BDT and Delete-FDT-Entry: the
        # legacy writes stay on the endpoint's client as on a standalone one.
        def older(datagram, source):
            if datagram[:2] in (b"\x81\x01", b"\x81\x08"):  # Write-BDT, Delete-FDT-Entry
                return [bvll(0x00, b"\x00\x00")]
            return None

        old = RawPeer(older)
        self.addCleanup(old.close)
        self.assertIsNone(await client.write_bdt(old.address, [old.address]))
        self.assertIsNone(await client.delete_fdt_entry(old.address, "127.0.0.1:47999"))
        [(write, source), (delete, _)] = old.received
        self.assertEqual(write[:2], b"\x81\x01")
        self.assertEqual(write[4:10], mac(old.address))
        self.assertEqual(delete[:2], b"\x81\x08")
        self.assertEqual("%s:%d" % source, self.local)
        # SC-only calls refuse plainly on a B/IP endpoint's client.
        with self.assertRaisesRegex(RuntimeError, "SC client"):
            await client.connection_status()

    async def test_cov_notifications_reach_the_endpoints_client(self):
        server, address = await self.server(PEER)
        client = await self.endpoint.client()
        notifications = await client.cov_notifications()
        await client.subscribe_cov(address, 1, AI_1, confirmed=False, lifetime=60)
        await client.subscribe_cov(address, 2, AI_1, confirmed=True, lifetime=60)
        await server.set_present_value_local(AI_1, rb.PropertyValue.real(15.0))
        delivered = set()
        async with asyncio.timeout(5):
            async for notification in notifications:
                delivered.add((notification.subscriber_process_identifier,
                               notification.delivery))
                if len(delivered) == 2:
                    break
        self.assertEqual(delivered, {(1, "unconfirmed"), (2, "confirmed")})

    async def test_frames_are_the_endpoints_and_end_with_it(self):
        client = await self.endpoint.client()
        mine = await client.frames(capacity=16)
        its = await self.endpoint.frames(capacity=16)
        peer = silent_peer(self)
        with self.assertRaises(rb.BacnetError):
            await client.read_property(peer.address, AI_1, P.PRESENT_VALUE)
        # Both iterators see the client's request leave the one socket.
        for frames in (mine, its):
            async with asyncio.timeout(5):
                async for frame in frames:
                    if frame.direction == "outbound" and frame.peer == peer.address:
                        break
        await asyncio.wait_for(self.endpoint.close(), 10)
        for frames in (mine, its):
            with self.assertRaises(StopAsyncIteration):
                async with asyncio.timeout(5):
                    while True:
                        await frames.__anext__()

    async def test_client_stop_ends_the_role_alone(self):
        client = await self.endpoint.client()
        await client.stop()
        await client.stop()  # twice is fine
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1") as other:
            self.assertEqual(await other.read_property(self.local, AI_1, P.PRESENT_VALUE),
                             rb.PropertyValue.real(21.5))
        # The server role answers on: its own read sees the object.
        server = await self.endpoint.server()
        self.assertEqual(await server.read_property(AI_1, P.PRESENT_VALUE),
                         rb.PropertyValue.real(21.5))
        fresh = await self.endpoint.client()
        _, peer_address = await self.server(PEER)
        self.assertEqual(await fresh.read_property(peer_address, AI_1, P.PRESENT_VALUE),
                         rb.PropertyValue.real(float(PEER)))

    async def test_a_client_still_stopping_is_not_replaced(self):
        client = await self.endpoint.client()
        peer = silent_peer(self)
        # A read in flight holds the stop until it ends.
        read = asyncio.ensure_future(client.read_property(peer.address, AI_1, P.PRESENT_VALUE))
        async with asyncio.timeout(5):
            while not peer.received:
                await asyncio.sleep(0.01)
        stop = asyncio.ensure_future(client.stop())
        # The stop is admitted once the client refuses new work.
        async with asyncio.timeout(5):
            while True:
                try:
                    await client.who_is()
                except RuntimeError as error:
                    self.assertIn("closing", str(error))
                    break
                await asyncio.sleep(0.01)
        with self.assertRaisesRegex(RuntimeError, "stopping"):
            await self.endpoint.client()
        read.cancel()
        await asyncio.gather(read, return_exceptions=True)
        await asyncio.wait_for(stop, 5)
        self.assertIsNot(await self.endpoint.client(), client)

    async def test_cancelled_station_discovery_releases_the_shared_client(self):
        client = await self.endpoint.client()
        loop = asyncio.get_running_loop()
        received = asyncio.Queue()

        def observe(datagram, source):
            if datagram[:2] == b"\x81\x0a":
                loop.call_soon_threadsafe(received.put_nowait, (datagram, source))
            return ()

        peer = RawPeer(observe)
        self.addCleanup(peer.close)
        target = rb.RoutedTarget(5, b"\x07", router=peer.address)
        async with asyncio.timeout(10):
            pending = asyncio.ensure_future(client.discover(scope=target, wait=60))
            datagram, source = await received.get()
            self.assertEqual(datagram[4:], bytes.fromhex("01 20 00 05 01 07 ff 10 08"))
            self.assertEqual("%s:%d" % source, self.local)
            self.assertFalse(pending.done())
            pending.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await pending
            await client.who_is(scope=target)
            self.assertEqual(await received.get(), (datagram, source))
            await client.stop()

    async def test_cancelling_a_call_in_flight_leaves_the_client_usable(self):
        client = await self.endpoint.client()
        peer = silent_peer(self)
        read = asyncio.ensure_future(client.read_property(peer.address, AI_1, P.PRESENT_VALUE))
        async with asyncio.timeout(5):
            while not peer.received:
                await asyncio.sleep(0.01)
        read.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await read
        async with asyncio.timeout(5):
            while (await self.endpoint.status())["active_leases"]:
                await asyncio.sleep(0.01)
        device = rb.ObjectIdentifier(O.DEVICE, ENDPOINT)
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1") as other:
            self.assertEqual(await other.read_property(self.local, device, P.OBJECT_NAME),
                             rb.PropertyValue.character_string("Scanner"))
        _, peer_address = await self.server(PEER)
        self.assertEqual(await client.read_property(peer_address, AI_1, P.PRESENT_VALUE),
                         rb.PropertyValue.real(float(PEER)))

    async def test_endpoint_close_with_calls_in_flight_stops_the_client_first(self):
        client = await self.endpoint.client()
        peer = silent_peer(self)
        reads = [asyncio.ensure_future(client.read_property(peer.address, AI_1, P.PRESENT_VALUE))
                 for _ in range(3)]
        async with asyncio.timeout(5):
            while len(peer.received) < 3:
                await asyncio.sleep(0.01)
        loop = asyncio.get_running_loop()
        started = loop.time()
        # Well inside the 2 s request timeout: the session's stop fails the
        # open requests and releases the link, then the client closes.
        await asyncio.wait_for(self.endpoint.close(), 1.5)
        for read in reads:
            with self.assertRaisesRegex(rb.BacnetError, "the client stopped while it waited"):
                await asyncio.wait_for(read, 1)
        self.assertLess(loop.time() - started, 1.5)
        with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
            await client.who_is()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await self.endpoint.client()

    async def test_a_restarted_endpoint_hands_out_a_new_client(self):
        client = await self.endpoint.client()
        await asyncio.wait_for(self.endpoint.close(), 10)
        await asyncio.wait_for(self.endpoint.start(), 10)
        fresh = await self.endpoint.client()
        self.assertIsNot(fresh, client)
        _, peer_address = await self.server(PEER)
        self.assertEqual(await fresh.read_property(peer_address, AI_1, P.PRESENT_VALUE),
                         rb.PropertyValue.real(float(PEER)))
        # The old client stays stopped on the new session, and sends nothing.
        peer = silent_peer(self)
        for call in (lambda: client.read_property(peer.address, AI_1, P.PRESENT_VALUE),
                     lambda: client.who_is(),
                     lambda: client.read_bdt(peer.address)):
            with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
                await asyncio.wait_for(call(), 5)
        await asyncio.sleep(0.2)
        self.assertEqual(peer.received, [])


class AuditedEndpointTests(unittest.IsolatedAsyncioTestCase):
    """An endpoint with a source Audit Reporter audits its client's reads
    and writes (#2617 step 3), each request owned by the endpoint once
    admitted, so a caller that is cancelled loses neither it nor its
    record."""

    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        sink = rb.BACnetServer(SINK, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1")
        sink.add_audit_log(1, "sink", str(Path(directory.name) / "audit"), buffer_size=32)
        sink.configure_audit_notification_sink(1, policy="allow_all")
        await asyncio.wait_for(sink.start(), 10)
        self.addAsyncCleanup(finish, sink.stop)
        self.sink_address = await sink.local_address()
        sink_device = rb.ObjectIdentifier(O.DEVICE, SINK)
        # A recipient on 127.0.0.1 can't share the link's broadcast address.
        self.endpoint = endpoint(
            broadcast_address="127.255.255.255",
            source_audit_reporter=rb.ObjectIdentifier(O.AUDIT_REPORTER, 1),
            audit_recipient={"kind": "device", "object_identifier": sink_device},
            audit_device_bindings=[(sink_device, self.sink_address)])
        self.endpoint.add_audit_reporter(
            1, "Source", audit_level="audit_all",
            auditable_operations=(1 << rb.AuditOperation.READ.to_raw())
            | (1 << rb.AuditOperation.WRITE.to_raw()),
            issue_confirmed_notifications=True)
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)

    async def records(self, count):
        """The sink's records of this endpoint's operations, once there are
        `count` of them."""
        query = {"audit_log": rb.ObjectIdentifier(O.AUDIT_LOG, 1),
                 "query_parameters": {"kind": "by_source", "successful_actions_only": 0,
                                      "source_device_identifier": rb.ObjectIdentifier(O.DEVICE, ENDPOINT)},
                 "requested_count": 32}
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1") as reader:
            async with asyncio.timeout(5):
                while True:
                    result = await reader.audit_log_query_typed(self.sink_address, query)
                    found = [entry["record"]["datum"]["audit_notification"]
                             for entry in result["records"]]
                    if len(found) >= count:
                        return found
                    await asyncio.sleep(0.02)

    async def test_reads_and_writes_are_recorded(self):
        target = rb.BACnetServer(PEER, interface="127.0.0.1", port=0,
                                 broadcast_address="127.0.0.1")
        target.add_analog_input(1, "Target temperature", 62, 12.5)
        await asyncio.wait_for(target.start(), 10)
        self.addAsyncCleanup(finish, target.stop)
        address = await target.local_address()
        client = await self.endpoint.client()
        self.assertIs(type(client), rb.BACnetClient)
        self.assertEqual(await client.read_property(address, AI_1, P.PRESENT_VALUE),
                         rb.PropertyValue.real(12.5))
        # A write with a priority is a command at that priority.
        await client.write_property(address, AI_1, P.DESCRIPTION,
                                    rb.PropertyValue.character_string("audited"), priority=8)
        # The sink keeps the two records in no promised order.
        found = {record["operation"]: record for record in await self.records(2)}
        read, write = found[rb.AuditOperation.READ], found[rb.AuditOperation.WRITE]
        station = {"kind": "address", "network_number": 0, "mac_address": mac(address)}
        for record in (read, write):
            self.assertEqual(record["target_device"], station)
            self.assertEqual(str(record["target_object"]), str(AI_1))
            self.assertIsNone(record.get("result"))
        self.assertEqual(write["target_priority"], 8)
        self.assertIsNone(read.get("target_priority"))

    async def test_a_cancelled_read_is_still_recorded(self):
        heard = []
        peer = RawPeer(lambda datagram, source: heard.append((datagram, source)))
        self.addCleanup(peer.close)
        client = await self.endpoint.client()
        read = asyncio.ensure_future(client.read_property(peer.address, AI_1, P.PRESENT_VALUE))
        async with asyncio.timeout(5):
            while not heard:
                await asyncio.sleep(0.01)
        read.cancel()
        await asyncio.gather(read, return_exceptions=True)
        # The answer comes after its caller went away: the endpoint still
        # takes it and records the read.
        datagram, source = heard[0]
        peer.send(read_property_ack(datagram, 1.5), source)
        [record] = await self.records(1)
        self.assertEqual(record["operation"], rb.AuditOperation.READ)
        self.assertEqual(record["invoke_id"], datagram[8])
        self.assertIsNone(record.get("result"))

if __name__ == "__main__":
    unittest.main()
