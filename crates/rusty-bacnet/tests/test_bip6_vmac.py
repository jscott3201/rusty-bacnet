"""B/IPv6 peers are named by VMAC (#1671).

A B/IPv6 node is addressed by its VMAC (Clause U.1.3), which is its device
instance when it has one (Clause H.7.2): ``bip6:1234`` or ``bip6:0x0004D2``.
An ``[ip]:port`` string is refused; ``resolve_vmac`` finds the VMAC of the
node at such an address with a unicast Virtual-Address-Resolution (Clause
U.5).

A node with no instance picks a random VMAC and must probe the multicast
group for it at start, which Linux loopback can't carry. So the pair on
``::1`` gives the client an instance too, ``ipv6_device_instance``, which
lets the test run on Linux CI.
"""
import asyncio
import errno
import socket
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetTransportError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)

DEVICE = ObjectIdentifier(ObjectType.DEVICE, 1234)
NAME = PropertyIdentifier.OBJECT_NAME
# The test client's own instance, so it starts without a multicast probe.
CLIENT_INSTANCE = 4321


def ipv6_loopback() -> bool:
    """True when this host can bind a UDP socket on ``::1``."""
    if not socket.has_ipv6:
        return False
    try:
        with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as probe:
            probe.bind(("::1", 0))
    except OSError:
        return False
    return True


def free_udp6_port() -> int:
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as probe:
        probe.bind(("::1", 0))
        return probe.getsockname()[1]


def loopback_client() -> BACnetClient:
    return BACnetClient(transport="ipv6", ipv6_interface="::1", port=0,
                        ipv6_device_instance=CLIENT_INSTANCE)


class Bip6AddressFormTests(unittest.IsolatedAsyncioTestCase):
    async def test_vmac_forms_parse_before_the_lifecycle_check(self) -> None:
        client = BACnetClient(transport="ipv6")
        for address in ("bip6:1234", "bip6:0x40ABCD", "bip6:0x0004d2",
                        "bip6:0X0004D2", "bip6:4194303", "00:04:d2"):
            with self.subTest(address=address), self.assertRaisesRegex(
                RuntimeError, "client not started"
            ):
                await client.read_property(address, DEVICE, NAME)

    async def test_an_instance_past_4194303_is_refused(self) -> None:
        client = BACnetClient(transport="ipv6")
        for address in ("bip6:4194304", "bip6:99999999999999999999"):
            with self.subTest(address=address), self.assertRaisesRegex(
                ValueError, r"^B/IPv6 device instance must be in 0\.\.=4194303, not \d+$"
            ):
                await client.read_property(address, DEVICE, NAME)

    async def test_malformed_vmac_forms_are_refused(self) -> None:
        client = BACnetClient(transport="ipv6")
        for address in ("bip6:", "bip6:-1", "bip6:0x", "bip6:0x0004D",
                        "bip6:0x0004D2F", "bip6:0xGG0000"):
            with self.subTest(address=address), self.assertRaisesRegex(
                ValueError, r"bip6:0x<6 hex digits>"
            ):
                await client.read_property(address, DEVICE, NAME)

    async def test_only_an_ipv6_client_takes_bip6_vmacs(self) -> None:
        for transport in ("bip", "mstp"):
            client = BACnetClient(transport=transport)
            with self.subTest(transport=transport), self.assertRaisesRegex(
                ValueError, r"bip6: addresses need transport='ipv6'"
            ):
                await client.read_property("bip6:1234", DEVICE, NAME)

    async def test_a_bracketed_address_points_to_resolve_vmac(self) -> None:
        client = BACnetClient(transport="ipv6")
        for address in ("[::1]:47808", "[fe80::1%3]:47808", "[::1]"):
            with self.subTest(address=address), self.assertRaises(
                ValueError
            ) as caught:
                await client.read_property(address, DEVICE, NAME)
            message = str(caught.exception)
            self.assertIn("'bip6:<device instance>'", message)
            self.assertIn("resolve_vmac", message)
        # Every call that takes an address shares the parser.
        server = BACnetServer(1)
        with self.assertRaisesRegex(ValueError, "resolve_vmac"):
            server.add_device_binding(2, "[::1]:47808")

    async def test_resolve_vmac_needs_a_started_ipv6_client(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "client not started"):
            await BACnetClient(transport="ipv6").resolve_vmac("[::1]:47808")
        for address in ("bip6:1234", "::1", "[::1]"):
            with self.subTest(address=address), self.assertRaisesRegex(
                ValueError, r"'\[ip\]:port'"
            ):
                await BACnetClient(transport="ipv6").resolve_vmac(address)
        async with BACnetClient(interface="127.0.0.1", port=0) as bip:
            with self.assertRaisesRegex(RuntimeError, "transport='ipv6'"):
                await bip.resolve_vmac("[::1]:47808")

    def test_ipv6_device_instance_is_checked_at_construction(self) -> None:
        for instance in (0, 4194303):
            BACnetClient(transport="ipv6", ipv6_device_instance=instance)
        with self.assertRaisesRegex(ValueError, r"0\.\.=4194303, not 4194304$"):
            BACnetClient(transport="ipv6", ipv6_device_instance=4194304)
        for transport in ("bip", "sc", "mstp"):
            with self.subTest(transport=transport), self.assertRaisesRegex(
                ValueError, "applies only to transport='ipv6'"
            ):
                BACnetClient(transport=transport, ipv6_device_instance=1)
        for instance in (-1, 2**32):
            with self.subTest(instance=instance), self.assertRaises(OverflowError):
                BACnetClient(transport="ipv6", ipv6_device_instance=instance)


