"""Read results keep every value the property holds (#1296), and
BACnetServer.read_property reads through the server's ReadProperty
evaluator, so a local read matches a network read (#1297).

A Recipient_List or a Group's Present_Value reads as typed elements (#1310).
Any other value whose octets carry a context tag comes back as
application_data holding those octets. A whole read of a standard array or
list is a list at every length. Any other read is the bare value when it
holds one element and a list otherwise.
"""

from __future__ import annotations

import asyncio
import contextlib
import unittest
from typing import Any

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    BipEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
DEVICE = 9296
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI_2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)
AO_1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
MSV_1 = ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 1)
DTV_1 = ObjectIdentifier(ObjectType.DATETIME_VALUE, 1)
NC_1 = ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1)
NF_1 = ObjectIdentifier(ObjectType.NOTIFICATION_FORWARDER, 1)
LC_1 = ObjectIdentifier(ObjectType.LOAD_CONTROL, 1)
NP_1 = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
LIFT_1 = ObjectIdentifier(ObjectType.LIFT, 1)
GROUP_1 = ObjectIdentifier(ObjectType.GROUP, 1)
GROUP_2 = ObjectIdentifier(ObjectType.GROUP, 2)
WILDCARD_DEVICE = ObjectIdentifier(ObjectType.DEVICE, 4194303)


def destination(device_instance: int, process: int) -> bytes:
    """One BACnetDestination: every day, all day, a device recipient."""
    return (
        b"\x82\x01\xfe"  # valid days
        b"\xb4\x00\x00\x00\x00"  # from 00:00
        b"\xb4\x17\x3b\x3b\x63"  # to 23:59:59.99
        + b"\x0c" + ((8 << 22) | device_instance).to_bytes(4, "big")  # device [0]
        + b"\x21" + bytes([process])  # process identifier
        + b"\x10"  # unconfirmed
        + b"\x82\x05\xe0"  # every transition
    )


def destination_read(device_instance: int, process: int) -> dict[str, Any]:
    """How a read gives back `destination(device_instance, process)`."""
    return {
        "recipient": {"kind": "device",
                      "object_identifier": ObjectIdentifier(ObjectType.DEVICE, device_instance)},
        "process_identifier": process,
        "valid_days": 0x7F,
        "from_time": (0, 0, 0, 0),
        "to_time": (23, 59, 59, 99),
        "issue_confirmed_notifications": False,
        "transitions": 0b111,
    }


RECIPIENTS = destination(10, 1) + destination(11, 2)
GROUP_MEMBERS: Any = [(AI_1, [(P.PRESENT_VALUE, None), (P.OBJECT_NAME, None)]),
                      (AI_2, [(P.PRESENT_VALUE, None)])]


def make_server() -> BACnetServer:
    server = BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    server.add_analog_input(1, "AI-1", present_value=21.5)
    server.add_analog_input(2, "AI-2", present_value=22.5)
    server.add_analog_output(1, "AO-1")
    server.add_multistate_value(1, "One state", 1)
    server.add_notification_class(1, "NC-1", 1)
    server.add_notification_forwarder(1, "NF-1")
    server.add_load_control(1, "LC-1")
    server.add_group(1, "Zone", GROUP_MEMBERS)
    server.add_group(2, "Empty")
    server.add_bip_network_port(1, "NP-1", udp_port=0)
    server.add_lift(1, "Lift-1", 1)
    server.add_date_time_value(1, "DTV-1")
    return server


async def configure(server: BACnetServer) -> None:
    for oid in (NC_1, NF_1):
        await server.write_property_local(
            oid, P.RECIPIENT_LIST, PropertyValue.application_data(RECIPIENTS), source_object=None
        )
    await server.write_property_local(
        AO_1, P.PRESENT_VALUE, PropertyValue.real(42.0), priority=8, source_object=None
    )
    await server.write_property_local(
        DTV_1, P.PRESENT_VALUE,
        PropertyValue.list([PropertyValue.date(2026, 10, 25, 7), PropertyValue.time(7, 30, 0, 0)]),
        priority=8, source_object=None,
    )


def rpm_values(results: list[dict[str, Any]]) -> list[Any]:
    """The values of a one-object ReadPropertyMultiple result, in order."""
    [only] = results
    for row in only["results"]:
        assert row["error"] is None, row
    return [row["value"] for row in only["results"]]


class ClientReadShapeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        # Cleanups are coroutine functions: the binding's awaitables need the
        # running loop when they are created.
        self.server = make_server()
        await self.server.start()

        async def stop_server() -> None:
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        await configure(self.server)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)
        reader = BipEndpoint(device_instance=9298, interface="127.0.0.1",
                             broadcast_address="127.0.0.1", port=0)
        await reader.start()

        async def close_reader() -> None:
            await reader.close()

        self.addAsyncCleanup(close_reader)
        self.role = await reader.client()

    async def read(self, oid: ObjectIdentifier, prop: PropertyIdentifier,
                   index: int | None = None) -> PropertyValue:
        """Read with BACnetClient, check that ReadPropertyMultiple, the
        endpoint client role and the local read all agree, and return the
        value."""
        value = await self.client.read_property(self.address, oid, prop, index)
        spec: Any = [(oid, [(prop, index)])]
        self.assertEqual(rpm_values(await self.client.read_property_multiple(self.address, spec)),
                         [value])
        self.assertEqual(await self.role.read_property(self.address, oid, prop, index), value)
        self.assertEqual(rpm_values(await self.role.read_property_multiple(self.address, spec)),
                         [value])
        self.assertEqual(await self.server.read_property(oid, prop, index), value)
        return value

    async def test_a_whole_object_list_is_complete(self) -> None:
        device = ObjectIdentifier(ObjectType.DEVICE, DEVICE)
        value = await self.read(device, P.OBJECT_LIST)
        self.assertEqual(value.tag, "list")
        self.assertTrue({device, AI_1, AI_2, AO_1, MSV_1, NC_1, NF_1, LC_1} <= set(value.value))
        count = await self.read(device, P.OBJECT_LIST, 0)
        self.assertEqual(count.value, len(value.value))
        self.assertEqual(await self.read(device, P.OBJECT_LIST, 2), PropertyValue.object_identifier(value.value[1]))

    async def test_a_whole_priority_array_has_sixteen_slots(self) -> None:
        value = await self.read(AO_1, P.PRIORITY_ARRAY)
        self.assertEqual(value, PropertyValue.list(
            [PropertyValue.null()] * 7 + [PropertyValue.real(42.0)] + [PropertyValue.null()] * 8
        ))
        self.assertEqual(await self.read(AO_1, P.PRIORITY_ARRAY, 8), PropertyValue.real(42.0))

    async def test_a_date_time_priority_array_has_sixteen_slots(self) -> None:
        # A set slot is the BACnetPriorityValue date-time choice, context tag
        # 1 around the Date and Time (#2098): one element holding its octets
        # as read, whose value is the date and time. A Present_Value write
        # takes the bare pair instead.
        slot = bytes([0x1E, 0xA4, 126, 10, 25, 7, 0xB4, 7, 30, 0, 0, 0x1F])
        when = [(2026, 10, 25, 7), (7, 30, 0, 0)]
        value = await self.read(DTV_1, P.PRIORITY_ARRAY)
        self.assertEqual(value, PropertyValue.list(
            [PropertyValue.null()] * 7 + [PropertyValue.application_data(slot)]
            + [PropertyValue.null()] * 8
        ))
        self.assertEqual(value.value, [None] * 7 + [when] + [None] * 8)
        one = await self.read(DTV_1, P.PRIORITY_ARRAY, 8)
        self.assertEqual(one, PropertyValue.application_data(slot))
        self.assertEqual(one.value, when)
        # Present_Value stays the bare date and time.
        self.assertEqual((await self.read(DTV_1, P.PRESENT_VALUE)).value, when)

    async def test_a_one_element_array_is_still_a_list(self) -> None:
        value = await self.read(MSV_1, P.STATE_TEXT)
        self.assertEqual(value, PropertyValue.list([PropertyValue.character_string("State 1")]))
        self.assertEqual(await self.read(MSV_1, P.STATE_TEXT, 1), PropertyValue.character_string("State 1"))
        # Arrays that only some object types carry: the default DNS server of
        # a B/IP Network Port, and a one-floor, one-door Lift's arrays.
        self.assertEqual(await self.read(NP_1, P.IP_DNS_SERVER),
                         PropertyValue.list([PropertyValue.octet_string(bytes(4))]))
        for prop in (P.FLOOR_TEXT, P.CAR_DOOR_STATUS, P.CAR_DOOR_COMMAND):
            with self.subTest(property=prop):
                value = await self.read(LIFT_1, prop)
                self.assertEqual(value.tag, "list")
                self.assertEqual(len(value.value), 1)
                self.assertEqual(await self.read(LIFT_1, prop, 0), PropertyValue.unsigned(1))

    async def test_recipient_lists_keep_every_destination(self) -> None:
        for oid in (NC_1, NF_1):
            with self.subTest(object=oid):
                value = await self.read(oid, P.RECIPIENT_LIST)
                self.assertEqual(value.tag, "list")
                self.assertEqual(value.value, [destination_read(10, 1), destination_read(11, 2)])

    async def test_group_present_value_holds_every_member(self) -> None:
        value = await self.read(GROUP_1, P.PRESENT_VALUE)
        self.assertEqual(value.value,
                         await self.client.read_property_multiple(self.address, GROUP_MEMBERS))
        self.assertEqual(len(value.value), 2)
        self.assertEqual(await self.read(GROUP_2, P.PRESENT_VALUE), PropertyValue.list([]))

    async def test_a_scalar_of_several_elements_is_a_list(self) -> None:
        # Start_Time is a BACnetDateTime: a Date, then a Time.
        value = await self.read(LC_1, P.START_TIME)
        self.assertEqual(value.tag, "list")
        self.assertEqual(value.value, [(255, 255, 255, 255), (255, 255, 255, 255)])
        self.assertEqual(await self.read(AI_1, P.PRESENT_VALUE), PropertyValue.real(21.5))

    async def test_a_cov_notification_keeps_every_element(self) -> None:
        notifications: asyncio.Queue[Any] = asyncio.Queue()
        iterator = await self.client.cov_notifications()

        async def collect() -> None:
            async for notification in iterator:
                notifications.put_nowait(notification)

        listener = asyncio.create_task(collect())
        try:
            await self.client.subscribe_cov(self.address, 1, LC_1, confirmed=False, lifetime=None)
            notification = await asyncio.wait_for(notifications.get(), 3)
        finally:
            listener.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await listener
        values = {item["property_id"]: item["value"] for item in notification.values}
        self.assertEqual(values[P.START_TIME], await self.server.read_property(LC_1, P.START_TIME))
        self.assertEqual(len(values[P.START_TIME].value), 2)
        # Requested_Shed_Level is a context-tagged CHOICE, read typed (#1499).
        shed = await self.server.read_property(LC_1, P.REQUESTED_SHED_LEVEL)
        self.assertEqual(shed.tag, "shed_level")
        self.assertEqual(values[P.REQUESTED_SHED_LEVEL], shed)


