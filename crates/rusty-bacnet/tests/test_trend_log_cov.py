"""Installed-artifact tests for a Trend Log configured from Python, logging
by COV (#1480).

`add_trend_log` takes the keyword settings `add_trend_log_multiple` does,
plus a reference and the two COV properties; a running server's poller then
logs each change of the referenced property that meets the increment, with
the monitored object's Status_Flags, which a client reads back with
ReadRange.
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


LOG = ObjectIdentifier(ObjectType.TREND_LOG, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
WAIT = 5.0
KEYWORDS = [
    "reference",
    "log_interval",
    "logging_type",
    "client_cov_increment",
    "cov_resubscription_interval",
    "start_time",
    "stop_time",
    "align_intervals",
    "interval_offset",
    "storage_path",
]


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


def real_records(item_data: bytes) -> list[tuple[float, int]]:
    """Each BACnetLogRecord in ``item_data`` as (value, Status_Flags octet):
    a [0] timestamp of an application Date and Time, [1] a log datum holding
    [2] a REAL, then [2] the four-bit Status_Flags."""
    size = 22
    assert len(item_data) % size == 0, item_data.hex()
    records = []
    for start in range(0, len(item_data), size):
        record = item_data[start : start + size]
        assert record[:2] == b"\x0e\xa4" and record[6] == 0xB4, record.hex()
        assert record[11:14] == b"\x0f\x1e\x2c" and record[18] == 0x1F, record.hex()
        assert record[19:21] == b"\x2a\x04", record.hex()
        records.append((struct.unpack(">f", record[14:18])[0], record[21]))
    return records


def assert_protocol_error(
    case: unittest.TestCase,
    raised: BacnetProtocolError,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> None:
    case.assertEqual(raised.error_class, error_class.to_raw())
    case.assertEqual(raised.error_code, error_code.to_raw())


class TrendLogStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_take_the_same_keyword_arguments(self) -> None:
        parameters = inspect.signature(BACnetServer.add_trend_log).parameters
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
        method = installed_stub_method("add_trend_log")
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs],
            [*KEYWORDS, "total_record_count"],
        )
        docs = ast.get_docstring(method)
        assert docs is not None
        for phrase in ("Status_Flags", "NULL", "VALUE_OUT_OF_RANGE", "12.25.9"):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)


class TrendLogConfigurationTests(unittest.TestCase):
    def test_bad_settings_are_refused_before_start(self) -> None:
        server = BACnetServer(1_480_000, interface="127.0.0.1", port=0)
        for keywords, code in [
            ({"logging_type": "triggered", "log_interval": 100}, ErrorCode.WRITE_ACCESS_DENIED),
            ({"client_cov_increment": -1.0}, ErrorCode.VALUE_OUT_OF_RANGE),
            ({"cov_resubscription_interval": 0}, ErrorCode.VALUE_OUT_OF_RANGE),
        ]:
            with self.subTest(keywords=keywords):
                with self.assertRaises(BacnetProtocolError) as raised:
                    server.add_trend_log(1, "TL-1", **keywords)
                assert_protocol_error(
                    self, raised.exception, ErrorClass.PROPERTY, code
                )
        with self.assertRaises(ValueError):
            server.add_trend_log(1, "TL-1", logging_type="sometimes")
        # Polled with a zero interval would switch to COV (Clause 12.25.9),
        # so the pair is refused and the reason named.
        with self.assertRaises(ValueError) as raised:
            server.add_trend_log(1, "TL-1", logging_type="polled", log_interval=0)
        self.assertIn("COV", str(raised.exception))
        # A zero interval alone is accepted, as before.
        server.add_trend_log(2, "TL-2", log_interval=0)
        with self.assertRaises(TypeError):
            server.add_trend_log(1, "TL-1", reference=ObjectType.ANALOG_VALUE)


class TrendLogCovLiveServerTests(unittest.TestCase):
    def test_a_cov_log_records_each_change_meeting_the_increment(self) -> None:
        asyncio.run(self._exercise())

    async def _exercise(self) -> None:
        server = BACnetServer(
            1_480_001,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_analog_value(1, "AV-1")
        server.add_trend_log(
            1,
            "TL-1",
            50,
            reference=(AV, PropertyIdentifier.PRESENT_VALUE),
            logging_type="cov",
            client_cov_increment=1.0,
            cov_resubscription_interval=600,
        )
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:

                async def logged(at_least: int) -> list[tuple[float, int]]:
                    async with asyncio.timeout(WAIT):
                        while True:
                            result = await client.read_range(
                                address, LOG, PropertyIdentifier.LOG_BUFFER
                            )
                            records = real_records(result["item_data"])
                            if len(records) >= at_least:
                                return records
                            await asyncio.sleep(0.05)

                async def write_value(value: float) -> None:
                    await server.write_property_local(
                        AV,
                        PropertyIdentifier.PRESENT_VALUE,
                        PropertyValue.real(value),
                        priority=8,
                        source_object=None,
                    )

                # The first look logs the value, with clear Status_Flags.
                self.assertEqual(await logged(1), [(0.0, 0x00)])
                for prop, value in [
                    (PropertyIdentifier.LOGGING_TYPE, PropertyValue.enumerated(1)),
                    (PropertyIdentifier.LOG_INTERVAL, PropertyValue.unsigned(0)),
                    (PropertyIdentifier.CLIENT_COV_INCREMENT, PropertyValue.real(1.0)),
                    (
                        PropertyIdentifier.COV_RESUBSCRIPTION_INTERVAL,
                        PropertyValue.unsigned(600),
                    ),
                ]:
                    with self.subTest(property=prop):
                        self.assertEqual(await server.read_property(LOG, prop), value)

                # Short of the increment: no record, even after a few passes.
                await write_value(0.5)
                await asyncio.sleep(0.4)
                self.assertEqual(len(await logged(1)), 1)
                # Past it: one record of the new value.
                await write_value(2.0)
                self.assertEqual(await logged(2), [(0.0, 0x00), (2.0, 0x00)])

                # A peer's nonzero Log_Interval turns it to POLLED (Clause
                # 12.25.9).
                await client.write_property(
                    address, LOG, PropertyIdentifier.LOG_INTERVAL, PropertyValue.unsigned(500)
                )
                self.assertEqual(
                    await server.read_property(LOG, PropertyIdentifier.LOGGING_TYPE),
                    PropertyValue.enumerated(0),
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
