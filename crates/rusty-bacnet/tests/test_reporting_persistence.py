"""Reporting settings a peer writes, kept across a restart (#2331).

Every builder of an object with intrinsic reporting takes keyword-only
``storage_path`` and ``clear_written_reporting``: the analog, binary and
multi-state ones, Access Door and Zone, and Event and Alert Enrollment. A write of
Notification_Class, Event_Enable, Notify_Type, Event_Detection_Enable,
Event_Message_Texts_Config or the Event_Algorithm_Inhibit pair is saved in
that file before it is answered, and a server started on the same file
serves it again, until ``clear_written_reporting=True`` drops it. A written
Description is kept the same way (#2364), and so are an analog object's
event algorithm parameters (#2331 PR 2).
"""

from __future__ import annotations

import ast
import inspect
import struct
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BacnetError,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
LOBBY = PropertyValue.character_string("Lobby")
EMPTY = PropertyValue.character_string("")
BUILDERS = (
    "add_analog_input",
    "add_analog_output",
    "add_analog_value",
    "add_binary_input",
    "add_binary_output",
    "add_binary_value",
    "add_multistate_input",
    "add_multistate_output",
    "add_multistate_value",
    "add_access_door",
    "add_access_zone",
    "add_event_enrollment",
    "add_alert_enrollment",
)
# One object of each family.
ANALOG = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
BINARY = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
MULTI_STATE = ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 1)


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


class SignatureTests(unittest.TestCase):
    def test_each_builder_takes_the_storage_keywords(self) -> None:
        for name in BUILDERS:
            parameters = inspect.signature(getattr(BACnetServer, name)).parameters
            stub = [argument.arg for argument in installed_stub_method(name).args.kwonlyargs]
            for keyword, default in (("storage_path", None), ("clear_written_reporting", False)):
                with self.subTest(name=name, keyword=keyword):
                    self.assertEqual(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                    self.assertIs(parameters[keyword].default, default)
                    self.assertIn(keyword, stub)


class WrittenReportingTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, clear: bool = False) -> BACnetServer:
        server = BACnetServer(2_331_001, interface="127.0.0.1", port=0)
        server.add_analog_input(
            1, "AI-1", storage_path=str(self.state / "ai-1"), clear_written_reporting=clear
        )
        server.add_binary_output(
            1, "BO-1", storage_path=str(self.state / "bo-1"), clear_written_reporting=clear
        )
        server.add_multistate_value(
            1, "MSV-1", 3, storage_path=str(self.state / "msv-1"), clear_written_reporting=clear
        )
        await server.start()
        return server

    async def read(self, server: BACnetServer, oid: ObjectIdentifier, prop) -> PropertyValue:
        return await server.read_property(oid, prop)

    async def test_written_settings_survive_a_restart_until_cleared(self) -> None:
        server = await self.start()
        try:
            for oid in (ANALOG, BINARY, MULTI_STATE):
                await server.write_property_local(
                    oid, P.NOTIFICATION_CLASS, PropertyValue.unsigned(9), source_object=None
                )
                await server.write_property_local(
                    oid, P.DESCRIPTION, LOBBY, source_object=None
                )
            await server.write_property_local(
                ANALOG, P.NOTIFY_TYPE, PropertyValue.enumerated(1), source_object=None
            )
        finally:
            await server.stop()

        server = await self.start()
        try:
            for oid in (ANALOG, BINARY, MULTI_STATE):
                with self.subTest(oid=oid):
                    self.assertEqual(
                        await self.read(server, oid, P.NOTIFICATION_CLASS),
                        PropertyValue.unsigned(9),
                    )
                    self.assertEqual(await self.read(server, oid, P.DESCRIPTION), LOBBY)
            self.assertEqual(
                await self.read(server, ANALOG, P.NOTIFY_TYPE), PropertyValue.enumerated(1)
            )
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            for oid in (ANALOG, BINARY, MULTI_STATE):
                with self.subTest(cleared=oid):
                    self.assertEqual(
                        await self.read(server, oid, P.NOTIFICATION_CLASS),
                        PropertyValue.unsigned(0),
                    )
                    self.assertEqual(await self.read(server, oid, P.DESCRIPTION), EMPTY)
        finally:
            await server.stop()
        for name in ("ai-1", "bo-1", "msv-1"):
            self.assertTrue((self.state / name).exists())

    async def test_written_algorithm_parameters_survive_a_restart_until_cleared(self) -> None:
        """An analog object keeps a written limit and time delay (#2331 PR 2);
        the other families' parameters last until the restart, as before."""
        written = (
            (P.HIGH_LIMIT, PropertyValue.real(80.5), PropertyValue.real(100.0)),
            (P.TIME_DELAY, PropertyValue.unsigned(7), PropertyValue.unsigned(0)),
        )
        server = await self.start()
        try:
            for prop, value, default in written:
                self.assertEqual(await self.read(server, ANALOG, prop), default)
                await server.write_property_local(ANALOG, prop, value, source_object=None)
            for oid in (BINARY, MULTI_STATE):
                await server.write_property_local(
                    oid, P.TIME_DELAY, PropertyValue.unsigned(7), source_object=None
                )
        finally:
            await server.stop()
        # Kind 6 of the format: the reporting settings, then the parameters.
        self.assertEqual((self.state / "ai-1").read_bytes()[12], 6)

        server = await self.start()
        try:
            for prop, value, _ in written:
                with self.subTest(prop=prop):
                    self.assertEqual(await self.read(server, ANALOG, prop), value)
            for oid in (BINARY, MULTI_STATE):
                with self.subTest(not_kept=oid):
                    self.assertEqual(
                        await self.read(server, oid, P.TIME_DELAY), PropertyValue.unsigned(0)
                    )
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            for prop, _, default in written:
                with self.subTest(cleared=prop):
                    self.assertEqual(await self.read(server, ANALOG, prop), default)
        finally:
            await server.stop()
        self.assertEqual((self.state / "ai-1").read_bytes()[12], 4)