class LocalReadTests(unittest.IsolatedAsyncioTestCase):
    async def test_derived_device_properties_match_the_network_read(self) -> None:
        server = make_server()
        await server.start()
        try:
            address = await server.local_address()
            device = ObjectIdentifier(ObjectType.DEVICE, DEVICE)
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                # Instance 4194303 names the server's own Device.
                for oid in (device, WILDCARD_DEVICE):
                    with self.subTest(object=oid):
                        self.assertEqual(await server.read_property(oid, P.OBJECT_NAME),
                                         await client.read_property(address, oid, P.OBJECT_NAME))
                # The Device's COV subscription list is projected from the live table.
                await client.subscribe_cov(address, 7, AI_1, confirmed=False, lifetime=None)
                local = await server.read_property(device, P.ACTIVE_COV_SUBSCRIPTIONS)
                # Each subscription reads as a mapping (#1345).
                self.assertEqual(local.tag, "list")
                [subscription] = local.value
                self.assertEqual(subscription["process_identifier"], 7)
                self.assertEqual(subscription["object_identifier"], AI_1)
                self.assertEqual(subscription["property_identifier"], P.PRESENT_VALUE)
                self.assertIs(subscription["issue_confirmed_notifications"], False)
                self.assertEqual(local, await client.read_property(address, device, P.ACTIVE_COV_SUBSCRIPTIONS))
            # An unknown object fails as a network read does.
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(ObjectIdentifier(ObjectType.ANALOG_INPUT, 99), P.PRESENT_VALUE)
            self.assertEqual(raised.exception.error_code, 31)  # UNKNOWN_OBJECT
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
