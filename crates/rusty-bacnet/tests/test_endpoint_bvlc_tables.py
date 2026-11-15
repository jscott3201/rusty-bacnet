"""BipEndpoint.read_bdt and read_fdt (#2612): a BBMD's tables read through
the endpoint's own socket, with the shapes and errors of BACnetClient's,
beside the virtual device answering and the client role's reads.

The raw UDP peers (bvll_peer) run in a thread, since Windows' default event
loop has no add_reader.
"""
import ast
import asyncio
import struct
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll
from test_endpoint_routed import answer, finish, request_of
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

O = rb.ObjectType
P = rb.PropertyIdentifier
ENDPOINT = 2615
BBMD = 2616
READ_BDT = b"\x81\x02\x00\x04"
READ_FDT = b"\x81\x06\x00\x04"
# One BDT row, 127.0.0.2:47808 with mask 255.255.255.255, and one FDT entry,
# 127.0.0.9:47808 with TTL 60 and 75 seconds left.
BDT_ACK = bvll(0x03, bytes([127, 0, 0, 2, 0xBA, 0xC0, 255, 255, 255, 255]))
FDT_ACK = bvll(0x07, bytes([127, 0, 0, 9, 0xBA, 0xC0, 0, 60, 0, 75]))


class EndpointBvlcTableTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.endpoint = rb.BipEndpoint(ENDPOINT, device_name="Scanner", interface="127.0.0.1",
                                       port=0, broadcast_address="127.0.0.1",
                                       apdu_timeout_ms=3000)
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        self.local = await self.endpoint.local_address()

    async def bbmd(self, **options):
        server = rb.BACnetServer(BBMD, interface="127.0.0.1", port=0,
                                 broadcast_address="127.0.0.1", bbmd=rb.Bbmd(**options))
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(finish, server.stop)
        return await server.local_address()

    def peer(self, handler):
        peer = RawPeer(handler)
        self.addCleanup(peer.close)
        return peer

    async def test_tables_read_as_the_standalone_client_reads_them(self):
        address = await self.bbmd(bdt=[("127.0.0.2:47808", "255.255.255.0")],
                                  accept_foreign_devices=True, management_acl=["127.0.0.1"])
        async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
            rows = await asyncio.wait_for(self.endpoint.read_bdt(address), 10)
            self.assertIn(("127.0.0.2:47808", "255.255.255.0"), rows)
            self.assertEqual(rows, await client.read_bdt(address))
            self.assertEqual(await self.endpoint.read_fdt(address), [])
            await client.register_foreign_device_bvlc(address, 60)
            [entry] = await asyncio.wait_for(self.endpoint.read_fdt(address), 10)
            self.assertEqual(set(entry), {"address", "ttl", "seconds_remaining"})
            self.assertEqual(entry["ttl"], 60)
            self.assertEqual([entry], await client.read_fdt(address))

    async def test_a_nak_keeps_its_code_apart_from_silence(self):
        # A node that is no BBMD refuses the table reads.
        plain = rb.BACnetServer(BBMD, interface="127.0.0.1", port=0)
        await asyncio.wait_for(plain.start(), 10)
        self.addAsyncCleanup(finish, plain.stop)
        with self.assertRaises(rb.BacnetBvlcNakError) as caught:
            await asyncio.wait_for(self.endpoint.read_bdt(await plain.local_address()), 10)
        self.assertEqual(caught.exception.result_code, 0x0020)
        nak = self.peer(lambda data, source: [bvll(0x00, struct.pack(">H", 0x0040))])
        with self.assertRaises(rb.BacnetBvlcNakError) as caught:
            await asyncio.wait_for(self.endpoint.read_fdt(nak.address), 10)
        self.assertEqual(caught.exception.result_code, 0x0040)
        silent = self.peer(lambda data, source: None)
        with self.assertRaises(rb.BacnetTimeoutError) as caught:
            await asyncio.wait_for(self.endpoint.read_bdt(silent.address), 10)
        self.assertNotIsInstance(caught.exception, rb.BacnetBvlcNakError)
        [(request, source)] = silent.received
        self.assertEqual(request, READ_BDT)
        self.assertEqual("%s:%d" % source, self.local)

    async def test_one_socket_beside_the_device_and_a_read_in_flight(self):
        held = []

        def handle(datagram, source):
            if datagram[:2] == b"\x81\x0a":  # the client role's read: hold it
                held.append(datagram)
                return None
            if datagram == READ_BDT:
                return [BDT_ACK]
            if datagram == READ_FDT:  # answer, then let the held read finish
                return [FDT_ACK] + [answer(None, None, request_of(read)) for read in held]
            return None

        peer = self.peer(handle)
        role = await self.endpoint.client()
        ai = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
        read = asyncio.ensure_future(role.read_property(peer.address, ai, P.PRESENT_VALUE))
        try:
            async with asyncio.timeout(5):
                while not held:
                    await asyncio.sleep(0.01)
            device = rb.ObjectIdentifier(O.DEVICE, ENDPOINT)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as second:
                names = [second.read_property(self.local, device, P.OBJECT_NAME)
                         for _ in range(10)]
                rows, entries, *names = await asyncio.wait_for(asyncio.gather(
                    self.endpoint.read_bdt(peer.address),
                    self.endpoint.read_fdt(peer.address), *names), 10)
            self.assertEqual(rows, [("127.0.0.2:47808", "255.255.255.255")])
            self.assertEqual(entries, [{"address": "127.0.0.9:47808", "ttl": 60,
                                        "seconds_remaining": 75}])
            self.assertEqual(names, [rb.PropertyValue.character_string("Scanner")] * 10)
            self.assertEqual(await asyncio.wait_for(read, 5), rb.PropertyValue.unsigned(0))
        finally:
            if not read.done():
                read.cancel()
            await asyncio.gather(read, return_exceptions=True)
        # The read and both table requests left from the endpoint's one socket.
        tables = sorted(data for data, _ in peer.received if data[:2] != b"\x81\x0a")
        self.assertEqual(tables, [READ_BDT, READ_FDT])
        self.assertEqual({"%s:%d" % source for _, source in peer.received}, {self.local})
        self.assertEqual(len(peer.received), 3)
        self.assertEqual((await self.endpoint.status())["active_leases"], 0)

    async def test_bad_arguments_and_lifecycle(self):
        for name in ("read_bdt", "read_fdt"):
            method = getattr(self.endpoint, name)
            for bad in ("not an address", "0.0.0.0:47808", "127.0.0.1:0", "255.255.255.255"):
                with self.subTest(method=name, address=bad), self.assertRaises(ValueError):
                    method(bad)
        unstarted = rb.BipEndpoint(ENDPOINT, interface="127.0.0.1", port=0)
        await self.endpoint.close()
        for owner in (unstarted, self.endpoint):
            for name in ("read_bdt", "read_fdt"):
                with self.subTest(method=name), \
                        self.assertRaisesRegex(RuntimeError, "not started"):
                    await getattr(owner, name)("127.0.0.1:47808")

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        for name, returns in (("read_bdt", "Awaitable[list[tuple[str, str]]]"),
                              ("read_fdt", "Awaitable[list[FdtEntry]]")):
            with self.subTest(method=name):
                node = stub_method(classes["BipEndpoint"], name)
                self.assertEqual(ast.unparse(node.returns), returns)
                self.assertEqual(runtime_arg_kinds(getattr(rb.BipEndpoint, name)),
                                 stub_arg_kinds(node))
                # The same shape as the standalone client's.
                self.assertEqual(ast.unparse(stub_method(classes["BACnetClient"], name).returns),
                                 returns)
        for cls in (rb.ScEndpoint, rb.MstpEndpoint, rb.EndpointClient):
            for name in ("read_bdt", "read_fdt"):
                self.assertFalse(hasattr(cls, name))


if __name__ == "__main__":
    unittest.main()
