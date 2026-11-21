"""Routed calls on BACnetClient (#2612, #2617): RoutedTarget, and
read_property, read_property_multiple, read_range, read_log_page and
write_property to a station behind a router, on the client from
endpoint.client() and on a standalone one, through a real BACnetRouter and
against a raw peer playing the router so each request's NPDU can be read
off the wire. The narrow endpoint client these ran on before #2617 is gone;
every routed case it had is kept here on the full client.

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
CALLS = ("read_property", "read_property_multiple", "read_range", "read_log_page",
         "write_property")
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
        for name in CALLS:
            with self.subTest(method=name):
                node = stub_method(classes["BACnetClient"], name)
                self.assertEqual(ast.unparse(node.args.args[1].annotation), "str | RoutedTarget")
                self.assertEqual(runtime_arg_kinds(getattr(rb.BACnetClient, name)),
                                 stub_arg_kinds(node))


def answer(network, station, request, value=0):
    """The answer to the confirmed request in `request`, relayed from
    `station` on `network` as a router relays it (SNET/SADR naming it), or
    sent direct when `network` is None; None for a request this peer
    doesn't answer."""
    apdu = request["apdu"]
    invoke, service, body = apdu[2], apdu[3], apdu[4:]
    if service == 12:    # ReadProperty: echo the reference, value Unsigned `value`
        reply = bytes([0x30, invoke, service]) + body + b"\x3e\x21" + bytes([value]) + b"\x3f"
    elif service == 14:  # ReadPropertyMultiple of one property: Unsigned 7
        reply = (bytes([0x30, invoke, service]) + body[:5] + b"\x1e\x29" + body[7:8]
                 + b"\x4e\x21\x07\x4f\x1f")
    elif service == 26:  # ReadRange: no items, no flags
        reply = bytes([0x30, invoke, service]) + body[:7] + b"\x3a\x05\x00\x49\x00\x5e\x5f"
    elif service == 15:  # WriteProperty: a SimpleACK
        reply = bytes([0x20, invoke, service])
    else:
        return None
    source = b"" if network is None else network.to_bytes(2, "big") + bytes([len(station)]) + station
    control = 0x00 if network is None else 0x08
    return bvll(0x0A, bytes([0x01, control]) + source + reply)


def i_am(network, station, instance):
    """An I-Am from `instance` at `station` on `network`, relayed as a
    router relays it: Max APDU 1476, no segmentation, vendor 555."""
    apdu = (b"\x10\x00\xc4" + ((8 << 22) | instance).to_bytes(4, "big")
            + b"\x22\x05\xc4\x91\x03\x22\x02\x2b")
    source = network.to_bytes(2, "big") + bytes([len(station)]) + station
    return bvll(0x0A, bytes([0x01, 0x08]) + source + apdu)


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


def calls(client, to):
    """Each routed call to `to`, by name, unawaited."""
    return {
        "read_property": lambda: client.read_property(to, AI_1, P.PRESENT_VALUE),
        "read_property_multiple": lambda: client.read_property_multiple(
            to, [(AI_1, [(P.PRESENT_VALUE, None)])]),
        "read_range": lambda: client.read_range(to, LOG, P.LOG_BUFFER),
        "read_log_page": lambda: client.read_log_page(to, LOG),
        "write_property": lambda: client.write_property(
            to, AI_1, P.DESCRIPTION, rb.PropertyValue.character_string("routed")),
    }


