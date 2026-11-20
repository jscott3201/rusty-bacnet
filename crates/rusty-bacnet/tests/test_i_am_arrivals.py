"""One record per I-Am received (#2612): ``i_am_arrivals()`` beside ``frames()``.

The devices are raw B/IP peers on 127.0.0.1 (``bvll_peer.RawPeer``), so a
test can make two of them claim one instance, forward an I-Am as a BBMD
would, or send one that doesn't decode. The routed case goes through a real
two-port ``BACnetRouter``. MS/TP has no in-memory wire reachable from
Python; its fields are covered by the Rust tests (``bacnet-ops``'s
``arrival_tests.rs`` and ``src/i_am_arrivals_tests.rs``).
"""
import asyncio
import gc
import struct
import time
import unittest

import rusty_bacnet as rb
import test_frames
from bvll_peer import RawPeer, bvll, mac
from port_retry import rerun_on_lost_port
from sc_client_fixture import ClientScFixture, mtls
from test_bip6_vmac import free_udp6_port, ipv6_loopback
from test_bip_modes import accepted_registration
from test_router import exclusive, port

ORIGINAL_UNICAST, ORIGINAL_BROADCAST, FORWARDED = 0x0A, 0x0B, 0x04
REGISTER_FOREIGN_DEVICE, DISTRIBUTE_BROADCAST = 0x05, 0x09


