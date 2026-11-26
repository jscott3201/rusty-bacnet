"""Occurrence attribution and typed item errors through the installed B/IP client."""
import asyncio
import ast
from pathlib import Path
import socket
import unittest

import rusty_bacnet as rb
from test_batch_concurrency import METHODS, OID, PID, receive, requests


async def reply(peer, remote, apdu):
    payload = b"\x01\x00" + apdu
    frame = b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload
    await asyncio.get_running_loop().sock_sendto(peer, frame, remote)


def success(wire):
    invoke, service = wire[8:10]
    if service == 15:
        return bytes([0x20, invoke, service])
    if service == 12:
        body = wire[10:] + b"\x3e\x21\x2a\x3f"
    else:
        # Object [0], list [1], property [2], value [4].
        body = b"\x0c" + ((8 << 22) | 9123).to_bytes(4, "big")
        body += b"\x1e\x29\x1c\x4e\x21\x2a\x4f\x1f"
    return bytes([0x30, invoke, service]) + body


class BatchResultTests(unittest.IsolatedAsyncioTestCase):
    async def test_identical_occurrences_reverse_mixed_completion_all_families(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, f"127.0.0.1:{peer.getsockname()[1]}")
                for name in METHODS:
                    with self.subTest(method=name):
                        task = asyncio.ensure_future(getattr(client, name)(requests(name, [9123, 9123]), max_concurrent=2))
                        try:
                            first, second = await receive(peer), await receive(peer)
                            self.assertEqual(first[0][9:], second[0][9:])
                            wire, remote = second
                            await reply(peer, remote, bytes([0x50, wire[8], wire[9]]) + b"\x91\x02\x91\x20")
                            await reply(peer, first[1], success(first[0]))
                            results = await asyncio.wait_for(task, 2)
                            expected_keys = {"request_index", "device_instance", "error"}
                            if name == METHODS[0]:
                                expected_keys.add("value")
                            elif name == METHODS[1]:
                                expected_keys.add("results")
                            for result in results:
                                self.assertEqual(set(result), expected_keys)
                            self.assertEqual([r.get("request_index") for r in results], [1, 0])
                            self.assertEqual([r["device_instance"] for r in results], [9123, 9123])
                            error = results[0]["error"]
                            self.assertIsInstance(error, rb.BacnetProtocolError)
                            self.assertEqual((error.error_class, error.error_code), (2, 32))
                            self.assertIsNone(results[1]["error"])
                            if name == METHODS[0]:
                                self.assertIsNone(results[0]["value"])
                                self.assertEqual(results[1]["value"], rb.PropertyValue.unsigned(42))
                            elif name == METHODS[1]:
                                self.assertIsNone(results[0]["results"])
                                self.assertEqual(results[1]["results"][0]["results"][0]["value"], rb.PropertyValue.unsigned(42))
                        finally:
                            task.cancel()
                            await asyncio.gather(task, return_exceptions=True)

    async def test_an_instance_the_table_lacks_is_a_lookup_error(self):
        # No row claims it: nothing is sent, and the batch row and the typed
        # call both carry the lookup error naming the instance (#1833).
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                [row] = await asyncio.wait_for(
                    client.read_property_from_devices(requests(METHODS[0], [9124])), 2)
                self.assertIsInstance(row["error"], rb.BacnetLookupError)
                self.assertEqual(row["error"].instance, 9124)
                with self.assertRaises(rb.BacnetLookupError) as caught:
                    await asyncio.wait_for(client.read_property_from_device(9124, OID, PID), 2)
                self.assertIsInstance(caught.exception, LookupError)
                self.assertEqual(caught.exception.target, "9124")
                with self.assertRaises(BlockingIOError):
                    peer.recv(2048)

    async def test_reject_reason_is_a_typed_item_error(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, f"127.0.0.1:{peer.getsockname()[1]}")
                task = asyncio.ensure_future(client.read_property_from_devices(requests(METHODS[0], [9123])))
                try:
                    wire, remote = await receive(peer)
                    await reply(peer, remote, bytes([0x60, wire[8], 9]))
                    result = (await asyncio.wait_for(task, 2))[0]
                    self.assertIsInstance(result["error"], rb.BacnetRejectError)
                    self.assertEqual(result["error"].reason, 9)
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

    async def test_rpm_nested_error_and_raw_value_are_result_data(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, f"127.0.0.1:{peer.getsockname()[1]}")
                task = asyncio.ensure_future(client.read_property_multiple_from_devices([
                    (9123, [(OID, [(PID, None), (rb.PropertyIdentifier.OBJECT_NAME, None)])])]))
                try:
                    wire, remote = await receive(peer)
                    body = b"\x0c" + ((8 << 22) | 9123).to_bytes(4, "big")
                    # A nested property error and a structurally framed reserved
                    # application tag, which PropertyValue holds as its octets.
                    body += b"\x1e\x29\x1c\x5e\x91\x02\x91\x20\x5f"
                    body += b"\x29\x4d\x4e\xd0\x4f\x1f"
                    await reply(peer, remote, bytes([0x30, wire[8], 14]) + body)
                    item = (await asyncio.wait_for(task, 2))[0]
                    self.assertEqual(set(item), {"request_index", "device_instance", "results", "error"})
                    self.assertEqual(item["request_index"], 0)
                    self.assertIsNone(item["error"])
                    values = item["results"][0]["results"]
                    self.assertEqual(values[0]["error"], (rb.ErrorClass.PROPERTY, rb.ErrorCode.UNKNOWN_PROPERTY))
                    self.assertIsNone(values[0]["value"])
                    self.assertEqual(values[1]["value"], rb.PropertyValue.application_data(b"\xd0"))
                    self.assertIsNone(values[1]["error"])
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

    async def test_read_value_without_a_typed_form_is_its_octets(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, f"127.0.0.1:{peer.getsockname()[1]}")
                task = asyncio.ensure_future(client.read_property_from_devices(requests(METHODS[0], [9123])))
                try:
                    wire, remote = await receive(peer)
                    # A well-framed reserved application tag is no error: the
                    # value comes back as the octets the device sent.
                    await reply(peer, remote, bytes([0x30, wire[8], 12]) + wire[10:] + b"\x3e\xd0\x3f")
                    item = (await asyncio.wait_for(task, 2))[0]
                    self.assertEqual(item["request_index"], 0)
                    self.assertEqual(item["value"], rb.PropertyValue.application_data(b"\xd0"))
                    self.assertIsNone(item["error"])
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

    def test_installed_stub_result_shapes_are_structural_only(self):
        source = Path(rb.__file__).with_suffix(".pyi").read_text()
        tree = ast.parse(source, feature_version=(3, 11))
        classes = {node.name: node for node in tree.body if isinstance(node, ast.ClassDef)}
        for method, shape, payload in zip(METHODS, (
            "_DeviceReadBatchResult", "_DeviceRpmBatchResult", "_DeviceWriteBatchResult"
        ), ({"value": "PropertyValue | None"}, {"results": "list[ReadAccessResult] | None"}, {})):
            with self.subTest(method=method):
                self.assertFalse(hasattr(rb, shape), "stub helpers are not nominal runtime classes")
                fields = {n.target.id: ast.unparse(n.annotation) for n in classes[shape].body if isinstance(n, ast.AnnAssign)}
                self.assertEqual(fields, {"request_index": "int", "device_instance": "int", "error": "BacnetError | None", **payload})
                decl = next(n for n in classes["BACnetClient"].body if isinstance(n, ast.FunctionDef) and n.name == method)
                self.assertEqual(ast.unparse(decl.returns), f"Awaitable[list[{shape}]]")

    async def test_native_future_contract_matches_awaitable_stub(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
            for method in METHODS:
                with self.subTest(method=method):
                    future = getattr(client, method)([])
                    self.assertIsInstance(future, asyncio.Future)
                    self.assertFalse(asyncio.iscoroutine(future))
                    with self.assertRaises(TypeError):
                        asyncio.create_task(future)
                    self.assertIs(asyncio.ensure_future(future), future)
                    self.assertEqual(await future, [])
