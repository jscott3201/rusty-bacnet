"""The B/IPv6 Network Port (#1579): a configured snapshot added with
``add_bip6_network_port``, and the same object registered with a
``transport="ipv6"`` server, which publishes its bound port and VMAC."""
import asyncio
import inspect
import ipaddress
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetProtocolError,
    ObjectIdentifier, ObjectType, PropertyIdentifier as P, PropertyValue as V,
)

from test_bip6_vmac import free_udp6_port, ipv6_loopback, loopback_client

PORT = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)


def v6(text: str) -> bytes:
    return ipaddress.IPv6Address(text).packed


class ConfiguredBip6PortTests(unittest.TestCase):
    def test_constructor_validation(self) -> None:
        server = BACnetServer(2868, transport="ipv6")
        signature = inspect.signature(server.add_bip6_network_port)
        for name in ("ip_address", "prefix_length", "multicast_address", "vmac"):
            self.assertEqual(signature.parameters[name].kind,
                             inspect.Parameter.KEYWORD_ONLY, name)
        for instance in (0, 256):
            with self.subTest(instance=instance), self.assertRaises(
                    (ValueError, BacnetProtocolError)):
                server.add_bip6_network_port(instance, "bad")
        for kwargs in ({"apdu_length": 49}, {"network_number": 65535},
                       {"dns_servers": []}, {"dns_servers": ["192.0.2.53"]},
                       {"ip_address": "10.0.0.1"}, {"default_gateway": "bad"},
                       {"prefix_length": 0}, {"prefix_length": 129},
                       {"multicast_address": "2001:db8::1"}, {"vmac": b"\x00\x01"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(
                    (ValueError, BacnetProtocolError, OverflowError)):
                server.add_bip6_network_port(1, "bad", **kwargs)
        self.assertIsNone(server.add_bip6_network_port(
            255, "valid", ip_address="2001:db8::5", udp_port=0, vmac=b"\x00\x04\xd2"))

    def test_registration_takes_b_ip_or_b_ipv6_only(self) -> None:
        BACnetServer(2868, transport="ipv6", registered_network_port=1)
        BACnetServer(2868, transport="bip", registered_network_port=1)
        for transport in ("sc", "mstp"):
            with self.subTest(transport=transport), self.assertRaisesRegex(
                    ValueError, "requires B/IP or B/IPv6"):
                BACnetServer(2868, transport=transport, registered_network_port=1)


@unittest.skipUnless(ipv6_loopback(), "this host has no IPv6 loopback")
class RegisteredBip6PortTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_registered_port_serves_the_bound_ipv6_rows(self) -> None:
        port = free_udp6_port()
        server = BACnetServer(1234, transport="ipv6", ipv6_interface="::1", port=port,
                              registered_network_port=1)
        server.add_bip6_network_port(
            1, "B/IPv6", ip_address="::1", udp_port=port, network_number=42,
            prefix_length=128, dns_servers=["2001:db8::53"])
        await asyncio.wait_for(server.start(), 10)
        try:
            async with loopback_client() as client:
                address = await asyncio.wait_for(client.resolve_vmac(f"[::1]:{port}"), 10)
                group = v6("ff05::bac0") + port.to_bytes(2, "big")
                for prop, expected in (
                    (P.NETWORK_TYPE, V.enumerated(9)),
                    (P.BACNET_IPV6_MODE, V.enumerated(0)),
                    (P.IPV6_ADDRESS, V.octet_string(v6("::1"))),
                    (P.IPV6_PREFIX_LENGTH, V.unsigned(128)),
                    (P.BACNET_IPV6_UDP_PORT, V.unsigned(port)),
                    (P.IPV6_DEFAULT_GATEWAY, V.octet_string(bytes(16))),
                    (P.BACNET_IPV6_MULTICAST_ADDRESS, V.octet_string(group)),
                    # The VMAC the server uses: its device instance.
                    (P.MAC_ADDRESS, V.octet_string(bytes([0x00, 0x04, 0xD2]))),
                    (P.NETWORK_NUMBER, V.unsigned(42)),
                    (P.CHANGES_PENDING, V.boolean(False)),
                ):
                    with self.subTest(property=prop):
                        read = await asyncio.wait_for(
                            client.read_property(address, PORT, prop), 10)
                        self.assertEqual(read, expected)
                        with self.assertRaises(BacnetProtocolError):
                            await client.write_property(address, PORT, prop, expected)
                self.assertEqual(
                    await client.read_property(address, PORT, P.IPV6_DNS_SERVER,
                                               array_index=1),
                    V.octet_string(v6("2001:db8::53")))
                for prop in (P.BACNET_IP_MODE, P.IP_ADDRESS, P.FD_BBMD_ADDRESS):
                    with self.subTest(absent=prop), self.assertRaises(BacnetProtocolError):
                        await client.read_property(address, PORT, prop)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