@unittest.skipUnless(ipv6_loopback(), "this host has no IPv6 loopback")
class Bip6PairTests(unittest.IsolatedAsyncioTestCase):
    async def test_server_vmac_is_its_instance_and_resolves_from_its_address(
        self,
    ) -> None:
        port = free_udp6_port()
        server = BACnetServer(1234, device_name="B/IPv6 server", transport="ipv6",
                              ipv6_interface="::1", port=port)
        await asyncio.wait_for(server.start(), 10)
        try:
            self.assertEqual(await server.local_address(), "bip6:0x0004D2")
            async with loopback_client() as client:
                vmac = await asyncio.wait_for(
                    client.resolve_vmac(f"[::1]:{port}"), 10)
                self.assertEqual(vmac, "bip6:0x0004D2")
                for address in (vmac, "bip6:1234", "00:04:d2"):
                    with self.subTest(address=address):
                        value = await asyncio.wait_for(
                            client.read_property(address, DEVICE, NAME), 10)
                        self.assertEqual(value.value, "B/IPv6 server")
        finally:
            await server.stop()

    async def test_the_client_answers_by_its_instance_vmac(self) -> None:
        """A client's VMAC is its ipv6_device_instance (H.7.2): a node that
        resolves the client's address gets bip6:0x0010E2 for instance 4322."""
        async with loopback_client() as first:
            port = free_udp6_port()
            async with BACnetClient(transport="ipv6", ipv6_interface="::1", port=port,
                                    ipv6_device_instance=CLIENT_INSTANCE + 1):
                vmac = await asyncio.wait_for(
                    first.resolve_vmac(f"[::1]:{port}"), 10)
        self.assertEqual(vmac, "bip6:0x0010E2")

    async def test_resolve_vmac_times_out_with_a_transport_error(self) -> None:
        with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as silent:
            silent.bind(("::1", 0))
            port = silent.getsockname()[1]
            async with loopback_client() as client:
                with self.assertRaises(BacnetTransportError) as caught:
                    await asyncio.wait_for(
                        client.resolve_vmac(f"[::1]:{port}"), 10)
        self.assertEqual(caught.exception.errno, errno.ETIMEDOUT)
        self.assertIn("Virtual-Address-Resolution", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
