"""Typed reads of the access-control collections (#1344) and of the other
constructed properties (#1345).

Access Rights rules and Accompaniment, Access Zone Entry_Points and
Exit_Points, and Access User Credentials, Members and Member_Of read back
through BACnetClient, its ReadPropertyMultiple, the endpoint client role and
its ReadPropertyMultiple, and the local BACnetServer.read_property in the
form their add_* keywords take, and the Device's Audit_Notification_Recipient
in the form configure_audit_recipient takes. A Schedule's Weekly_Schedule,
Exception_Schedule and Effective_Period, a Calendar's Date_List and
Event_Time_Stamps read as typed values too, and an Accumulator's Scale and
Prescale as the values add_accumulator takes (#1487), and an Access Point's
Authentication_Policy_List as the policies add_access_point takes (#1325).
Each typed read writes back as the octets it was read from.
"""

from __future__ import annotations

import inspect
import unittest
from typing import Any

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BACnetTimeStamp,
    BacnetProtocolError,
    BipEndpoint,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
DEVICE = 9344
DEVICE_ID = ObjectIdentifier(ObjectType.DEVICE, DEVICE)
REMOTE_DEVICE = ObjectIdentifier(ObjectType.DEVICE, 99)
RIGHTS_1 = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
ZONE_1 = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
USER_1 = ObjectIdentifier(ObjectType.ACCESS_USER, 1)
SCHED_1 = ObjectIdentifier(ObjectType.SCHEDULE, 1)
CAL_1 = ObjectIdentifier(ObjectType.CALENDAR, 1)
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
ACC_1 = ObjectIdentifier(ObjectType.ACCUMULATOR, 1)
ACC_2 = ObjectIdentifier(ObjectType.ACCUMULATOR, 2)
ACC_3 = ObjectIdentifier(ObjectType.ACCUMULATOR, 3)
LOBBY = ObjectIdentifier(ObjectType.ACCESS_POINT, 2)
REMOTE_POINT = ObjectIdentifier(ObjectType.ACCESS_POINT, 4)
REMOTE_ZONE = ObjectIdentifier(ObjectType.ACCESS_ZONE, 3)
BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 1)
REMOTE_BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 4)
TEAM_MEMBER = ObjectIdentifier(ObjectType.ACCESS_USER, 2)
REMOTE_TEAM = ObjectIdentifier(ObjectType.ACCESS_USER, 5)
POINT_1 = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
CARD_READER = ObjectIdentifier(ObjectType.CREDENTIAL_DATA_INPUT, 1)
REMOTE_KEYPAD = ObjectIdentifier(ObjectType.CREDENTIAL_DATA_INPUT, 2)
# A card alone, then a card and a remote keypad's PIN, in order, in 30 s.
POLICIES: Any = [([(CARD_READER, 1)], False, 0),
                 ([(CARD_READER, 1), ((REMOTE_DEVICE, REMOTE_KEYPAD), 2)], True, 30)]
POLICY_NAMES: Any = ["card", "card and PIN"]

# Rules with every key, as a read gives them back.
BUSINESS_HOURS: Any = {
    "enable": True,
    "time_range": {
        "object_identifier": SCHED_1,
        "property_identifier": P.PRESENT_VALUE,
        "property_array_index": None,
        "device_identifier": None,
    },
    "location": LOBBY,
}
ANYWHERE_OFF: Any = {"enable": False, "time_range": None, "location": None}
REMOTE_LOCKDOWN: Any = {"enable": True, "time_range": None,
                        "location": (REMOTE_DEVICE, REMOTE_ZONE)}
