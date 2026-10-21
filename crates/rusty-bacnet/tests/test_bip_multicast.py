"""B/IP multicast from Python (#941): the multicast= keyword, on loopback."""
import asyncio
import socket
import sys
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, Bbmd, BipEndpoint, ForeignDevice, Multicast,
    ObjectIdentifier, ObjectType, PropertyIdentifier as P, PropertyValue as V,
)

GROUP = "239.255.97.1"


def free_port():
    """A UDP port no socket holds now, for a group."""
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("0.0.0.0", 0))
        return probe.getsockname()[1]


class OptionTests(unittest.TestCase):
    def test_multicast_keeps_its_settings(self):
        group = Multicast(GROUP, ttl=4, port=47809)
        self.assertEqual((group.group, group.ttl, group.port), (GROUP, 4, 47809))
        default = Multicast(GROUP)
        self.assertEqual((default.ttl, default.port), (1, None))
        self.assertIn(GROUP, repr(group))
        # A value: equal settings compare and hash equal.
        self.assertEqual(group, Multicast(GROUP, ttl=4, port=47809))
        self.assertNotEqual(group, Multicast(GROUP, ttl=4))
        self.assertEqual(hash(group), hash(Multicast(GROUP, ttl=4, port=47809)))
        self.assertNotEqual(group, GROUP)

    def test_multicast_refuses_bad_settings(self):
        for bad in ("10.0.0.1", "255.255.255.255", "224.0.0.1", "not an address"):
            with self.subTest(group=bad), self.assertRaises(ValueError):
                Multicast(bad)
        with self.assertRaises(ValueError):
            Multicast(GROUP, ttl=0)
        with self.assertRaises(OverflowError):
            Multicast(GROUP, ttl=256)
        with self.assertRaises(ValueError):
            Multicast(GROUP, port=0)

    def test_multicast_needs_b_ip_an_interface_and_no_foreign_device(self):
        group = Multicast(GROUP, port=47809)
        # The default interface is 0.0.0.0.
        with self.assertRaises(ValueError):
            BACnetClient(multicast=group)
        with self.assertRaises(ValueError):
            BACnetServer(3200, multicast=group)
        with self.assertRaises(ValueError):
            BipEndpoint(3200, multicast=group)
        with self.assertRaises(ValueError):
            BACnetClient(interface="127.0.0.1", transport="ipv6", multicast=group)
        with self.assertRaises(ValueError):
            BACnetServer(3200, interface="127.0.0.1",
                         foreign_device=ForeignDevice("192.0.2.1:47808"),
                         multicast=group)
        # A BBMD serves the group (Annex J.8.3).
        BACnetServer(3200, interface="127.0.0.1", bbmd=Bbmd(), multicast=group)
        # A port shared by address, or a picked port with no group port.
        with self.assertRaises(ValueError):
            BACnetClient(interface="127.0.0.1", port=47808,
                         share_port_by_address=True, multicast=group)
        with self.assertRaises(ValueError):
            BACnetServer(3200, interface="127.0.0.1", port=47808,
                         share_port_by_address=True, multicast=group)
        with self.assertRaises(ValueError):
            BipEndpoint(3200, interface="127.0.0.1", port=47808,
                        share_port_by_address=True, multicast=group)
        no_port = Multicast(GROUP)
        with self.assertRaises(ValueError):
            BACnetClient(interface="127.0.0.1", port=0, multicast=no_port)
        with self.assertRaises(ValueError):
            BACnetServer(3200, interface="127.0.0.1", port=0, multicast=no_port)
        with self.assertRaises(ValueError):
            BipEndpoint(3200, interface="127.0.0.1", port=0, multicast=no_port)
        BACnetClient(interface="127.0.0.1", port=47808, multicast=no_port)
        BACnetClient(interface="127.0.0.1", port=0, multicast=group)
        BACnetServer(3200, interface="127.0.0.1", port=0, multicast=group)
        BipEndpoint(3200, interface="127.0.0.1", port=0, multicast=group)


@unittest.skipIf(sys.platform == "win32", "Windows multicast is untested")
class LoopbackTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_client_discovers_a_server_through_their_group(self):
        group = Multicast(GROUP, port=free_port())
        server = BACnetServer(3201, interface="127.0.0.1", port=0,
                              broadcast_address="127.255.255.255",
                              registered_network_port=1, multicast=group)
        server.add_bip_network_port(1, "member", ip_address="127.0.0.1", udp_port=0)
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    multicast=group) as client:
                # The Who-Is and the I-Am both go through the group.
                deadline = asyncio.get_running_loop().time() + 5
                device = None
                while device is None:
                    self.assertLess(asyncio.get_running_loop().time(), deadline,
                                    "the server was not discovered")
                    await client.who_is(3201, 3201)
                    await asyncio.sleep(0.2)
                    device = await client.get_device(3201)
                self.assertEqual(device.object_identifier,
                                 ObjectIdentifier(ObjectType.DEVICE, 3201))
                oid = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
                self.assertEqual(
                    await client.read_property(address, oid, P.BACNET_IP_MULTICAST_ADDRESS),
                    V.octet_string(bytes([239, 255, 97, 1])))
        finally:
            await server.stop()

    async def test_a_member_discovers_the_bbmd_that_serves_its_group(self):
        group = Multicast(GROUP, port=free_port())
        server = BACnetServer(3202, interface="127.0.0.1", port=0,
                              broadcast_address="127.255.255.255",
                              registered_network_port=1, bbmd=Bbmd(),
                              multicast=group)
        server.add_bip_network_port(1, "group BBMD", ip_address="127.0.0.1",
                                    udp_port=0)
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    multicast=group) as client:
                deadline = asyncio.get_running_loop().time() + 5
                device = None
                while device is None:
                    self.assertLess(asyncio.get_running_loop().time(), deadline,
                                    "the BBMD was not discovered")
                    await client.who_is(3202, 3202)
                    await asyncio.sleep(0.2)
                    device = await client.get_device(3202)
                oid = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
                self.assertEqual(
                    await client.read_property(address, oid, P.BACNET_IP_MULTICAST_ADDRESS),
                    V.octet_string(bytes([239, 255, 97, 1])))
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