def unsigned(value):
    """An application-tagged Unsigned, in as few octets as it takes."""
    length = max(1, (value.bit_length() + 7) // 8)
    return bytes([0x20 | length]) + value.to_bytes(length, "big")


def i_am(instance, *, object_type=8, max_apdu=1476, segmentation=3, vendor=260):
    """An I-Am APDU: Unconfirmed-Request, service choice 0, its four
    application-tagged parameters."""
    identifier = (object_type << 22) | instance
    return (bytes([0x10, 0x00, 0xC4]) + identifier.to_bytes(4, "big")
            + unsigned(max_apdu) + bytes([0x91, segmentation]) + unsigned(vendor))


def local(apdu):
    """``apdu`` in an NPDU with no routing."""
    return b"\x01\x00" + apdu


def answering(instance):
    """A device that answers each Who-Is with a unicast I-Am."""
    def handler(data, source):
        if data[:1] == b"\x81" and data.endswith(b"\x10\x08"):
            return [bvll(ORIGINAL_UNICAST, local(i_am(instance)))]
        return None
    return handler


class Arrivals(unittest.IsolatedAsyncioTestCase):
    async def client(self):
        client = rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        await client.__aenter__()

        async def stop():
            await asyncio.wait_for(client.stop(), 10)
        self.addAsyncCleanup(stop)
        return client

    def peer(self, handler=None):
        peer = RawPeer(handler or (lambda data, source: None))
        self.addCleanup(peer.close)
        return peer

    async def next(self, arrivals):
        # Outer liveness guard, never a timing assertion.
        return await asyncio.wait_for(anext(arrivals), 10)

    async def address_of(self, client):
        """The client's own "ip:port", as its frames name it."""
        frames = await client.frames()
        await client.who_is(scope=self.peer().address)
        address = (await asyncio.wait_for(anext(frames), 10)).local_address
        await frames.aclose()
        return address

    async def endpoint(self, instance, **options):
        endpoint = rb.BipEndpoint(instance, interface="127.0.0.1", port=0, **options)
        await endpoint.start()

        async def close():
            await endpoint.close()
        self.addAsyncCleanup(close)
        return endpoint

    @staticmethod
    def to(address):
        host, number = address.rsplit(":", 1)
        return host, int(number)


class IAmArrivalTests(Arrivals):
    async def test_duplicate_instances_and_repeats_are_kept_in_order(self):
        client = await self.client()
        arrivals = await client.i_am_arrivals()
        self.assertIsInstance(arrivals, rb.IAmArrivalIterator)
        one, two = self.peer(answering(4242)), self.peer(answering(4242))
        seen = []
        for device in (one, two, one):
            await client.who_is(scope=device.address)
            seen.append(await self.next(arrivals))
        self.assertEqual([r.link_sender for r in seen], [one.address, two.address, one.address])
        for record in seen:
            self.assertIsInstance(record, rb.IAmArrival)
            self.assertEqual(record.object_identifier,
                             rb.ObjectIdentifier(rb.ObjectType.DEVICE, 4242))
            self.assertEqual((record.max_apdu_length, record.vendor_id), (1476, 260))
            self.assertEqual(record.segmentation_supported, rb.Segmentation.NO_SEGMENTATION)
            self.assertEqual(record.link, "bip")
            self.assertEqual(record.link_source, record.link_sender)
            self.assertEqual(record.device_address, record.link_sender)
            self.assertFalse(record.forwarded)
            self.assertFalse(record.broadcast, "answered by unicast")
            for absent in ("original_source", "source_network", "source_address",
                           "next_hop_router", "forwarding_bbmd"):
                self.assertIsNone(getattr(record, absent), absent)
            self.assertEqual(record.missed_before, 0)
            with self.assertRaises(AttributeError):
                record.forwarded = True
        # Equal fields, other arrival times: two records.
        self.assertNotEqual(seen[0], seen[2])
        self.assertEqual((arrivals.missed, arrivals.malformed), (0, 0))

    async def test_arrivals_follow_python_s_clock(self):
        client = await self.client()
        arrivals = await client.i_am_arrivals()
        device = self.peer(answering(7))
        before = time.monotonic(), time.time()
        await client.who_is(scope=device.address)
        records = [await self.next(arrivals)]
        await client.who_is(scope=device.address)
        records.append(await self.next(arrivals))
        after = time.monotonic(), time.time()
        test_frames.FrameTests.assert_captured_between(self, records, before, after)

    async def test_routed_arrivals_keep_both_npdu_addresses(self):
        import pickle
        client = await self.client()
        arrivals = await client.i_am_arrivals()
        peer = self.peer()
        target = self.to(await self.address_of(client))
        for dnet, dadr in ((9, bytes([17, 18])), (9, b""), (65535, b"")):
            # Include both a destination (DNET/DADR) and a different source (SNET/SADR).
            npdu = (bytes([1, 0x28]) + dnet.to_bytes(2, "big") + bytes([len(dadr)]) + dadr
                    + bytes([0, 5, 1, 7, 255]) + i_am(77))
            peer.send(bvll(ORIGINAL_UNICAST, npdu), target)
            record = await self.next(arrivals)
            self.assertEqual((record.source_network, record.source_address), (5, bytes([7])))
            self.assertEqual((record.destination_network, record.destination_address), (dnet, dadr))
            self.assertEqual(pickle.loads(pickle.dumps(record)), record)
        peer.send(bvll(ORIGINAL_UNICAST, local(i_am(77))), target)
        direct = await self.next(arrivals)
        self.assertEqual((direct.destination_network, direct.destination_address), (None, None))

    async def test_a_forwarded_i_am_names_the_device_and_the_bbmd(self):
        client = await self.client()
        target = self.to(await self.address_of(client))
        arrivals = await client.i_am_arrivals()
        bbmd = self.peer()
        bbmd.send(bvll(FORWARDED, mac("192.0.2.3:47808") + local(i_am(77))), target)
        record = await self.next(arrivals)
        self.assertEqual(record.link_sender, bbmd.address)
        self.assertEqual(record.forwarding_bbmd, bbmd.address)
        self.assertTrue(record.forwarded)
        for field in ("link_source", "original_source", "device_address"):
            self.assertEqual(getattr(record, field), "192.0.2.3:47808", field)
        self.assertTrue(record.broadcast)
        self.assertIsNone(record.next_hop_router)

    async def test_a_client_leaves_nothing_out(self):
        # A client sends no I-Am, so even one from its own address, as a
        # BBMD would return a node's own broadcast, is another node's.
        client = await self.client()
        own = await self.address_of(client)
        arrivals = await client.i_am_arrivals()
        bbmd = self.peer()
        bbmd.send(bvll(FORWARDED, mac(own) + local(i_am(77))), self.to(own))
        record = await self.next(arrivals)
        self.assertEqual((record.object_identifier.instance, record.link_source), (77, own))
        self.assertEqual(arrivals.own_echoes, 0)

    async def test_malformed_i_ams_are_counted_and_odd_ones_kept(self):
        client = await self.client()
        target = self.to(await self.address_of(client))
        arrivals = await client.i_am_arrivals()
        device = self.peer()
        # Parameters cut short, then a vendor octet too many.
        device.send(bvll(ORIGINAL_BROADCAST, local(i_am(5)[:6])), target)
        device.send(bvll(ORIGINAL_BROADCAST, local(i_am(5) + b"\x00")), target)
        # What an old device may send: another object type, a max APDU
        # below 50, vendor 0 and a segmentation value of no name.
        odd = i_am(9, object_type=0, max_apdu=49, segmentation=9, vendor=0)
        device.send(bvll(ORIGINAL_BROADCAST, local(odd)), target)
        record = await self.next(arrivals)
        self.assertEqual(arrivals.malformed, 2)
        self.assertEqual(record.object_identifier,
                         rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 9))
        self.assertEqual((record.max_apdu_length, record.vendor_id), (49, 0))
        self.assertEqual(record.segmentation_supported.to_raw(), 9)

    async def test_a_full_queue_reports_the_gap_before_the_next_record(self):
        client = await self.client()
        target = self.to(await self.address_of(client))
        slow = await client.i_am_arrivals(capacity=1)
        witness = await client.frames(capacity=8)
        device = self.peer()
        frame = bvll(ORIGINAL_BROADCAST, local(i_am(11)))
        for _ in range(3):
            device.send(frame, target)
            await asyncio.wait_for(anext(witness), 10)  # Fence the receipt.
        self.assertEqual(slow.missed, 2)
        self.assertEqual((await self.next(slow)).missed_before, 0)
        device.send(frame, target)
        self.assertEqual((await self.next(slow)).missed_before, 2)
        await slow.aclose()
        self.assertEqual(slow.missed, 2)
        with self.assertRaises(StopAsyncIteration):
            await self.next(slow)

    async def test_an_outbound_i_am_is_left_out(self):
        # A foreign device's broadcast goes to its BBMD alone (Annex J.5),
        # so nothing comes back to the endpoint's own address.
        def bbmd(data, source):
            if data[:2] == bytes([0x81, REGISTER_FOREIGN_DEVICE]):
                return [bvll(0x00, b"\x00\x00")]
            return None
        server = self.peer(bbmd)
        endpoint = await self.endpoint(750, foreign_device=rb.ForeignDevice(server.address, ttl=60))
        await accepted_registration(endpoint)
        own = await endpoint.local_address()
        arrivals = await endpoint.i_am_arrivals()
        frames = await endpoint.frames()
        await endpoint.broadcast_i_am()
        while True:
            frame = await asyncio.wait_for(anext(frames), 10)
            summary = rb.decode_frame(frame.octets, frame.link)
            if summary.service == "i-am":
                break
        self.assertEqual((frame.direction, frame.peer, summary.device),
                         ("outbound", server.address, 750))
        server.send(bvll(ORIGINAL_UNICAST, local(i_am(751))), self.to(own))
        record = await self.next(arrivals)
        self.assertEqual(record.object_identifier.instance, 751)

    async def test_this_nodes_own_i_am_come_back_is_left_out(self):
        endpoint = await self.endpoint(720, broadcast_address="127.0.0.1")
        own = await endpoint.local_address()
        arrivals = await endpoint.i_am_arrivals()
        frames = await endpoint.frames()
        peer = self.peer()
        # The endpoint's own I-Am goes out, and comes back to its socket as
        # the broadcast to 127.0.0.1 it is: both frames are tapped.
        await endpoint.broadcast_i_am()
        seen = []
        while len(seen) < 2:
            frame = await asyncio.wait_for(anext(frames), 10)
            summary = rb.decode_frame(frame.octets, frame.link)
            if summary.service == "i-am":
                seen.append((frame.direction, frame.peer, summary.device))
        self.assertEqual(seen, [("outbound", own, 720), ("inbound", own, 720)])
        # Its own I-Am come back through a BBMD, then another device's.
        peer.send(bvll(FORWARDED, mac(own) + local(i_am(720))), self.to(own))
        peer.send(bvll(ORIGINAL_BROADCAST, local(i_am(721))), self.to(own))
        record = await self.next(arrivals)
        self.assertEqual((record.object_identifier.instance, record.link_sender),
                         (721, peer.address))
        # Both echoes, the direct one and the one the BBMD returned.
        self.assertEqual((arrivals.own_echoes, arrivals.malformed, arrivals.missed), (2, 0, 0))
        self.assertEqual(record.missed_before, 0, "an echo is no loss")
        # Behind NAT another site may reuse this address (Annex J.7.5): a
        # forwarded origin equal to it, of another instance, is a record.
        peer.send(bvll(FORWARDED, mac(own) + local(i_am(9999))), self.to(own))
        record = await self.next(arrivals)
        self.assertEqual((record.object_identifier.instance, record.link_source), (9999, own))
        self.assertTrue(record.forwarded)
        self.assertEqual(arrivals.own_echoes, 2)

    async def test_an_endpoint_sees_arrivals_on_the_socket_its_roles_share(self):
        endpoint = await self.endpoint(730)
        other = await self.endpoint(731)
        arrivals = await endpoint.i_am_arrivals()
        device = self.peer()
        device.send(bvll(ORIGINAL_BROADCAST, local(i_am(4242))),
                    self.to(await endpoint.local_address()))
        oid = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 731)
        client = await endpoint.client()
        value = await client.read_property(await other.local_address(), oid,
                                           rb.PropertyIdentifier.OBJECT_IDENTIFIER)
        self.assertEqual(value, rb.PropertyValue.object_identifier(oid))
        record = await self.next(arrivals)
        self.assertEqual((record.object_identifier.instance, record.link_sender),
                         (4242, device.address))
        self.assertTrue(record.broadcast)
        pending = asyncio.ensure_future(anext(arrivals))
        await endpoint.close()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)
        with self.assertRaises(RuntimeError):
            await endpoint.i_am_arrivals()

    async def test_cancel_close_and_stop(self):
        client = await self.client()
        arrivals = await client.i_am_arrivals()
        pending = asyncio.ensure_future(anext(arrivals))
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        device = self.peer(answering(3))
        await client.who_is(scope=device.address)
        self.assertEqual((await self.next(arrivals)).object_identifier.instance, 3)
        waiting = [asyncio.ensure_future(anext(arrivals)) for _ in range(3)]
        await arrivals.aclose()
        await arrivals.aclose()
        for waiter in waiting:
            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(waiter, 10)
        # Closing observation leaves the client working.
        later = await client.i_am_arrivals()
        pending = asyncio.ensure_future(anext(later))
        await client.stop()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)
        with self.assertRaises(RuntimeError):
            await client.i_am_arrivals()

    async def test_the_owner_is_not_kept_alive(self):
        endpoint = rb.BipEndpoint(740, interface="127.0.0.1", port=0)
        await endpoint.start()
        arrivals = await endpoint.i_am_arrivals()
        pending = asyncio.ensure_future(anext(arrivals))
        del endpoint
        gc.collect()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)

    async def test_arguments_and_inactive_owners_refuse(self):
        endpoint = rb.BipEndpoint(741, interface="127.0.0.1", port=0)
        client = rb.BACnetClient(interface="127.0.0.1", port=0)
        for owner in (endpoint, client):
            for capacity in (0, 65537):
                with self.assertRaises(ValueError):
                    owner.i_am_arrivals(capacity=capacity)
            with self.assertRaises(RuntimeError):
                await owner.i_am_arrivals()


