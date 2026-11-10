"""Reporting settings a peer writes, kept across a restart (#2331).

Every builder of an object with intrinsic reporting takes keyword-only
``storage_path`` and ``clear_written_reporting``: the analog, binary and
multi-state ones, Access Door and Zone, and Event and Alert Enrollment. A write of
Notification_Class, Event_Enable, Notify_Type, Event_Detection_Enable,
Event_Message_Texts_Config or the Event_Algorithm_Inhibit pair is saved in
that file before it is answered, and a server started on the same file
serves it again, until ``clear_written_reporting=True`` drops it. A written
Description is kept the same way (#2364), and so are an analog, binary or
multi-state object's, an Access Door's, an Access Zone's and an Event
Enrollment's event algorithm parameters (#2331).
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
BINARY_VALUE = ObjectIdentifier(ObjectType.BINARY_VALUE, 1)
MULTI_STATE = ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 1)


def states(*raw: int) -> PropertyValue:
    """An Alarm_Values list of a multi-state object."""
    return PropertyValue.list([PropertyValue.unsigned(state) for state in raw])


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

    def build(self, clear: bool = False, number_of_states: int = 3) -> BACnetServer:
        server = BACnetServer(2_331_001, interface="127.0.0.1", port=0)
        server.add_analog_input(
            1, "AI-1", storage_path=str(self.state / "ai-1"), clear_written_reporting=clear
        )
        server.add_binary_output(
            1, "BO-1", storage_path=str(self.state / "bo-1"), clear_written_reporting=clear
        )
        server.add_binary_value(
            1, "BV-1", storage_path=str(self.state / "bv-1"), clear_written_reporting=clear
        )
        server.add_multistate_value(
            1,
            "MSV-1",
            number_of_states,
            storage_path=str(self.state / "msv-1"),
            clear_written_reporting=clear,
        )
        return server

    async def start(self, clear: bool = False, number_of_states: int = 3) -> BACnetServer:
        server = self.build(clear, number_of_states)
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
        # Nothing was written to the Binary Value, so it has no file.
        self.assertFalse((self.state / "bv-1").exists())

    async def test_written_algorithm_parameters_survive_a_restart_until_cleared(self) -> None:
        """Each family keeps its written parameters (#2331): an analog
        object a limit and a time delay, a Binary Output a time delay, a
        Binary Value its Alarm_Value, a Multi-state Value its Alarm_Values.
        Each row is the object, the property, the value written and the
        value the object is built with."""
        written = (
            (ANALOG, P.HIGH_LIMIT, PropertyValue.real(80.5), PropertyValue.real(100.0)),
            (ANALOG, P.TIME_DELAY, PropertyValue.unsigned(7), PropertyValue.unsigned(0)),
            (BINARY, P.TIME_DELAY, PropertyValue.unsigned(7), PropertyValue.unsigned(0)),
            (
                BINARY_VALUE,
                P.ALARM_VALUE,
                PropertyValue.enumerated(0),
                PropertyValue.enumerated(1),
            ),
            (MULTI_STATE, P.ALARM_VALUES, states(3, 2), states()),
            (
                MULTI_STATE,
                P.TIME_DELAY_NORMAL,
                PropertyValue.unsigned(4),
                PropertyValue.unsigned(0),
            ),
        )
        files = ("ai-1", "bo-1", "bv-1", "msv-1")
        server = await self.start()
        try:
            for oid, prop, value, default in written:
                with self.subTest(built=oid, prop=prop):
                    self.assertEqual(await self.read(server, oid, prop), default)
                await server.write_property_local(oid, prop, value, source_object=None)
        finally:
            await server.stop()
        # Kind 6 of the format: the reporting settings, then the parameters.
        for name in files:
            with self.subTest(kind_of=name):
                self.assertEqual((self.state / name).read_bytes()[12], 6)

        server = await self.start()
        try:
            for oid, prop, value, _ in written:
                with self.subTest(oid=oid, prop=prop):
                    self.assertEqual(await self.read(server, oid, prop), value)
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            for oid, prop, _, default in written:
                with self.subTest(cleared=oid, prop=prop):
                    self.assertEqual(await self.read(server, oid, prop), default)
        finally:
            await server.stop()
        for name in files:
            with self.subTest(cleared_kind_of=name):
                self.assertEqual((self.state / name).read_bytes()[12], 4)

    async def test_saved_alarm_values_past_the_state_count_are_restored(self) -> None:
        """A saved Alarm_Values list naming a state the object is built
        without doesn't stop the builder: the state count isn't kept, so a
        peer may have added the state before the restart. The list is read
        back as written, a write of it is refused, and the Multi-state Value
        reports CONFIGURATION_ERROR (10) until a list within its states is
        written. The clear drops it like any other saved value."""
        fault = PropertyValue.enumerated(10)
        sound = PropertyValue.enumerated(0)
        server = await self.start()
        try:
            await server.write_property_local(
                MULTI_STATE, P.ALARM_VALUES, states(3, 2), source_object=None
            )
            self.assertEqual(await self.read(server, MULTI_STATE, P.RELIABILITY), sound)
        finally:
            await server.stop()

        server = await self.start(number_of_states=2)
        try:
            self.assertEqual(
                await self.read(server, MULTI_STATE, P.NUMBER_OF_STATES),
                PropertyValue.unsigned(2),
            )
            self.assertEqual(await self.read(server, MULTI_STATE, P.ALARM_VALUES), states(3, 2))
            self.assertEqual(await self.read(server, MULTI_STATE, P.RELIABILITY), fault)
            with self.assertRaises(BacnetError):
                await server.write_property_local(
                    MULTI_STATE, P.ALARM_VALUES, states(3, 2), source_object=None
                )
            # Another kept write saves the stored list with it, unchanged.
            await server.write_property_local(
                MULTI_STATE, P.TIME_DELAY, PropertyValue.unsigned(7), source_object=None
            )
        finally:
            await server.stop()

        server = await self.start(number_of_states=2)
        try:
            self.assertEqual(await self.read(server, MULTI_STATE, P.ALARM_VALUES), states(3, 2))
            self.assertEqual(
                await self.read(server, MULTI_STATE, P.TIME_DELAY), PropertyValue.unsigned(7)
            )
            await server.write_property_local(
                MULTI_STATE, P.ALARM_VALUES, states(2), source_object=None
            )
            self.assertEqual(await self.read(server, MULTI_STATE, P.RELIABILITY), sound)
            await server.write_property_local(
                MULTI_STATE, P.ALARM_VALUES, states(1, 2), source_object=None
            )
        finally:
            await server.stop()

        # A stranded list written again from three states, then cleared.
        server = await self.start()
        try:
            await server.write_property_local(
                MULTI_STATE, P.ALARM_VALUES, states(3), source_object=None
            )
        finally:
            await server.stop()
        server = await self.start(clear=True, number_of_states=2)
        try:
            self.assertEqual(await self.read(server, MULTI_STATE, P.ALARM_VALUES), states())
            self.assertEqual(await self.read(server, MULTI_STATE, P.RELIABILITY), sound)
        finally:
            await server.stop()

    async def test_an_applications_own_parameter_writes_are_kept_like_a_peers(self) -> None:
        """The builders' parameter keywords are never saved (#2410,
        test_algorithm_parameter_keywords.py), but a ``write_property_local``
        after the builder call is kept like a peer's write: it is served
        after a restart over the keyword, until a clear."""
        own = PropertyValue.real(30.0)
        written = PropertyValue.real(50.0)

        async def run(*, clear: bool = False, write=None) -> PropertyValue:
            server = BACnetServer(2_331_010, interface="127.0.0.1", port=0)
            server.add_analog_input(
                1, "AI-1", high_limit=30.0, storage_path=str(self.state / "ai-1"),
                clear_written_reporting=clear,
            )
            await server.start()
            try:
                if write is not None:
                    await server.write_property_local(
                        ANALOG, P.HIGH_LIMIT, write, source_object=None
                    )
                return await self.read(server, ANALOG, P.HIGH_LIMIT)
            finally:
                await server.stop()

        self.assertEqual(await run(), own)
        self.assertEqual(await run(write=written), written)
        self.assertEqual(await run(), written)
        self.assertEqual(await run(clear=True), own)
        self.assertEqual(await run(), own)


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



def listed(*raw: int) -> PropertyValue:
    """A list of an Access Door's or Access Zone's states."""
    return PropertyValue.list([PropertyValue.enumerated(state) for state in raw])


class AccessParameterTests(unittest.IsolatedAsyncioTestCase):
    """An Access Door's and an Access Zone's written lists and delays."""

    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, clear: bool = False) -> BACnetServer:
        server = BACnetServer(2_331_006, interface="127.0.0.1", port=0)
        # The application's own lists, given at every start: FORCED_OPEN
        # and DOOR_FAULT on the door, ABOVE_UPPER_LIMIT on the zone.
        server.add_access_door(
            1,
            "Door-1",
            alarm_values=[3],
            fault_values=[5],
            storage_path=str(self.state / "door-1"),
            clear_written_reporting=clear,
        )
        server.add_access_zone(
            1,
            "Zone-1",
            alarm_values=[4],
            storage_path=str(self.state / "zone-1"),
            clear_written_reporting=clear,
        )
        await server.start()
        return server

    async def test_written_lists_and_delays_go_over_the_keywords_until_cleared(self) -> None:
        written = [
            (DOOR, P.ALARM_VALUES, listed(4, 300)),
            (DOOR, P.FAULT_VALUES, listed(6)),
            (DOOR, P.MASKED_ALARM_VALUES, listed(7)),
            (DOOR, P.TIME_DELAY, PropertyValue.unsigned(7)),
            (DOOR, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(3)),
            (ZONE, P.ALARM_VALUES, listed(3, 100)),
            (ZONE, P.TIME_DELAY, PropertyValue.unsigned(7)),
            (ZONE, P.TIME_DELAY_NORMAL, PropertyValue.unsigned(3)),
        ]
        configured = {
            (DOOR, P.ALARM_VALUES): listed(3),
            (DOOR, P.FAULT_VALUES): listed(5),
            (DOOR, P.MASKED_ALARM_VALUES): listed(),
            (ZONE, P.ALARM_VALUES): listed(4),
        }
        server = await self.start()
        try:
            for (oid, prop), value in configured.items():
                self.assertEqual(await server.read_property(oid, prop), value)
            for oid, prop, value in written:
                await server.write_property_local(oid, prop, value, source_object=None)
            # A list a write refuses is refused with storage too.
            for refused in (listed(0), listed(9)):
                with self.assertRaises(BacnetError):
                    await server.write_property_local(
                        DOOR, P.FAULT_VALUES, refused, source_object=None
                    )
        finally:
            await server.stop()

        # The keywords are given again, and the written lists are served.
        server = await self.start()
        try:
            for oid, prop, value in written:
                with self.subTest(oid=oid, prop=prop):
                    self.assertEqual(await server.read_property(oid, prop), value)
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            for oid, prop, _ in written:
                expected = configured.get((oid, prop), PropertyValue.unsigned(0))
                with self.subTest(cleared=oid, prop=prop):
                    self.assertEqual(await server.read_property(oid, prop), expected)
        finally:
            await server.stop()


# OUT_OF_RANGE above 40 or below 20 after 30 seconds, and
# FAULT_OUT_OF_RANGE outside -5 to 50, each in its BACnet encoding.
OUT_OF_RANGE = bytes.fromhex("5e091e1c41a000002c422000003c3f8000005f")
RANGE_FAULT = bytes.fromhex("6e0e5508c0140000000000000f1e550840490000000000001f6f")


def enrollment_parameters_file(instance: int, bit: int, row: bytes) -> bytes:
    """A written-configuration file of kind 6 for an Event Enrollment
    holding one parameter in its BACnet encoding: no reporting setting, then
    the presence mask (bit 10 for Event_Parameters, 11 for
    Fault_Parameters), the row's length and its octets.
    """
    enrollment = ObjectType.EVENT_ENROLLMENT.to_raw()
    identifier = struct.pack(">I", (enrollment << 22) | instance)
    body = bytes([6, 0, 0, 0, 0, 0, 0, 0]) + struct.pack(">II", 1 << bit, len(row))
    return b"RBNWCF01" + identifier + body + row


class EnrollmentParameterTests(unittest.IsolatedAsyncioTestCase):
    """An Event Enrollment's written Event_Parameters, Fault_Parameters and
    Time_Delay_Normal."""

    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    async def start(self, clear: bool = False, storage: bool = True) -> BACnetServer:
        server = BACnetServer(2_331_008, interface="127.0.0.1", port=0)
        if storage:
            server.add_event_enrollment(
                1,
                "EE-1",
                storage_path=str(self.state / "ee-1"),
                clear_written_reporting=clear,
            )
        else:
            server.add_event_enrollment(1, "EE-1")
        await server.start()
        return server

    async def served(self, server: BACnetServer) -> tuple:
        """The three rows and the two types that follow them."""
        event = await server.read_property(ENROLLMENT, P.EVENT_PARAMETERS)
        fault = await server.read_property(ENROLLMENT, P.FAULT_PARAMETERS)
        delay = await server.read_property(ENROLLMENT, P.TIME_DELAY_NORMAL)
        event_type = await server.read_property(ENROLLMENT, P.EVENT_TYPE)
        fault_type = await server.read_property(ENROLLMENT, P.FAULT_TYPE)
        return event.octets, fault.octets, delay.value, event_type.value, fault_type.value

    async def test_written_parameters_survive_a_restart_until_cleared(self) -> None:
        # Unset: the none choice of each, no delay, and the types NONE.
        unset = (bytes.fromhex("f814"), bytes.fromhex("08"), 0, 20, 0)
        # Written: the delay of its own, and OUT_OF_RANGE (5) and
        # FAULT_OUT_OF_RANGE (6) as the types.
        written = (OUT_OF_RANGE, RANGE_FAULT, 9, 5, 6)
        server = await self.start()
        try:
            self.assertEqual(await self.served(server), unset)
            for prop, value in (
                (P.EVENT_PARAMETERS, PropertyValue.application_data(OUT_OF_RANGE)),
                (P.FAULT_PARAMETERS, PropertyValue.application_data(RANGE_FAULT)),
                (P.TIME_DELAY_NORMAL, PropertyValue.unsigned(9)),
            ):
                await server.write_property_local(ENROLLMENT, prop, value, source_object=None)
            self.assertEqual(await self.served(server), written)
            # Parameters whose encoding passes 64 KiB aren't kept, so with
            # storage they are refused, and the written ones stay.
            long = PropertyValue.octet_string(bytes(65_533))
            with self.assertRaises(BacnetError):
                await server.write_property_local(
                    ENROLLMENT, P.EVENT_PARAMETERS, long, source_object=None
                )
            self.assertEqual(await self.served(server), written)
        finally:
            await server.stop()

        server = await self.start()
        try:
            self.assertEqual(await self.served(server), written)
        finally:
            await server.stop()

        server = await self.start(clear=True)
        try:
            self.assertEqual(await self.served(server), unset)
        finally:
            await server.stop()

    async def test_an_enrollment_without_storage_takes_longer_parameters(self) -> None:
        server = await self.start(storage=False)
        try:
            long = PropertyValue.octet_string(bytes(65_533))
            await server.write_property_local(
                ENROLLMENT, P.EVENT_PARAMETERS, long, source_object=None
            )
            read = await server.read_property(ENROLLMENT, P.EVENT_PARAMETERS)
            self.assertEqual(len(read.octets), 65_537)
        finally:
            await server.stop()


class EnrollmentParameterFileTests(unittest.TestCase):
    def test_a_stored_row_that_is_not_one_whole_value_fails_the_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_331_009, interface="127.0.0.1", port=0)

            def path(name: str, octets: bytes) -> str:
                file = Path(directory) / name
                file.write_bytes(octets)
                return str(file)

            refused = (
                (10, b""),
                (10, OUT_OF_RANGE[:-1]),
                (10, OUT_OF_RANGE + b"\x00"),
                (10, bytes.fromhex("6e6f")),
                (11, RANGE_FAULT[:-1]),
                (11, RANGE_FAULT + b"\x08"),
                (11, bytes.fromhex("8e8f")),
            )
            for bit, row in refused:
                file = path("ee-1", enrollment_parameters_file(1, bit, row))
                with self.subTest(bit=bit, row=row.hex()), self.assertRaises(BacnetError):
                    server.add_event_enrollment(1, "EE-1", storage_path=file)
            # A length past the cap is refused though the file ends there.
            cut = enrollment_parameters_file(1, 10, b"")[:-4] + struct.pack(">I", 65_537)
            with self.assertRaises(BacnetError):
                server.add_event_enrollment(1, "EE-1", storage_path=path("ee-1", cut))
            # Whole values load.
            for instance, (bit, row) in enumerate(((10, OUT_OF_RANGE), (11, RANGE_FAULT)), 2):
                file = path(f"ee-{instance}", enrollment_parameters_file(instance, bit, row))
                server.add_event_enrollment(instance, f"EE-{instance}", storage_path=file)


def alarm_values_file(object_type: int, instance: int, *states: int) -> bytes:
    """A written-configuration file of kind 6 holding only Alarm_Values: no
    reporting setting, then the presence mask (bit 7), the count and each
    state in four octets.
    """
    identifier = struct.pack(">I", (object_type << 22) | instance)
    body = bytes([6, 0, 0, 0, 0, 0, 0, 0]) + struct.pack(">II", 1 << 7, len(states))
    return b"RBNWCF01" + identifier + body + b"".join(struct.pack(">I", s) for s in states)


class AccessListFileTests(unittest.TestCase):
    def test_a_stored_list_is_held_to_its_own_types_states(self) -> None:
        door = ObjectType.ACCESS_DOOR.to_raw()
        zone = ObjectType.ACCESS_ZONE.to_raw()
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(2_331_007, interface="127.0.0.1", port=0)

            def path(name: str, octets: bytes) -> str:
                file = Path(directory) / name
                file.write_bytes(octets)
                return str(file)

            # NORMAL, and a number past the named states and short of the
            # proprietary ones, on each type: 8 is a door's EGRESS_OPEN and
            # no occupancy state.
            for states in ((0,), (3, 0), (9,), (65_536,)):
                with self.subTest(door=states), self.assertRaises(BacnetError):
                    server.add_access_door(
                        1, "Door-1", storage_path=path("door", alarm_values_file(door, 1, *states))
                    )
            for states in ((0,), (8,), (63,), (65_536,)):
                with self.subTest(zone=states), self.assertRaises(BacnetError):
                    server.add_access_zone(
                        1, "Zone-1", storage_path=path("zone", alarm_values_file(zone, 1, *states))
                    )
            # The clear doesn't help: the file is read before it is cleared.
            with self.assertRaises(BacnetError):
                server.add_access_door(
                    1,
                    "Door-1",
                    storage_path=path("door", alarm_values_file(door, 1, 0)),
                    clear_written_reporting=True,
                )
            # What each takes loads.
            server.add_access_door(
                2, "Door-2", storage_path=path("door-2", alarm_values_file(door, 2, 8, 256))
            )
            server.add_access_zone(
                2, "Zone-2", storage_path=path("zone-2", alarm_values_file(zone, 2, 6, 64))
            )


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
            # The clear doesn't help: the file is read before it is cleared.
            with self.assertRaises(BacnetError):
                server.add_analog_input(
                    1, "AI-1", storage_path=str(refused), clear_written_reporting=True
                )
            # A binary object keeps no High_Limit: the file isn't one it can
            # have written, though it keeps other parameters now.
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
