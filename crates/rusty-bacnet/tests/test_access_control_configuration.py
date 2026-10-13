"""Installed-artifact tests for the access-control keyword arguments.

add_access_door(door_members=...), add_access_point(access_doors=...) and
add_credential_data_input(supported_formats=...) set arrays that are
read-only over the network (#1249), and add_access_zone(entry_points=...,
exit_points=...) the zone's lists of Access Points (#1306). add_access_point
also takes the policy count, the supported authorization modes and
Priority_For_Writing, which are read-only over the network too (#1307).
add_access_door also takes the door's starting Alarm_Values, Fault_Values and
Masked_Alarm_Values, which keep Door_Alarm_State to the states they admit
(#1149). A zone's Alarm_Values, written locally, refuses NORMAL as the door's
lists do (#1401), and add_access_zone(alarm_values=...) sets its starting
list with the same checks (#1421). add_access_user(credentials=..., members=...,
member_of=...) sets the user's lists of Access Credentials and Access Users
(#1394).
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

LOCK = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
REMOTE_DEVICE = ObjectIdentifier(ObjectType.DEVICE, 99)
REMOTE_DOOR = ObjectIdentifier(ObjectType.ACCESS_DOOR, 4)
LOBBY_POINT = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
REMOTE_POINT = ObjectIdentifier(ObjectType.ACCESS_POINT, 4)
BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 1)
REMOTE_BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 4)
TEAM_MEMBER = ObjectIdentifier(ObjectType.ACCESS_USER, 2)
REMOTE_TEAM = ObjectIdentifier(ObjectType.ACCESS_USER, 5)


# Each registration method and its keyword-only arguments.
KEYWORDS = (
    (
        "add_access_door",
        ["door_members", "alarm_values", "fault_values", "masked_alarm_values"],
    ),
    (
        "add_access_point",
        [
            "access_doors",
            "number_of_authentication_policies",
            "authentication_policies",
            "supported_authorization_modes",
            "priority_for_writing",
        ],
    ),
    ("add_access_zone", ["entry_points", "exit_points", "alarm_values"]),
    ("add_access_user", ["credentials", "members", "member_of"]),
    ("add_credential_data_input", ["supported_formats"]),
)


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


def make_server() -> BACnetServer:
    return BACnetServer(
        device_instance=503_249,
        device_name="Access Control Configuration Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )


def factor(format_type: int, format_class: int, value: bytes) -> PropertyValue:
    """A BACnetAuthenticationFactor: format type [0], class [1], value [2]."""
    return PropertyValue.application_data(
        bytes([0x09, format_type, 0x19, format_class, 0x28 | len(value)]) + value
    )


class AccessControlStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_keywords(self) -> None:
        for method_name, keywords in KEYWORDS:
            with self.subTest(method=method_name):
                parameters = inspect.signature(
                    getattr(BACnetServer, method_name)
                ).parameters
                self.assertEqual(
                    list(parameters), ["self", "instance", "name", *keywords]
                )
                for keyword in keywords:
                    self.assertIs(
                        parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY
                    )
                    self.assertIsNone(parameters[keyword].default)
                method = installed_stub_method(method_name)
                self.assertEqual(
                    [argument.arg for argument in method.args.args],
                    ["self", "instance", "name"],
                )
                self.assertEqual(
                    [argument.arg for argument in method.args.kwonlyargs], keywords
                )
                for default in method.args.kw_defaults:
                    self.assertIsInstance(default, ast.Constant)
                    self.assertIsNone(default.value)


class AccessControlConfigurationTests(unittest.TestCase):
    def assert_value_out_of_range(self, error: BacnetProtocolError) -> None:
        self.assertEqual(error.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())

    def test_door_members_reach_the_array(self) -> None:
        asyncio.run(self._door_members())

    async def _door_members(self) -> None:
        server = make_server()
        server.add_access_door(
            1, "Main Entry", door_members=[LOCK, (REMOTE_DEVICE, REMOTE_DOOR)]
        )
        server.add_access_door(2, "Side Entry")
        await server.start()
        try:
            door = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
            members = PropertyIdentifier.DOOR_MEMBERS
            self.assertEqual((await server.read_property(door, members, 0)).value, 2)
            # Each element reads in the form it was given (#1310).
            self.assertEqual((await server.read_property(door, members, 1)).value, LOCK)
            self.assertEqual(
                (await server.read_property(door, members, 2)).value,
                (REMOTE_DEVICE, REMOTE_DOOR),
            )
            side = ObjectIdentifier(ObjectType.ACCESS_DOOR, 2)
            self.assertEqual((await server.read_property(side, members, 0)).value, 0)
        finally:
            await server.stop()

    def test_access_doors_reach_the_array_and_name_doors_only(self) -> None:
        asyncio.run(self._access_doors())

    async def _access_doors(self) -> None:
        server = make_server()
        server.add_access_point(1, "Lobby", access_doors=[(REMOTE_DEVICE, REMOTE_DOOR)])
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_access_point(2, "Wrong", access_doors=[LOCK])
        self.assert_value_out_of_range(raised.exception)
        with self.assertRaises(TypeError):
            server.add_access_point(3, "Malformed", access_doors=["door"])
        await server.start()
        try:
            point = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
            doors = PropertyIdentifier.ACCESS_DOORS
            self.assertEqual((await server.read_property(point, doors, 0)).value, 1)
            self.assertEqual(
                (await server.read_property(point, doors, 1)).value,
                (REMOTE_DEVICE, REMOTE_DOOR),
            )
            # The refused registrations left no object behind.
            for instance in (2, 3):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.read_property(
                        ObjectIdentifier(ObjectType.ACCESS_POINT, instance), doors, 0
                    )
                self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw())
        finally:
            await server.stop()

    def test_door_alarm_lists_reach_the_door_and_gate_its_alarm_state(self) -> None:
        asyncio.run(self._door_alarm_lists())

    async def _door_alarm_lists(self) -> None:
        server = make_server()
        # Alarms on DOOR_OPEN_TOO_LONG (2) and FORCED_OPEN (3), a fault on
        # DOOR_FAULT (5), and TAMPER (4) masked.
        server.add_access_door(
            1,
            "Main Entry",
            alarm_values=[2, 3],
            fault_values=[5],
            masked_alarm_values=[4],
        )
        server.add_access_door(2, "Side Entry")
        # A reserved state, and NORMAL (0) in any list.
        for settings in (
            {"alarm_values": [9]},
            {"alarm_values": [0]},
            {"fault_values": [5, 0]},
            {"masked_alarm_values": [0]},
        ):
            with self.assertRaises(BacnetProtocolError) as raised:
                server.add_access_door(3, "Refused", **settings)
            self.assert_value_out_of_range(raised.exception)
        await server.start()
        try:
            door = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
            state = PropertyIdentifier.DOOR_ALARM_STATE
            masked = PropertyIdentifier.MASKED_ALARM_VALUES

            def states(*raw: int) -> PropertyValue:
                return PropertyValue.list([PropertyValue.enumerated(r) for r in raw])

            for property, expected in (
                (PropertyIdentifier.ALARM_VALUES, states(2, 3)),
                (PropertyIdentifier.FAULT_VALUES, states(5)),
                (masked, states(4)),
            ):
                self.assertEqual(await server.read_property(door, property), expected)
            side = ObjectIdentifier(ObjectType.ACCESS_DOOR, 2)
            self.assertEqual(await server.read_property(side, masked), states())

            async def write(property: PropertyIdentifier, value: PropertyValue) -> None:
                await server.write_property_local(door, property, value, source_object=None)

            # Out of service a client simulates an alarm value, but neither
            # a masked state nor one in no list.
            await write(PropertyIdentifier.OUT_OF_SERVICE, PropertyValue.boolean(True))
            for refused in (4, 6):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await write(state, PropertyValue.enumerated(refused))
                self.assert_value_out_of_range(raised.exception)
            await write(state, PropertyValue.enumerated(3))
            self.assertEqual(
                await server.read_property(door, state), PropertyValue.enumerated(3)
            )
            # Masking the state the door is in returns it to NORMAL.
            await write(masked, states(3, 4))
            self.assertEqual(
                await server.read_property(door, state), PropertyValue.enumerated(0)
            )
        finally:
            await server.stop()

    def test_entry_and_exit_points_reach_the_lists_and_name_points_only(self) -> None:
        asyncio.run(self._entry_and_exit_points())

    async def _entry_and_exit_points(self) -> None:
        server = make_server()
        server.add_access_zone(
            1,
            "Building A",
            entry_points=[LOBBY_POINT, (REMOTE_DEVICE, REMOTE_POINT)],
            exit_points=[(REMOTE_DEVICE, REMOTE_POINT)],
        )
        server.add_access_zone(2, "Building B")
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_access_zone(3, "Wrong", exit_points=[REMOTE_DOOR])
        self.assert_value_out_of_range(raised.exception)
        with self.assertRaises(ValueError):
            server.add_access_zone(
                3, "Wrong", entry_points=[(REMOTE_DOOR, LOBBY_POINT)]
            )
        await server.start()
        try:
            zone = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
            entry = PropertyIdentifier.ENTRY_POINTS
            exit_ = PropertyIdentifier.EXIT_POINTS
            # The lists read back in the form the keywords take (#1344).
            self.assertEqual(
                (await server.read_property(zone, entry)).value,
                [LOBBY_POINT, (REMOTE_DEVICE, REMOTE_POINT)],
            )
            self.assertEqual(
                (await server.read_property(zone, exit_)).value,
                [(REMOTE_DEVICE, REMOTE_POINT)],
            )
            bare = ObjectIdentifier(ObjectType.ACCESS_ZONE, 2)
            self.assertEqual((await server.read_property(bare, entry)).value, [])
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(
                    ObjectIdentifier(ObjectType.ACCESS_ZONE, 3), entry
                )
            self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw())
        finally:
            await server.stop()

    def test_user_lists_reach_the_user_and_name_their_types(self) -> None:
        asyncio.run(self._user_lists())

    async def _user_lists(self) -> None:
        server = make_server()
        server.add_access_user(
            1,
            "Jane Doe",
            credentials=[BADGE, (REMOTE_DEVICE, REMOTE_BADGE)],
            members=[TEAM_MEMBER],
            member_of=[(REMOTE_DEVICE, REMOTE_TEAM)],
        )
        server.add_access_user(2, "John Doe")
        # Credentials names Access Credentials, the other two Access Users.
        for settings in (
            {"credentials": [TEAM_MEMBER]},
            {"members": [BADGE]},
            {"member_of": [(REMOTE_DEVICE, LOBBY_POINT)]},
        ):
            with self.subTest(settings=settings):
                with self.assertRaises(BacnetProtocolError) as raised:
                    server.add_access_user(3, "Refused", **settings)
                self.assert_value_out_of_range(raised.exception)
        with self.assertRaises(ValueError):
            server.add_access_user(3, "Refused", credentials=[(REMOTE_DOOR, BADGE)])
        await server.start()
        try:
            user = ObjectIdentifier(ObjectType.ACCESS_USER, 1)
            credentials = PropertyIdentifier.CREDENTIALS
            # The lists read back in the form the keywords take (#1344).
            for property, expected in (
                (credentials, [BADGE, (REMOTE_DEVICE, REMOTE_BADGE)]),
                (PropertyIdentifier.MEMBERS, [TEAM_MEMBER]),
                (PropertyIdentifier.MEMBER_OF, [(REMOTE_DEVICE, REMOTE_TEAM)]),
            ):
                self.assertEqual((await server.read_property(user, property)).value, expected)
            # A BACnetLIST takes no index.
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(user, credentials, 1)
            self.assertEqual(
                raised.exception.error_code, ErrorCode.PROPERTY_IS_NOT_AN_ARRAY.to_raw()
            )
            bare = ObjectIdentifier(ObjectType.ACCESS_USER, 2)
            self.assertEqual((await server.read_property(bare, credentials)).value, [])
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(
                    ObjectIdentifier(ObjectType.ACCESS_USER, 3), credentials
                )
            self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw())
        finally:
            await server.stop()

    def test_zone_alarm_values_refuse_normal(self) -> None:
        asyncio.run(self._zone_alarm_values())

    async def _zone_alarm_values(self) -> None:
        server = make_server()
        server.add_access_zone(1, "Building A")
        await server.start()
        try:
            zone = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
            alarms = PropertyIdentifier.ALARM_VALUES

            def states(*raw: int) -> PropertyValue:
                return PropertyValue.list([PropertyValue.enumerated(r) for r in raw])

            async def write(value: PropertyValue) -> None:
                await server.write_property_local(zone, alarms, value, source_object=None)

            # ABOVE_UPPER_LIMIT (4), DISABLED (5) and NOT_SUPPORTED (6) may
            # alarm, but NORMAL (0) may not (#1401); the refusal names the
            # element and keeps the list.
            await write(states(4, 5, 6))
            for refused, element in ((states(3, 0), 2), (states(0), 1)):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await write(refused)
                self.assert_value_out_of_range(raised.exception)
                self.assertEqual(raised.exception.first_failed_element_number, element)
            self.assertEqual(await server.read_property(zone, alarms), states(4, 5, 6))
        finally:
            await server.stop()

    def test_zone_alarm_values_keyword_sets_the_starting_list(self) -> None:
        asyncio.run(self._zone_alarm_values_keyword())

    async def _zone_alarm_values_keyword(self) -> None:
        server = make_server()
        # ABOVE_UPPER_LIMIT (4), DISABLED (5) and a proprietary 64.
        server.add_access_zone(1, "Building A", alarm_values=[4, 5, 64])
        server.add_access_zone(2, "Building B")
        # NORMAL (#1401), a reserved state and one past 65535, each refused
        # naming its element from 1, and registering nothing.
        for values, element in (([4, 0], 2), ([0], 1), ([7], 1), ([65_536], 1)):
            with self.subTest(values=values):
                with self.assertRaises(BacnetProtocolError) as raised:
                    server.add_access_zone(3, "Refused", alarm_values=values)
                self.assert_value_out_of_range(raised.exception)
                self.assertEqual(raised.exception.first_failed_element_number, element)
        with self.assertRaises(TypeError):
            server.add_access_zone(3, "Refused", alarm_values=["NORMAL"])
        await server.start()
        try:
            alarms = PropertyIdentifier.ALARM_VALUES

            def states(*raw: int) -> PropertyValue:
                return PropertyValue.list([PropertyValue.enumerated(r) for r in raw])

            zone = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
            self.assertEqual(await server.read_property(zone, alarms), states(4, 5, 64))
            # Left out, the list starts empty.
            bare = ObjectIdentifier(ObjectType.ACCESS_ZONE, 2)
            self.assertEqual(await server.read_property(bare, alarms), states())
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(
                    ObjectIdentifier(ObjectType.ACCESS_ZONE, 3), alarms
                )
            self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw())
        finally:
            await server.stop()

    def test_point_settings_reach_the_rows_and_gate_writes(self) -> None:
        asyncio.run(self._point_settings())

    async def _point_settings(self) -> None:
        server = make_server()
        # Three policies; AUTHORIZE (0), DENY_ALL (2) and a proprietary 300;
        # doors commanded at priority 8.
        server.add_access_point(
            1,
            "Lobby",
            number_of_authentication_policies=3,
            supported_authorization_modes=[0, 2, 300],
            priority_for_writing=8,
        )
        server.add_access_point(2, "Side")
        for instance, settings in (
            (3, {"number_of_authentication_policies": 0}),
            (4, {"supported_authorization_modes": [1, 2]}),  # no AUTHORIZE
            (5, {"supported_authorization_modes": [0, 6]}),  # reserved
            (6, {"priority_for_writing": 17}),
        ):
            with self.assertRaises(BacnetProtocolError) as raised:
                server.add_access_point(instance, "Refused", **settings)
            self.assert_value_out_of_range(raised.exception)
        await server.start()
        try:
            lobby = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
            side = ObjectIdentifier(ObjectType.ACCESS_POINT, 2)
            policy = PropertyIdentifier.ACTIVE_AUTHENTICATION_POLICY
            policies = PropertyIdentifier.NUMBER_OF_AUTHENTICATION_POLICIES
            mode = PropertyIdentifier.AUTHORIZATION_MODE
            priority = PropertyIdentifier.PRIORITY_FOR_WRITING

            async def value(point: ObjectIdentifier, property: PropertyIdentifier) -> int:
                return (await server.read_property(point, property)).value

            self.assertEqual(await value(lobby, policies), 3)
            self.assertEqual(await value(lobby, priority), 8)
            self.assertEqual(
                [await value(side, p) for p in (policy, policies, mode, priority)],
                [1, 1, 0, 16],
            )

            async def write(property: PropertyIdentifier, value: PropertyValue) -> None:
                await server.write_property_local(lobby, property, value, source_object=None)

            # A point registered without supported modes takes AUTHORIZE
            # alone, so DENY_ALL (2) is refused there.
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.write_property_local(
                    side, mode, PropertyValue.enumerated(2), source_object=None
                )
            self.assert_value_out_of_range(raised.exception)

            await write(policy, PropertyValue.unsigned(3))
            await write(mode, PropertyValue.enumerated(300))
            self.assertEqual(await value(lobby, policy), 3)
            self.assertEqual(await value(lobby, mode), 300)
            # Past the policy count, and a mode the point didn't declare.
            for property, refused in (
                (policy, PropertyValue.unsigned(4)),
                (mode, PropertyValue.enumerated(1)),
            ):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await write(property, refused)
                self.assert_value_out_of_range(raised.exception)
            # Another datatype.
            with self.assertRaises(BacnetProtocolError) as raised:
                await write(mode, PropertyValue.unsigned(2))
            self.assertEqual(
                raised.exception.error_code, ErrorCode.INVALID_DATA_TYPE.to_raw()
            )
            for property in (policies, priority):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await write(property, PropertyValue.unsigned(2))
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw()
                )
            self.assertEqual(
                [await value(lobby, p) for p in (policy, policies, mode, priority)],
                [3, 3, 300, 8],
            )
        finally:
            await server.stop()

    def test_device_reference_pairs_name_a_device(self) -> None:
        asyncio.run(self._device_reference_pairs())

    async def _device_reference_pairs(self) -> None:
        server = make_server()
        # A pair's device must be a Device (#1285); the refusal registers
        # nothing.
        not_a_device = ObjectIdentifier(ObjectType.ANALOG_VALUE, 99)
        with self.assertRaises(ValueError):
            server.add_access_door(2, "Wrong", door_members=[LOCK, (not_a_device, LOCK)])
        with self.assertRaises(ValueError):
            server.add_access_point(
                2, "Wrong", access_doors=[(not_a_device, REMOTE_DOOR)]
            )
        server.add_access_door(1, "Main Entry", door_members=[(REMOTE_DEVICE, LOCK)])
        await server.start()
        try:
            members = PropertyIdentifier.DOOR_MEMBERS
            door = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
            self.assertEqual((await server.read_property(door, members, 0)).value, 1)
            for refused in (
                ObjectIdentifier(ObjectType.ACCESS_DOOR, 2),
                ObjectIdentifier(ObjectType.ACCESS_POINT, 2),
            ):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.read_property(
                        refused, PropertyIdentifier.OBJECT_NAME
                    )
                self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw())
        finally:
            await server.stop()

    def test_supported_formats_reach_both_arrays_and_gate_simulated_reads(self) -> None:
        asyncio.run(self._supported_formats())

    async def _supported_formats(self) -> None:
        server = make_server()
        # Wiegand 26 (8) in class 0, and vendor 260's CUSTOM (2) format 7 in
        # class 3.
        server.add_credential_data_input(
            1, "Card Reader", supported_formats=[(8, 0), ((2, 260, 7), 3)]
        )
        for formats in (
            [(2, 0)],  # CUSTOM without its vendor members
            [((8, 260, 7), 0)],  # vendor members on a standard format
            [(25, 0)],  # past the closed production
        ):
            with self.assertRaises(BacnetProtocolError) as raised:
                server.add_credential_data_input(2, "Refused", supported_formats=formats)
            self.assert_value_out_of_range(raised.exception)
        # A vendor member past Unsigned16 overflows before the object sees it
        # (#1360), and a triple of another length is a ValueError.
        with self.assertRaises(OverflowError):
            server.add_credential_data_input(2, "Refused", supported_formats=[((2, 65_536, 7), 0)])
        with self.assertRaises(ValueError):
            server.add_credential_data_input(2, "Refused", supported_formats=[((2, 260), 0)])
        await server.start()
        try:
            reader = ObjectIdentifier(ObjectType.CREDENTIAL_DATA_INPUT, 1)
            formats = PropertyIdentifier.SUPPORTED_FORMATS
            classes = PropertyIdentifier.SUPPORTED_FORMAT_CLASSES
            self.assertEqual((await server.read_property(reader, formats, 0)).value, 2)
            self.assertEqual((await server.read_property(reader, formats)).value,
                             [8, (2, 260, 7)])
            self.assertEqual((await server.read_property(reader, formats, 1)).value, 8)
            self.assertEqual(
                (await server.read_property(reader, formats, 2)).value, (2, 260, 7)
            )
            self.assertEqual((await server.read_property(reader, classes)).value, [0, 3])

            # Out of service a simulated read must name a declared format with
            # its class.
            await server.write_property_local(
                reader,
                PropertyIdentifier.OUT_OF_SERVICE,
                PropertyValue.boolean(True),
                source_object=None,
            )
            card = factor(2, 3, b"\xab")
            await server.write_property_local(
                reader, PropertyIdentifier.PRESENT_VALUE, card, source_object=None
            )
            self.assertEqual(
                await server.read_property(reader, PropertyIdentifier.PRESENT_VALUE),
                card,
            )
            for undeclared in (factor(9, 0, b"\x01"), factor(8, 3, b"\x01")):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.write_property_local(
                        reader,
                        PropertyIdentifier.PRESENT_VALUE,
                        undeclared,
                        source_object=None,
                    )
                self.assert_value_out_of_range(raised.exception)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