class RoutedArrivalTests(Arrivals):
    """An I-Am from network 5 through a real router, to a client that is a
    foreign device on network 1."""

    async def asyncSetUp(self):
        self.router = rb.BACnetRouter(
            [port(n, bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True))
             for n in (1, 5)])
        await self.router.start()

        async def close():
            await asyncio.wait_for(self.router.close(), 10)
        self.addAsyncCleanup(close)
        self.ports = await self.router.ports()
        self.routed_client = rb.BACnetClient(
            interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            foreign_device=rb.ForeignDevice(self.ports[0].address, ttl=60))
        await self.routed_client.__aenter__()

        async def stop():
            await asyncio.wait_for(self.routed_client.stop(), 10)
        self.addAsyncCleanup(stop)
        await accepted_registration(self.routed_client)

    async def test_the_device_by_network_and_the_router_as_next_hop(self):
        arrivals = await self.routed_client.i_am_arrivals()
        device = self.peer()
        network_5 = self.to(self.ports[1].address)
        # The device registers with network 5's BBMD, then broadcasts its
        # I-Am to every network (DNET 65535), which the router passes on.
        device.send(bvll(REGISTER_FOREIGN_DEVICE, struct.pack(">H", 60)), network_5)
        deadline = asyncio.get_running_loop().time() + 5
        while not any(data[:6] == b"\x81\x00\x00\x06\x00\x00" for data, _ in device.received):
            self.assertLess(asyncio.get_running_loop().time(), deadline, "registered")
            await asyncio.sleep(0.02)
        globally = b"\x01\x20\xff\xff\x00\xff" + i_am(5005)
        device.send(bvll(DISTRIBUTE_BROADCAST, globally), network_5)
        record = await self.next(arrivals)
        while record.object_identifier.instance != 5005:
            record = await self.next(arrivals)
        router = self.ports[0].address
        self.assertEqual(record.source_network, 5)
        self.assertEqual(record.source_address, mac(device.address))
        self.assertEqual(record.device_address, "5:0x" + mac(device.address).hex().upper())
        self.assertEqual((record.link_source, record.next_hop_router), (router, router))
        self.assertEqual(record.link_sender, router)
        self.assertFalse(record.forwarded, "the router's own broadcast, not a relay's")
        self.assertIsNone(record.forwarding_bbmd)
        self.assertTrue(record.broadcast)
        # The calls that take an address take the device as a RoutedTarget
        # built from the record.
        target = rb.RoutedTarget(record.source_network, record.source_address,
                                 router=record.next_hop_router)
        self.assertEqual(target, rb.RoutedTarget(5, mac(device.address), router=router))

    async def test_a_routed_record_names_the_device_to_read_through_its_router(self):
        far = rb.BACnetServer(5006, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                              foreign_device=rb.ForeignDevice(self.ports[1].address, ttl=60))
        far.add_analog_input(1, "Far temperature", 62, 12.5)
        await asyncio.wait_for(far.start(), 10)

        async def stop():
            await asyncio.wait_for(far.stop(), 10)
        self.addAsyncCleanup(stop)
        await accepted_registration(far)
        arrivals = await self.routed_client.i_am_arrivals()
        await self.routed_client.who_is(scope=5)
        record = await self.next(arrivals)
        while record.object_identifier.instance != 5006:
            record = await self.next(arrivals)
        self.assertEqual(record.source_network, 5)
        self.assertEqual(record.next_hop_router, self.ports[0].address)
        target = rb.RoutedTarget(record.source_network, record.source_address,
                                 router=record.next_hop_router)
        value = await asyncio.wait_for(self.routed_client.read_property(
            target, rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1),
            rb.PropertyIdentifier.PRESENT_VALUE), 10)
        self.assertEqual(value, rb.PropertyValue.real(12.5))


