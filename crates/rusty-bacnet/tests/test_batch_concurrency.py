"""Installed batch limit validation, actual B/IP progress, and cancellation."""
import asyncio
import ast
from pathlib import Path
import socket
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue

METHODS = ("read_property_from_devices", "read_property_multiple_from_devices", "write_property_to_devices")
OID = ObjectIdentifier(ObjectType.DEVICE, 9123)
PID = PropertyIdentifier.DESCRIPTION


def requests(name, instances):
    if name == METHODS[0]:
        return [(i, OID, PID, None) for i in instances]
    if name == METHODS[1]:
        return [(i, [(OID, [(PID, None)])]) for i in instances]
    return [(i, OID, PID, PropertyValue.character_string("batch"), None, None) for i in instances]


async def receive(peer):
    return await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), 2)


async def reply_read(peer, wire, remote):
    body = wire[10:] + b"\x3e\x21\x2a\x3f"
    payload = b"\x01\x00\x30" + bytes([wire[8], 12]) + body
    await asyncio.get_running_loop().sock_sendto(peer, b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload, remote)


class BatchConcurrencyTests(unittest.IsolatedAsyncioTestCase):
    async def test_zero_and_native_bounds_are_synchronous_even_empty_unstarted(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for name in METHODS:
            for items in ([], requests(name, [9123])):
                for limit, error in ((0, ValueError), (-1, OverflowError), (1 << 128, OverflowError)):
                    with self.subTest(method=name, empty=not items, limit=limit):
                        with self.assertRaises(error):
                            getattr(client, name)(items, max_concurrent=limit)
        tree = ast.parse(Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
        cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "BACnetClient")
        for name in METHODS:
            method = next(n for n in cls.body if isinstance(n, ast.AsyncFunctionDef) and n.name == name)
            arg = next(a for a in method.args.args if a.arg == "max_concurrent")
            self.assertEqual(ast.unparse(arg.annotation), "Optional[int]")
            self.assertIn("Zero raises ValueError synchronously", ast.get_docstring(method))

    async def test_default_positive_and_empty_batches_complete_over_bip(self):
        server = BACnetServer(9123, interface="127.0.0.1", port=0)
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, await server.local_address())
                for name in METHODS:
                    for limit in (None, 1, 2):
                        with self.subTest(method=name, limit=limit):
                            self.assertEqual(await asyncio.wait_for(getattr(client, name)([], max_concurrent=limit), 2), [])
                            results = await asyncio.wait_for(getattr(client, name)(requests(name, [9123, 9123]), max_concurrent=limit), 3)
                            self.assertEqual(len(results), 2)
                            self.assertTrue(all(r["error"] is None for r in results), results)
                self.assertEqual(await client.read_property(await server.local_address(), OID, PID), PropertyValue.character_string("batch"))
        finally:
            await server.stop()

    async def test_limit_and_completion_order(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                for instance in (1, 2, 3):
                    await client.add_device(instance, f"127.0.0.1:{peer.getsockname()[1]}")
                task = asyncio.ensure_future(client.read_property_from_devices(requests(METHODS[0], [1, 2, 3]), max_concurrent=2))
                try:
                    first, second = await receive(peer), await receive(peer)
                    with self.assertRaises(TimeoutError):
                        await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), .05)
                    await reply_read(peer, *second)
                    third = await receive(peer)
                    await reply_read(peer, *third)
                    await reply_read(peer, *first)
                    results = await asyncio.wait_for(task, 2)
                    self.assertEqual([r["device_instance"] for r in results], [2, 3, 1])
                    self.assertTrue(all(r["value"] == PropertyValue.unsigned(42) for r in results))
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

    async def test_cancel_pending_batches_does_not_start_queued_requests(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, f"127.0.0.1:{peer.getsockname()[1]}")
                for name in METHODS:
                    with self.subTest(method=name):
                        task = asyncio.ensure_future(getattr(client, name)(requests(name, [9123] * 3), max_concurrent=1))
                        await receive(peer)
                        task.cancel()
                        with self.assertRaises(asyncio.CancelledError):
                            await task
                        with self.assertRaises(TimeoutError):
                            await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), .1)
                        # The same client remains usable after each cancellation.
                        later = asyncio.ensure_future(client.read_property_from_devices(requests(METHODS[0], [9123]), max_concurrent=1))
                        try:
                            await reply_read(peer, *await receive(peer))
                            result = await asyncio.wait_for(later, 2)
                            self.assertEqual(result[0]["value"], PropertyValue.unsigned(42))
                        finally:
                            later.cancel()
                            await asyncio.gather(later, return_exceptions=True)
