"""Typed keywords for add_global_group, add_schedule and add_calendar take
the forms their typed reads give, and building from a read reproduces the
same bytes (#1499)."""
import unittest
from contextlib import asynccontextmanager

from rusty_bacnet import (BACnetClient, BACnetServer, BacnetProtocolError, ErrorCode,
                          ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue)

P = PropertyIdentifier
O = ObjectType
DEVICE = 9498
GROUP = ObjectIdentifier(O.GLOBAL_GROUP, 1)
SCHEDULE = ObjectIdentifier(O.SCHEDULE, 1)
CALENDAR = ObjectIdentifier(O.CALENDAR, 1)
AV1 = ObjectIdentifier(O.ANALOG_VALUE, 1)

MEMBERS = [(AV1, P.PRESENT_VALUE),
           (AV1, P.PRIORITY_ARRAY, 8),
           {"object_identifier": ObjectIdentifier(O.ANALOG_INPUT, 3),
            "property_identifier": P.PRESENT_VALUE,
            "device_identifier": ObjectIdentifier(O.DEVICE, 77)}]
DATE_LIST = [{"kind": "date", "date": (2026, 12, 25, 5)},
             {"kind": "date_range", "start_date": (2026, 7, 1, 255),
              "end_date": (2026, 8, 31, 255)},
             {"kind": "week_n_day", "month": 255, "week_of_month": 255, "day_of_week": 6}]
WEEK = [[((7, 0, 0, 0), PropertyValue.real(21.0)), ((18, 30, 0, 0), PropertyValue.null())],
        [((8, 0, 0, 0), PropertyValue.real(20.5))], [], [], [],
        [((9, 0, 0, 0), PropertyValue.unsigned(2))], []]
EXCEPTIONS = [
    {"period": CALENDAR, "time_values": [((0, 0, 0, 0), PropertyValue.real(16.0))],
     "priority": 3},
    {"period": {"kind": "date", "date": (2026, 12, 31, 4)},
     "time_values": [((12, 0, 0, 0), PropertyValue.null())], "priority": 16},
]
PERIOD = ((2026, 1, 1, 255), (2026, 12, 31, 255))
OPEN_PERIOD = ((255, 255, 255, 255), (255, 255, 255, 255))


@asynccontextmanager
async def running(build):
    server = BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    server.add_analog_value(1, "Setpoint")
    build(server)
    await server.start()
    try:
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            yield server, client, await server.local_address()
    finally:
        await server.stop()


