"""Event algorithm parameter keywords on the analog, binary and multi-state
builders (#2410).

They are the application's configuration, applied before storage attaches:
never saved, gone under a value a peer wrote and the file holds, and back on
a start that passes ``clear_written_reporting=True``. Each is checked as a
write of its property is.
"""

from __future__ import annotations

import asyncio
import inspect
import math
import struct
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    BipEndpoint,
    BitString,
    ErrorCode,
    EventState,
    MstpEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

from test_reporting_persistence import installed_stub_method

P = PropertyIdentifier
O = ObjectType
DELAYS = ("time_delay", "time_delay_normal")
ANALOG = ("high_limit", "low_limit", "deadband", "limit_enable") + DELAYS
KEYWORDS = {
    "add_analog_input": ANALOG,
    "add_analog_output": ANALOG,
    "add_analog_value": ANALOG,
    "add_binary_input": ("alarm_value",) + DELAYS,
    "add_binary_output": DELAYS,
    "add_binary_value": ("alarm_value",) + DELAYS,
    "add_multistate_input": ("alarm_values",) + DELAYS,
    "add_multistate_output": DELAYS,
    "add_multistate_value": ("alarm_values",) + DELAYS,
}
ENDPOINT_BUILDERS = ("add_analog_input", "add_analog_value", "add_binary_input", "add_binary_value")
HIGH_ONLY = BitString(6, b"\x40")


def states(*raw: int) -> PropertyValue:
    return PropertyValue.list([PropertyValue.unsigned(state) for state in raw])


# Per builder: its positional arguments, then (keyword, value, property, the
# value it serves, a peer's value).
OBJECTS = (
    ("add_analog_input", O.ANALOG_INPUT, (), (
        ("high_limit", 30.0, P.HIGH_LIMIT, PropertyValue.real(30.0), PropertyValue.real(80.5)),
        ("limit_enable", HIGH_ONLY, P.LIMIT_ENABLE, PropertyValue.bit_string(6, b"\x40"),
         PropertyValue.bit_string(6, b"\x80")),
    )),
    ("add_analog_output", O.ANALOG_OUTPUT, (), (
        ("deadband", 2.0, P.DEADBAND, PropertyValue.real(2.0), PropertyValue.real(3.5)),
        ("time_delay", 5, P.TIME_DELAY, PropertyValue.unsigned(5), PropertyValue.unsigned(9)),
    )),
    ("add_analog_value", O.ANALOG_VALUE, (), (
        ("low_limit", -5.0, P.LOW_LIMIT, PropertyValue.real(-5.0), PropertyValue.real(1.0)),
        ("time_delay_normal", 6, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(6),
         PropertyValue.unsigned(9)),
    )),
    ("add_binary_input", O.BINARY_INPUT, (), (
        ("alarm_value", 0, P.ALARM_VALUE, PropertyValue.enumerated(0), PropertyValue.enumerated(1)),
    )),
    ("add_binary_output", O.BINARY_OUTPUT, (), (
        ("time_delay", 5, P.TIME_DELAY, PropertyValue.unsigned(5), PropertyValue.unsigned(9)),
    )),
    ("add_binary_value", O.BINARY_VALUE, (), (
        ("alarm_value", 0, P.ALARM_VALUE, PropertyValue.enumerated(0), PropertyValue.enumerated(1)),
        ("time_delay_normal", 6, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(6),
         PropertyValue.unsigned(9)),
    )),
    ("add_multistate_input", O.MULTI_STATE_INPUT, (3,), (
        ("alarm_values", [2], P.ALARM_VALUES, states(2), states(3)),
    )),
    ("add_multistate_output", O.MULTI_STATE_OUTPUT, (3,), (
        ("time_delay_normal", 4, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(4),
         PropertyValue.unsigned(9)),
    )),
    ("add_multistate_value", O.MULTI_STATE_VALUE, (3,), (
        ("alarm_values", [1, 3], P.ALARM_VALUES, states(1, 3), states(2)),
        ("time_delay", 7, P.TIME_DELAY, PropertyValue.unsigned(7), PropertyValue.unsigned(9)),
    )),
)


