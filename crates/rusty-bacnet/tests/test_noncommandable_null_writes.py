"""A NULL written to a property that isn't commandable and has no NULL in
its datatype succeeds and leaves the property as it is (#1396), through
write_property_local as over a network WriteProperty. A read-only property
still refuses it, a commandable Present_Value still relinquishes, and a
property whose datatype has a NULL still stores it.
"""

from __future__ import annotations

import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
DEVICE = 13_960
RIGHTS = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
CREDENTIAL = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 1)
AV_1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
BO_1 = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
NC_1 = ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1)
SCHED_1 = ObjectIdentifier(ObjectType.SCHEDULE, 1)

# (object, property, array index) that take a NULL as a no-op: Access
# Rights Enable and a rule array, a Global_Identifier, raw-octet and plain
# BACnetLIST properties, and scalars.
UNCHANGED: list[tuple[ObjectIdentifier, PropertyIdentifier, int | None]] = [
    (RIGHTS, P.ENABLE, None),
    (RIGHTS, P.POSITIVE_ACCESS_RULES, None),
    (RIGHTS, P.POSITIVE_ACCESS_RULES, 1),
    (CREDENTIAL, P.GLOBAL_IDENTIFIER, None),
    (NC_1, P.RECIPIENT_LIST, None),
    (AV_1, P.COV_INCREMENT, None),
    (AV_1, P.DESCRIPTION, None),
    (AV_1, P.OUT_OF_SERVICE, None),
    (SCHED_1, P.WEEKLY_SCHEDULE, 2),
]


def make_server() -> BACnetServer:
    server = BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    server.add_access_rights(1, "Rights", positive_access_rules=[{"enable": False}])
    server.add_access_credential(1, "Credential")
    server.add_analog_value(1, "AV-1")
    server.add_binary_output(1, "BO-1")
    server.add_notification_class(1, "NC-1", 1)
    server.add_schedule(1, "SCHED-1")
    return server


class NoncommandableNullTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
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

    async def test_a_null_leaves_the_property_as_it_is_locally_and_over_the_network(self) -> None:
        for oid, prop, index in UNCHANGED:
            with self.subTest(object=oid, property=prop, index=index):
                before = await self.server.read_property(oid, prop)
                await self.server.write_property_local(
                    oid, prop, PropertyValue.null(), array_index=index, source_object=None
                )
                self.assertEqual(await self.server.read_property(oid, prop), before)
                await self.client.write_property(
                    self.address, oid, prop, PropertyValue.null(), array_index=index
                )
                self.assertEqual(await self.server.read_property(oid, prop), before)

    async def test_a_read_only_property_still_refuses_a_null(self) -> None:
        for oid, prop in ((AV_1, P.STATUS_FLAGS), (CREDENTIAL, P.OBJECT_TYPE)):
            with self.subTest(object=oid, property=prop):
                with self.assertRaises(BacnetProtocolError) as local:
                    await self.server.write_property_local(
                        oid, prop, PropertyValue.null(), source_object=None
                    )
                self.assertEqual(
                    local.exception.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw()
                )
                with self.assertRaises(BacnetProtocolError) as network:
                    await self.client.write_property(
                        self.address, oid, prop, PropertyValue.null()
                    )
                self.assertEqual(
                    network.exception.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw()
                )

    async def test_a_commandable_present_value_still_relinquishes(self) -> None:
        inactive = await self.server.read_property(BO_1, P.PRESENT_VALUE)
        await self.server.write_property_local(
            BO_1, P.PRESENT_VALUE, PropertyValue.enumerated(1), 8, source_object=None
        )
        self.assertNotEqual(await self.server.read_property(BO_1, P.PRESENT_VALUE), inactive)
        await self.server.write_property_local(
            BO_1, P.PRESENT_VALUE, PropertyValue.null(), 8, source_object=None
        )
        self.assertEqual(await self.server.read_property(BO_1, P.PRESENT_VALUE), inactive)

    async def test_a_null_is_stored_where_the_datatype_has_one(self) -> None:
        await self.server.write_property_local(
            SCHED_1, P.SCHEDULE_DEFAULT, PropertyValue.real(5.0), source_object=None
        )
        await self.server.write_property_local(
            SCHED_1, P.SCHEDULE_DEFAULT, PropertyValue.null(), source_object=None
        )
        self.assertEqual(
            await self.server.read_property(SCHED_1, P.SCHEDULE_DEFAULT), PropertyValue.null()
        )


if __name__ == "__main__":
    unittest.main()
