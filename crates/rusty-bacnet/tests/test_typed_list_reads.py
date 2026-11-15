"""Typed reads of the constructed collections the binding also writes as
typed values (#1310).

A property configured from typed values (Recipient_List, Port_Filter, a
Group's members, a Command's Action, the access-control object references,
Supported_Formats, Stages and Target_References) reads back through
BACnetClient, its ReadPropertyMultiple, the endpoint client role and its
ReadPropertyMultiple, and the local BACnetServer.read_property as a list of
those same values, and an indexed read as one of them. A Group's
Present_Value reads as the read_property_multiple results of its members. A
COV notification value decodes the same way, and a value that isn't the
expected production reads as its tags (``tagged``, #2618).
"""

from __future__ import annotations

import asyncio
import contextlib
import socket
import unittest
from typing import Any

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    BipEndpoint,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
DEVICE = 9310
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI_2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)
AO_1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
BO_1 = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
BV_1 = ObjectIdentifier(ObjectType.BINARY_VALUE, 1)
NC_1 = ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1)
NF_1 = ObjectIdentifier(ObjectType.NOTIFICATION_FORWARDER, 1)
GROUP_1 = ObjectIdentifier(ObjectType.GROUP, 1)
GROUP_2 = ObjectIdentifier(ObjectType.GROUP, 2)
CMD_1 = ObjectIdentifier(ObjectType.COMMAND, 1)
DOOR_1 = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
POINT_1 = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
READER_1 = ObjectIdentifier(ObjectType.CREDENTIAL_DATA_INPUT, 1)
STG_1 = ObjectIdentifier(ObjectType.STAGING, 1)
REMOTE_DEVICE = ObjectIdentifier(ObjectType.DEVICE, 99)
REMOTE_DOOR = ObjectIdentifier(ObjectType.ACCESS_DOOR, 4)