def build(server, builder: str, positional: tuple, keywords: dict) -> None:
    getattr(server, builder)(1, builder, *positional, **keywords)


class SignatureTests(unittest.TestCase):
    def test_each_builder_takes_its_keywords_in_the_binding_and_the_stub(self) -> None:
        classes = [(BACnetServer, name, KEYWORDS[name]) for name in KEYWORDS]
        for endpoint in (BipEndpoint, MstpEndpoint):
            classes += [(endpoint, name, KEYWORDS[name]) for name in ENDPOINT_BUILDERS]
        for owner, name, keywords in classes:
            parameters = inspect.signature(getattr(owner, name)).parameters
            stub = []
            if owner is BACnetServer:
                stub = [argument.arg for argument in installed_stub_method(name).args.kwonlyargs]
            for keyword in keywords:
                with self.subTest(owner=owner.__name__, name=name, keyword=keyword):
                    self.assertEqual(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                    self.assertIsNone(parameters[keyword].default)
                    if stub:
                        self.assertIn(keyword, stub)


class KeywordStorageTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, clear: bool = False) -> BACnetServer:
        server = BACnetServer(2_410_001, interface="127.0.0.1", port=0)
        for builder, _, positional, rows in OBJECTS:
            keywords = {keyword: value for keyword, value, *_ in rows}
            build(server, builder, positional, dict(
                keywords, storage_path=str(self.state / builder),
                clear_written_reporting=clear))
        await server.start()
        return server

    async def served(self, expected_column: int, clear: bool = False) -> None:
        server = await self.start(clear)
        try:
            for builder, object_type, _, rows in OBJECTS:
                oid = ObjectIdentifier(object_type, 1)
                for row in rows:
                    with self.subTest(builder=builder, prop=row[2], column=expected_column):
                        self.assertEqual(await server.read_property(oid, row[2]),
                                         row[expected_column])
        finally:
            await server.stop()

    def assert_no_parameter_saved(self) -> None:
        """Each file is absent or holds kind 4 of the format: reporting
        settings and no parameter."""
        for builder, *_ in OBJECTS:
            file = self.state / builder
            with self.subTest(file=builder):
                self.assertTrue(not file.exists() or file.read_bytes()[12] == 4)

    async def test_keywords_apply_until_a_peer_writes_and_again_after_a_clear(self) -> None:
        await self.served(3)
        for builder, *_ in OBJECTS:
            self.assertFalse((self.state / builder).exists(), builder)
        # A kept write that isn't a parameter saves no keyword value.
        server = await self.start()
        try:
            await server.write_property_local(
                ObjectIdentifier(O.ANALOG_INPUT, 1), P.NOTIFICATION_CLASS,
                PropertyValue.unsigned(9), source_object=None)
        finally:
            await server.stop()
        self.assertTrue((self.state / "add_analog_input").exists())
        self.assert_no_parameter_saved()
        await self.served(3)

        # A peer's values go over the keywords once the files hold them.
        server = await self.start()
        try:
            for _, object_type, _, rows in OBJECTS:
                for row in rows:
                    await server.write_property_local(
                        ObjectIdentifier(object_type, 1), row[2], row[4], source_object=None)
        finally:
            await server.stop()
        await self.served(4)

        # A clearing start, and the ones after it, serve the keywords again.
        await self.served(3, clear=True)
        self.assert_no_parameter_saved()
        await self.served(3)

    async def test_keywords_set_the_values_without_storage(self) -> None:
        server = BACnetServer(2_410_002, interface="127.0.0.1", port=0)
        for builder, _, positional, rows in OBJECTS:
            build(server, builder, positional, {keyword: value for keyword, value, *_ in rows})
        await server.start()
        try:
            for builder, object_type, _, rows in OBJECTS:
                for row in rows:
                    with self.subTest(builder=builder, prop=row[2]):
                        self.assertEqual(
                            await server.read_property(ObjectIdentifier(object_type, 1), row[2]),
                            row[3])
        finally:
            await server.stop()


def saved_parameters(file: Path) -> int:
    """The presence mask of a kind-6 file whose object saved no reporting
    setting: bit 0 Time_Delay, 2 High_Limit, 4 Deadband, 7 Alarm_Values."""
    octets = file.read_bytes()
    assert octets[12] == 6, octets[12]
    return struct.unpack(">I", octets[20:24])[0]


