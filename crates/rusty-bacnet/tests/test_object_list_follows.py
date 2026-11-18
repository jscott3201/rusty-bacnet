"""The Device's Object_List and Database_Revision follow the objects the
server holds (#2669).

The list starts as the server's start built it, the Device first and then
each object in the order it was added, and a peer's CreateObject and
DeleteObject change it while the server runs. Before #2669 a created object
was missing from the list, a deleted one stayed in it, and the revision
stayed 0. Both a peer's read and the server's own read see the same list."""
import asyncio
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier,
    PropertyValue,
)

DEVICE = ObjectIdentifier(ObjectType.DEVICE, 2669)
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
BI1 = ObjectIdentifier(ObjectType.BINARY_INPUT, 1)
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
LIST = PropertyIdentifier.OBJECT_LIST
REVISION = PropertyIdentifier.DATABASE_REVISION


def listed(*objects):
    return PropertyValue.list([PropertyValue.object_identifier(o) for o in objects])


class ObjectListFollowsTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.server = BACnetServer(2669, interface="127.0.0.1", port=0)
        self.server.add_analog_input(1, "AI-1", units=62)
        self.server.add_binary_input(1, "BI-1")
        await self.server.start()

        async def stop_server():
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0)
        await self.client.__aenter__()

        async def stop_client():
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    async def expect(self, objects, revision):
        """Both the peer and the server read `objects` whole, its count at
        index 0 and its last entry by index, and `revision`."""
        count = len(objects)
        for read in (self.remote, self.local):
            self.assertEqual(await read(LIST), listed(*objects))
            self.assertEqual(await read(LIST, 0), PropertyValue.unsigned(count))
            self.assertEqual(
                await read(LIST, count), PropertyValue.object_identifier(objects[-1])
            )
            self.assertEqual(await read(REVISION), PropertyValue.unsigned(revision))

    async def remote(self, property_id, index=None):
        return await asyncio.wait_for(
            self.client.read_property(self.address, DEVICE, property_id, index), 3
        )

    async def local(self, property_id, index=None):
        return await self.server.read_property(DEVICE, property_id, index)

    async def test_a_peers_create_and_delete_move_the_list_and_revision(self):
        # As the start built it: the Device, then the objects as added.
        await self.expect([DEVICE, AI1, BI1], 3)

        created = await asyncio.wait_for(
            self.client.create_object(self.address, ObjectType.ANALOG_VALUE), 3
        )
        self.assertEqual(created, AV1)
        await self.expect([DEVICE, AI1, BI1, AV1], 4)

        await asyncio.wait_for(self.client.delete_object(self.address, BI1), 3)
        await self.expect([DEVICE, AI1, AV1], 5)


if __name__ == "__main__":
    unittest.main()
