"""I-Have delivery and router discovery on BACnetClient (#1850).

An in-process BBMD server answers Who-Has; a raw B/IP peer (bvll_peer, run
in a thread since Windows' default event loop has no add_reader) plays a
BBMD with a router behind it. The client registers with either as a foreign
device, so its broadcasts reach them on loopback.
"""
import ast
import asyncio
import struct
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetError, Bbmd, DiscoveredRouter, ForeignDevice,
    IHave, NetworkReport, ObjectIdentifier, ObjectType,
)
from bvll_peer import RawPeer, bvll, mac
from test_bip_modes import accepted_registration
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

DEVICE = 8 << 22
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 0)


def i_have(device, instance, name):
    """An Original-Unicast-NPDU carrying an unconfirmed I-Have."""
    encoded = b"\x00" + name.encode()
    apdu = (b"\x10\x01" + b"\xc4" + struct.pack(">I", DEVICE | device)
            + b"\xc4" + struct.pack(">I", instance) + b"\x75" + bytes([len(encoded)])
            + encoded)
    return bvll(0x0A, b"\x01\x00" + apdu)


class RouterPeer:
    """A BBMD that accepts registrations and, for a router behind it,
    answers each Who-Is-Router-To-Network with networks 5 and 7."""

    def __init__(self):
        self.who_is_router = []
        self.peer = RawPeer(self.handle)
        self.address = self.peer.address
        self.client = None

    def handle(self, data, source):
        function, npdu = data[1], data[4:]
        if function == 0x05:  # Register-Foreign-Device
            self.client = source
            return [bvll(0x00, b"\x00\x00")]
        if function == 0x09 and npdu[:3] == b"\x01\x80\x00":  # DBTN: Who-Is-Router
            self.who_is_router.append(npdu[3:])
            return [self.forwarded(b"\x01\x80\x01" + struct.pack(">HH", 5, 7))]
        return None

    def forwarded(self, npdu):
        return bvll(0x04, mac(self.address) + npdu)

    def send(self, data):
        self.peer.send(data, self.client)


class DiscoveryTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def stop(owner):
        await asyncio.wait_for(owner.stop(), 10)

    async def foreign_client(self, bbmd):
        client = BACnetClient(interface="127.0.0.1", port=0,
                              foreign_device=ForeignDevice(bbmd, ttl=60, renewal=10))
        await asyncio.wait_for(client.__aenter__(), 10)
        self.addAsyncCleanup(self.stop, client)
        await accepted_registration(client)
        return client

    async def router_peer(self):
        router = RouterPeer()
        self.addCleanup(router.peer.close)
        return router

    async def test_who_has_replies_come_from_an_in_process_server(self):
        server = BACnetServer(3301, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              bbmd=Bbmd(accept_foreign_devices=True))
        server.add_analog_input(0, "Zone Temp", 62, 21.5)
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(self.stop, server)
        client = await self.foreign_client(await server.local_address())
        events = await client.i_have_events()

        replies = []
        for _ in range(10):  # the first broadcast can race the registration
            replies = await client.who_has_replies(AI, timeout_ms=500)
            if replies:
                break
        [reply] = replies
        self.assertIsInstance(reply, IHave)
        self.assertEqual(reply.device_identifier, ObjectIdentifier(ObjectType.DEVICE, 3301))
        self.assertEqual((reply.object_identifier, reply.object_name), (AI, "Zone Temp"))
        self.assertIsNone(reply.source_network)
        by_name = await client.who_has_replies("Zone Temp", 3301, 3301, timeout_ms=1500)
        self.assertEqual([r.object_identifier for r in by_name], [AI])
        self.assertEqual(await client.who_has_replies("Nothing", timeout_ms=300), [])
        # Out of range: the device's own I-Have doesn't answer this Who-Has.
        self.assertEqual(await client.who_has_replies(AI, 1, 2, timeout_ms=300), [])

        # The iterator saw every I-Have, the out-of-range answer included.
        first = await asyncio.wait_for(events.__anext__(), 5)
        self.assertEqual(first.object_name, "Zone Temp")
        self.assertEqual(events.missed, 0)

    async def test_a_lagging_i_have_iterator_counts_its_gap_and_holds_nothing_up(self):
        router = await self.router_peer()
        client = await self.foreign_client(router.address)
        lagging = await client.i_have_events()
        # Past the 256 the stream holds, paced so a small loopback receive
        # buffer (Windows) keeps up; how many arrive doesn't matter.
        for n in range(400):
            router.send(i_have(77, n, f"AI-{n}"))
            if n % 25 == 24:
                await asyncio.sleep(0.02)
        # The client keeps working while nobody reads: discovery answers.
        [found] = await client.discover_routers(timeout_ms=1500)
        self.assertEqual(found.networks, [5, 7])
        router.send(i_have(77, 1000, "after"))

        # Read with async for, not polling timeouts, up to the marker: a
        # stream that ended at the gap would never reach it.
        got = []

        async def until_after():
            async for item in lagging:
                got.append(item)
                if item.object_name == "after":
                    return
        await asyncio.wait_for(until_after(), 20)
        self.assertEqual(got[-1].object_name, "after")
        self.assertGreater(lagging.missed, 0)
        # The marker may land after the reader has skipped to the oldest
        # retained item, so it can come on top of a full 256.
        self.assertLessEqual(len(got) - 1, 256)
        # What was left is the newest, in order, after the gap.
        numbers = [item.object_identifier.instance for item in got]
        self.assertEqual(numbers, sorted(numbers))

    async def test_routers_and_network_reports_from_a_raw_router(self):
        router = await self.router_peer()
        client = await self.foreign_client(router.address)
        reports = await client.network_reports()

        [found] = await client.discover_routers(timeout_ms=1500)
        self.assertIsInstance(found, DiscoveredRouter)
        self.assertEqual((found.mac, found.networks), (mac(router.address), [5, 7]))
        self.assertEqual(router.who_is_router[-1], b"")  # every network
        [only] = await client.discover_routers(7, timeout_ms=1500)
        self.assertEqual(only.networks, [5, 7])
        self.assertEqual(router.who_is_router[-1], struct.pack(">H", 7))
        self.assertEqual(await client.discover_routers(9, timeout_ms=500), [])
        with self.assertRaises(BacnetError):
            await client.discover_routers(0, timeout_ms=100)

        # Unsolicited: a Network-Number-Is broadcast and a reject, unicast.
        router.send(router.forwarded(b"\x01\x80\x13" + struct.pack(">HB", 3, 1)))
        router.send(bvll(0x0A, b"\x01\x80\x03\x02" + struct.pack(">H", 9)))
        seen = []

        async def collect():
            async for report in reports:
                self.assertIsInstance(report, NetworkReport)
                seen.append(report)
                if report.kind == "reject_message_to_network":
                    return
        await asyncio.wait_for(collect(), 10)
        kinds = [report.kind for report in seen]
        self.assertEqual(kinds[:3], ["i_am_router_to_network"] * 3, kinds)
        self.assertEqual(kinds[-2:], ["network_number_is", "reject_message_to_network"])
        first, number, reject = seen[0], seen[-2], seen[-1]
        self.assertEqual((first.source_mac, first.networks, first.network),
                         (mac(router.address), [5, 7], None))
        self.assertEqual((number.network, number.configured, number.networks), (3, True, None))
        self.assertEqual((reject.reason, reject.network, reject.configured), (2, 9, None))
        self.assertEqual(reports.missed, 0)

        # Stopping the client ends the iteration.
        await self.stop(client)
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(reports.__anext__(), 10)
        with self.assertRaises(RuntimeError):
            await client.network_reports()

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        client = classes["BACnetClient"]
        for name, returns in (
            ("who_has_replies", "Awaitable[list[IHave]]"),
            ("i_have_events", "Awaitable[IHaveIterator]"),
            ("discover_routers", "Awaitable[list[DiscoveredRouter]]"),
            ("network_reports", "Awaitable[NetworkReportIterator]"),
        ):
            with self.subTest(method=name):
                node = stub_method(client, name)
                self.assertEqual(ast.unparse(node.returns), returns)
                self.assertEqual(runtime_arg_kinds(getattr(BACnetClient, name)),
                                 stub_arg_kinds(node))
        for cls, names in (
            ("IHave", {"device_identifier", "object_identifier", "object_name",
                       "source_mac", "source_network", "source_address"}),
            ("DiscoveredRouter", {"mac", "networks"}),
            ("NetworkReport", {"kind", "source_mac", "source_network", "source_address",
                               "networks", "network", "configured", "reason"}),
            ("IHaveIterator", {"missed"}),
            ("NetworkReportIterator", {"missed"}),
        ):
            with self.subTest(cls=cls):
                stubbed = {node.name for node in classes[cls].body
                           if isinstance(node, ast.FunctionDef) and node.decorator_list}
                self.assertEqual(stubbed, names)


if __name__ == "__main__":
    unittest.main()
