"""Installed standalone/endpoint ReadRange contract and real B/IP operations."""
import asyncio
import ast
import inspect
from pathlib import Path
import unittest

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, EndpointClient, BacnetError,
    ObjectIdentifier, ObjectType, PropertyIdentifier,
)


class EndpointReadRangeTests(unittest.IsolatedAsyncioTestCase):
    async def test_shared_parser_wire_results_and_closed_handle(self):
        target = BACnetServer(9123, interface="127.0.0.1", port=0)
        target.add_analog_input(1, "AI-1")
        target.add_analog_input(2, "AI-2")
        target.add_trend_log(1, "Empty log")
        endpoint = BipEndpoint(device_instance=9124, interface="127.0.0.1", port=0)
        await target.start()
        await endpoint.start()
        try:
            role = await endpoint.client()
            self.assertEqual(role.service_scope(), {"initiates": ["read_property", "read_range"], "executes": []})
            address = await target.local_address()
            oid = ObjectIdentifier(ObjectType.DEVICE, 9123)
            pid = PropertyIdentifier.OBJECT_LIST
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                for options, count in (({}, 4), ({"range_type": "position", "reference_index": 1, "count": 2}, 2),
                                       ({"range_type": "position", "reference_index": 0, "count": 2}, 0)):
                    async with asyncio.timeout(5):
                        actual = await role.read_range(address, oid, pid, **options)
                        expected = await client.read_range(address, oid, pid, **options)
                    self.assertEqual(actual, expected)
                    self.assertEqual(actual["item_count"], count)
                    self.assertIsInstance(actual["item_data"], bytes)
                    self.assertIsInstance(actual["result_flags"], tuple)
                    self.assertEqual(len(actual["result_flags"]), 3)
                    self.assertTrue(all(type(flag) is bool for flag in actual["result_flags"]))
                    self.assertIsNone(actual["first_sequence_number"])
                    if count == 4:
                        self.assertEqual(actual["item_data"], b"\xc4\x02\x00\x23\xa3\xc4\x00\x00\x00\x01\xc4\x00\x00\x00\x02\xc4\x05\x00\x00\x01")
                log = ObjectIdentifier(ObjectType.TREND_LOG, 1)
                for reader in (role, client):
                    result = await reader.read_range(address, log, PropertyIdentifier.LOG_BUFFER,
                                                     range_type="sequence", reference_seq=0, count=2)
                    self.assertEqual(result["item_count"], 0)
                    self.assertEqual(result["item_data"], b"")
                    self.assertIsNone(result["first_sequence_number"])
                for reader in (role, client):
                    for options in ({"range_type": "time"}, {"range_type": "position"},
                                    {"range_type": "sequence", "count": 0},
                                    {"range_type": "position", "count": 32768},
                                    {"range_type": "position", "count": -32769}, {"array_index": 0}):
                        with self.subTest(reader=type(reader), options=options):
                            # Bad destination would fail parsing if validation were delayed.
                            with self.assertRaises(ValueError):
                                reader.read_range("not-an-address", oid, pid, **options)
                    for count in (1 << 31, -(1 << 31) - 1):
                        with self.subTest(reader=type(reader), overflowing_count=count):
                            # Native argument conversion precedes address parsing and I/O.
                            with self.assertRaises(OverflowError):
                                reader.read_range("not-an-address", oid, pid,
                                                  range_type="position", count=count)
                    for selector in (PropertyIdentifier.ALL, PropertyIdentifier.REQUIRED, PropertyIdentifier.OPTIONAL):
                        with self.assertRaises(ValueError):
                            reader.read_range("not-an-address", oid, selector)
            await endpoint.close()
            with self.assertRaises(BacnetError):
                await role.read_range(address, oid, pid)
        finally:
            await endpoint.close()
            await target.stop()

    def test_signatures_and_installed_stub_share_exact_shape(self):
        self.assertEqual(inspect.signature(EndpointClient.read_range), inspect.signature(BACnetClient.read_range))
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text()
        tree = ast.parse(stub)
        for name in ("EndpointClient", "BACnetClient"):
            cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == name)
            method = next(n for n in cls.body if isinstance(n, ast.AsyncFunctionDef) and n.name == "read_range")
            self.assertEqual(ast.unparse(method.returns), "ReadRangeResult")
        shape = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "ReadRangeResult")
        annotations = {n.target.id: ast.unparse(n.annotation) for n in shape.body if isinstance(n, ast.AnnAssign)}
        self.assertEqual(annotations["result_flags"], "tuple[bool, bool, bool]")
        self.assertEqual(annotations["first_sequence_number"], "int | None")