class SeatedArrivalTests(Arrivals):
    """The client an endpoint's ``client()`` returns: it reads the
    endpoint's link and follows the endpoint's own-echo rule."""

    async def test_a_seated_client_leaves_out_the_endpoints_own_i_am(self):
        endpoint = await self.endpoint(760, broadcast_address="127.0.0.1")
        own = await endpoint.local_address()
        client = await endpoint.client()
        streams = [await client.i_am_arrivals(), await endpoint.i_am_arrivals()]
        frames = await endpoint.frames()
        # Out, and back to the endpoint's socket as the broadcast to
        # 127.0.0.1 it is.
        await endpoint.broadcast_i_am()
        echoed = False
        while not echoed:
            frame = await asyncio.wait_for(anext(frames), 10)
            summary = rb.decode_frame(frame.octets, frame.link)
            echoed = summary.service == "i-am" and frame.direction == "inbound"
        peer = self.peer()
        # Returned by a BBMD, then another instance at the endpoint's address.
        peer.send(bvll(FORWARDED, mac(own) + local(i_am(760))), self.to(own))
        peer.send(bvll(FORWARDED, mac(own) + local(i_am(9999))), self.to(own))
        for arrivals in streams:
            record = await self.next(arrivals)
            self.assertEqual((record.object_identifier.instance, record.link_source), (9999, own))
            self.assertEqual((arrivals.own_echoes, arrivals.missed), (2, 0))

    async def test_seated_and_endpoint_streams_end_with_their_owners(self):
        endpoint = await self.endpoint(761)
        own = await endpoint.local_address()
        client = await endpoint.client()
        mine = await client.i_am_arrivals()
        its = await endpoint.i_am_arrivals()
        # The client's stop ends its stream and leaves the endpoint's.
        pending = asyncio.ensure_future(anext(mine))
        await client.stop()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)
        peer = self.peer()
        peer.send(bvll(ORIGINAL_BROADCAST, local(i_am(5))), self.to(own))
        self.assertEqual((await self.next(its)).object_identifier.instance, 5)
        # The endpoint's close ends both its stream and a new client's.
        again = await (await endpoint.client()).i_am_arrivals()
        waiting = [asyncio.ensure_future(anext(again)), asyncio.ensure_future(anext(its))]
        await endpoint.close()
        for waiter in waiting:
            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(waiter, 10)
        # Neither stream, still held, keeps the socket: its port binds again.
        exclusive(own).close()
        self.assertEqual((mine.missed, its.missed, again.missed), (0, 0, 0))


