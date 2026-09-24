"""Installed WPM preflight is whole-request, synchronous and precedes I/O."""
import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue

OID = ObjectIdentifier(ObjectType.DEVICE, 9123)
VALUE = PropertyValue.character_string("accepted")


def specs(priority=None):
    return [(OID, [(PropertyIdentifier.DESCRIPTION, VALUE, priority, None)])]


def invalid_requests():
    yield []
    yield [(OID, [])]
    yield specs() + [(OID, [])]
    for priority in (0, 17, 255):
        yield specs() + specs(priority)
    for selector in (PropertyIdentifier.ALL, PropertyIdentifier.REQUIRED, PropertyIdentifier.OPTIONAL):
        yield specs() + [(OID, [(selector, PropertyValue.null(), None, None)])]


def calls(client, request, address="invalid-address"):
    return (
        lambda: client.write_property_multiple(address, request),
        lambda: client.write_property_multiple_to_device(9123, request),
    )


class WpmValidationTests(unittest.IsolatedAsyncioTestCase):
    async def test_invalid_whole_request_synchronously_before_start_or_address(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for case, request in enumerate(invalid_requests()):
            for method, call in enumerate(calls(client, request)):
                with self.subTest(case=case, method=method):
                    with self.assertRaises(ValueError):
                        call()
        for priority in (-1, 256):
            for call in calls(client, specs(priority)):
                with self.assertRaises(OverflowError):
                    call()

    async def test_invalid_later_write_dispatches_no_prefix_or_discovery(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, address)
                for request in invalid_requests():
                    for call in calls(client, request, address):
                        with self.assertRaises(ValueError):
                            call()
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), .1)

    async def test_valid_priorities_and_noncommandable_null_keep_state(self):
        server = BACnetServer(9123, interface="127.0.0.1", port=0)
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                address = await server.local_address()
                await client.add_device(9123, address)
                for priority in (None, *range(1, 17)):
                    for call in calls(client, specs(priority), address):
                        await asyncio.wait_for(call(), 3)
                    null = [(OID, [(PropertyIdentifier.DESCRIPTION, PropertyValue.null(), priority, None)])]
                    for call in calls(client, null, address):
                        await asyncio.wait_for(call(), 3)
                    self.assertEqual(await client.read_property(address, OID, PropertyIdentifier.DESCRIPTION), VALUE)
        finally:
            await server.stop()

    async def test_proprietary_index_zero_null_and_empty_list_payload_exact_wire(self):
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.add_device(9123, address)
                for value, encoded in ((PropertyValue.null(), b"\0"), (PropertyValue.list([]), b"")):
                    request = [(OID, [(PropertyIdentifier.from_raw(600), value, 16, 0)])]
                    for call in calls(client, request, address):
                        operation = call()
                        packet, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                        self.assertEqual(packet[:2], b"\x81\x0a")
                        self.assertEqual(packet[4:6], b"\x01\x04")
                        self.assertEqual(packet[6] & 0xf0, 0)
                        self.assertEqual(packet[9], 16)  # WritePropertyMultiple
                        self.assertEqual(packet[10:], b"\x0c\x02\x00\x23\xa3\x1e\x0a\x02\x58\x19\x00\x2e" + encoded + b"\x2f\x39\x10\x1f")
                        reply = b"\x81\x0a\x00\x09\x01\x00\x20" + bytes((packet[8], 16))
                        await loop.sock_sendto(peer, reply, sender)
                        await asyncio.wait_for(operation, 2)
