"""share_port_by_address: devices on two addresses of one host share a port (#1538)."""
import asyncio
import socket
import sys
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, ObjectIdentifier, ObjectType,
    PropertyIdentifier as P, PropertyValue as V,
)


def two_local_addresses():
    """Two local addresses to bind, or None. Linux answers for all of
    127.0.0.0/8; macOS and Windows pair 127.0.0.1 with the default-route
    address."""
    if sys.platform.startswith("linux"):
        return "127.0.0.2", "127.0.0.3"
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        try:
            probe.connect(("8.8.8.8", 80))
        except OSError:
            return None
        route = probe.getsockname()[0]
    return None if route.startswith("127.") else ("127.0.0.1", route)


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("0.0.0.0", 0))
        return probe.getsockname()[1]


class SharePortByAddressTests(unittest.IsolatedAsyncioTestCase):
    def test_only_b_ip_takes_it(self):
        with self.assertRaises(ValueError):
            BACnetServer(1101, transport="ipv6", share_port_by_address=True)
        with self.assertRaises(ValueError):
            BACnetClient(transport="ipv6", share_port_by_address=True)

    async def test_the_wildcard_interface_refuses_it(self):
        port = free_port()
        server = BACnetServer(1102, interface="0.0.0.0", port=port, share_port_by_address=True)
        with self.assertRaises(Exception):
            await asyncio.wait_for(server.start(), 10)
        endpoint = BipEndpoint(1103, interface="0.0.0.0", port=port, share_port_by_address=True)
        with self.assertRaises(Exception):
            await asyncio.wait_for(endpoint.start(), 10)

    async def test_two_servers_share_a_port_and_each_answers_on_its_address(self):
        addresses = two_local_addresses()
        if addresses is None:
            self.skipTest("no second local address to share the port with")
        port = free_port()
        servers = [
            BACnetServer(1110 + i, device_name=f"Shared {i}", interface=ip, port=port,
                         share_port_by_address=True)
            for i, ip in enumerate(addresses)
        ]
        for server in servers:
            await asyncio.wait_for(server.start(), 10)
        try:
            async with BACnetClient(interface="0.0.0.0", port=0) as client:
                for i, ip in enumerate(addresses):
                    device = ObjectIdentifier(ObjectType.DEVICE, 1110 + i)
                    name = await asyncio.wait_for(
                        client.read_property(f"{ip}:{port}", device, P.OBJECT_NAME), 10)
                    self.assertEqual(name, V.character_string(f"Shared {i}"))
        finally:
            for server in servers:
                await asyncio.wait_for(server.stop(), 10)


if __name__ == "__main__":
    unittest.main()
