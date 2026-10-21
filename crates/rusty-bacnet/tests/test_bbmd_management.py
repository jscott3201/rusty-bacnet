"""BBMD management helpers on BACnetClient against loopback BBMDs (#1841).

The raw UDP peers (bvll_peer) run in a thread, since Windows' default event
loop has no add_reader.
"""
import ast
import asyncio
import inspect
import struct
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetBvlcNakError, BacnetBvlcUnexpectedResponseError,
    BacnetError, BacnetTimeoutError, Bbmd,
)
from bvll_peer import RawPeer
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

METHODS = ("read_bdt", "write_bdt", "read_fdt", "delete_fdt_entry",
           "register_foreign_device_bvlc")


class BbmdManagementTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def stop(owner):
        await asyncio.wait_for(owner.stop(), 10)

    async def bbmd(self, instance, **options):
        server = BACnetServer(instance, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", bbmd=Bbmd(**options))
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(self.stop, server)
        return server, await server.local_address()

    async def client(self):
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await asyncio.wait_for(client.__aenter__(), 10)
        self.addAsyncCleanup(self.stop, client)
        return client

    def peer(self, reply):
        peer = RawPeer(lambda data, source: reply and [reply])
        self.addCleanup(peer.close)
        return peer

    async def test_tables_registration_and_deletion_round_trip(self):
        _, address = await self.bbmd(
            3201, bdt=[("127.0.0.2:47808", "255.255.255.0")],
            accept_foreign_devices=True, management_acl=["127.0.0.1"])
        client = await self.client()
        rows = await asyncio.wait_for(client.read_bdt(address), 10)
        self.assertIn(("127.0.0.2:47808", "255.255.255.0"), rows)
        self.assertIn((address, "255.255.255.255"), rows)  # its own row, added
        self.assertEqual(await client.read_fdt(address), [])

        self.assertIsNone(await client.register_foreign_device_bvlc(address, 60))
        [entry] = await client.read_fdt(address)
        self.assertEqual(set(entry), {"address", "ttl", "seconds_remaining"})
        self.assertTrue(entry["address"].startswith("127.0.0.1:"))
        self.assertEqual(entry["ttl"], 60)
        # The BBMD adds its grace period to the TTL it reports.
        self.assertGreaterEqual(entry["seconds_remaining"], 60)

        self.assertIsNone(await client.delete_fdt_entry(address, entry["address"]))
        self.assertEqual(await client.read_fdt(address), [])

    async def test_a_nak_carries_its_result_code_apart_from_no_reply(self):
        _, closed = await self.bbmd(3202)  # refuses registrations and deletions
        client = await self.client()
        # One at a time: each call starts its exchange, and a second one to
        # the same peer while the first is in flight is refused locally.
        cases = [
            (lambda: client.write_bdt(closed, [closed]), 0x0010),
            (lambda: client.register_foreign_device_bvlc(closed, 60), 0x0030),
            (lambda: client.delete_fdt_entry(closed, "127.0.0.1:47999"), 0x0050),
        ]
        for request, code in cases:
            with self.subTest(code=hex(code)):
                with self.assertRaises(BacnetBvlcNakError) as caught:
                    await asyncio.wait_for(request(), 10)
                self.assertEqual(caught.exception.result_code, code)
                self.assertIsInstance(caught.exception, BacnetError)
                self.assertNotIsInstance(caught.exception, BacnetTimeoutError)

        silent = self.peer(None)
        with self.assertRaises(BacnetTimeoutError) as caught:
            await asyncio.wait_for(client.read_bdt(silent.address), 10)
        self.assertNotIsInstance(caught.exception, BacnetBvlcNakError)
        # The request went out as a Read-Broadcast-Distribution-Table.
        self.assertEqual(silent.received[0][0], b"\x81\x02\x00\x04")

    async def test_a_raw_peer_nak_is_a_nak_and_mismatches_have_their_own_class(self):
        # A Read-BDT NAK, as a node that is no BBMD sends.
        nak = self.peer(b"\x81\x00\x00\x06" + struct.pack(">H", 0x0020))
        client = await self.client()
        with self.assertRaises(BacnetBvlcNakError) as caught:
            await asyncio.wait_for(client.read_bdt(nak.address), 10)
        self.assertEqual(caught.exception.result_code, 0x0020)
        # The transport hands a request only the replies it can take, so a
        # mismatch can't be driven from the wire; the Rust test
        # bvlc_exchange_errors_keep_their_classes_apart builds one.
        self.assertTrue(issubclass(BacnetBvlcUnexpectedResponseError, BacnetError))
        self.assertFalse(issubclass(BacnetBvlcUnexpectedResponseError,
                                    (BacnetBvlcNakError, BacnetTimeoutError)))

    async def test_bad_arguments_and_lifecycle(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.read_bdt("127.0.0.1:47808")
        for bad in ("not an address", "0.0.0.0:47808", "127.0.0.1:0"):
            with self.assertRaises(ValueError):
                client.read_bdt(bad)
        with self.assertRaises(ValueError):
            client.write_bdt("127.0.0.1", [("127.0.0.2", "not a mask")])
        with self.assertRaises(OverflowError):
            client.register_foreign_device_bvlc("127.0.0.1", 70000)
        self.assertIsNone(BacnetBvlcNakError.result_code)
        self.assertIsNone(BacnetBvlcUnexpectedResponseError.got)

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        for name in METHODS:
            with self.subTest(method=name):
                node = stub_method(classes["BACnetClient"], name)
                self.assertEqual(ast.unparse(node.returns.value), "Awaitable")
                self.assertEqual(runtime_arg_kinds(getattr(BACnetClient, name)),
                                 stub_arg_kinds(node))
        self.assertEqual([ast.unparse(base) for base in classes["BacnetBvlcNakError"].bases],
                         ["BacnetError"])
        self.assertEqual(
            [ast.unparse(base) for base in classes["BacnetBvlcUnexpectedResponseError"].bases],
            ["BacnetError"])
        self.assertIn("FdtEntry", classes)
        self.assertEqual(inspect.getmro(BacnetBvlcNakError)[1], BacnetError)


if __name__ == "__main__":
    unittest.main()