@unittest.skipUnless(ipv6_loopback(), "this host has no IPv6 loopback")
class Bip6ArrivalTests(unittest.IsolatedAsyncioTestCase):
    async def test_the_vmac_and_the_udp_sender(self):
        await rerun_on_lost_port(self.observe)

    async def observe(self):
        server_port = free_udp6_port()
        client_port = free_udp6_port()
        while client_port == server_port:
            client_port = free_udp6_port()
        server = rb.BACnetServer(1234, device_name="B/IPv6 arrivals", transport="ipv6",
                                 ipv6_interface="::1", port=server_port)
        await asyncio.wait_for(server.start(), 10)
        try:
            async with rb.BACnetClient(transport="ipv6", ipv6_interface="::1", port=client_port,
                                       ipv6_device_instance=4321) as client:
                arrivals = await client.i_am_arrivals()
                vmac = await asyncio.wait_for(client.resolve_vmac(f"[::1]:{server_port}"), 10)
                await client.who_is(scope=vmac)
                record = await asyncio.wait_for(anext(arrivals), 10)
        finally:
            await server.stop()
        self.assertEqual(record.link, "ipv6")
        self.assertEqual(record.object_identifier.instance, 1234)
        self.assertEqual(record.link_sender, f"[::1]:{server_port}")
        self.assertEqual(record.link_source, vmac)
        self.assertEqual(record.device_address, vmac)
        self.assertFalse(record.forwarded)