class MixedRestoreTests(unittest.IsolatedAsyncioTestCase):
    """A peer's write of one parameter is kept; the keywords for the others
    stay the application's, so a changed keyword takes effect at the next
    start (#2410)."""

    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, deadband: float, alarm_values: list[int]) -> BACnetServer:
        server = BACnetServer(2_410_005, interface="127.0.0.1", port=0)
        server.add_analog_input(1, "AI", high_limit=30.0, deadband=deadband, time_delay=5,
                                storage_path=str(self.state / "ai"))
        server.add_multistate_value(1, "MSV", 3, alarm_values=alarm_values, time_delay=5,
                                    storage_path=str(self.state / "msv"))
        await server.start()
        return server

    async def test_a_peers_write_is_kept_and_a_changed_keyword_applies(self) -> None:
        ai = ObjectIdentifier(O.ANALOG_INPUT, 1)
        msv = ObjectIdentifier(O.MULTI_STATE_VALUE, 1)
        server = await self.start(deadband=1.0, alarm_values=[2])
        try:
            await server.write_property_local(ai, P.HIGH_LIMIT, PropertyValue.real(80.5),
                                              source_object=None)
            await server.write_property_local(msv, P.TIME_DELAY, PropertyValue.unsigned(9),
                                              source_object=None)
        finally:
            await server.stop()

        server = await self.start(deadband=2.5, alarm_values=[1, 3])
        try:
            served = (
                (ai, P.HIGH_LIMIT, PropertyValue.real(80.5)),
                (ai, P.DEADBAND, PropertyValue.real(2.5)),
                (ai, P.TIME_DELAY, PropertyValue.unsigned(5)),
                (msv, P.TIME_DELAY, PropertyValue.unsigned(9)),
                (msv, P.ALARM_VALUES, states(1, 3)),
            )
            for oid, prop, value in served:
                with self.subTest(oid=oid, prop=prop):
                    self.assertEqual(await server.read_property(oid, prop), value)
        finally:
            await server.stop()
        # Each file holds the peer's parameter alone.
        self.assertEqual(saved_parameters(self.state / "ai"), 1 << 2)
        self.assertEqual(saved_parameters(self.state / "msv"), 1 << 0)


class KeywordDetectionTests(unittest.IsolatedAsyncioTestCase):
    async def test_event_detection_uses_the_keyword_values(self) -> None:
        """Each family raises an alarm only its keyword value causes: the
        defaults are High_Limit 100 with both limits disabled, Alarm_Value
        ACTIVE and no Alarm_Values."""
        server = BACnetServer(2_410_003, interface="127.0.0.1", port=0)
        server.add_notification_class(0, "NC")
        server.add_analog_input(1, "AI", high_limit=30.0, limit_enable=HIGH_ONLY)
        server.add_binary_input(1, "BI", alarm_value=0)
        server.add_multistate_input(1, "MSI", 3, alarm_values=[2])
        cases = (
            (ObjectIdentifier(O.ANALOG_INPUT, 1), (PropertyValue.real(40.0),),
             EventState.HIGH_LIMIT),
            # INACTIVE after ACTIVE: ACTIVE alarms by default, INACTIVE only
            # by the keyword.
            (ObjectIdentifier(O.BINARY_INPUT, 1),
             (PropertyValue.enumerated(1), PropertyValue.enumerated(0)), EventState.OFFNORMAL),
            (ObjectIdentifier(O.MULTI_STATE_INPUT, 1), (PropertyValue.unsigned(2),),
             EventState.OFFNORMAL),
        )
        await server.start()
        try:
            for oid, values, expected in cases:
                with self.subTest(oid=oid):
                    for value in values:
                        await server.set_present_value_local(oid, value)
                    expected = PropertyValue.enumerated(expected.to_raw())
                    for _ in range(100):
                        state = await server.read_property(oid, P.EVENT_STATE)
                        if state == expected:
                            break
                        await asyncio.sleep(0.05)
                    self.assertEqual(state, expected)
        finally:
            await server.stop()