class RoutedWireTests(unittest.IsolatedAsyncioTestCase):
    """A raw peer is the router: every routed request names the station in
    its NPDU, and the answer relayed with SNET/SADR completes it, on the
    endpoint's client and on a standalone one alike."""

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

    async def test_directed_discovery_filters_replies_without_losing_other_arrivals(self):
        stations = [(6, self.STATION), (5, b"\x0b"), (5, self.STATION)]

        def replies(datagram, source):
            if datagram[:2] == b"\x81\x0a":
                request = request_of(datagram)
                self.requests.append(request)
                return [i_am(network, address, FAR) for network, address in stations]
            return ()

        self.router.handler = replies
        async with asyncio.timeout(10):
            arrivals = await self.role.i_am_arrivals()
            found = await self.role.discover(scope=self.target, low=FAR, high=FAR)
            [row] = found
            self.assertEqual((row.network, bytes(row.entry.source_address), row.instance,
                              row.duplicate), (5, self.STATION, FAR, True))
            observed = [await anext(arrivals) for _ in stations]
            self.assertCountEqual([(r.source_network, bytes(r.source_address)) for r in observed],
                                  stations)
            known = await self.role.discovered_devices()
            self.assertCountEqual([(r.source_network, bytes(r.source_address)) for r in known
                                   if r.object_identifier.instance == FAR], stations)
            [query] = self.requests
            self.assertEqual((query["dnet"], query["dadr"]), (5, self.STATION))

    def assert_routed(self, requests, service):
        self.assertTrue(requests)
        for request in requests:
            # DNET present, expecting a reply, normal priority, no SNET.
            self.assertEqual(request["control"], 0x24)
            self.assertEqual((request["dnet"], request["dadr"]), (5, self.STATION))
            self.assertEqual(request["hop_count"], 255)
            self.assertEqual(request["apdu"][0] >> 4, 0)  # Confirmed-Request
            self.assertIn(request["apdu"][3], service)

    async def every_call_names_the_station(self, client):
        value = await asyncio.wait_for(client.read_property(self.target, AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        self.assert_routed(self.requests, {12})

        self.requests.clear()
        [result] = await asyncio.wait_for(
            client.read_property_multiple(self.target, [(AI_1, [(P.PRESENT_VALUE, None)])]), 5)
        self.assertEqual(result["results"][0]["value"], rb.PropertyValue.unsigned(7))
        self.assert_routed(self.requests, {14})

        self.requests.clear()
        for validation in ("strict", "lenient"):
            with self.subTest(validation=validation):
                result = await asyncio.wait_for(client.read_range(
                    self.target, LOG, P.LOG_BUFFER, range_type="position",
                    reference_index=1, count=5, validation=validation), 5)
                self.assertEqual((result["item_count"], result["item_data"]), (0, b""))
        self.assert_routed(self.requests, {26})

        # From the oldest record: the three count reads go there too, and
        # find the log empty.
        self.requests.clear()
        page = await asyncio.wait_for(client.read_log_page(self.target, LOG), 5)
        self.assertEqual((page["records"], page["done"]), ([], True))
        self.assertEqual(len(self.requests), 3)
        self.assert_routed(self.requests, {12})
        # By position: one ReadRange.
        self.requests.clear()
        page = await asyncio.wait_for(
            client.read_log_page(self.target, LOG, ("position", 1), page_size=5), 5)
        self.assertTrue(page["done"])
        self.assertEqual(len(self.requests), 1)
        self.assert_routed(self.requests, {26})

        # WriteProperty takes one too (#2617).
        self.requests.clear()
        self.assertIsNone(await asyncio.wait_for(client.write_property(
            self.target, AI_1, P.DESCRIPTION, rb.PropertyValue.character_string("x")), 5))
        self.assert_routed(self.requests, {15})

    async def test_every_call_names_the_station_behind_the_router(self):
        await self.every_call_names_the_station(self.role)
        self.assertEqual((await self.endpoint.status())["active_leases"], 0)

    async def test_a_standalone_client_takes_the_same_target(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.every_call_names_the_station(client)

    async def test_a_str_address_is_still_a_direct_station(self):
        value = await asyncio.wait_for(
            self.role.read_property(self.router.address, AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        [request] = self.requests
        self.assertEqual(request["control"], 0x04)  # expecting a reply, no DNET
        self.assertIsNone(request["dnet"])
        # Text that reads as "net:mac" in the target grammar is no routed
        # address here: it is read as a local address, and refused as one,
        # never sent to a station on network 5.
        self.requests.clear()
        with self.assertRaises(ValueError):
            await asyncio.wait_for(self.role.read_property("5:0a", AI_1, P.PRESENT_VALUE), 5)
        self.assertEqual(self.requests, [])

    async def test_a_wrong_address_type_raises_at_the_call(self):
        made = calls(self.role, 7)
        self.assertEqual(set(made), set(CALLS))
        for name, call in made.items():
            with self.subTest(method=name), \
                    self.assertRaisesRegex(TypeError, "str or a RoutedTarget, not int"):
                call()
        self.assertEqual(self.router.received, [])

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
                target = rb.RoutedTarget(5, self.STATION, router=router)
                for name, call in calls(role, target).items():
                    with self.subTest(router=router, method=name), \
                            self.assertRaisesRegex(rb.BacnetError, "broadcast or group"):
                        await asyncio.wait_for(call(), 5)
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


class LearnedRouteWireTests(unittest.IsolatedAsyncioTestCase):
    """A RoutedTarget with no router: the client uses the route it learns.
    The endpoint is a foreign device of a raw BBMD that is also the router,
    so its broadcasts reach the BBMD as Distribute-Broadcast-To-Network,
    where they can be read: the first call asks the station with one Who-Is,
    the station's I-Am relayed by the router gives the route, and every
    request then goes to that router by unicast, naming the station."""

    STATION = b"\x0a"

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
                    return [i_am(5, self.STATION, FAR)]
                return None
            if datagram[:2] != b"\x81\x0a":
                return None
            request = request_of(datagram)
            self.requests.append(request)
            reply = answer(request["dnet"], request["dadr"], request)
            return reply and [reply]

        self.bbmd = RawPeer(handle)
        self.addCleanup(self.bbmd.close)
        self.endpoint = rb.BipEndpoint(
            NEAR + 30, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2000, foreign_device=rb.ForeignDevice(self.bbmd.address, ttl=60))
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        await accepted_registration(self.endpoint)
        self.role = await self.endpoint.client()

    async def test_the_first_call_learns_the_router_and_the_rest_reuse_it(self):
        target = rb.RoutedTarget(5, self.STATION)
        value = await asyncio.wait_for(self.role.read_property(target, AI_1, P.PRESENT_VALUE), 10)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        [who_is] = self.who_is
        self.assertEqual(who_is["dadr"], self.STATION)
        for name, call in calls(self.role, target).items():
            with self.subTest(method=name):
                await asyncio.wait_for(call(), 5)
        # One Who-Is in all; each request went to the router by unicast,
        # naming the station.
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual({(r["control"], r["dnet"], r["dadr"]) for r in self.requests},
                         {(0x24, 5, self.STATION)})
        self.assertEqual({"%s:%d" % source for data, source in self.bbmd.received
                          if data[:2] == b"\x81\x0a"}, {await self.endpoint.local_address()})
        # The station is in the client's table, at its router.
        [row] = [row for row in await self.role.discovered_devices()
                 if row.object_identifier.instance == FAR]
        self.assertEqual((row.source_network, bytes(row.source_address)), (5, self.STATION))
        self.assertEqual(bytes(row.mac_address), mac(self.bbmd.address))

    async def test_a_silent_station_says_to_name_its_router_and_isnt_asked_again(self):
        # 0x0b sends no I-Am, as an MS/TP slave doesn't: the BBMD answers
        # the Who-Is for 0x0a only.
        silent = rb.RoutedTarget(5, b"\x0b")
        loop = asyncio.get_running_loop()
        started = loop.time()
        with self.assertRaises(rb.BacnetLookupError) as caught:
            await asyncio.wait_for(self.role.read_property(silent, AI_1, P.PRESENT_VALUE), 10)
        self.assertGreaterEqual(loop.time() - started, 2.5)  # the Who-Is's wait
        text = str(caught.exception)
        for words in ("5:0x0B", "a Who-Is addressed to it", "router=", "discover_routers()"):
            self.assertIn(words, text)
        self.assertEqual(len(self.who_is), 2)
        # Asked again soon: it raises at once, and no second Who-Is goes.
        started = loop.time()
        for name, call in calls(self.role, silent).items():
            with self.subTest(method=name), self.assertRaises(rb.BacnetLookupError) as again:
                await asyncio.wait_for(call(), 1)
        self.assertLess(loop.time() - started, 1)
        self.assertIn("in the last 30 s", str(again.exception))
        self.assertEqual(len(self.who_is), 2)
        self.assertEqual([d for d, _ in self.bbmd.received if d[:2] == b"\x81\x0a"], [])
        # Naming the router works at once.
        through = rb.RoutedTarget(5, b"\x0b", router=self.bbmd.address)
        self.assertEqual(await asyncio.wait_for(
            self.role.read_property(through, AI_1, P.PRESENT_VALUE), 5), rb.PropertyValue.unsigned(0))
        # So does an I-Am the station sends on its own: its row is the route.
        ip, port = (await self.endpoint.local_address()).rsplit(":", 1)
        self.bbmd.send(i_am(5, b"\x0b", FAR + 1), (ip, int(port)))
        async with asyncio.timeout(5):
            while not any(row.object_identifier.instance == FAR + 1
                          for row in await self.role.discovered_devices()):
                await asyncio.sleep(0.01)
        self.assertEqual(await asyncio.wait_for(
            self.role.read_property(silent, AI_1, P.PRESENT_VALUE), 1), rb.PropertyValue.unsigned(0))
        self.assertEqual(len(self.who_is), 2)


    async def test_calls_made_together_share_one_who_is(self):
        silent = rb.RoutedTarget(5, b"\x0c")
        results = await asyncio.wait_for(asyncio.gather(
            *[self.role.read_property(silent, AI_1, P.PRESENT_VALUE) for _ in range(20)],
            return_exceptions=True), 10)
        # One lookup: the Who-Is addressed to the station, then one to its network.
        self.assertEqual(len(self.who_is), 2)
        self.assertTrue(all(isinstance(result, rb.BacnetLookupError) for result in results), results)
        self.assertEqual(len({str(result) for result in results}), 1)

    async def test_cancelling_the_call_that_asked_strands_no_other(self):
        silent = rb.RoutedTarget(5, b"\x0d")
        first = asyncio.ensure_future(self.role.read_property(silent, AI_1, P.PRESENT_VALUE))
        async with asyncio.timeout(5):
            while not self.who_is:
                await asyncio.sleep(0.01)
        rest = [asyncio.ensure_future(self.role.read_property(silent, AI_1, P.PRESENT_VALUE))
                for _ in range(5)]
        await asyncio.sleep(0.05)
        first.cancel()
        results = await asyncio.wait_for(asyncio.gather(*rest, return_exceptions=True), 10)
        self.assertTrue(all(isinstance(result, rb.BacnetLookupError) for result in results), results)
        # The cancelled asker sent one; the call it handed the asking to sends
        # both forms (or the asker sent both first).
        self.assertIn(len(self.who_is), (2, 3))
        await asyncio.gather(first, return_exceptions=True)


class RoutedRouterTests(unittest.IsolatedAsyncioTestCase):
    """Through a real BACnetRouter to a real server on network 5. Both
    ports are BBMDs that take foreign devices, so a foreign device's
    broadcasts reach the router on loopback and the router's reach the
    server."""

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
                                   foreign_device=rb.ForeignDevice(ports[1].address, ttl=60))
        self.far.add_analog_input(1, "Far temperature", 62, 12.5)
        self.far.add_event_log(1, "EL-1", 64, log_received_notifications=True)
        await asyncio.wait_for(self.far.start(), 10)
        self.addAsyncCleanup(finish, self.far.stop)
        await accepted_registration(self.far)
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

    async def match_direct_reads(self, role, target):
        routed = await asyncio.wait_for(role.read_property(target, AI_1, P.PRESENT_VALUE), 10)
        self.assertEqual(routed, rb.PropertyValue.real(12.5))
        # The same station reached direct, as before.
        direct = await role.read_property(self.far_address, AI_1, P.PRESENT_VALUE)
        self.assertEqual(routed, direct)
        specs = [(AI_1, [(P.PRESENT_VALUE, None), (P.OBJECT_NAME, None)])]
        self.assertEqual(await role.read_property_multiple(target, specs),
                         await role.read_property_multiple(self.far_address, specs))
        for validation in ("strict", "lenient"):
            with self.subTest(validation=validation):
                routed = await role.read_range(target, LOG, P.LOG_BUFFER, validation=validation)
                self.assertGreaterEqual(routed["item_count"], NOTIFICATIONS)
                self.assertEqual(routed, await role.read_range(
                    self.far_address, LOG, P.LOG_BUFFER, validation=validation))
        records = await self.pages(role, target)
        self.assertEqual(records, await self.pages(role, self.far_address))
        self.assertGreaterEqual(len(records), NOTIFICATIONS)
        # A routed write lands on the far server.
        await role.write_property(target, AI_1, P.DESCRIPTION,
                                  rb.PropertyValue.character_string("written through"))

    async def test_routed_reads_match_direct_ones(self):
        endpoint = self.endpoint()
        async with endpoint:
            await self.match_direct_reads(await endpoint.client(), self.target)
            self.assertEqual((await endpoint.status())["active_leases"], 0)
        # And on a standalone client.
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.match_direct_reads(client, self.target)

    async def test_with_no_router_the_client_learns_the_route_through_the_router(self):
        endpoint = self.endpoint(foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        async with endpoint:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            await self.match_direct_reads(role, rb.RoutedTarget(5, self.far_address))
            [row] = [row for row in await role.discovered_devices()
                     if row.object_identifier.instance == FAR and row.source_network == 5]
            self.assertEqual(bytes(row.mac_address), mac(self.router_address))
            self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_a_network_the_router_cant_reach(self):
        endpoint = self.endpoint(apdu_timeout_ms=500, apdu_retries=0,
                                 foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        async with endpoint:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            # Through the router: no answer, so the request times out, as a
            # typed call's does (an abort for the transaction's timeout).
            nowhere = rb.RoutedTarget(9, self.far_address, router=self.router_address)
            with self.assertRaises(rb.BacnetAbortError):
                await asyncio.wait_for(role.read_property(nowhere, AI_1, P.PRESENT_VALUE), 10)
            # With no router: nobody answers the Who-Is to network 9.
            with self.assertRaises(rb.BacnetLookupError):
                await asyncio.wait_for(
                    role.read_property(rb.RoutedTarget(9, self.far_address), AI_1,
                                       P.PRESENT_VALUE), 10)
            self.assertEqual((await endpoint.status())["active_leases"], 0)

    async def test_routed_reads_and_writes_are_audited_naming_the_station(self):
        # Restored from before #2617 on the full client (step 3): the
        # endpoint's routed reads are recorded in the sink's Audit Log,
        # naming the station, and so is a routed write, as a command at
        # the priority it was sent with.
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
            source_audit_reporter=rb.ObjectIdentifier(O.AUDIT_REPORTER, 1),
            audit_recipient={"kind": "device", "object_identifier": sink_device},
            audit_device_bindings=[(sink_device, sink_address)],
            foreign_device=rb.ForeignDevice(self.router_address, ttl=60))
        endpoint.add_audit_reporter(
            1, "Source", audit_level="audit_all",
            auditable_operations=(1 << rb.AuditOperation.READ.to_raw())
            | (1 << rb.AuditOperation.WRITE.to_raw()),
            issue_confirmed_notifications=True)
        query = {"audit_log": rb.ObjectIdentifier(O.AUDIT_LOG, 1),
                 "query_parameters": {"kind": "by_source", "successful_actions_only": 0,
                                      "source_device_identifier": rb.ObjectIdentifier(O.DEVICE, NEAR)},
                 "requested_count": 32}
        async with endpoint, rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1") as client:
            await accepted_registration(endpoint)
            role = await endpoint.client()
            await role.read_property(self.target, AI_1, P.PRESENT_VALUE)
            # With no router the client learns the route; its read is
            # audited naming the same station.
            await role.read_property(rb.RoutedTarget(5, self.far_address), AI_1, P.PRESENT_VALUE)
            # A page from the oldest record: three count reads and a ReadRange.
            page = await role.read_log_page(self.target, LOG)
            self.assertTrue(page["done"])
            await role.write_property(self.target, AI_1, P.DESCRIPTION,
                                      rb.PropertyValue.character_string("audited"), priority=8)

            async def records():
                result = await client.audit_log_query_typed(sink_address, query)
                return [entry["record"]["datum"]["audit_notification"]
                        for entry in result["records"]]
            async with asyncio.timeout(5):
                while len(found := await records()) < 7:
                    await asyncio.sleep(0.02)
        station = {"kind": "address", "network_number": 5, "mac_address": mac(self.far_address)}
        self.assertEqual(len(found), 7)
        for record in found:
            self.assertEqual(record["target_device"], station)
        reads = [record for record in found if record["operation"] == rb.AuditOperation.READ]
        self.assertEqual(len(reads), 6)
        self.assertEqual(sorted({str(record["target_object"]) for record in reads}),
                         sorted({str(AI_1), str(LOG)}))
        [write] = [record for record in found if record["operation"] == rb.AuditOperation.WRITE]
        self.assertEqual(str(write["target_object"]), str(AI_1))
        self.assertEqual(write["target_priority"], 8)

if __name__ == "__main__":
    unittest.main()
