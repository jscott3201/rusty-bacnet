"""An Access Rights object's written rules and Enable kept across a restart (#1392)."""
import asyncio
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetError, BacnetProtocolError, ErrorClass, ErrorCode,
    ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)

POSITIVE = PropertyIdentifier.POSITIVE_ACCESS_RULES
NEGATIVE = PropertyIdentifier.NEGATIVE_ACCESS_RULES
ENABLE = PropertyIdentifier.ENABLE

# BACnetAccessRule encodings: ALWAYS, ALL and disabled; ALWAYS in Access Zone 3
# of Device 99; and the rule an index-0 write appends (SPECIFIED with
# unspecified references, disabled).
ANYWHERE_OFF = bytes([0x09, 0x01, 0x29, 0x01, 0x49, 0x00])
REMOTE_LOCKDOWN = bytes(
    [0x09, 0x01, 0x29, 0x00, 0x3E, 0x0C, 0x02, 0x00, 0x00, 0x63]
    + [0x1C, 0x09, 0x00, 0x00, 0x03, 0x3F, 0x49, 0x01]
)
GROWN = bytes(
    [0x09, 0x00, 0x1E, 0x0C, 0x04, 0x7F, 0xFF, 0xFF, 0x19, 0x55, 0x1F]
    + [0x29, 0x00, 0x3E, 0x1C, 0x08, 0x7F, 0xFF, 0xFF, 0x3F, 0x49, 0x00]
)

# The same rules as a read gives them back (#1344): every key present, None
# standing for ALWAYS and ALL.
def reference(oid: ObjectIdentifier) -> dict:
    return {
        "object_identifier": oid,
        "property_identifier": PropertyIdentifier.PRESENT_VALUE,
        "property_array_index": None,
        "device_identifier": None,
    }


ANYWHERE_OFF_RULE = {"enable": False, "time_range": None, "location": None}
REMOTE_LOCKDOWN_RULE = {
    "enable": True,
    "time_range": None,
    "location": (ObjectIdentifier(ObjectType.DEVICE, 99), ObjectIdentifier(ObjectType.ACCESS_ZONE, 3)),
}
GROWN_RULE = {
    "enable": False,
    "time_range": reference(ObjectIdentifier(ObjectType.SCHEDULE, 4194303)),
    "location": ObjectIdentifier(ObjectType.ACCESS_POINT, 4194303),
}
# A rule whose location is an Access Door, which the object refuses.
DOOR_RULE = bytes([0x09, 0x01, 0x29, 0x00, 0x3E, 0x1C, 0x07, 0x80, 0x00, 0x04, 0x3F, 0x49, 0x01])

LOBBY = ObjectIdentifier(ObjectType.ACCESS_POINT, 2)


def rights_oid(instance: int) -> ObjectIdentifier:
    return ObjectIdentifier(ObjectType.ACCESS_RIGHTS, instance)


def lobby_rule() -> dict:
    """A configured rule: any time, at the lobby Access Point."""
    return {"location": LOBBY, "enable": True}


class AccessRightsRegistrationTests(unittest.TestCase):
    def test_an_empty_storage_path_is_refused(self) -> None:
        server = BACnetServer(9881)
        with self.assertRaisesRegex(BacnetError, "path must not be empty"):
            server.add_access_rights(1, "Empty path", storage_path="")
        self.assertEqual(server._pending_registration_count(), 0)

    def test_storage_path_is_a_str_naming_a_file_this_backend_wrote(self) -> None:
        server = BACnetServer(9882)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "rights-1"
            with self.assertRaises(TypeError):
                server.add_access_rights(1, "Path object", storage_path=path)
            path.write_bytes(b"not an access rights file")
            with self.assertRaisesRegex(BacnetError, "has no valid header"):
                server.add_access_rights(1, "Corrupt file", storage_path=str(path))
            # A file in this format, for this object, holding a rule the
            # object refuses: the magic tag, Access Rights 1, then the
            # positive rules framed by context tag 0.
            header = b"RBNACR01" + bytes([0x08, 0x80, 0x00, 0x01])
            path.write_bytes(header + b"\x0e" + DOOR_RULE + b"\x0f")
            with self.assertRaises(BacnetProtocolError) as raised:
                server.add_access_rights(1, "Refused rule", storage_path=str(path))
            self.assertEqual(
                raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw()
            )
        self.assertEqual(server._pending_registration_count(), 0)


class AccessRightsRestartTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name) / "state"
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    async def start(self) -> BACnetServer:
        """A server with Access Rights 1 kept in the state directory and 2 in
        memory, both configured with the lobby rule and Enable TRUE."""
        server = BACnetServer(9883, interface="127.0.0.1", port=0)
        server.add_access_rights(
            1,
            "Kept",
            positive_access_rules=[lobby_rule()],
            enable=True,
            storage_path=str(self.state / "rights-1"),
        )
        server.add_access_rights(2, "In memory", positive_access_rules=[lobby_rule()])
        await server.start()
        self.address = await server.local_address()
        return server

    async def read(self, instance: int, prop: PropertyIdentifier, index=None):
        value = await asyncio.wait_for(
            self.client.read_property(self.address, rights_oid(instance), prop, index), 3
        )
        return value.value

    async def write(
        self, instance: int, prop: PropertyIdentifier, value: PropertyValue, index=None
    ) -> None:
        await asyncio.wait_for(
            self.client.write_property(
                self.address, rights_oid(instance), prop, value, array_index=index
            ),
            3,
        )

    async def test_written_rules_and_enable_survive_a_restart(self) -> None:
        server = await self.start()
        try:
            configured = await self.read(1, POSITIVE)
            for instance in (1, 2):
                # A whole array, an index-0 resize, and Enable.
                await self.write(
                    instance, POSITIVE,
                    PropertyValue.application_data(ANYWHERE_OFF + REMOTE_LOCKDOWN),
                )
                await self.write(instance, NEGATIVE, PropertyValue.unsigned(2), 0)
                await self.write(instance, ENABLE, PropertyValue.boolean(False))
            # One WritePropertyMultiple writes an element of each array; each
            # attempt is (property, value, priority, index).
            await asyncio.wait_for(
                self.client.write_property_multiple(
                    self.address,
                    [(rights_oid(1), [
                        (POSITIVE, PropertyValue.application_data(ANYWHERE_OFF), None, 2),
                        (NEGATIVE, PropertyValue.application_data(REMOTE_LOCKDOWN), None, 1),
                    ])],
                ),
                3,
            )
            self.assertEqual(await self.read(1, POSITIVE), [ANYWHERE_OFF_RULE, ANYWHERE_OFF_RULE])
        finally:
            await server.stop()

        server = await self.start()
        try:
            # The kept object serves what was written, over the configuration
            # it was registered with again.
            self.assertEqual(await self.read(1, POSITIVE), [ANYWHERE_OFF_RULE, ANYWHERE_OFF_RULE])
            self.assertEqual(await self.read(1, NEGATIVE), [REMOTE_LOCKDOWN_RULE, GROWN_RULE])
            self.assertIs(await self.read(1, ENABLE), False)
            # The other starts from its configuration.
            self.assertEqual(await self.read(2, POSITIVE), configured)
            self.assertEqual(await self.read(2, NEGATIVE, 0), 0)
            self.assertIs(await self.read(2, ENABLE), True)
        finally:
            await server.stop()

        # Object 1's file names object 1, so another object can't share it.
        other = BACnetServer(9884)
        with self.assertRaisesRegex(BacnetError, "belongs to another object"):
            other.add_access_rights(3, "Shares a path", storage_path=str(self.state / "rights-1"))

    async def test_a_write_that_cannot_be_saved_is_refused_and_the_old_rules_stay(self) -> None:
        server = await self.start()
        try:
            await self.write(1, POSITIVE, PropertyValue.application_data(ANYWHERE_OFF))
            # A file where the storage directory belongs makes the next save
            # fail, and the write that needed it is refused.
            (self.state / "rights-1").unlink()
            self.state.rmdir()
            self.state.write_bytes(b"")
            for prop, value, index in (
                (POSITIVE, PropertyValue.application_data(REMOTE_LOCKDOWN), None),
                (POSITIVE, PropertyValue.unsigned(3), 0),
                (ENABLE, PropertyValue.boolean(False), None),
            ):
                with self.subTest(prop=prop, index=index):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await self.write(1, prop, value, index)
                    self.assertEqual(raised.exception.error_class, ErrorClass.DEVICE.to_raw())
                    self.assertEqual(
                        raised.exception.error_code, ErrorCode.OPERATIONAL_PROBLEM.to_raw()
                    )
            self.assertEqual(await self.read(1, POSITIVE), [ANYWHERE_OFF_RULE])
            self.assertIs(await self.read(1, ENABLE), True)
            # Object 2 keeps its rules in memory, so the same write succeeds.
            await self.write(2, POSITIVE, PropertyValue.application_data(REMOTE_LOCKDOWN))
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