# One object of each further family: the Access objects and the enrollments.
DOOR = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
ZONE = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
ENROLLMENT = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, 1)
ALERT = ObjectIdentifier(ObjectType.ALERT_ENROLLMENT, 1)


class FurtherTypesTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, clear: bool = False) -> BACnetServer:
        server = BACnetServer(2_331_002, interface="127.0.0.1", port=0)
        kept = lambda name: {  # noqa: E731
            "storage_path": str(self.state / name),
            "clear_written_reporting": clear,
        }
        server.add_access_door(1, "Door-1", **kept("door-1"))
        server.add_access_zone(1, "Zone-1", **kept("zone-1"))
        server.add_event_enrollment(1, "EE-1", **kept("ee-1"))
        server.add_alert_enrollment(1, "AE-1", ANALOG, **kept("ae-1"))
        await server.start()
        return server

    async def test_written_settings_survive_a_restart_until_cleared(self) -> None:
        server = await self.start()
        try:
            for oid in (DOOR, ZONE, ENROLLMENT, ALERT):
                await server.write_property_local(
                    oid, P.NOTIFICATION_CLASS, PropertyValue.unsigned(9), source_object=None
                )
                await server.write_property_local(
                    oid, P.DESCRIPTION, LOBBY, source_object=None
                )
            await server.write_property_local(
                ALERT, P.EVENT_DETECTION_ENABLE, PropertyValue.boolean(False), source_object=None
            )
        finally:
            await server.stop()

        server = await self.start()
        try:
            for oid in (DOOR, ZONE, ENROLLMENT, ALERT):
                with self.subTest(oid=oid):
                    self.assertEqual(
                        await server.read_property(oid, P.NOTIFICATION_CLASS),
                        PropertyValue.unsigned(9),
                    )
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), LOBBY)
            self.assertEqual(
                await server.read_property(ALERT, P.EVENT_DETECTION_ENABLE),
                PropertyValue.boolean(False),
            )
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            for oid in (DOOR, ZONE, ENROLLMENT, ALERT):
                with self.subTest(cleared=oid):
                    self.assertEqual(
                        await server.read_property(oid, P.NOTIFICATION_CLASS),
                        PropertyValue.unsigned(0),
                    )
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), EMPTY)
        finally:
            await server.stop()



def ack_notification_file(object_type: int, instance: int) -> bytes:
    """A written-configuration file (RBNWCF01, kind 4) holding only a
    Notify_Type of ACK_NOTIFICATION (2), in the layout
    ``bacnet_objects::snapshot::write_config`` documents: the tag, the
    object identifier, the kind, then each setting behind a presence octet.
    """
    identifier = struct.pack(">I", (object_type << 22) | instance)
    body = bytes([4, 0, 0, 1]) + struct.pack(">I", 2) + bytes([0, 0, 0, 0])
    return b"RBNWCF01" + identifier + body


def high_limit_file(object_type: int, instance: int, limit: float) -> bytes:
    """A written-configuration file of kind 6 holding only a High_Limit: no
    reporting setting, then the presence mask (bit 2) and the limit as an
    IEEE 754 single.
    """
    identifier = struct.pack(">I", (object_type << 22) | instance)
    body = bytes([6, 0, 0, 0, 0, 0, 0, 0]) + struct.pack(">If", 1 << 2, limit)
    return b"RBNWCF01" + identifier + body


class AlgorithmParameterFileTests(unittest.TestCase):
    def test_a_stored_limit_the_object_refuses_on_the_wire_fails_the_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_331_005, interface="127.0.0.1", port=0)
            analog = ObjectType.ANALOG_INPUT.to_raw()
            refused = Path(directory) / "ai-1"
            refused.write_bytes(high_limit_file(analog, 1, float("nan")))
            with self.assertRaises(BacnetError):
                server.add_analog_input(1, "AI-1", storage_path=str(refused))
            # A binary object keeps no High_Limit: the file isn't one of its.
            binary = Path(directory) / "bi-1"
            binary.write_bytes(high_limit_file(ObjectType.BINARY_INPUT.to_raw(), 1, 80.5))
            with self.assertRaises(BacnetError):
                server.add_binary_input(1, "BI-1", storage_path=str(binary))
            # The same file with a finite limit loads.
            taken = Path(directory) / "ai-2"
            taken.write_bytes(high_limit_file(analog, 2, 80.5))
            server.add_analog_input(2, "AI-2", storage_path=str(taken))


class NotifyTypeFileTests(unittest.TestCase):
    def test_an_alert_enrollment_file_holding_ack_notification_fails_the_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_331_004, interface="127.0.0.1", port=0)
            alert = Path(directory) / "ae-1"
            alert.write_bytes(ack_notification_file(ObjectType.ALERT_ENROLLMENT.to_raw(), 1))
            with self.assertRaises(BacnetError):
                server.add_alert_enrollment(1, "AE-1", ANALOG, storage_path=str(alert))
            # The same settings load on an Event Enrollment, which takes all
            # three Notify_Types.
            enrollment = Path(directory) / "ee-1"
            enrollment.write_bytes(
                ack_notification_file(ObjectType.EVENT_ENROLLMENT.to_raw(), 1)
            )
            server.add_event_enrollment(1, "EE-1", storage_path=str(enrollment))


if __name__ == "__main__":
    unittest.main()
