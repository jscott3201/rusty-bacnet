"""Two addresses claiming one device instance (#2560): the discovery table
lists both, each answers by its address, and a call that names the instance
raises BacnetAmbiguousDeviceError naming both."""
import asyncio
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetAmbiguousDeviceError,
    BacnetError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)

INSTANCE = 702_560


def bip_mac(address: str) -> bytes:
    """The B/IP MAC of an ``ip:port`` address: the IPv4 octets and the port."""
    host, port = address.rsplit(":", 1)
    return bytes(int(octet) for octet in host.split(".")) + int(port).to_bytes(2, "big")


class DuplicateInstanceTests(unittest.IsolatedAsyncioTestCase):
    async def start_server(self) -> str:
        server = BACnetServer(INSTANCE, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        await server.start()

        async def stop() -> None:
            await server.stop()

        self.addAsyncCleanup(stop)
        return await server.local_address()

    async def test_both_claimants_are_listed_reachable_and_refused_by_instance(self) -> None:
        addresses = [await self.start_server() for _ in range(2)]
        expected = sorted((0, bip_mac(address)) for address in addresses)
        device = ObjectIdentifier(ObjectType.DEVICE, INSTANCE)
        name = PropertyIdentifier.OBJECT_NAME
        async with BACnetClient(interface="127.0.0.1", port=0,
                                apdu_timeout_ms=2000) as client:
            for address in addresses:
                await client.who_is_directed(address)
            for _ in range(250):
                devices = await client.discovered_devices()
                if len(devices) == 2:
                    break
                await asyncio.sleep(0.02)
            self.assertEqual(sorted((0, bytes(d.mac_address)) for d in devices), expected)
            for found in devices:
                self.assertEqual(found.object_identifier, device)

            for address in addresses:
                await client.read_property(address, device, name)

            with self.assertRaises(BacnetAmbiguousDeviceError) as caught:
                await client.read_property_from_device(INSTANCE, device, name)
            self.assertIsInstance(caught.exception, BacnetError)
            self.assertEqual(caught.exception.instance, INSTANCE)
            self.assertEqual(sorted(caught.exception.candidates), expected)
            with self.assertRaises(BacnetAmbiguousDeviceError):
                await client.get_device(INSTANCE)


if __name__ == "__main__":
    unittest.main()
