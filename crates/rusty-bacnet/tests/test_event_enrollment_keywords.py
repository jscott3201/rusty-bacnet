"""Event algorithm parameter keywords on ``add_event_enrollment`` (#2426).

As on the analog, binary and multi-state builders (#2410), they are the
application's configuration, applied before storage attaches: never saved,
gone under a value a peer wrote and the file holds, and back on a start that
passes ``clear_written_reporting=True``. Each is checked as a
``write_property_local`` of its property is.

The builder can't set Object_Property_Reference, so an enrollment it builds
monitors nothing; that the enrollment pass evaluates with the keyword
parameters is pinned by the crate's Rust tests
(``the_enrollment_pass_uses_the_keyword_parameters``).
"""

from __future__ import annotations

import ast
import inspect
import struct
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

from test_reporting_persistence import installed_stub_method

P = PropertyIdentifier
ENROLLMENT = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, 1)
KEYWORDS = ("event_parameters", "fault_parameters", "time_delay_normal")

# OUT_OF_RANGE (Event_Type 5) above 40 or below 20, and the peer's above 50,
# both after 30 seconds with a deadband of 1; FAULT_OUT_OF_RANGE (Fault_Type
# 6) outside -5 to 50, and the peer's outside -5 to 60. Each in its BACnet
# encoding.
EVENT = bytes.fromhex("5e091e1c41a000002c422000003c3f8000005f")
PEER_EVENT = bytes.fromhex("5e091e1c41a000002c424800003c3f8000005f")
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
        peers = (PEER_EVENT, PEER_FAULT, 9, 5, 6)
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
                (P.EVENT_PARAMETERS, encoded(PEER_EVENT)),
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
        # The application now configures other Event_Parameters: they are
        # served beside the peer's Fault_Parameters, and the file holds the
        # peer's row alone.
        await self.serves((PEER_EVENT, PEER_FAULT, 6, 5, 6), event=PEER_EVENT)
        self.assertEqual(saved_parameters(self.file), FAULT_BIT)

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
        # With storage, an encoding past 64 KiB, as the write with storage.
        long = PropertyValue.octet_string(bytes(65_533))
        self.assert_refused(ErrorCode.NO_SPACE_TO_WRITE_PROPERTY, event_parameters=long)
        server = BACnetServer(2_426_003, interface="127.0.0.1", port=0)
        for delay in (-1, 2**32):
            with self.subTest(delay=delay), self.assertRaises(OverflowError):
                server.add_event_enrollment(1, "EE-1", time_delay_normal=delay)


class LongParametersWithoutStorageTests(unittest.IsolatedAsyncioTestCase):
    async def test_an_enrollment_without_storage_takes_longer_parameters(self) -> None:
        server = BACnetServer(2_426_004, interface="127.0.0.1", port=0)
        server.add_event_enrollment(
            1, "EE-1", event_parameters=PropertyValue.octet_string(bytes(65_533))
        )
        await server.start()
        try:
            read = await server.read_property(ENROLLMENT, P.EVENT_PARAMETERS)
            self.assertEqual(len(read.octets), 65_537)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
