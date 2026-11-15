"""Routed reads on the endpoint client (#2612): RoutedTarget, and
read_property, read_property_multiple, read_range and read_log_page to a
station behind a router, through a real BACnetRouter and against a raw peer
playing the router so each request's NPDU can be read off the wire.

The raw peer (bvll_peer) runs in a thread, since Windows' default event
loop has no add_reader.
"""
import ast
import asyncio
import copy
import inspect
import pickle
import socket
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll, mac
from test_bip_modes import accepted_registration
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)
from test_read_log import FRAME, LOG, NOTIFICATIONS

O = rb.ObjectType
P = rb.PropertyIdentifier
AI_1 = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
READS = ("read_property", "read_property_multiple", "read_range", "read_log_page")
NEAR = 2612
FAR = 2613
SINK = 2614


async def finish(method):
    """Await a native close or stop: its future needs the running loop."""
    await asyncio.wait_for(method(), 10)


def port(network):
    return rb.BipRouterPort(network, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1")


class RoutedTargetTests(unittest.TestCase):
    def test_positional_and_keyword_construction_name_the_same_station(self):
        positional = rb.RoutedTarget(5, "127.0.0.1:47808", "10.0.0.1:47808")
        keywords = rb.RoutedTarget(router="10.0.0.1:47808", address="127.0.0.1:47808",
                                   network=5)
        self.assertEqual(positional, keywords)
        self.assertEqual(positional.network, 5)
        self.assertEqual(positional.address, bytes([127, 0, 0, 1, 0xBA, 0xC0]))
        self.assertEqual(positional.router, bytes([10, 0, 0, 1, 0xBA, 0xC0]))
        self.assertIsNone(rb.RoutedTarget(5, b"\x0a").router)
        self.assertEqual(list(inspect.signature(rb.RoutedTarget).parameters),
                         ["network", "address", "router"])

    def test_each_address_form_is_the_mac_it_names(self):
        forms = {
            "127.0.0.1:47808": bytes([127, 0, 0, 1, 0xBA, 0xC0]),  # a B/IP station
            "bip6:1234": bytes([0x00, 0x04, 0xD2]),                 # a B/IPv6 VMAC
            "bip6:0x0A0B0C": bytes([0x0A, 0x0B, 0x0C]),
            "0a:0b": bytes([0x0A, 0x0B]),                           # hex octets
            "aa:bb:cc:dd:ee:ff": bytes.fromhex("aabbccddeeff"),
            "10": b"\x0a",                                          # one octet, decimal
            "mstp:254": b"\xfe",
        }
        self.assertEqual(rb.RoutedTarget(5, "10"), rb.RoutedTarget(5, b"\x0a"))
        for text, octets in forms.items():
            with self.subTest(text=text):
                target = rb.RoutedTarget(7, text, router=text)
                self.assertEqual(target.address, octets)
                self.assertEqual(target.router, octets)
                # However it is written, it is the same station.
                self.assertEqual(target, rb.RoutedTarget(7, octets, router=octets))
                self.assertEqual(hash(target), hash(rb.RoutedTarget(7, octets, router=octets)))
        self.assertEqual(rb.RoutedTarget(7, bytes(range(18))).address, bytes(range(18)))

    def test_bad_values_raise_before_anything_is_sent(self):
        for args in ((0, b"\x0a"), (65535, b"\x0a"), (5, b""), (5, bytes(19)),
                     (5, ":".join(["00"] * 19)), (5, "[::1]:47808"), (5, "mstp:255"),
                     (5, "not an address"), (5, b"\x0a", b""), (5, b"\x0a", "nowhere")):
            with self.subTest(args=args), self.assertRaises(ValueError):
                rb.RoutedTarget(*args)
        # The string forms spell an MS/TP station, so 255, its broadcast, is
        # refused; as bytes it may be an ARCNET station (whose broadcast is 0).
        for args in ((5, "255"), (5, b"\x0a", "mstp:255")):
            with self.subTest(args=args), self.assertRaisesRegex(ValueError, "broadcast"):
                rb.RoutedTarget(*args)
        self.assertEqual(rb.RoutedTarget(5, b"\xff").address, b"\xff")
        self.assertEqual(rb.RoutedTarget(5, b"\x0a", router=b"\xff").router, b"\xff")
        for args in ((-1, b"\x0a"), (65536, b"\x0a")):
            with self.subTest(args=args), self.assertRaises(OverflowError):
                rb.RoutedTarget(*args)
        for args in ((5, 10), (5, bytearray(b"\x0a")), (5, b"\x0a", 7), (5.0, b"\x0a"),
                     (True, b"\x0a"), (False, b"\x0a"),
                     # The reporter's fork's order: (router, network, address).
                     ("10.0.0.1:47808", 5, b"\x0a")):
            with self.subTest(args=args), self.assertRaises(TypeError):
                rb.RoutedTarget(*args)

    def test_frozen_repr_copy_and_pickle(self):
        target = rb.RoutedTarget(5, "127.0.0.1:47808", router=b"\x0a")
        self.assertEqual(repr(target),
                         r"RoutedTarget(network=5, address=b'\x7f\x00\x00\x01\xba\xc0', "
                         r"router=b'\n')")
        self.assertEqual(eval(repr(target), {"RoutedTarget": rb.RoutedTarget}), target)
        for name, value in (("network", 6), ("address", b"\x01"), ("router", None)):
            with self.subTest(name=name), self.assertRaises(AttributeError):
                setattr(target, name, value)
        copies = [copy.copy(target), copy.deepcopy(target)] + [
            pickle.loads(pickle.dumps(target, protocol))
            for protocol in range(pickle.HIGHEST_PROTOCOL + 1)]
        for copied in copies:
            self.assertIs(type(copied), rb.RoutedTarget)
            self.assertEqual(copied, target)
            self.assertEqual(hash(copied), hash(target))
        self.assertNotEqual(target, rb.RoutedTarget(5, "127.0.0.1:47808"))
        self.assertNotEqual(target, rb.RoutedTarget(6, "127.0.0.1:47808", router=b"\x0a"))
        self.assertNotEqual(target, "127.0.0.1:47808")

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        node = stub_method(classes["RoutedTarget"], "__init__")
        self.assertEqual(runtime_arg_kinds(rb.RoutedTarget), stub_arg_kinds(node))
        self.assertEqual([ast.unparse(arg.annotation) for arg in node.args.args[1:]],
                         ["int", "str | bytes", "str | bytes | None"])
        for name in READS:
            with self.subTest(method=name):
                node = stub_method(classes["EndpointClient"], name)
                self.assertEqual(ast.unparse(node.args.args[1].annotation), "str | RoutedTarget")
                self.assertEqual(runtime_arg_kinds(getattr(rb.EndpointClient, name)),
                                 stub_arg_kinds(node))
        # write_property stays direct.
        node = stub_method(classes["EndpointClient"], "write_property")
        self.assertEqual(ast.unparse(node.args.args[1].annotation), "str")


def answer(network, station, request, value=0):
    """A ComplexAck to the confirmed request in `request`, relayed from
    `station` on `network` as a router relays it (SNET/SADR naming it), or
    sent direct when `network` is None; None for a request this peer
    doesn't answer."""
    apdu = request["apdu"]
    invoke, service, body = apdu[2], apdu[3], apdu[4:]
    if service == 12:    # ReadProperty: echo the reference, value Unsigned `value`
        reply = body + b"\x3e\x21" + bytes([value]) + b"\x3f"
    elif service == 14:  # ReadPropertyMultiple of one property: Unsigned 7
        reply = body[:5] + b"\x1e\x29" + body[7:8] + b"\x4e\x21\x07\x4f\x1f"
    elif service == 26:  # ReadRange: no items, no flags
        reply = body[:7] + b"\x3a\x05\x00\x49\x00\x5e\x5f"
    else:
        return None
    source = b"" if network is None else network.to_bytes(2, "big") + bytes([len(station)]) + station
    control = 0x00 if network is None else 0x08
    return bvll(0x0A, bytes([0x01, control]) + source + bytes([0x30, invoke, service]) + reply)


def request_of(datagram, function=0x0A):
    """The NPDU fields and APDU of an Original-Unicast-NPDU, or of the BVLC
    message with `function` (0x09, a foreign device's broadcast)."""
    assert datagram[:2] == bytes([0x81, function]), datagram
    npdu = datagram[4:]
    control = npdu[1]
    fields = {"control": control, "dnet": None, "dadr": None, "hop_count": None}
    at = 2
    if control & 0x20:
        fields["dnet"] = int.from_bytes(npdu[2:4], "big")
        length = npdu[4]
        fields["dadr"] = bytes(npdu[5:5 + length])
        at = 5 + length
    if control & 0x08:
        raise AssertionError("a request from the endpoint carries no SNET")
    if control & 0x20:
        fields["hop_count"] = npdu[at]
        at += 1
    fields["apdu"] = bytes(npdu[at:])
    return fields


class EndpointRoutedWireTests(unittest.IsolatedAsyncioTestCase):
    """A raw peer is the router: every routed request names the station in
    its NPDU, and the answer relayed with SNET/SADR completes it."""

    STATION = b"\x0a"  # one-octet MAC, as an MS/TP station behind the router

    async def asyncSetUp(self):
        self.requests = []

        def handle(datagram, source):
            if datagram[:2] != b"\x81\x0a":
                return None
            request = request_of(datagram)
            self.requests.append(request)
            reply = answer(request["dnet"], request["dadr"], request)
            return reply and [reply]

        self.router = RawPeer(handle)
        self.addCleanup(self.router.close)
        self.endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                       broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        self.role = await self.endpoint.client()
        self.target = rb.RoutedTarget(5, self.STATION, router=self.router.address)

    def assert_routed(self, requests, service):
        self.assertTrue(requests)
        for request in requests:
            # DNET present, expecting a reply, normal priority, no SNET.
            self.assertEqual(request["control"], 0x24)
            self.assertEqual((request["dnet"], request["dadr"]), (5, self.STATION))
            self.assertEqual(request["hop_count"], 255)
            self.assertEqual(request["apdu"][0] >> 4, 0)  # Confirmed-Request
            self.assertIn(request["apdu"][3], service)

    async def test_every_read_names_the_station_behind_the_router(self):
        value = await asyncio.wait_for(
            self.role.read_property(self.target, AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        self.assert_routed(self.requests, {12})

        self.requests.clear()
        [result] = await asyncio.wait_for(
            self.role.read_property_multiple(self.target, [(AI_1, [(P.PRESENT_VALUE, None)])]), 5)
        self.assertEqual(result["results"][0]["value"], rb.PropertyValue.unsigned(7))
        self.assert_routed(self.requests, {14})

        self.requests.clear()
        for validation in ("strict", "lenient"):
            with self.subTest(validation=validation):
                result = await asyncio.wait_for(self.role.read_range(
                    self.target, LOG, P.LOG_BUFFER, range_type="position",
                    reference_index=1, count=5, validation=validation), 5)
                self.assertEqual((result["item_count"], result["item_data"]), (0, b""))
        self.assert_routed(self.requests, {26})

        # From the oldest record: the three count reads go there too, and
        # find the log empty.
        self.requests.clear()
        page = await asyncio.wait_for(self.role.read_log_page(self.target, LOG), 5)
        self.assertEqual((page["records"], page["done"]), ([], True))
        self.assertEqual(len(self.requests), 3)
        self.assert_routed(self.requests, {12})
        # By position: one ReadRange.
        self.requests.clear()
        page = await asyncio.wait_for(
            self.role.read_log_page(self.target, LOG, ("position", 1), page_size=5), 5)
        self.assertTrue(page["done"])
        self.assertEqual(len(self.requests), 1)
        self.assert_routed(self.requests, {26})
        self.assertEqual((await self.endpoint.status())["active_leases"], 0)

    async def test_a_str_address_is_still_a_direct_station(self):
        value = await asyncio.wait_for(
            self.role.read_property(self.router.address, AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        [request] = self.requests
        self.assertEqual(request["control"], 0x04)  # expecting a reply, no DNET
        self.assertIsNone(request["dnet"])

    async def test_a_wrong_address_type_raises_at_the_call(self):
        calls = {
            "read_property": lambda to: self.role.read_property(to, AI_1, P.PRESENT_VALUE),
            "read_property_multiple": lambda to: self.role.read_property_multiple(
                to, [(AI_1, [(P.PRESENT_VALUE, None)])]),
            "read_range": lambda to: self.role.read_range(to, LOG, P.LOG_BUFFER),
            "read_log_page": lambda to: self.role.read_log_page(to, LOG),
        }
        self.assertEqual(set(calls), set(READS))
        for name, call in calls.items():
            with self.subTest(method=name), \
                    self.assertRaisesRegex(TypeError, "str or a RoutedTarget, not int"):
                call(7)
        self.assertEqual(self.router.received, [])
        # write_property takes a direct address only.
        with self.assertRaises(TypeError):
            self.role.write_property(self.target, AI_1, P.PRESENT_VALUE,
                                     rb.PropertyValue.real(1.0), commandability="commandable")

    async def test_a_group_router_is_refused_before_anything_is_sent(self):
        endpoint = rb.BipEndpoint(NEAR + 10, interface="127.0.0.1", port=0,
                                  broadcast_address="127.255.255.255", apdu_timeout_ms=2000)
        async with endpoint:
            role = await endpoint.client()
            own = (await endpoint.local_address()).rsplit(":", 1)[1]
            # The configured broadcast (a loopback one counts at the link's
            # own port), the limited broadcast and a multicast group.
            for router in (f"127.255.255.255:{own}", "255.255.255.255:47808",
                           "239.255.1.1:47808"):
                with self.subTest(router=router):
                    target = rb.RoutedTarget(5, self.STATION, router=router)
                    with self.assertRaisesRegex(rb.BacnetError, "name no router"):
                        await asyncio.wait_for(
                            role.read_property(target, AI_1, P.PRESENT_VALUE), 5)
                    self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_the_endpoints_own_network_goes_direct_without_its_router(self):
        station_requests = []

        def station(datagram, source):
            request = request_of(datagram)
            station_requests.append(request)
            return [answer(None, None, request, value=5)]

        peer = RawPeer(station)
        self.addCleanup(peer.close)
        # A registered Network Port tells the endpoint its network: 7.
        endpoint = rb.BipEndpoint(NEAR + 20, interface="127.0.0.1", port=0,
                                  broadcast_address="127.255.255.255", network_number=7,
                                  registered_network_port=1, apdu_timeout_ms=2000)
        async with endpoint:
            role = await endpoint.client()
            for router in (self.router.address, None):
                with self.subTest(router=router):
                    station_requests.clear()
                    target = rb.RoutedTarget(7, peer.address, router=router)
                    value = await asyncio.wait_for(
                        role.read_property(target, AI_1, P.PRESENT_VALUE), 5)
                    self.assertEqual(value, rb.PropertyValue.unsigned(5))
                    [request] = station_requests
                    self.assertEqual(request["control"], 0x04)  # no DNET
            self.assertEqual(self.router.received, [])


class EndpointBroadcastFormTests(unittest.IsolatedAsyncioTestCase):
    """A RoutedTarget with no router: each request is one broadcast that
    names the station's network. The endpoint is a foreign device of a raw
    BBMD here, so its broadcasts reach the BBMD as Distribute-Broadcast-To-
    Network, where they can be read; two routers both forward each one."""

    STATION = b"\x0a"

    async def asyncSetUp(self):
        self.requests = []
        self.second = RawPeer(lambda data, source: None)
        self.addCleanup(self.second.close)

        def handle(datagram, source):
            if datagram[:2] == b"\x81\x05":  # Register-Foreign-Device: accept
                return [bvll(0x00, b"\x00\x00")]
            if datagram[:2] != b"\x81\x09":
                return None
            request = request_of(datagram, 0x09)
            if request["control"] & 0x80 or request["apdu"][0] >> 4 != 0:
                return None  # a network message, or no confirmed request
            self.requests.append(request)
            # This BBMD's router answers first, a second router after it.
            first = answer(request["dnet"], request["dadr"], request, value=0)
            self.bbmd.send(first, source)
            self.second.send(answer(request["dnet"], request["dadr"], request, value=9), source)
            return None

        self.bbmd = RawPeer(handle)
        self.addCleanup(self.bbmd.close)
        self.endpoint = rb.BipEndpoint(
            NEAR + 30, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2000, foreign_device=rb.ForeignDevice(self.bbmd.address, ttl=60))
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        await accepted_registration(self.endpoint)
        self.role = await self.endpoint.client()
        self.target = rb.RoutedTarget(5, self.STATION)

    async def unclaimed(self, at_least):
        async with asyncio.timeout(5):
            while (count := (await self.endpoint.status())["unclaimed_terminal"]) < at_least:
                await asyncio.sleep(0.01)
        return count

    def assert_broadcast(self, requests, service):
        self.assertTrue(requests)
        for request in requests:
            # DNET present, expecting a reply, no SNET, the station named.
            self.assertEqual(request["control"], 0x24)
            self.assertEqual((request["dnet"], request["dadr"]), (5, self.STATION))
            self.assertEqual(request["hop_count"], 255)
            self.assertIn(request["apdu"][3], service)

    async def test_the_first_answer_completes_a_read_and_the_second_is_counted(self):
        before = (await self.endpoint.status())["unclaimed_terminal"]
        value = await asyncio.wait_for(
            self.role.read_property(self.target, AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))  # the first router's
        self.assert_broadcast(self.requests, {12})
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(await self.unclaimed(before + 1), before + 1)
        self.assertEqual((await self.endpoint.status())["active_leases"], 0)

        # Every read takes the form: one broadcast per request.
        self.requests.clear()
        [result] = await asyncio.wait_for(self.role.read_property_multiple(
            self.target, [(AI_1, [(P.PRESENT_VALUE, None)])]), 5)
        self.assertEqual(result["results"][0]["value"], rb.PropertyValue.unsigned(7))
        result = await asyncio.wait_for(self.role.read_range(
            self.target, LOG, P.LOG_BUFFER, range_type="position", reference_index=1,
            count=5), 5)
        self.assertEqual(result["item_count"], 0)
        page = await asyncio.wait_for(self.role.read_log_page(self.target, LOG), 5)
        self.assertEqual((page["records"], page["done"]), ([], True))
        self.assert_broadcast(self.requests, {12, 14, 26})
        self.assertEqual([request["apdu"][3] for request in self.requests], [14, 26, 12, 12, 12])
        await self.unclaimed(before + 6)
        self.assertEqual((await self.endpoint.status())["active_leases"], 0)


class EndpointRoutedRouterTests(unittest.IsolatedAsyncioTestCase):
    """Through a real BACnetRouter to a real server on network 5."""

    async def asyncSetUp(self):
        # Port 1 is a BBMD that takes foreign devices, so a foreign device's
        # broadcast reaches the router on loopback.
        self.router = rb.BACnetRouter([
            rb.BipRouterPort(1, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                             bbmd=rb.Bbmd(accept_foreign_devices=True)),
            port(5)])
        await asyncio.wait_for(self.router.start(), 10)
        self.addAsyncCleanup(finish, self.router.close)
        self.router_address = (await self.router.ports())[0].address
        self.far = rb.BACnetServer(FAR, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1")
        self.far.add_analog_input(1, "Far temperature", 62, 12.5)
        self.far.add_event_log(1, "EL-1", 64, log_received_notifications=True)
        await asyncio.wait_for(self.far.start(), 10)
        self.addAsyncCleanup(finish, self.far.stop)
        self.far_address = await self.far.local_address()
        ip, number = self.far_address.split(":")
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            for _ in range(NOTIFICATIONS):
                peer.sendto(FRAME, (ip, int(number)))
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1") as client:
            async with asyncio.timeout(5):
                while (await client.read_range(self.far_address, LOG, P.LOG_BUFFER)
                       )["item_count"] < NOTIFICATIONS:
                    await asyncio.sleep(0.05)
        self.target = rb.RoutedTarget(5, self.far_address, router=self.router_address)

    @staticmethod
    def endpoint(**keywords):
        options = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1",
                   "apdu_timeout_ms": 2000, **keywords}
        return rb.BipEndpoint(NEAR, **options)

    async def pages(self, role, address):
        records, cursor = [], None
        for _ in range(20):
            page = await asyncio.wait_for(role.read_log_page(address, LOG, cursor, page_size=2), 5)
            records.extend(page["records"])
            cursor = page["next"]
            if page["done"]:
                return records
        self.fail("the read did not finish")

    async def test_routed_reads_match_direct_ones(self):
        endpoint = self.endpoint()
        async with endpoint:
            role = await endpoint.client()
            routed = await asyncio.wait_for(role.read_property(self.target, AI_1, P.PRESENT_VALUE), 5)
            self.assertEqual(routed, rb.PropertyValue.real(12.5))
            # The same station reached direct, as before.
            direct = await role.read_property(self.far_address, AI_1, P.PRESENT_VALUE)
            self.assertEqual(routed, direct)
            specs = [(AI_1, [(P.PRESENT_VALUE, None), (P.OBJECT_NAME, None)])]
            self.assertEqual(await role.read_property_multiple(self.target, specs),
                             await role.read_property_multiple(self.far_address, specs))
            for validation in ("strict", "lenient"):
                with self.subTest(validation=validation):
                    routed = await role.read_range(self.target, LOG, P.LOG_BUFFER,
                                                   validation=validation)
                    self.assertGreaterEqual(routed["item_count"], NOTIFICATIONS)
                    self.assertEqual(routed, await role.read_range(
                        self.far_address, LOG, P.LOG_BUFFER, validation=validation))
            records = await self.pages(role, self.target)
            self.assertEqual(records, await self.pages(role, self.far_address))
            self.assertGreaterEqual(len(records), NOTIFICATIONS)
            self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_the_broadcast_form_reaches_the_station_through_the_router(self):
        endpoint = self.endpoint(foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        async with endpoint:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            nearest = rb.RoutedTarget(5, self.far_address)
            self.assertEqual(
                await asyncio.wait_for(role.read_property(nearest, AI_1, P.PRESENT_VALUE), 5),
                await role.read_property(self.far_address, AI_1, P.PRESENT_VALUE))
            specs = [(AI_1, [(P.PRESENT_VALUE, None), (P.OBJECT_NAME, None)])]
            self.assertEqual(await role.read_property_multiple(nearest, specs),
                             await role.read_property_multiple(self.far_address, specs))
            self.assertEqual(await role.read_range(nearest, LOG, P.LOG_BUFFER),
                             await role.read_range(self.far_address, LOG, P.LOG_BUFFER))
            self.assertEqual(await self.pages(role, nearest),
                             await self.pages(role, self.far_address))
            self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_a_network_the_router_cant_reach_times_out(self):
        endpoint = self.endpoint(apdu_timeout_ms=500,
                                 foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        async with endpoint:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            for router in (self.router_address, None):
                with self.subTest(router=router):
                    nowhere = rb.RoutedTarget(9, self.far_address, router=router)
                    with self.assertRaises(rb.BacnetTimeoutError):
                        await asyncio.wait_for(
                            role.read_property(nowhere, AI_1, P.PRESENT_VALUE), 5)
                    self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_routed_reads_are_audited_naming_the_station(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        sink = rb.BACnetServer(SINK, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        sink.add_audit_log(1, "sink", str(Path(directory.name) / "audit"), buffer_size=32)
        sink.configure_audit_notification_sink(1, policy="allow_all")
        await asyncio.wait_for(sink.start(), 10)
        self.addAsyncCleanup(finish, sink.stop)
        sink_address = await sink.local_address()
        sink_device = rb.ObjectIdentifier(O.DEVICE, SINK)
        # A recipient on 127.0.0.1 can't share the link's broadcast address.
        endpoint = self.endpoint(
            broadcast_address="127.255.255.255",
            device_write_policy=lambda context: True,
            source_audit_reporter=rb.ObjectIdentifier(O.AUDIT_REPORTER, 1),
            audit_recipient={"kind": "device", "object_identifier": sink_device},
            audit_device_bindings=[(sink_device, sink_address)],
            foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        endpoint.add_audit_reporter(
            1, "Source", audit_level="audit_all",
            auditable_operations=1 << rb.AuditOperation.READ.to_raw(),
            issue_confirmed_notifications=True)
        query = {"audit_log": rb.ObjectIdentifier(O.AUDIT_LOG, 1),
                 "query_parameters": {"kind": "by_source", "successful_actions_only": 0,
                                      "source_device_identifier": rb.ObjectIdentifier(O.DEVICE, NEAR)},
                 "requested_count": 32}
        async with endpoint, rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1") as client:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            await role.read_property(self.target, AI_1, P.PRESENT_VALUE)
            # The broadcast form is audited the same way.
            await role.read_property(rb.RoutedTarget(5, self.far_address), AI_1, P.PRESENT_VALUE)
            # A page from the oldest record: three count reads and a ReadRange.
            page = await role.read_log_page(self.target, LOG)
            self.assertTrue(page["done"])

            async def records():
                result = await client.audit_log_query_typed(sink_address, query)
                return [entry["record"]["datum"]["audit_notification"]
                        for entry in result["records"]]
            async with asyncio.timeout(5):
                while len(found := await records()) < 6:
                    await asyncio.sleep(0.02)
        station = {"kind": "address", "network_number": 5, "mac_address": mac(self.far_address)}
        self.assertEqual(len(found), 6)
        for record in found:
            self.assertEqual(record["operation"], rb.AuditOperation.READ)
            self.assertEqual(record["target_device"], station)
        self.assertEqual(sorted({str(record["target_object"]) for record in found}),
                         sorted({str(AI_1), str(LOG)}))


if __name__ == "__main__":
    unittest.main()