class TypedConstructionTests(unittest.IsolatedAsyncioTestCase):
    async def reads(self, context, oid, prop):
        server, client, address = context
        local = await server.read_property(oid, prop)
        direct = await client.read_property(address, oid, prop)
        rpm = await client.read_property_multiple(address, [(oid, [(prop, None)])])
        for value in (direct, rpm[0]["results"][0]["value"]):
            self.assertEqual(value, local)
            self.assertEqual(value.value, local.value)
        return local

    async def round_trip(self, add, oid, keyword, value, prop, expected):
        """Build with `value`, read the property on every route, build again
        from the read's `.value`, and check the second read is the same."""
        async with running(lambda server: add(server, **{keyword: value})) as context:
            first = await self.reads(context, oid, prop)
        self.assertEqual(first.value, expected)
        async with running(lambda server: add(server, **{keyword: first.value})) as context:
            second = await self.reads(context, oid, prop)
        self.assertEqual(second, first)
        self.assertEqual(second.octets, first.octets)
        return first

    async def test_global_group_members(self):
        def add(server, **keywords):
            server.add_global_group(1, "Group", **keywords)
        expected = [
            {"object_identifier": AV1, "property_identifier": P.PRESENT_VALUE,
             "property_array_index": None, "device_identifier": None},
            {"object_identifier": AV1, "property_identifier": P.PRIORITY_ARRAY,
             "property_array_index": 8, "device_identifier": None},
            {"object_identifier": ObjectIdentifier(O.ANALOG_INPUT, 3),
             "property_identifier": P.PRESENT_VALUE, "property_array_index": None,
             "device_identifier": ObjectIdentifier(O.DEVICE, 77)},
        ]
        read = await self.round_trip(add, GROUP, "members", MEMBERS, P.GROUP_MEMBERS, expected)
        self.assertEqual(read.tag, "list")
        await self.round_trip(add, GROUP, "members", [], P.GROUP_MEMBERS, [])
        # This server's own Device is stored in its local form.
        own = [{"object_identifier": AV1, "property_identifier": P.PRESENT_VALUE,
                "device_identifier": ObjectIdentifier(O.DEVICE, DEVICE)}]
        await self.round_trip(add, GROUP, "members", own, P.GROUP_MEMBERS, expected[:1])

    async def test_calendar_date_list(self):
        def add(server, **keywords):
            server.add_calendar(1, "Holidays", **keywords)
        await self.round_trip(add, CALENDAR, "date_list", DATE_LIST, P.DATE_LIST, DATE_LIST)
        await self.round_trip(add, CALENDAR, "date_list", [], P.DATE_LIST, [])

    async def test_schedule_weekly_schedule(self):
        def add(server, **keywords):
            server.add_schedule(1, "Week", **keywords)
        await self.round_trip(add, SCHEDULE, "weekly_schedule", WEEK, P.WEEKLY_SCHEDULE, WEEK)
        await self.round_trip(add, SCHEDULE, "weekly_schedule", [[]] * 7, P.WEEKLY_SCHEDULE,
                              [[]] * 7)

    async def test_schedule_exception_schedule(self):
        def add(server, **keywords):
            server.add_calendar(1, "Holidays")
            server.add_schedule(1, "Week", **keywords)
        await self.round_trip(add, SCHEDULE, "exception_schedule", EXCEPTIONS,
                              P.EXCEPTION_SCHEDULE, EXCEPTIONS)
        await self.round_trip(add, SCHEDULE, "exception_schedule", [], P.EXCEPTION_SCHEDULE, [])

    async def test_schedule_effective_period(self):
        def add(server, **keywords):
            server.add_schedule(1, "Week", **keywords)
        read = await self.round_trip(add, SCHEDULE, "effective_period", PERIOD,
                                     P.EFFECTIVE_PERIOD, PERIOD)
        self.assertEqual(read.tag, "date_range")
        await self.round_trip(add, SCHEDULE, "effective_period", OPEN_PERIOD,
                              P.EFFECTIVE_PERIOD, OPEN_PERIOD)
        async with running(lambda server: add(server)) as context:
            default = await self.reads(context, SCHEDULE, P.EFFECTIVE_PERIOD)
        self.assertEqual(default.value, OPEN_PERIOD)

    async def test_whole_schedule_from_one_read_of_each_property(self):
        def full(server):
            server.add_calendar(1, "Holidays", date_list=DATE_LIST)
            server.add_schedule(1, "Week", weekly_schedule=WEEK, exception_schedule=EXCEPTIONS,
                                effective_period=PERIOD)
        props = (P.WEEKLY_SCHEDULE, P.EXCEPTION_SCHEDULE, P.EFFECTIVE_PERIOD)
        async with running(full) as context:
            first = [await self.reads(context, SCHEDULE, prop) for prop in props]
            dates = await self.reads(context, CALENDAR, P.DATE_LIST)

        def copy(server):
            server.add_calendar(1, "Holidays", date_list=dates.value)
            server.add_schedule(1, "Week", weekly_schedule=first[0].value,
                                exception_schedule=first[1].value,
                                effective_period=first[2].value)
        async with running(copy) as context:
            second = [await self.reads(context, SCHEDULE, prop) for prop in props]
            self.assertEqual(await self.reads(context, CALENDAR, P.DATE_LIST), dates)
        self.assertEqual(second, first)

    def test_shapes_are_checked_and_the_objects_refuse_what_bacnet_does_not_allow(self):
        server = BACnetServer(DEVICE, interface="127.0.0.1", port=0)
        for keywords, error in [
            ({"weekly_schedule": [[]] * 6}, ValueError),
            ({"weekly_schedule": "week"}, TypeError),
            ({"weekly_schedule": [[((7, 0, 0, 0), 21.0)]] + [[]] * 6}, TypeError),
            ({"weekly_schedule": [[(7, PropertyValue.null())]] + [[]] * 6}, TypeError),
            ({"exception_schedule": [{"period": CALENDAR, "time_values": []}]}, ValueError),
            ({"exception_schedule": [{"period": {"kind": "weekly"}, "time_values": [],
                                      "priority": 1}]}, ValueError),
            ({"effective_period": ((2026, 1, 1, 255),)}, TypeError),
            ({"effective_period": ((1800, 1, 1, 255), (2026, 1, 1, 255))}, ValueError),
        ]:
            with self.subTest(**keywords), self.assertRaises(error):
                server.add_schedule(1, "Week", **keywords)
        for keywords in [
            {"weekly_schedule": [[((7, 0, 0, 0), PropertyValue.null()),
                                  ((7, 0, 0, 0), PropertyValue.null())]] + [[]] * 6},
            {"exception_schedule": [{"period": CALENDAR, "time_values": [], "priority": 17}]},
            {"effective_period": ((2026, 13, 1, 255), (2026, 12, 31, 255))},
        ]:
            with self.subTest(**keywords), self.assertRaises(BacnetProtocolError):
                server.add_schedule(1, "Week", **keywords)
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_calendar(1, "Bad", date_list=[{"kind": "week_n_day", "month": 15,
                                                      "week_of_month": 1, "day_of_week": 1}])
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
        with self.assertRaises(ValueError):
            server.add_calendar(1, "Bad", date_list=[{"kind": "date", "date": (2026, 1, 1, 1),
                                                      "extra": 1}])
        with self.assertRaises(ValueError):
            server.add_global_group(1, "Bad", members=[{
                "object_identifier": AV1, "property_identifier": P.PRESENT_VALUE,
                "device_identifier": AV1}])


if __name__ == "__main__":
    unittest.main()
