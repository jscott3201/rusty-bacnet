"""Installed standalone/endpoint ReadRange contract and real B/IP operations."""
import asyncio
import ast
import inspect
from pathlib import Path
import tempfile
import unittest

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, BitString, EndpointClient, BacnetError,
    BacnetProtocolError, ObjectIdentifier, ObjectType, PropertyIdentifier,
)


class EndpointReadRangeTests(unittest.IsolatedAsyncioTestCase):
    async def test_shared_parser_wire_results_and_closed_handle(self):
        target = BACnetServer(9123, interface="127.0.0.1", port=0)
        target.add_analog_input(1, "AI-1")
        target.add_analog_input(2, "AI-2")
        target.add_trend_log(1, "Empty log")
        target.add_calendar(1, "CAL-1")
        endpoint = BipEndpoint(device_instance=9124, interface="127.0.0.1", port=0)
        await target.start()
        await endpoint.start()
        try:
            role = await endpoint.client()
            self.assertEqual(role.service_scope(), {"initiates": ["read_property", "read_range", "read_property_multiple", "write_property"], "executes": []})
            address = await target.local_address()
            # ReadRange reads a BACnetLIST, not an array such as Object_List:
            # four date [0] entries in Calendar 1's Date_List.
            oid = ObjectIdentifier(ObjectType.CALENDAR, 1)
            pid = PropertyIdentifier.DATE_LIST
            entries = b"".join(bytes([0x0C, 126, 9, day, 0xFF]) for day in range(1, 5))
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                await client.add_list_element(address, oid, pid, entries)
                for options, count in (({}, 4), ({"range_type": "position", "reference_index": 1, "count": 2}, 2),
                                       ({"range_type": "position", "reference_index": 0, "count": 2}, 0)):
                    async with asyncio.timeout(5):
                        actual = await role.read_range(address, oid, pid, **options)
                        expected = await client.read_range(address, oid, pid, **options)
                    self.assertEqual(actual, expected)
                    self.assertEqual(actual["item_count"], count)
                    self.assertIsInstance(actual["item_data"], bytes)
                    self.assertIsInstance(actual["result_flags"], BitString)
                    self.assertEqual(len(actual["result_flags"]), 3)
                    self.assertTrue(all(type(flag) is bool for flag in actual["result_flags"].bits))
                    self.assertIsNone(actual["first_sequence_number"])
                    if count == 4:
                        self.assertEqual(actual["item_data"], entries)
                log = ObjectIdentifier(ObjectType.TREND_LOG, 1)
                for reader in (role, client):
                    result = await reader.read_range(address, log, PropertyIdentifier.LOG_BUFFER,
                                                     range_type="sequence", reference_seq=0, count=2)
                    self.assertEqual(result["item_count"], 0)
                    self.assertEqual(result["item_data"], b"")
                    self.assertIsNone(result["first_sequence_number"])
                for reader in (role, client):
                    for options in ({"range_type": "time"}, {"range_type": "position"},
                                    {"range_type": "sequence", "count": 0}, {"array_index": 0}):
                        with self.subTest(reader=type(reader), options=options):
                            # Bad destination would fail parsing if validation were delayed.
                            with self.assertRaises(ValueError):
                                reader.read_range("not-an-address", oid, pid, **options)
                    # Outside INTEGER16 overflows (#1360), before address
                    # parsing and I/O.
                    for count in (32768, -32769, 1 << 31, -(1 << 31) - 1):
                        with self.subTest(reader=type(reader), overflowing_count=count):
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

    async def test_audit_log_buffer_reads_only_through_read_range(self):
        # An Audit Log's Log_Buffer is open to ReadRange and AuditLogQuery
        # only (Clause 12.64.10, #1092), and its sequence numbers and
        # positions are Unsigned64, so references past 2^32 - 1 are accepted.
        with tempfile.TemporaryDirectory() as directory:
            target = BACnetServer(9125, interface="127.0.0.1", port=0)
            target.add_audit_log(1, "Audit", str(Path(directory) / "audit"), buffer_size=4)
            await target.start()
            try:
                address = await target.local_address()
                log = ObjectIdentifier(ObjectType.AUDIT_LOG, 1)
                pid = PropertyIdentifier.LOG_BUFFER
                async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                    with self.assertRaises(BacnetProtocolError) as refused:
                        await client.read_property(address, log, pid)
                    # PROPERTY (2) / READ_ACCESS_DENIED (27).
                    self.assertEqual((refused.exception.error_class, refused.exception.error_code), (2, 27))
                    for options in ({}, {"range_type": "sequence", "reference_seq": 1 << 40, "count": 1},
                                    {"range_type": "position", "reference_index": 1 << 33, "count": -1}):
                        with self.subTest(options=options):
                            result = await client.read_range(address, log, pid, **options)
                            self.assertEqual(result["item_count"], 0)
                            self.assertEqual(result["item_data"], b"")
                            self.assertIsNone(result["first_sequence_number"])
            finally:
                await target.stop()

    def test_signatures_and_installed_stub_share_exact_shape(self):
        self.assertEqual(inspect.signature(EndpointClient.read_range), inspect.signature(BACnetClient.read_range))
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text()
        tree = ast.parse(stub)
        for name in ("EndpointClient", "BACnetClient"):
            cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == name)
            method = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == "read_range")
            self.assertEqual(ast.unparse(method.returns), "Awaitable[ReadRangeResult]")
        shape = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "ReadRangeResult")
        annotations = {n.target.id: ast.unparse(n.annotation) for n in shape.body if isinstance(n, ast.AnnAssign)}
        self.assertEqual(annotations["result_flags"], "BitString")
        self.assertEqual(annotations["first_sequence_number"], "int | None")