ACCOMPANIMENT = (REMOTE_DEVICE, TEAM_MEMBER)
ENTRY_POINTS: Any = [LOBBY, (REMOTE_DEVICE, REMOTE_POINT)]
EXIT_POINTS: Any = [(REMOTE_DEVICE, REMOTE_POINT)]
CREDENTIALS: Any = [BADGE, (REMOTE_DEVICE, REMOTE_BADGE)]
MEMBERS: Any = [TEAM_MEMBER]
MEMBER_OF: Any = [(REMOTE_DEVICE, REMOTE_TEAM)]
AUDIT_RECIPIENT: Any = {"kind": "device",
                        "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 9)}

# Monday 08:00 REAL 21.0, then 17:30 NULL; the other days are empty.
MONDAY = bytes.fromhex("0e" "b408000000" "4441a80000" "b4111e0000" "00" "0f")
EMPTY_DAY = bytes.fromhex("0e0f")
WEEKLY: Any = [[((8, 0, 0, 0), PropertyValue.real(21.0)),
                ((17, 30, 0, 0), PropertyValue.null())]] + [[]] * 6
# Calendar entries: 2026-12-25 (a Friday), 2026-01-01 onwards, and any day of
# November's fourth week.
DATE = bytes.fromhex("0c7e0c1905")
FROM_NEW_YEAR = bytes.fromhex("1e" "a47e010104" "a4ffffffff" "1f")
NOVEMBER_WEEK_4 = bytes.fromhex("2b0b04ff")
CHRISTMAS: Any = {"kind": "date", "date": (2026, 12, 25, 5)}
DATE_LIST: Any = [
    CHRISTMAS,
    {"kind": "date_range", "start_date": (2026, 1, 1, 4), "end_date": (255, 255, 255, 255)},
    {"kind": "week_n_day", "month": 11, "week_of_month": 4, "day_of_week": 255},
]
# Exception events: Christmas at priority 3, and Calendar 1 at priority 16.
CHRISTMAS_EVENT = bytes.fromhex("0e" "0c7e0c1905" "0f" "2e" "b408000000" "4441a80000" "2f" "3903")
CALENDAR_EVENT = bytes.fromhex("1c01800001" "2e2f" "3910")
EXCEPTIONS: Any = [
    {"period": CHRISTMAS, "time_values": [((8, 0, 0, 0), PropertyValue.real(21.0))],
     "priority": 3},
    {"period": CAL_1, "time_values": [], "priority": 16},
]
EFFECTIVE_PERIOD = bytes.fromhex("a47e010104" "a4ffffffff")


def make_server(instance: int = DEVICE) -> BACnetServer:
    server = BACnetServer(instance, interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    server.add_access_rights(1, "Employee Access",
                             positive_access_rules=[BUSINESS_HOURS, ANYWHERE_OFF],
                             negative_access_rules=[REMOTE_LOCKDOWN],
                             accompaniment=ACCOMPANIMENT)
    server.add_access_zone(1, "Building A", entry_points=ENTRY_POINTS,
                           exit_points=EXIT_POINTS)
    server.add_access_user(1, "Jane Doe", credentials=CREDENTIALS, members=MEMBERS,
                           member_of=MEMBER_OF)
    server.add_access_point(1, "Lobby",
                            authentication_policies=list(zip(POLICY_NAMES, POLICIES)))
    server.configure_audit_recipient(AUDIT_RECIPIENT)
    server.add_audit_reporter(1, "Reporter")
    server.configure_audit_reporters([{"instance": 1, "audit_level": "none",
                                       "auditable_operations": 0,
                                       "issue_confirmed_notifications": False}])
    server.add_schedule(1, "SCHED-1")
    server.add_calendar(1, "CAL-1")
    server.add_analog_input(1, "AI-1")
    server.add_accumulator(1, "kWh", 70, scale=2.5, prescale=(5, 100))
    server.add_accumulator(2, "Pulses", scale=-2)
    server.add_accumulator(3, "Plain")
    return server


def rpm_values(results: list[dict[str, Any]]) -> list[Any]:
    """The values of a one-object ReadPropertyMultiple result, in order."""
    [only] = results
    for row in only["results"]:
        assert row["error"] is None, row
    return [row["value"] for row in only["results"]]


class TypedConstructedReadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = make_server()
        await self.server.start()

        async def stop_server() -> None:
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        application = PropertyValue.application_data
        for oid, prop, value in (
            (SCHED_1, P.WEEKLY_SCHEDULE,
             PropertyValue.list([application(MONDAY)] + [application(EMPTY_DAY)] * 6)),
            (SCHED_1, P.EXCEPTION_SCHEDULE,
             PropertyValue.list([application(CHRISTMAS_EVENT), application(CALENDAR_EVENT)])),
            (SCHED_1, P.EFFECTIVE_PERIOD, application(EFFECTIVE_PERIOD)),
            (CAL_1, P.DATE_LIST, PropertyValue.list(
                [application(DATE), application(FROM_NEW_YEAR), application(NOVEMBER_WEEK_4)])),
        ):
            await self.server.write_property_local(oid, prop, value, source_object=None)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)
        reader = BipEndpoint(device_instance=DEVICE + 1, interface="127.0.0.1",
                             broadcast_address="127.0.0.1", port=0)
        await reader.start()

        async def close_reader() -> None:
            await reader.close()

        self.addAsyncCleanup(close_reader)
        self.role = await reader.client()

    async def read(self, oid: ObjectIdentifier, prop: PropertyIdentifier,
                   index: int | None = None) -> PropertyValue:
        """Read with BACnetClient, check that its ReadPropertyMultiple, the
        endpoint client role (both services) and the local read agree, and
        return the value."""
        value = await self.client.read_property(self.address, oid, prop, index)
        spec: Any = [(oid, [(prop, index)])]
        self.assertEqual(rpm_values(await self.client.read_property_multiple(self.address, spec)),
                         [value])
        self.assertEqual(await self.role.read_property(self.address, oid, prop, index), value)
        self.assertEqual(rpm_values(await self.role.read_property_multiple(self.address, spec)),
                         [value])
        self.assertEqual(await self.server.read_property(oid, prop, index), value)
        return value

    async def assert_list(self, oid: ObjectIdentifier, prop: PropertyIdentifier,
                          expected: list[Any], element: str, indexed: bool = True) -> None:
        """A whole read is a list of `expected`; for an array, each index is
        one element tagged `element`."""
        value = await self.read(oid, prop)
        self.assertEqual(value.tag, "list")
        self.assertEqual(value.value, expected)
        if not indexed:
            return
        elements = []
        for index, item in enumerate(expected, 1):
            with self.subTest(prop=prop, index=index):
                one = await self.read(oid, prop, index)
                self.assertEqual(one.tag, element)
                self.assertEqual(one.value, item)
                elements.append(one)
        self.assertEqual(PropertyValue.list(elements), value)

    async def assert_single(self, oid: ObjectIdentifier, prop: PropertyIdentifier,
                            expected: Any, element: str) -> PropertyValue:
        value = await self.read(oid, prop)
        self.assertEqual(value.tag, element)
        self.assertEqual(value.value, expected)
        return value

    async def test_access_rules_and_accompaniment_read_as_configured(self) -> None:
        await self.assert_list(RIGHTS_1, P.POSITIVE_ACCESS_RULES,
                               [BUSINESS_HOURS, ANYWHERE_OFF], "access_rule")
        await self.assert_list(RIGHTS_1, P.NEGATIVE_ACCESS_RULES, [REMOTE_LOCKDOWN],
                               "access_rule")
        await self.assert_single(RIGHTS_1, P.ACCOMPANIMENT, ACCOMPANIMENT,
                                 "device_object_reference")

    async def test_zone_and_user_lists_read_as_configured(self) -> None:
        # BACnetLISTs take no index.
        for oid, prop, expected in (
            (ZONE_1, P.ENTRY_POINTS, ENTRY_POINTS),
            (ZONE_1, P.EXIT_POINTS, EXIT_POINTS),
            (USER_1, P.CREDENTIALS, CREDENTIALS),
            (USER_1, P.MEMBERS, MEMBERS),
            (USER_1, P.MEMBER_OF, MEMBER_OF),
        ):
            with self.subTest(prop=prop):
                await self.assert_list(oid, prop, expected, "device_object_reference",
                                       indexed=False)

    async def test_authentication_policies_read_as_configured(self) -> None:
        await self.assert_list(POINT_1, P.AUTHENTICATION_POLICY_LIST, POLICIES,
                               "authentication_policy")

    async def test_the_audit_recipient_reads_as_configured(self) -> None:
        await self.assert_single(DEVICE_ID, P.AUDIT_NOTIFICATION_RECIPIENT, AUDIT_RECIPIENT,
                                 "recipient")

    async def test_schedule_and_calendar_properties_read_typed(self) -> None:
        await self.assert_list(SCHED_1, P.WEEKLY_SCHEDULE, WEEKLY, "daily_schedule")
        await self.assert_list(SCHED_1, P.EXCEPTION_SCHEDULE, EXCEPTIONS, "special_event")
        await self.assert_single(SCHED_1, P.EFFECTIVE_PERIOD,
                                 ((2026, 1, 1, 4), (255, 255, 255, 255)), "date_range")
        await self.assert_list(CAL_1, P.DATE_LIST, DATE_LIST, "calendar_entry", indexed=False)

    async def test_event_time_stamps_read_as_timestamps(self) -> None:
        stamps = await self.read(AI_1, P.EVENT_TIME_STAMPS)
        self.assertEqual(stamps.tag, "list")
        self.assertEqual(len(stamps.value), 3)
        for stamp in stamps.value:
            self.assertIsInstance(stamp, BACnetTimeStamp)
        one = await self.read(AI_1, P.EVENT_TIME_STAMPS, 2)
        self.assertEqual(one.tag, "timestamp")
        self.assertEqual(one.value, stamps.value[1])

    async def test_accumulator_scale_and_prescale_read_as_configured(self) -> None:
        scale = await self.assert_single(ACC_1, P.SCALE, 2.5, "scale")
        self.assertIsInstance(scale.value, float)
        await self.assert_single(ACC_1, P.PRESCALE, (5, 100), "prescale")
        power = await self.assert_single(ACC_2, P.SCALE, -2, "scale")
        self.assertIsInstance(power.value, int)
        # The default is a float scale of 1.0, and no Prescale is served.
        await self.assert_single(ACC_3, P.SCALE, 1.0, "scale")
        with self.assertRaises(BacnetProtocolError) as raised:
            await self.server.read_property(ACC_3, P.PRESCALE)
        self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw())

    async def test_typed_reads_write_back_unchanged(self) -> None:
        for oid, prop in (
            (RIGHTS_1, P.POSITIVE_ACCESS_RULES),
            (RIGHTS_1, P.ACCOMPANIMENT),
            (SCHED_1, P.WEEKLY_SCHEDULE),
            (SCHED_1, P.EXCEPTION_SCHEDULE),
            (SCHED_1, P.EFFECTIVE_PERIOD),
            (CAL_1, P.DATE_LIST),
        ):
            with self.subTest(prop=prop):
                value = await self.server.read_property(oid, prop)
                await self.client.write_property(self.address, oid, prop, value)
                self.assertEqual(await self.server.read_property(oid, prop), value)

    async def test_each_read_is_accepted_by_its_typed_write(self) -> None:
        async def value(oid: ObjectIdentifier, prop: PropertyIdentifier) -> Any:
            return (await self.server.read_property(oid, prop)).value

        copy = BACnetServer(DEVICE + 2, interface="127.0.0.1", port=0)
        copy.add_access_rights(
            1, "Employee Access",
            positive_access_rules=await value(RIGHTS_1, P.POSITIVE_ACCESS_RULES),
            negative_access_rules=await value(RIGHTS_1, P.NEGATIVE_ACCESS_RULES),
            accompaniment=await value(RIGHTS_1, P.ACCOMPANIMENT),
        )
        copy.add_access_zone(1, "Building A", entry_points=await value(ZONE_1, P.ENTRY_POINTS),
                             exit_points=await value(ZONE_1, P.EXIT_POINTS))
        copy.add_access_user(1, "Jane Doe", credentials=await value(USER_1, P.CREDENTIALS),
                             members=await value(USER_1, P.MEMBERS),
                             member_of=await value(USER_1, P.MEMBER_OF))
        copy.add_access_point(1, "Lobby", authentication_policies=list(zip(
            await value(POINT_1, P.AUTHENTICATION_POLICY_NAMES),
            await value(POINT_1, P.AUTHENTICATION_POLICY_LIST))))
        copy.configure_audit_recipient(await value(DEVICE_ID, P.AUDIT_NOTIFICATION_RECIPIENT))
        copy.add_audit_reporter(1, "Reporter")
        copy.configure_audit_reporters([{"instance": 1, "audit_level": "none",
                                         "auditable_operations": 0,
                                         "issue_confirmed_notifications": False}])
        copy.add_accumulator(1, "kWh", 70, scale=await value(ACC_1, P.SCALE),
                             prescale=await value(ACC_1, P.PRESCALE))
        copy.add_accumulator(2, "Pulses", scale=await value(ACC_2, P.SCALE))
        await copy.start()
        try:
            # The copy serves what the original does, octet for octet.
            copy_device = ObjectIdentifier(ObjectType.DEVICE, DEVICE + 2)
            for oid, prop in (
                (RIGHTS_1, P.POSITIVE_ACCESS_RULES),
                (RIGHTS_1, P.NEGATIVE_ACCESS_RULES),
                (RIGHTS_1, P.ACCOMPANIMENT),
                (ZONE_1, P.ENTRY_POINTS),
                (ZONE_1, P.EXIT_POINTS),
                (USER_1, P.CREDENTIALS),
                (USER_1, P.MEMBERS),
                (USER_1, P.MEMBER_OF),
                (POINT_1, P.AUTHENTICATION_POLICY_LIST),
                (POINT_1, P.AUTHENTICATION_POLICY_NAMES),
                (DEVICE_ID, P.AUDIT_NOTIFICATION_RECIPIENT),
                (ACC_1, P.SCALE),
                (ACC_1, P.PRESCALE),
                (ACC_2, P.SCALE),
            ):
                with self.subTest(prop=prop):
                    served = await copy.read_property(
                        copy_device if oid == DEVICE_ID else oid, prop)
                    self.assertEqual(served, await self.server.read_property(oid, prop))
        finally:
            await copy.stop()