# Destinations with every key, as a read gives them back.
DEVICE_10: Any = {
    "recipient": {"kind": "device", "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 10)},
    "process_identifier": 1,
    "valid_days": 0x7F,
    "from_time": (0, 0, 0, 0),
    "to_time": (23, 59, 59, 99),
    "issue_confirmed_notifications": False,
    "transitions": 0b111,
}
WEEKDAYS: Any = {
    "recipient": {"kind": "address", "network_number": 5, "mac_address": b"\x0a\x0b"},
    "process_identifier": 2,
    "valid_days": 0b0011111,  # Monday to Friday
    "from_time": (8, 0, 0, 0),
    "to_time": (17, 30, 15, 50),
    "issue_confirmed_notifications": True,
    "transitions": 0b101,  # to-offnormal and to-normal
}
DESTINATIONS = [DEVICE_10, WEEKDAYS]
# The same two destinations as Recipient_List carries them.
DESTINATION_OCTETS = (
    b"\x82\x01\xfe" b"\xb4\x00\x00\x00\x00" b"\xb4\x17\x3b\x3b\x63"
    b"\x0c\x02\x00\x00\x0a" b"\x21\x01" b"\x10" b"\x82\x05\xe0"
    b"\x82\x01\xf8" b"\xb4\x08\x00\x00\x00" b"\xb4\x11\x1e\x0f\x32"
    b"\x1e\x21\x05\x62\x0a\x0b\x1f" b"\x21\x02" b"\x11" b"\x82\x05\xa0"
)
PORT_FILTER = [(0, True), (1, False)]
PORT_FILTER_OCTETS = b"\x09\x00\x19\x01" b"\x09\x01\x19\x00"
GROUP_MEMBERS: Any = [(AI_1, [(P.PRESENT_VALUE, None), (P.OBJECT_NAME, None)]),
                      (AI_2, [(P.PRESENT_VALUE, None)])]


def command(target: ObjectIdentifier, value: float, priority: int, **extra: Any) -> Any:
    """An ActionCommand with every key, as a read gives it back."""
    return {
        "device_identifier": None,
        "object_identifier": target,
        "property_identifier": P.PRESENT_VALUE,
        "property_array_index": None,
        "property_value": PropertyValue.real(value),
        "priority": priority,
        "post_delay": None,
        "quit_on_failure": False,
        "write_successful": False,
        **extra,
    }


ACTION = [
    [command(AO_1, 50.0, 8), command(AI_1, 21.5, 9, post_delay=0)],
    [command(AO_1, 10.0, 8, quit_on_failure=True, device_identifier=REMOTE_DEVICE)],
]
# add_command takes write_successful but ignores it: only a run sets the flag,
# so the Command still reads back as ACTION.
ACTION_WRITTEN = [[ACTION[0][0], {**ACTION[0][1], "write_successful": True}], ACTION[1]]
DOOR_MEMBERS: Any = [BO_1, (REMOTE_DEVICE, REMOTE_DOOR)]
ACCESS_DOORS: Any = [(REMOTE_DEVICE, REMOTE_DOOR)]
# Wiegand 26 (8) in class 0, vendor 260's CUSTOM (2) format 7 in class 3, and
# format 10 with a zero vendor id and no vendor format in class 1.
SUPPORTED_FORMATS: Any = [(8, 0), ((2, 260, 7), 3), ((10, 0, None), 1)]
STAGES = [(10.0, [False, True], 1.0), (20.0, [True, True], 2.0)]
TARGET_REFERENCES = [BO_1, BV_1]


def make_server() -> BACnetServer:
    server = BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    server.add_analog_input(1, "AI-1", present_value=21.5)
    server.add_analog_input(2, "AI-2", present_value=22.5)
    server.add_analog_output(1, "AO-1")
    server.add_binary_output(1, "BO-1")
    server.add_binary_value(1, "BV-1")
    server.add_notification_class(1, "NC-1", 1)
    server.add_notification_forwarder(1, "NF-1", recipients=DESTINATIONS,
                                      port_filter=PORT_FILTER)
    server.add_group(1, "Zone", GROUP_MEMBERS)
    server.add_group(2, "Empty")
    server.add_command(1, "CMD-1", action=ACTION_WRITTEN)
    server.add_access_door(1, "Main Entry", door_members=DOOR_MEMBERS)
    server.add_access_point(1, "Lobby", access_doors=ACCESS_DOORS)
    server.add_credential_data_input(1, "Card Reader", supported_formats=SUPPORTED_FORMATS)
    server.add_staging(1, "STG-1", 5.0, 0.0, 62, 8, STAGES, TARGET_REFERENCES)
    return server


def rpm_values(results: list[dict[str, Any]]) -> list[Any]:
    """The values of a one-object ReadPropertyMultiple result, in order."""
    [only] = results
    for row in only["results"]:
        assert row["error"] is None, row
    return [row["value"] for row in only["results"]]


def frame(apdu: bytes) -> bytes:
    """A B/IP original-unicast frame carrying `apdu` with no NPDU options."""
    payload = b"\x01\x00" + apdu
    return b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload


def context_unsigned(tag: int, value: int) -> bytes:
    data = value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big")
    return bytes([(tag << 4) | 0x08 | len(data)]) + data


def object_id(tag: int, oid: ObjectIdentifier) -> bytes:
    raw = (oid.object_type.to_raw() << 22) | oid.instance
    return bytes([(tag << 4) | 0x0C]) + raw.to_bytes(4, "big")


class TypedListReadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        # Cleanups are coroutine functions: the binding's awaitables need the
        # running loop when they are created.
        self.server = make_server()
        await self.server.start()

        async def stop_server() -> None:
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)
        reader = BipEndpoint(device_instance=9311, interface="127.0.0.1",
                             broadcast_address="127.0.0.1", port=0)
        await reader.start()

        async def close_reader() -> None:
            await reader.close()

        self.addAsyncCleanup(close_reader)
        self.role = await reader.client()

    async def read(self, oid: ObjectIdentifier, prop: PropertyIdentifier,
                   index: int | None = None) -> PropertyValue:
        """Read with BACnetClient, check that its ReadPropertyMultiple, the
        endpoint client role (both services) and the local read all agree,
        and return the value."""
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
                          expected: list[Any], element: str) -> None:
        """A whole read is a list of `expected`, each index one element
        tagged `element`, and PropertyValue.list of the elements the whole
        read."""
        value = await self.read(oid, prop)
        self.assertEqual(value.tag, "list")
        self.assertEqual(value.value, expected)
        self.assertEqual(await self.read(oid, prop, 0), PropertyValue.unsigned(len(expected)))
        elements = []
        for index, item in enumerate(expected, 1):
            with self.subTest(index=index):
                one = await self.read(oid, prop, index)
                self.assertEqual(one.tag, element)
                self.assertEqual(one.value, item)
                elements.append(one)
        self.assertEqual(PropertyValue.list(elements), value)

    async def test_recipient_lists_read_as_the_destinations_written(self) -> None:
        recipients = await self.read(NF_1, P.RECIPIENT_LIST)
        self.assertEqual(recipients.tag, "list")
        self.assertEqual(recipients.value, DESTINATIONS)
        # The typed read writes back as the octets it was read from.
        await self.server.write_property_local(NC_1, P.RECIPIENT_LIST, recipients,
                                               source_object=None)
        self.assertEqual(await self.read(NC_1, P.RECIPIENT_LIST), recipients)
        await self.server.write_property_local(NC_1, P.RECIPIENT_LIST, PropertyValue.list([]),
                                               source_object=None)
        self.assertEqual(await self.read(NC_1, P.RECIPIENT_LIST), PropertyValue.list([]))

    async def test_port_filter_reads_as_the_pairs_written(self) -> None:
        await self.assert_list(NF_1, P.PORT_FILTER, PORT_FILTER, "port_permission")

    async def test_group_members_read_as_the_specs_written(self) -> None:
        members = await self.read(GROUP_1, P.LIST_OF_GROUP_MEMBERS)
        self.assertEqual(members.tag, "list")
        self.assertEqual(members.value, GROUP_MEMBERS)
        self.assertEqual(await self.read(GROUP_2, P.LIST_OF_GROUP_MEMBERS),
                         PropertyValue.list([]))

    async def test_group_present_value_reads_as_its_members_results(self) -> None:
        present_value = await self.read(GROUP_1, P.PRESENT_VALUE)
        self.assertEqual(present_value.tag, "list")
        self.assertEqual(present_value.value,
                         await self.client.read_property_multiple(self.address, GROUP_MEMBERS))
        [ai_1, ai_2] = present_value.value
        self.assertEqual(ai_1["object_id"], AI_1)
        self.assertEqual([row["value"] for row in ai_1["results"]],
                         [PropertyValue.real(21.5), PropertyValue.character_string("AI-1")])
        self.assertEqual(ai_2["results"][0]["value"], PropertyValue.real(22.5))
        self.assertEqual(await self.read(GROUP_2, P.PRESENT_VALUE), PropertyValue.list([]))

    async def test_command_action_reads_as_the_action_commands_written(self) -> None:
        await self.assert_list(CMD_1, P.ACTION, ACTION, "action_list")

    async def test_object_references_read_as_written(self) -> None:
        await self.assert_list(DOOR_1, P.DOOR_MEMBERS, DOOR_MEMBERS, "device_object_reference")
        await self.assert_list(POINT_1, P.ACCESS_DOORS, ACCESS_DOORS, "device_object_reference")
        await self.assert_list(STG_1, P.TARGET_REFERENCES, TARGET_REFERENCES,
                               "device_object_reference")

    async def test_supported_formats_read_as_the_formats_written(self) -> None:
        formats = [entry[0] for entry in SUPPORTED_FORMATS]
        await self.assert_list(READER_1, P.SUPPORTED_FORMATS, formats,
                               "authentication_factor_format")

    async def test_stages_read_as_the_triples_written(self) -> None:
        await self.assert_list(STG_1, P.STAGES, STAGES, "stage_limit_value")

    async def test_each_read_is_accepted_by_its_typed_write(self) -> None:
        async def value(oid: ObjectIdentifier, prop: PropertyIdentifier) -> Any:
            return (await self.server.read_property(oid, prop)).value

        copy = BACnetServer(DEVICE + 2, interface="127.0.0.1", port=0)
        copy.add_notification_forwarder(1, "NF-1", recipients=await value(NF_1, P.RECIPIENT_LIST),
                                        port_filter=await value(NF_1, P.PORT_FILTER))
        copy.add_group(1, "Zone", await value(GROUP_1, P.LIST_OF_GROUP_MEMBERS))
        copy.add_command(1, "CMD-1", action=await value(CMD_1, P.ACTION))
        copy.add_access_door(1, "Main Entry", door_members=await value(DOOR_1, P.DOOR_MEMBERS))
        copy.add_access_point(1, "Lobby", access_doors=await value(POINT_1, P.ACCESS_DOORS))
        copy.add_credential_data_input(1, "Card Reader", supported_formats=list(zip(
            await value(READER_1, P.SUPPORTED_FORMATS),
            await value(READER_1, P.SUPPORTED_FORMAT_CLASSES),
        )))
        copy.add_binary_output(1, "BO-1")
        copy.add_binary_value(1, "BV-1")
        # This server's targets are local, so they read as plain identifiers;
        # add_staging refuses a (device, object) pair, which only names a
        # target in another device.
        copy.add_staging(1, "STG-1", 5.0, 0.0, 62, 8, await value(STG_1, P.STAGES),
                         await value(STG_1, P.TARGET_REFERENCES))
        self.assertEqual(copy._pending_registration_count(), 9)
        # A Supported_Formats triple with a vendor member left out, as a read
        # of another device may give it, is a value the object checks, not a
        # type error.
        with self.assertRaises(BacnetProtocolError) as raised:
            copy.add_credential_data_input(2, "Refused", supported_formats=[((3, 4, None), 0)])
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())

    async def test_a_cov_notification_value_reads_typed(self) -> None:
        notifications: asyncio.Queue[Any] = asyncio.Queue()
        iterator = await self.client.cov_notifications()

        async def collect() -> None:
            async for notification in iterator:
                notifications.put_nowait(notification)

        listener = asyncio.create_task(collect())
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            try:
                # A device that acknowledges the subscription, then reports
                # the forwarder's two lists in the octets this server serves.
                subscribe = asyncio.ensure_future(self.client.subscribe_cov(
                    f"127.0.0.1:{peer.getsockname()[1]}", 1, NF_1, confirmed=False,
                    lifetime=None))
                wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                await loop.sock_sendto(peer, frame(bytes([0x20, wire[8], 5])), remote)
                await asyncio.wait_for(subscribe, 2)
                values = b"".join(
                    context_unsigned(0, prop.to_raw()) + b"\x2e" + octets + b"\x2f"
                    for prop, octets in ((P.RECIPIENT_LIST, DESTINATION_OCTETS),
                                         (P.PORT_FILTER, PORT_FILTER_OCTETS))
                )
                body = (context_unsigned(0, 1) + object_id(1, REMOTE_DEVICE)
                        + object_id(2, NF_1) + context_unsigned(3, 0)
                        + b"\x4e" + values + b"\x4f")
                await loop.sock_sendto(peer, frame(b"\x10\x02" + body), remote)
                notification = await asyncio.wait_for(notifications.get(), 3)
            finally:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
        values = {item["property_id"]: item["value"] for item in notification.values}
        self.assertEqual(values[P.RECIPIENT_LIST].value, DESTINATIONS)
        self.assertEqual(values[P.PORT_FILTER].value, PORT_FILTER)
        # Equal values carry equal octets, so the server serves these.
        for prop in (P.RECIPIENT_LIST, P.PORT_FILTER):
            self.assertEqual(values[prop], await self.server.read_property(NF_1, prop))


class FallbackTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_value_that_is_not_the_expected_production_keeps_its_octets(self) -> None:
        # Each is well framed; none is a list of destinations.
        not_destinations = (
            b"\x09\x01",  # a context element where a destination starts
            DESTINATION_OCTETS + b"\x21\x01",  # a trailing value
            PORT_FILTER_OCTETS,  # another production
        )
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                for octets in not_destinations:
                    with self.subTest(octets=octets):
                        expected = PropertyValue.tagged(octets)
                        read = asyncio.ensure_future(
                            client.read_property(address, NF_1, P.RECIPIENT_LIST))
                        wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                        ack = bytes([0x30, wire[8], 12]) + wire[10:] + b"\x3e" + octets + b"\x3f"
                        await loop.sock_sendto(peer, frame(ack), remote)
                        self.assertEqual(await asyncio.wait_for(read, 2), expected)
                        rpm = asyncio.ensure_future(client.read_property_multiple(
                            address, [(NF_1, [(P.RECIPIENT_LIST, None)])]))
                        wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                        body = (object_id(0, NF_1) + b"\x1e" + context_unsigned(2, 102)
                                + b"\x4e" + octets + b"\x4f\x1f")
                        await loop.sock_sendto(peer, frame(bytes([0x30, wire[8], 14]) + body),
                                               remote)
                        self.assertEqual(rpm_values(await asyncio.wait_for(rpm, 2)), [expected])


if __name__ == "__main__":
    unittest.main()
