"""Installed-artifact tests for a Trend Log Multiple configured from Python
(#1235).

`add_trend_log_multiple` takes the members, Log_Interval, Logging_Type, the
Start_Time / Stop_Time window and clock alignment; the running server's
poller then logs one value per member in each record, which a client reads
back with ReadRange.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import struct
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


POLLED_LOG = ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 1)
TRIGGERED_LOG = ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 2)
WAIT = 5.0
KEYWORDS = [
    "members",
    "log_interval",
    "logging_type",
    "start_time",
    "stop_time",
    "align_intervals",
    "interval_offset",
]


def member(instance: int) -> dict:
    return {
        "object_identifier": ObjectIdentifier(ObjectType.ANALOG_INPUT, instance),
        "property_identifier": PropertyIdentifier.PRESENT_VALUE,
    }


def tuple_member(instance: int) -> tuple:
    return (
        ObjectIdentifier(ObjectType.ANALOG_INPUT, instance),
        PropertyIdentifier.PRESENT_VALUE,
    )


def read_member(instance: int, index: int | None = None) -> dict:
    """AI-``instance``'s Present_Value as a read of a member gives it back
    (#1345): a DeviceObjectPropertyReference mapping with every key."""
    return {**member(instance), "property_array_index": index, "device_identifier": None}


def installed_stub_method(name: str) -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return next(
        node
        for node in server.body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


def value_records(item_data: bytes, values: tuple[float, ...]) -> int:
    """Count the BACnetLogMultipleRecords in ``item_data``, each holding
    ``values`` as REALs: a [0] timestamp of an application Date and Time,
    then [1] log data wrapping [1] one real-value [1] per member."""
    body = (
        b"\x1e\x1e"
        + b"".join(b"\x1c" + struct.pack(">f", value) for value in values)
        + b"\x1f\x1f"
    )
    size = 12 + len(body)
    assert len(item_data) % size == 0, item_data.hex()
    for start in range(0, len(item_data), size):
        record = item_data[start : start + size]
        assert record[:2] == b"\x0e\xa4" and record[6] == 0xB4, record.hex()
        assert record[11] == 0x0F and record[12:] == body, record.hex()
    return len(item_data) // size


def assert_protocol_error(
    case: unittest.TestCase,
    raised: BacnetProtocolError,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> None:
    case.assertEqual(raised.error_class, error_class.to_raw())
    case.assertEqual(raised.error_code, error_code.to_raw())


class TrendLogMultipleStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_take_the_same_keyword_arguments(self) -> None:
        parameters = inspect.signature(BACnetServer.add_trend_log_multiple).parameters
        self.assertEqual(
            list(parameters),
            ["self", "instance", "name", "buffer_size", *KEYWORDS, "total_record_count"],
        )
        for name in [*KEYWORDS, "total_record_count"]:
            with self.subTest(keyword=name):
                self.assertEqual(parameters[name].kind, inspect.Parameter.KEYWORD_ONLY)
        for name in KEYWORDS:
            with self.subTest(keyword=name):
                self.assertIsNone(parameters[name].default)
        self.assertEqual(parameters["total_record_count"].default, 0)
        method = installed_stub_method("add_trend_log_multiple")
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs],
            [*KEYWORDS, "total_record_count"],
        )
        docs = ast.get_docstring(method)
        assert docs is not None
        for phrase in ("Trigger", "write_property_local", "VALUE_OUT_OF_RANGE"):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)


class TrendLogMultipleConfigurationTests(unittest.TestCase):
    def test_bad_settings_are_refused_before_start(self) -> None:
        server = BACnetServer(1_235_000, interface="127.0.0.1", port=0)
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_trend_log_multiple(1, "TLM-1", logging_type="cov")
        assert_protocol_error(
            self, raised.exception, ErrorClass.PROPERTY, ErrorCode.VALUE_OUT_OF_RANGE
        )
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_trend_log_multiple(
                1, "TLM-1", logging_type="triggered", log_interval=100
            )
        assert_protocol_error(
            self, raised.exception, ErrorClass.PROPERTY, ErrorCode.WRITE_ACCESS_DENIED
        )
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_trend_log_multiple(
                1, "TLM-1", start_time=((255, 10, 3, 6), (12, 0, 0, 0))
            )
        assert_protocol_error(
            self, raised.exception, ErrorClass.PROPERTY, ErrorCode.VALUE_OUT_OF_RANGE
        )
        with self.assertRaises(ValueError):
            server.add_trend_log_multiple(1, "TLM-1", logging_type="sometimes")
        with self.assertRaises(ValueError):
            server.add_trend_log_multiple(1, "TLM-1", start_time=(2026, 10, 3, 6))
        with self.assertRaises(ValueError):
            server.add_trend_log_multiple(
                1, "TLM-1", members=[{**member(1), "array_index": 1}]
            )
        with self.assertRaises(ValueError):
            server.add_trend_log_multiple(
                1,
                "TLM-1",
                members=[
                    {
                        **member(1),
                        "device_identifier": ObjectIdentifier(ObjectType.ANALOG_INPUT, 9),
                    }
                ],
            )
        with self.assertRaises(TypeError):
            server.add_trend_log_multiple(1, "TLM-1", members=[ObjectType.ANALOG_INPUT])