class AccumulatorKeywordTests(unittest.TestCase):
    def test_scale_and_prescale_are_keyword_only(self) -> None:
        parameters = inspect.signature(BACnetServer.add_accumulator).parameters
        self.assertEqual(list(parameters), ["self", "instance", "name", "units", "scale",
                                            "prescale"])
        for keyword in ("scale", "prescale"):
            self.assertIs(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(parameters[keyword].default)

    def test_values_outside_the_types_raise(self) -> None:
        server = BACnetServer(DEVICE + 3, interface="127.0.0.1", port=0)
        for keywords, error in (
            ({"scale": True}, TypeError),
            ({"scale": "2"}, TypeError),
            ({"scale": 2**31}, OverflowError),
            ({"scale": float("inf")}, ValueError),
            ({"prescale": (5,)}, ValueError),
            ({"prescale": (5, 0)}, ValueError),  # fits the type, divides by nothing
            ({"prescale": "5/100"}, TypeError),
            ({"prescale": (-1, 100)}, OverflowError),
            ({"prescale": (1, 2**32)}, OverflowError),
        ):
            with self.subTest(keywords=keywords), self.assertRaises(error):
                server.add_accumulator(1, "Refused", **keywords)
        self.assertEqual(server._pending_registration_count(), 0)


if __name__ == "__main__":
    unittest.main()