class KeywordRefusalTests(unittest.TestCase):
    def assert_refused(self, call, code: ErrorCode = ErrorCode.VALUE_OUT_OF_RANGE) -> None:
        with self.assertRaises(BacnetProtocolError) as raised:
            call()
        self.assertEqual(raised.exception.error_code, code.to_raw())

    def test_values_the_object_refuses_on_the_wire_raise_from_the_builder(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_410_004, interface="127.0.0.1", port=0)
            path = str(Path(directory) / "refused")
            for keywords in ({"high_limit": math.nan}, {"low_limit": math.inf},
                             {"deadband": -1.0}, {"high_limit": 1e300}):
                with self.subTest(keywords=keywords):
                    self.assert_refused(lambda: server.add_analog_value(
                        1, "AV", storage_path=path, **keywords))
            # Limit_Enable is two bits wide; three are a malformed encoding.
            self.assert_refused(
                lambda: server.add_analog_input(1, "AI", limit_enable=BitString(5, b"\x40")),
                ErrorCode.INVALID_DATA_ENCODING)
            self.assert_refused(lambda: server.add_binary_value(1, "BV", alarm_value=2))
            for refused in ([0], [1, 4]):
                with self.subTest(alarm_values=refused):
                    self.assert_refused(lambda: server.add_multistate_input(
                        1, "MSI", 3, alarm_values=refused))
                    self.assert_refused(lambda: server.add_multistate_value(
                        1, "MSV", 3, alarm_values=refused))
            with self.assertRaises(OverflowError):
                server.add_binary_output(1, "BO", time_delay=-1)
            with self.assertRaises(OverflowError):
                server.add_multistate_output(1, "MSO", 3, time_delay_normal=2**32)
            self.assertFalse(Path(path).exists())

    def test_endpoint_builders_refuse_them_too(self) -> None:
        endpoints = [BipEndpoint(device_instance=2410, interface="127.0.0.1", port=0,
                                 broadcast_address="127.255.255.255"),
                     MstpEndpoint(device_instance=2410, serial_port="/tmp/nonexistent")]
        for endpoint in endpoints:
            with self.subTest(endpoint=type(endpoint).__name__):
                self.assert_refused(lambda: endpoint.add_analog_input(1, "AI", high_limit=math.nan))
                self.assert_refused(lambda: endpoint.add_analog_value(1, "AV", deadband=-1.0))
                self.assert_refused(lambda: endpoint.add_binary_input(1, "BI", alarm_value=2))
                self.assert_refused(lambda: endpoint.add_binary_value(1, "BV", alarm_value=3))
                self.assertEqual(endpoint._pending_registration_count(), 0)


class EndpointKeywordTests(unittest.IsolatedAsyncioTestCase):
    async def test_endpoint_objects_serve_the_keyword_values(self) -> None:
        endpoint = BipEndpoint(device_instance=2411, interface="127.0.0.1", port=0,
                               broadcast_address="127.255.255.255")
        endpoint.add_analog_input(1, "AI", high_limit=30.0, time_delay=5)
        endpoint.add_analog_value(1, "AV", limit_enable=HIGH_ONLY)
        endpoint.add_binary_input(1, "BI", alarm_value=0)
        endpoint.add_binary_value(1, "BV", time_delay_normal=6)
        expected = (
            (O.ANALOG_INPUT, P.HIGH_LIMIT, 30.0),
            (O.ANALOG_INPUT, P.TIME_DELAY, 5),
            (O.BINARY_INPUT, P.ALARM_VALUE, 0),
            (O.BINARY_VALUE, P.TIME_DELAY_NORMAL, 6),
        )
        async with endpoint, BACnetClient(interface="127.0.0.1", port=0) as client:
            address = await endpoint.local_address()
            for object_type, prop, value in expected:
                with self.subTest(object_type=object_type, prop=prop):
                    read = await client.read_property(address, ObjectIdentifier(object_type, 1),
                                                      prop)
                    self.assertEqual(read.value, value)
            limit_enable = await client.read_property(
                address, ObjectIdentifier(O.ANALOG_VALUE, 1), P.LIMIT_ENABLE)
            self.assertEqual(limit_enable.value, HIGH_ONLY)


if __name__ == "__main__":
    unittest.main()
