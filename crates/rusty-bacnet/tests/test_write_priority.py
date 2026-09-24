"""Installed WP validation is synchronous and a mixed batch dispatches no prefix."""
import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue

OID = ObjectIdentifier(ObjectType.DEVICE, 9123)
PID = PropertyIdentifier.DESCRIPTION
VALUE = PropertyValue.character_string("accepted")


def calls(client, priority, address="invalid-address"):
    return (
        lambda: client.write_property(address, OID, PID, VALUE, priority=priority),
        lambda: client.write_property_to_device(9123, OID, PID, VALUE, priority=priority),
        lambda: client.write_property_to_devices([
            (9123, OID, PID, VALUE, None, None),
            (9123, OID, PID, VALUE, priority, None),
        ]),
    )


class WritePriorityTests(unittest.IsolatedAsyncioTestCase):
    async def test_invalid_priority_all_entrypoints_synchronously_before_start(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for priority in (0, 17, 255):
            for index, call in enumerate(calls(client, priority)):
                with self.subTest(priority=priority, entrypoint=index):
                    with self.assertRaises(ValueError):
                        call()
        # Native u8 extraction remains separate from the priority domain.
        for priority in (-1, 256):
            for call in calls(client, priority):
                with self.assertRaises(OverflowError):
                    call()

    async def test_mixed_invalid_batch_has_no_valid_prefix_traffic(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                address = f"127.0.0.1:{peer.getsockname()[1]}"
                await client.add_device(9123, address)
                for priority in (0, 17, 255):
                    for call in calls(client, priority, address):
                        with self.assertRaises(ValueError):
                            call()
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), .1)

    async def test_all_valid_priorities_noncommandable_and_null_unchanged(self):
        server = BACnetServer(9123, interface="127.0.0.1", port=0)
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                address = await server.local_address()
                await client.add_device(9123, address)
                for priority in (None, *range(1, 17)):
                    for index, call in enumerate(calls(client, priority, address)):
                        with self.subTest(priority=priority, entrypoint=index):
                            result = await asyncio.wait_for(call(), 3)
                            if index == 2:
                                self.assertTrue(all(item["error"] is None for item in result), result)
                            self.assertEqual(await client.read_property(address, OID, PID), VALUE)
                    # Noncommandable NULL is a successful no-op, even with supplied priority.
                    await client.write_property(address, OID, PID, PropertyValue.null(), priority=priority)
                    await client.write_property_to_device(9123, OID, PID, PropertyValue.null(), priority=priority)
                    result = await client.write_property_to_devices([(9123, OID, PID, PropertyValue.null(), priority, None)])
                    self.assertIsNone(result[0]["error"])
                    self.assertEqual(await client.read_property(address, OID, PID), VALUE)
        finally:
            await server.stop()
