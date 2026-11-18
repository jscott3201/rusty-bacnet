"""Routed path size limits (#2667): configure_routed_path_max_npdu,
clear_routed_path_limit, routed_path_limits() and the two counts of too-long
rejects the client did not believe.

A raw peer plays the router, so each request's NPDU can be measured off the
wire and each answer scripted: a limit set by hand sizes the segments a
request goes out in, a too-long reject for a request above 228 octets is
learned, and one for a request of 228 octets or fewer is dropped, counted
and shows on its path (#2651, #2665).

The raw peer (bvll_peer) runs in a thread, since Windows' default event
loop has no add_reader.
"""
import asyncio
import copy
import pickle
import threading
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll, mac
from test_endpoint_routed import answer, finish, request_of
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

O = rb.ObjectType
P = rb.PropertyIdentifier
AI_1 = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
NETWORK = 5
STATION = b"\x0a"
NEAR = 2667
# What a router adds to a request it forwards from this B/IP client to a
# one-octet station: the fixed NPCI, the DADR and the client's six-octet MAC
# as SADR. A path limit less this is the largest APDU the client sends.
FORWARDED_NPCI = 9 + 1 + 6
CALLS = ("configure_routed_path_max_npdu", "clear_routed_path_limit",
         "routed_path_limits", "too_long_reject_drops", "below_minimum_reject_drops")


def too_long(network):
    """A router's Reject-Message-To-Network, reason 4 (message too long),
    for `network`."""
    return bvll(0x0A, b"\x01\x80\x03\x04" + network.to_bytes(2, "big"))


def relayed(apdu):
    """`apdu` from the station, as the router relays it."""
    source = NETWORK.to_bytes(2, "big") + bytes([len(STATION)]) + STATION
    return bvll(0x0A, bytes([0x01, 0x08]) + source + apdu)


def description(length):
    return rb.PropertyValue.character_string("x" * length)


class Wire:
    """The raw router's side: keeps each confirmed request and answers it
    with what `script` returns for it. A script may also send ahead of its
    answer itself, through `send(datagram)`."""

    def __init__(self):
        self.requests = []
        self.script = lambda request: None
        self.peer = None
        self.source = None

    def send(self, datagram):
        self.peer.send(datagram, self.source)

    def handle(self, datagram, source):
        if datagram[:2] != b"\x81\x0a":
            return None
        request = request_of(datagram)
        if request["apdu"][0] >> 4 != 0:  # only Confirmed-Requests
            return None
        self.requests.append(request)
        self.source = source
        return self.script(request)


def segmented(request):
    return bool(request["apdu"][0] & 0x08)


def abort(request):
    """The station's Abort (segmentation not supported) for `request`: an
    answer, so the path is free at once."""
    return [relayed(bytes([0x71, request["apdu"][2], 4]))]


def acked(request):
    """The station's answer to `request`, or an Abort for a segmented one."""
    if segmented(request):
        return abort(request)
    return [answer(NETWORK, STATION, request)]


class RoutedPathLimitWireTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.wire = Wire()
        self.router = RawPeer(self.wire.handle)
        self.wire.peer = self.router
        self.addCleanup(self.router.close)
        self.client = rb.BACnetClient(interface="127.0.0.1", port=0,
                                      broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        await asyncio.wait_for(self.client.__aenter__(), 10)
        self.addAsyncCleanup(finish, self.client.stop)
        self.target = rb.RoutedTarget(NETWORK, STATION, router=self.router.address)

    async def first_frame_of_a_long_write(self, client):
        """Write 300 octets of description and return the first frame the
        router saw; a segmented request is aborted by the station."""
        self.wire.requests.clear()
        self.wire.script = acked
        try:
            await asyncio.wait_for(client.write_property(
                self.target, AI_1, P.DESCRIPTION, description(300)), 5)
        except rb.BacnetAbortError:
            pass
        return self.wire.requests[0]

    async def test_a_configured_limit_sizes_the_request_on_the_wire(self):
        # The default, 228 octets: segments of 212 APDU octets.
        first = await self.first_frame_of_a_long_write(self.client)
        self.assertTrue(segmented(first))
        self.assertEqual(len(first["apdu"]), 228 - FORWARDED_NPCI)

        # Below the default, for a router that carries less.
        await self.client.configure_routed_path_max_npdu(self.router.address, NETWORK, 128)
        first = await self.first_frame_of_a_long_write(self.client)
        self.assertTrue(segmented(first))
        self.assertEqual(len(first["apdu"]), 128 - FORWARDED_NPCI)
        [path] = await self.client.routed_path_limits()
        self.assertEqual((path.router, path.network, path.max_npdu, path.source),
                         (mac(self.router.address), NETWORK, 128, "configured"))

        # Wide enough for the whole request: it goes out whole, and the
        # station's acknowledgement completes it.
        await self.client.configure_routed_path_max_npdu(mac(self.router.address), NETWORK, 1497)
        first = await self.first_frame_of_a_long_write(self.client)
        self.assertFalse(segmented(first))
        self.assertGreater(len(first["apdu"]), 300)

        # Cleared: back on the default, and nothing listed.
        await self.client.clear_routed_path_limit(self.router.address, NETWORK)
        self.assertEqual(await self.client.routed_path_limits(), [])
        first = await self.first_frame_of_a_long_write(self.client)
        self.assertEqual(len(first["apdu"]), 228 - FORWARDED_NPCI)

    async def test_a_too_long_reject_for_a_small_request_is_counted_not_believed(self):
        # The router refuses a 20-octet read as too long, then the station
        # answers it anyway: the reject belongs to other traffic. The answer
        # is held until the client has counted the reject, so it lands while
        # the read is still active.
        counted = threading.Event()

        def script(request):
            self.wire.send(too_long(NETWORK))
            counted.wait(5)
            return [answer(NETWORK, STATION, request)]

        self.wire.script = script
        read = asyncio.ensure_future(
            self.client.read_property(self.target, AI_1, P.PRESENT_VALUE))
        try:
            async with asyncio.timeout(5):
                while await self.client.below_minimum_reject_drops() < 1:
                    await asyncio.sleep(0.01)
        except TimeoutError:
            counted.set()
            read.cancel()
            await asyncio.gather(read, return_exceptions=True)
            raise
        counted.set()
        value = await asyncio.wait_for(read, 5)
        self.assertEqual(value, rb.PropertyValue.unsigned(0))
        self.assertEqual(await self.client.below_minimum_reject_drops(), 1)
        self.assertEqual(await self.client.too_long_reject_drops(), 0)
        [path] = await self.client.routed_path_limits()
        self.assertEqual(
            (path.router, path.network, path.max_npdu, path.source, path.configured_max_npdu,
             path.learned_max_npdu, path.below_minimum_reject_seen),
            (mac(self.router.address), NETWORK, 228, "default", None, None, True))
        self.assertEqual(repr(path), "RoutedPathLimit(router='%s', network=5, max_npdu=228, "
                                     "source='default')" % self.router.address)

        # Setting the limit by hand is the remedy, and resets the mark.
        await self.client.configure_routed_path_max_npdu(self.router.address, NETWORK, 128)
        [path] = await self.client.routed_path_limits()
        self.assertEqual((path.max_npdu, path.source, path.below_minimum_reject_seen),
                         (128, "configured", False))
        self.assertEqual(await self.client.below_minimum_reject_drops(), 1)

    async def test_a_too_long_reject_for_a_large_request_is_learned(self):
        await self.client.configure_routed_path_max_npdu(self.router.address, NETWORK, 1497)
        self.wire.script = lambda request: [too_long(NETWORK)]
        with self.assertRaises(rb.BacnetError):
            await asyncio.wait_for(self.client.write_property(
                self.target, AI_1, P.DESCRIPTION, description(300)), 5)
        [refused] = self.wire.requests
        self.assertFalse(segmented(refused))
        learned = FORWARDED_NPCI + len(refused["apdu"]) - 1
        [path] = await self.client.routed_path_limits()
        self.assertEqual(
            (path.max_npdu, path.source, path.configured_max_npdu, path.learned_max_npdu,
             path.below_minimum_reject_seen),
            (learned, "learned", 1497, learned, False))
        self.assertEqual(await self.client.below_minimum_reject_drops(), 0)

        # The next request is sized to what was learned.
        first = await self.first_frame_of_a_long_write(self.client)
        self.assertTrue(segmented(first))
        self.assertEqual(len(first["apdu"]), learned - FORWARDED_NPCI)

    async def test_the_endpoint_client_has_the_same_calls(self):
        endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        await asyncio.wait_for(endpoint.start(), 10)
        self.addAsyncCleanup(finish, endpoint.close)
        client = await endpoint.client()
        await client.configure_routed_path_max_npdu(self.router.address, NETWORK, 128)
        first = await self.first_frame_of_a_long_write(client)
        self.assertEqual(len(first["apdu"]), 128 - FORWARDED_NPCI)
        [path] = await client.routed_path_limits()
        self.assertEqual((path.network, path.max_npdu, path.source), (NETWORK, 128, "configured"))
        self.assertEqual((await client.too_long_reject_drops(),
                          await client.below_minimum_reject_drops()), (0, 0))

    async def test_arguments_are_checked_before_anything_is_set(self):
        router = self.router.address
        # 66 on B/IP: the 50-octet minimum APDU and 16 octets of NPCI.
        with self.assertRaisesRegex(ValueError, "give at least 66"):
            await self.client.configure_routed_path_max_npdu(router, NETWORK, 65)
        self.assertEqual(await self.client.routed_path_limits(), [])
        await self.client.configure_routed_path_max_npdu(router, NETWORK, 66)
        [path] = await self.client.routed_path_limits()
        self.assertEqual(path.max_npdu, 66)

        for network in (0, 65535):
            with self.subTest(network=network), self.assertRaises(ValueError):
                await self.client.configure_routed_path_max_npdu(router, network, 480)
            with self.subTest(network=network), self.assertRaises(ValueError):
                await self.client.clear_routed_path_limit(router, network)
        for bad in (b"", bytes(19), "not an address", "5:1234"):
            with self.subTest(router=bad), self.assertRaises(ValueError):
                await self.client.configure_routed_path_max_npdu(bad, NETWORK, 480)
        # A device or a routed station named as the router says so, in this
        # call's own words.
        for station in ("5:1234", "5:0x0a", "1234"):
            with self.subTest(router=station):
                with self.assertRaises(ValueError) as refused:
                    await self.client.configure_routed_path_max_npdu(station, NETWORK, 480)
                self.assertIn("a routed path names its router by its address",
                              str(refused.exception))
                self.assertNotIn("routing-table", str(refused.exception))
        for args in ((5, NETWORK, 480), (router, True, 480), (router, "5", 480),
                     (router, NETWORK, True), (router, NETWORK, 480.0)):
            with self.subTest(args=args), self.assertRaises(TypeError):
                await self.client.configure_routed_path_max_npdu(*args)
        for args in ((router, -1, 480), (router, 65536, 480), (router, NETWORK, 65536)):
            with self.subTest(args=args), self.assertRaises(OverflowError):
                await self.client.configure_routed_path_max_npdu(*args)
        self.assertEqual(len(await self.client.routed_path_limits()), 1)

    async def test_records_are_frozen_and_refuse_copy_and_pickle(self):
        await self.client.configure_routed_path_max_npdu(self.router.address, NETWORK, 480)
        [path] = await self.client.routed_path_limits()
        self.assertIsInstance(path, rb.RoutedPathLimit)
        with self.assertRaises(AttributeError):
            path.max_npdu = 1
        for duplicate in (copy.copy, copy.deepcopy, pickle.dumps):
            with self.subTest(duplicate=duplicate.__name__), self.assertRaises(TypeError):
                duplicate(path)


class RoutedPathLimitLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def test_each_call_needs_a_started_client(self):
        client = rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        calls = (
            lambda: client.configure_routed_path_max_npdu("127.0.0.1:47808", NETWORK, 480),
            lambda: client.clear_routed_path_limit("127.0.0.1:47808", NETWORK),
            client.routed_path_limits,
            client.too_long_reject_drops,
            client.below_minimum_reject_drops,
        )
        for call in calls:
            with self.assertRaises(RuntimeError):
                await call()

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        for name in CALLS:
            with self.subTest(method=name):
                node = stub_method(classes["BACnetClient"], name)
                self.assertEqual(runtime_arg_kinds(getattr(rb.BACnetClient, name)),
                                 stub_arg_kinds(node))
        for name in ("router", "network", "max_npdu", "source", "configured_max_npdu",
                     "learned_max_npdu", "below_minimum_reject_seen"):
            with self.subTest(attribute=name):
                stub_method(classes["RoutedPathLimit"], name)
                self.assertTrue(hasattr(rb.RoutedPathLimit, name))


if __name__ == "__main__":
    unittest.main()
