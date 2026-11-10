"""Event algorithm parameter keywords on ``add_event_enrollment`` (#2426).

As on the analog, binary and multi-state builders (#2410), they are the
application's configuration, applied before storage attaches: never saved,
gone under a value a peer wrote and the file holds, and back on a start that
passes ``clear_written_reporting=True``. Each is checked as a
``write_property_local`` of its property is, by the rules of an enrollment
without storage. ``object_property_reference`` is configuration peers can't
write: it applies at every start.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import struct
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    EventState,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

from test_reporting_persistence import installed_stub_method

P = PropertyIdentifier
ENROLLMENT = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, 1)
KEYWORDS = (
    "object_property_reference",
    "event_parameters",
    "fault_parameters",
    "time_delay_normal",
)

# OUT_OF_RANGE (Event_Type 5) above 40 or below 20, and another above 50,
# both after 30 seconds with a deadband of 1; FAULT_OUT_OF_RANGE (Fault_Type
# 6) outside -5 to 50, and the peer's outside -5 to 60. Each in its BACnet
# encoding.
EVENT = bytes.fromhex("5e091e1c41a000002c422000003c3f8000005f")
OTHER_EVENT = bytes.fromhex("5e091e1c41a000002c424800003c3f8000005f")
# The same range with no delay, so the first pass raises it.
UNDELAYED_EVENT = bytes.fromhex("5e09001c41a000002c422000003c3f8000005f")
FAULT = bytes.fromhex("6e0e5508c0140000000000000f1e550840490000000000001f6f")
PEER_FAULT = bytes.fromhex("6e0e5508c0140000000000000f1e5508404e0000000000001f6f")
# The presence mask's bit for Fault_Parameters.
FAULT_BIT = 1 << 11


def encoded(octets: bytes) -> PropertyValue:
    return PropertyValue.application_data(octets)


def saved_parameters(file: Path) -> int:
    """The presence mask of a kind-6 file whose enrollment saved no
    reporting setting."""
    octets = file.read_bytes()
    assert octets[12] == 6, octets[12]
    return struct.unpack(">I", octets[20:24])[0]


class SignatureTests(unittest.TestCase):
    def test_the_builder_takes_the_keywords_in_the_binding_and_the_stub(self) -> None:
        parameters = inspect.signature(BACnetServer.add_event_enrollment).parameters
        stub = installed_stub_method("add_event_enrollment").args.kwonlyargs
        annotations = {argument.arg: ast.unparse(argument.annotation) for argument in stub}
        expected = {
            "object_property_reference": "tuple[ObjectIdentifier, PropertyIdentifier] "
            "| tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] "
            "| DeviceObjectPropertyReference | None",
            "event_parameters": "Optional[PropertyValue]",
            "fault_parameters": "Optional[PropertyValue]",
            "time_delay_normal": "Optional[int]",
        }
        for keyword in KEYWORDS:
            with self.subTest(keyword=keyword):
                self.assertEqual(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameters[keyword].default)
                self.assertEqual(annotations[keyword], expected[keyword])


class KeywordStorageTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.file = Path(directory.name) / "ee-1"

    async def start(
        self,
        event: bytes = EVENT,
        clear: bool = False,
        storage: bool = True,
    ) -> BACnetServer:
        server = BACnetServer(2_426_001, interface="127.0.0.1", port=0)
        kept = {"storage_path": str(self.file), "clear_written_reporting": clear}
        server.add_event_enrollment(
            1,
            "EE-1",
            event_parameters=encoded(event),
            fault_parameters=encoded(FAULT),
            time_delay_normal=6,
            **(kept if storage else {}),
        )
        await server.start()
        return server

    async def served(self, server: BACnetServer) -> tuple:
        """The three parameters and the two types that follow them."""
        event = await server.read_property(ENROLLMENT, P.EVENT_PARAMETERS)
        fault = await server.read_property(ENROLLMENT, P.FAULT_PARAMETERS)
        delay = await server.read_property(ENROLLMENT, P.TIME_DELAY_NORMAL)
        event_type = await server.read_property(ENROLLMENT, P.EVENT_TYPE)
        fault_type = await server.read_property(ENROLLMENT, P.FAULT_TYPE)
        return event.octets, fault.octets, delay.value, event_type.value, fault_type.value

    async def serves(self, expected: tuple, **start) -> None:
        server = await self.start(**start)
        try:
            self.assertEqual(await self.served(server), expected)
        finally:
            await server.stop()

    def assert_no_parameter_saved(self) -> None:
        """The file is absent or holds kind 4: reporting settings alone."""
        self.assertTrue(not self.file.exists() or self.file.read_bytes()[12] == 4)

    async def test_keywords_apply_until_a_peer_writes_and_again_after_a_clear(self) -> None:
        configured = (EVENT, FAULT, 6, 5, 6)
        # The peer writes the other Event_Parameters.
        peers = (OTHER_EVENT, PEER_FAULT, 9, 5, 6)
        await self.serves(configured)
        self.assertFalse(self.file.exists(), "the keyword values are never saved")

        # A kept write that isn't a parameter saves no keyword value.
        server = await self.start()
        try:
            await server.write_property_local(
                ENROLLMENT, P.NOTIFICATION_CLASS, PropertyValue.unsigned(9), source_object=None
            )
        finally:
            await server.stop()
        self.assertTrue(self.file.exists())
        self.assert_no_parameter_saved()
        await self.serves(configured)

        # A peer's values go over the keywords once the file holds them.
        server = await self.start()
        try:
            for prop, value in (
                (P.EVENT_PARAMETERS, encoded(OTHER_EVENT)),
                (P.FAULT_PARAMETERS, encoded(PEER_FAULT)),
                (P.TIME_DELAY_NORMAL, PropertyValue.unsigned(9)),
            ):
                await server.write_property_local(ENROLLMENT, prop, value, source_object=None)
        finally:
            await server.stop()
        await self.serves(peers)

        # A clearing start, and the ones after it, serve the keywords again.
        await self.serves(configured, clear=True)
        self.assert_no_parameter_saved()
        await self.serves(configured)

    async def test_a_peers_write_is_kept_and_a_changed_keyword_applies(self) -> None:
        server = await self.start()
        try:
            await server.write_property_local(
                ENROLLMENT, P.FAULT_PARAMETERS, encoded(PEER_FAULT), source_object=None
            )
        finally:
            await server.stop()
        # The application now configures other Event_Parameters (the
        # changed keyword): they are served beside the peer's
        # Fault_Parameters, and the file holds the peer's row alone.
        await self.serves((OTHER_EVENT, PEER_FAULT, 6, 5, 6), event=OTHER_EVENT)
        self.assertEqual(saved_parameters(self.file), FAULT_BIT)

    async def test_a_keyword_past_64_kib_is_taken_and_never_saved(self) -> None:
        # The cap meets only a saved write; a keyword value isn't one.
        long = PropertyValue.octet_string(bytes(65_533))
        server = await self.start_long(long)
        try:
            await server.write_property_local(
                ENROLLMENT, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(9), source_object=None
            )
        finally:
            await server.stop()
        self.assertLess(self.file.stat().st_size, 1_024)
        server = await self.start_long(long)
        try:
            read = await server.read_property(ENROLLMENT, P.EVENT_PARAMETERS)
            self.assertEqual(len(read.octets), 65_537)
            delay = await server.read_property(ENROLLMENT, P.TIME_DELAY_NORMAL)
            self.assertEqual(delay.value, 9)
        finally:
            await server.stop()

    async def start_long(self, event_parameters: PropertyValue) -> BACnetServer:
        server = BACnetServer(2_426_005, interface="127.0.0.1", port=0)
        server.add_event_enrollment(
            1, "EE-1", event_parameters=event_parameters, storage_path=str(self.file)
        )
        await server.start()
        return server

    async def test_keywords_set_the_values_without_storage(self) -> None:
        await self.serves((EVENT, FAULT, 6, 5, 6), storage=False)
        self.assertFalse(self.file.exists())


class KeywordRefusalTests(unittest.TestCase):
    def assert_refused(self, code: ErrorCode, **keywords) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_426_002, interface="127.0.0.1", port=0)
            path = Path(directory) / "ee-1"
            with self.assertRaises(BacnetProtocolError) as raised:
                server.add_event_enrollment(1, "EE-1", storage_path=str(path), **keywords)
            self.assertEqual(raised.exception.error_code, code.to_raw())
            self.assertFalse(path.exists())
            # Nothing was added: the instance is still free.
            server.add_event_enrollment(1, "EE-1")

    def test_values_the_enrollment_refuses_raise_from_the_builder(self) -> None:
        # Octets cut short of one whole value, and a value of another
        # datatype, are refused as the write refuses them.
        self.assert_refused(
            ErrorCode.INVALID_DATA_ENCODING, event_parameters=encoded(EVENT[:-1])
        )
        self.assert_refused(
            ErrorCode.INVALID_DATA_TYPE, fault_parameters=PropertyValue.real(1.0)
        )
        # A reference naming a Device that isn't one.
        with self.assertRaises(ValueError):
            BACnetServer(2_426_006, interface="127.0.0.1", port=0).add_event_enrollment(
                1,
                "EE-1",
                object_property_reference={
                    "object_identifier": ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                    "property_identifier": P.PRESENT_VALUE,
                    "device_identifier": ObjectIdentifier(ObjectType.ANALOG_INPUT, 2),
                },
            )
        server = BACnetServer(2_426_003, interface="127.0.0.1", port=0)
        for delay in (-1, 2**32):
            with self.subTest(delay=delay), self.assertRaises(OverflowError):
                server.add_event_enrollment(1, "EE-1", time_delay_normal=delay)


class KeywordDetectionTests(unittest.IsolatedAsyncioTestCase):
    async def test_the_enrollment_alarms_only_under_the_keyword_parameters(self) -> None:
        """Two enrollments monitor an Analog Input at 50: the one given the
        OUT_OF_RANGE keyword goes to HIGH_LIMIT on the server's first pass,
        and the one left on the default parameters stays NORMAL."""
        analog = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        server = BACnetServer(2_426_007, interface="127.0.0.1", port=0)
        server.add_analog_input(1, "AI", present_value=50.0)
        server.add_event_enrollment(
            1,
            "EE-1",
            object_property_reference=(analog, P.PRESENT_VALUE),
            event_parameters=encoded(UNDELAYED_EVENT),
        )
        server.add_event_enrollment(
            2,
            "EE-2",
            object_property_reference={
                "object_identifier": analog,
                "property_identifier": P.PRESENT_VALUE,
            },
        )
        await server.start()
        try:
            reference = await server.read_property(ENROLLMENT, P.OBJECT_PROPERTY_REFERENCE)
            self.assertEqual(reference.value["object_identifier"], analog)
            expected = PropertyValue.enumerated(EventState.HIGH_LIMIT.to_raw())
            for _ in range(100):
                state = await server.read_property(ENROLLMENT, P.EVENT_STATE)
                if state == expected:
                    break
                await asyncio.sleep(0.05)
            self.assertEqual(state, expected)
            control = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, 2)
            self.assertEqual(
                await server.read_property(control, P.EVENT_STATE),
                PropertyValue.enumerated(EventState.NORMAL.to_raw()),
            )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