class ScArrivalTests(ClientScFixture):
    async def test_the_originating_vmac_and_the_hub_connection(self):
        hub, _ = await self.serving_hub()
        client = self.client(await hub.url())
        await asyncio.wait_for(client.__aenter__(), 5)
        arrivals = await client.i_am_arrivals()
        await client.who_is()
        record = await asyncio.wait_for(anext(arrivals), 10)
        await arrivals.aclose()
        self.assertEqual(record.link, "sc")
        self.assertEqual(record.object_identifier.instance, 3000)
        self.assertEqual(record.link_sender, "primary")
        self.assertEqual(record.link_source, "02:00:00:00:00:04")
        self.assertEqual(record.device_address, "02:00:00:00:00:04")
        self.assertFalse(record.forwarded)

    async def test_an_sc_endpoints_client_gives_its_arrivals(self):
        # ScEndpoint has no i_am_arrivals() of its own: its client's reads
        # the endpoint's hub connection.
        hub, _ = await self.serving_hub()
        endpoint = rb.ScEndpoint(device_instance=4106, sc_hub=await hub.url(),
                                 sc_vmac=b"\x02\0\0\0\0\x05",
                                 sc_ca_cert=self.path("site.pem"),
                                 sc_client_cert=self.path("client.pem"),
                                 sc_client_key=self.path("client.key"),
                                 sc_device_uuid=mtls.CLIENT_UUID)
        await asyncio.wait_for(endpoint.start(), 10)

        async def close():
            await asyncio.wait_for(endpoint.close(), 5)
        self.addAsyncCleanup(close)
        self.assertFalse(hasattr(endpoint, "i_am_arrivals"))
        client = await endpoint.client()
        arrivals = await client.i_am_arrivals()
        await client.who_is()
        record = await asyncio.wait_for(anext(arrivals), 10)
        self.assertEqual((record.link, record.object_identifier.instance), ("sc", 3000))
        self.assertEqual((record.link_sender, record.link_source), ("primary", "02:00:00:00:00:04"))
        self.assertEqual(arrivals.own_echoes, 0)


if __name__ == "__main__":
    unittest.main()