class TrendLogMultipleMemberFormTests(unittest.TestCase):
    def test_tuple_and_mapping_members_and_the_own_device(self) -> None:
        asyncio.run(self._member_forms())

    async def _member_forms(self) -> None:
        own_device = ObjectIdentifier(ObjectType.DEVICE, 1_235_002)
        server = BACnetServer(1_235_002, interface="127.0.0.1", port=0)
        server.add_trend_log_multiple(
            1,
            "TLM-1",
            members=[
                tuple_member(1),
                (*tuple_member(2), 3),
                (*tuple_member(3), None),
                # Naming this server's Device, stored without it.
                {**member(4), "device_identifier": own_device},
            ],
        )
        with self.assertRaises(TypeError):
            server.add_trend_log_multiple(2, "TLM-2", members=[(tuple_member(1)[0],)])
        with self.assertRaises(OverflowError):
            server.add_trend_log_multiple(2, "TLM-2", members=[(*tuple_member(1), -1)])
        await server.start()
        try:
            elements = [
                (
                    await server.read_property(
                        POLLED_LOG, PropertyIdentifier.LOG_DEVICE_OBJECT_PROPERTY, index
                    )
                ).value
                for index in range(1, 5)
            ]
            self.assertEqual(
                elements,
                [
                    read_member(1),
                    read_member(2, 3),
                    read_member(3),
                    read_member(4),
                ],
            )
            with self.assertRaises(BacnetProtocolError):
                await server.read_property(
                    ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 2),
                    PropertyIdentifier.OBJECT_NAME,
                )
        finally:
            await server.stop()


class TrendLogMultipleLiveServerTests(unittest.TestCase):
    def test_polled_and_triggered_logs_configured_from_python(self) -> None:
        asyncio.run(self._exercise())

    async def _exercise(self) -> None:
        server = BACnetServer(
            1_235_001,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_analog_input(1, "AI-1", present_value=21.5)
        server.add_analog_input(2, "AI-2", present_value=7.0)
        # Every 100 ms, inside an open window; Align_Intervals with an
        # interval that divides a second still logs every 100 ms.
        server.add_trend_log_multiple(
            1,
            "TLM-1",
            50,
            # A tuple member and a mapping member log the same way.
            members=[tuple_member(1), member(2)],
            log_interval=10,
            logging_type="polled",
            start_time=((255, 255, 255, 255), (255, 255, 255, 255)),
            stop_time=((2154, 12, 31, 255), (23, 59, 59, 99)),
            align_intervals=True,
            interval_offset=0,
        )
        server.add_trend_log_multiple(
            2, "TLM-2", members=[member(2)], logging_type="triggered"
        )
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:

                async def read_log(oid: ObjectIdentifier) -> dict:
                    return await client.read_range(
                        address, oid, PropertyIdentifier.LOG_BUFFER
                    )

                async def records(oid: ObjectIdentifier, values: tuple, at_least: int) -> int:
                    async with asyncio.timeout(WAIT):
                        while True:
                            result = await read_log(oid)
                            count = value_records(result["item_data"], values)
                            self.assertEqual(count, result["item_count"])
                            if count >= at_least:
                                return count
                            await asyncio.sleep(0.05)

                # The polled log fills from its members, in member order.
                self.assertGreaterEqual(await records(POLLED_LOG, (21.5, 7.0), 2), 2)
                self.assertEqual(
                    await server.read_property(
                        POLLED_LOG, PropertyIdentifier.LOG_INTERVAL
                    ),
                    PropertyValue.unsigned(10),
                )

                # The triggered log waits for a Trigger, from a peer or the
                # application, and logs once for each.
                self.assertEqual((await read_log(TRIGGERED_LOG))["item_count"], 0)
                with self.assertRaises(BacnetProtocolError) as raised:
                    await client.write_property(
                        address,
                        TRIGGERED_LOG,
                        PropertyIdentifier.LOG_INTERVAL,
                        PropertyValue.unsigned(100),
                    )
                assert_protocol_error(
                    self,
                    raised.exception,
                    ErrorClass.PROPERTY,
                    ErrorCode.WRITE_ACCESS_DENIED,
                )
                await server.write_property_local(
                    TRIGGERED_LOG,
                    PropertyIdentifier.TRIGGER,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                self.assertEqual(await records(TRIGGERED_LOG, (7.0,), 1), 1)
                self.assertEqual(
                    await server.read_property(TRIGGERED_LOG, PropertyIdentifier.TRIGGER),
                    PropertyValue.boolean(False),
                )
                await client.write_property(
                    address,
                    TRIGGERED_LOG,
                    PropertyIdentifier.TRIGGER,
                    PropertyValue.boolean(True),
                )
                self.assertEqual(await records(TRIGGERED_LOG, (7.0,), 2), 2)

                # COV is refused over the wire too, and nothing changes.
                with self.assertRaises(BacnetProtocolError) as raised:
                    await client.write_property(
                        address,
                        TRIGGERED_LOG,
                        PropertyIdentifier.LOGGING_TYPE,
                        PropertyValue.enumerated(1),
                    )
                assert_protocol_error(
                    self,
                    raised.exception,
                    ErrorClass.PROPERTY,
                    ErrorCode.VALUE_OUT_OF_RANGE,
                )
                self.assertEqual(
                    await server.read_property(
                        TRIGGERED_LOG, PropertyIdentifier.LOGGING_TYPE
                    ),
                    PropertyValue.enumerated(2),
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
