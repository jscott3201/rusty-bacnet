"""B/IPv6 foreign-device mode from Python (#1941): ``foreign_device=`` with a
B/IPv6 BBMD on ``transport="ipv6"``.

A ``ForeignDevice`` names a B/IPv6 BBMD as ``"ip"`` or ``"[ip]:port"``. The
BBMD here is a plain UDP socket on ``::1`` that answers each
Register-Foreign-Device (Clause U.4.5) with a successful BVLC-Result, and
records the time-to-live each one asks for.
"""
import asyncio
import socket
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, Bbmd, BipEndpoint, ForeignDevice, ObjectIdentifier,
    ObjectType, PropertyIdentifier as P, PropertyValue as V, ReinitializedState as S,
)

from test_bip6_vmac import CLIENT_INSTANCE, free_udp6_port, ipv6_loopback, loopback_client

PORT = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
INSTANCE = 1941
REGISTER = b"\x82\x09\x00\x09"
DISTRIBUTE_BROADCAST = b"\x82\x0c"
# BVLC-Result, successful completion, from the BBMD's VMAC.
RESULT_OK = b"\x82\x00\x00\x09\x0b\xbb\xbb\x00\x00"


def vmac(instance: int) -> bytes:
    """The VMAC a device instance gives a B/IPv6 node (Clause H.7.2)."""
    return instance.to_bytes(3, "big")


class FakeBbmd:
    """A B/IPv6 BBMD on ``::1`` that accepts every registration."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
        self.sock.bind(("::1", 0))
        self.sock.setblocking(False)

    def close(self):
        self.sock.close()

    @property
    def port(self) -> int:
        return self.sock.getsockname()[1]

    @property
    def address(self) -> str:
        return f"[::1]:{self.port}"

    def host_n_port(self) -> bytes:
        """This BBMD as an encoded BACnetHostNPort: [0] { ip-address [1] }
        port [1]."""
        ip = socket.inet_pton(socket.AF_INET6, "::1")
        return b"\x0e\x1d\x10" + ip + b"\x0f\x1a" + self.port.to_bytes(2, "big")

    async def next(self, prefix: bytes, seconds: float = 10.0):
        """The next datagram starting with ``prefix`` and its sender, polled
        since Windows' default event loop has no add_reader."""
        async with asyncio.timeout(seconds):
            while True:
                try:
                    datagram, sender = self.sock.recvfrom(2048)
                except BlockingIOError:
                    await asyncio.sleep(0.01)
                    continue
                if datagram.startswith(prefix):
                    return datagram, sender

    async def registration(self):
        """The VMAC and time-to-live of the next registration, answered."""
        datagram, sender = await self.next(REGISTER)
        self.sock.sendto(RESULT_OK, sender[:2])
        return datagram[4:7], int.from_bytes(datagram[7:9], "big")


class OptionTests(unittest.TestCase):
    def test_a_b_ipv6_bbmd_keeps_its_settings(self):
        foreign = ForeignDevice("2001:db8::bb", ttl=600)
        self.assertEqual((foreign.bbmd, foreign.ttl, foreign.renewal),
                         ("[2001:db8::bb]:47808", 600, None))
        self.assertEqual(ForeignDevice("[::1]:47809").bbmd, "[::1]:47809")
        # Brackets without a port take the default one.
        self.assertEqual(ForeignDevice("[::1]").bbmd, "[::1]:47808")
        # Without brackets a trailing group is part of the address.
        self.assertEqual(ForeignDevice("2001:db8::10:4780").bbmd,
                         "[2001:db8::10:4780]:47808")
        self.assertEqual(ForeignDevice("[fd00::bb]:47809").ttl, 300)
        self.assertEqual(repr(ForeignDevice("::1", ttl=60)),
                         "ForeignDevice(bbmd='[::1]:47808', ttl=60, renewal=None)")

    def test_each_refusal_says_why(self):
        for bbmd, reason in (("::", "unspecified"), ("[::]:47808", "unspecified"),
                             ("ff05::bac0", "multicast"), ("fe80::1", "link-local"),
                             ("[fe80::1]:47808", "link-local"),
                             ("::ffff:192.0.2.1", "IPv4-mapped"),
                             ("::192.0.2.1", "IPv4-compatible"),
                             ("[::1]:0", "port is 0")):
            with self.subTest(bbmd=bbmd), self.assertRaisesRegex(
                    ValueError, rf"IPv6 unicast host.*{reason}"):
                ForeignDevice(bbmd)
        with self.assertRaisesRegex(ValueError, "ttl must be at least 1"):
            ForeignDevice("::1", ttl=0)
        with self.assertRaisesRegex(ValueError, "renewal applies only to a B/IP BBMD"):
            ForeignDevice("::1", ttl=60, renewal=10)
        for bbmd in ("[fe80::1%3]:47808", "fe80::1%3", "[::1]:47808x", "::1::2"):
            with self.subTest(bbmd=bbmd), self.assertRaises(ValueError):
                ForeignDevice(bbmd)
        with self.assertRaisesRegex(ValueError, "not an IPv4 or IPv6 address"):
            ForeignDevice("nowhere")

    def test_the_bbmd_family_must_match_the_transport(self):
        ipv4, ipv6 = ForeignDevice("192.0.2.10"), ForeignDevice("::1")
        with self.assertRaisesRegex(ValueError, "transport='ipv6' needs a B/IPv6 BBMD"):
            BACnetServer(INSTANCE, transport="ipv6", foreign_device=ipv4)
        with self.assertRaisesRegex(ValueError, "transport='ipv6' needs a B/IPv6 BBMD"):
            BACnetClient(transport="ipv6", ipv6_device_instance=1, foreign_device=ipv4)
        with self.assertRaisesRegex(ValueError, "transport='bip' needs an IPv4 BBMD"):
            BACnetServer(INSTANCE, foreign_device=ipv6)
        with self.assertRaisesRegex(ValueError, "transport='bip' needs an IPv4 BBMD"):
            BACnetClient(foreign_device=ipv6)
        with self.assertRaisesRegex(ValueError, "transport='bip' needs an IPv4 BBMD"):
            BipEndpoint(INSTANCE, foreign_device=ipv6)
        for transport in ("sc", "mstp"):
            with self.subTest(transport=transport), self.assertRaisesRegex(
                    ValueError, "foreign_device applies only to transport='bip' or 'ipv6'"):
                BACnetServer(INSTANCE, transport=transport, foreign_device=ipv6)
        with self.assertRaisesRegex(ValueError, "bbmd applies only to transport='bip'"):
            BACnetServer(INSTANCE, transport="ipv6", bbmd=Bbmd())
        # The matching family is accepted.
        BACnetServer(INSTANCE, transport="ipv6", foreign_device=ipv6)
        BACnetClient(transport="ipv6", ipv6_device_instance=1, foreign_device=ipv6)

    def test_an_ipv6_foreign_client_needs_a_device_instance(self):
        with self.assertRaisesRegex(ValueError, "needs ipv6_device_instance"):
            BACnetClient(transport="ipv6", foreign_device=ForeignDevice("::1"))


@unittest.skipUnless(ipv6_loopback(), "this host has no IPv6 loopback")
class LoopbackTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.bbmd = FakeBbmd()

    async def asyncTearDown(self):
        self.bbmd.close()

    async def test_a_foreign_server_registers_and_reports_foreign_mode(self):
        port = free_udp6_port()
        server = BACnetServer(INSTANCE, transport="ipv6", ipv6_interface="::1", port=port,
                              registered_network_port=1,
                              foreign_device=ForeignDevice(self.bbmd.address, ttl=60))
        server.add_bip6_network_port(1, "B/IPv6 foreign", ip_address="::1", udp_port=port,
                                     prefix_length=128)
        await asyncio.wait_for(server.start(), 10)
        try:
            self.assertEqual(await self.bbmd.registration(), (vmac(INSTANCE), 60))
            async with loopback_client() as client:
                address = await asyncio.wait_for(client.resolve_vmac(f"[::1]:{port}"), 10)
                for prop, expected in (
                    (P.BACNET_IPV6_MODE, V.enumerated(1)),
                    (P.FD_SUBSCRIPTION_LIFETIME, V.unsigned(60)),
                    (P.FD_BBMD_ADDRESS, V.application_data(self.bbmd.host_n_port())),
                ):
                    with self.subTest(property=prop):
                        read = await asyncio.wait_for(
                            client.read_property(address, PORT, prop), 10)
                        self.assertEqual(read, expected)
        finally:
            await server.stop()

    async def test_activate_changes_carries_a_written_lifetime_and_bbmd(self):
        second = FakeBbmd()
        port = free_udp6_port()
        server = BACnetServer(INSTANCE, transport="ipv6", ipv6_interface="::1", port=port,
                              registered_network_port=1,
                              foreign_device=ForeignDevice(self.bbmd.address, ttl=60),
                              on_reinitialize=lambda request: None)
        server.add_bip6_network_port(1, "B/IPv6 foreign", ip_address="::1", udp_port=port,
                                     prefix_length=128)
        await asyncio.wait_for(server.start(), 10)
        try:
            self.assertEqual(await self.bbmd.registration(), (vmac(INSTANCE), 60))
            async with loopback_client() as client:
                address = await asyncio.wait_for(client.resolve_vmac(f"[::1]:{port}"), 10)
                await client.write_property(address, PORT, P.FD_SUBSCRIPTION_LIFETIME,
                                            V.unsigned(120))
                self.assertEqual(await client.read_property(address, PORT, P.CHANGES_PENDING),
                                 V.boolean(True))
                await client.reinitialize_device(address, S.ACTIVATE_CHANGES)
                self.assertEqual(await client.read_property(address, PORT, P.CHANGES_PENDING),
                                 V.boolean(False))
                self.assertEqual(
                    await client.read_property(address, PORT, P.FD_SUBSCRIPTION_LIFETIME),
                    V.unsigned(120))
                # The next registration, sent at once, carries the new lifetime.
                self.assertEqual(await self.bbmd.registration(), (vmac(INSTANCE), 120))

                # A written BBMD moves the registration there.
                await client.write_property(address, PORT, P.FD_BBMD_ADDRESS,
                                            V.application_data(second.host_n_port()))
                await client.reinitialize_device(address, S.ACTIVATE_CHANGES)
                self.assertEqual(await second.registration(), (vmac(INSTANCE), 120))
        finally:
            await server.stop()
            second.close()

    async def test_a_foreign_client_registers_and_broadcasts_through_its_bbmd(self):
        foreign = ForeignDevice(self.bbmd.address, ttl=90)
        async with BACnetClient(transport="ipv6", ipv6_interface="::1", port=0,
                                ipv6_device_instance=CLIENT_INSTANCE,
                                foreign_device=foreign) as client:
            self.assertEqual(await self.bbmd.registration(), (vmac(CLIENT_INSTANCE), 90))
            # A Who-Is goes to the BBMD as a Distribute-Broadcast-To-Network.
            await client.who_is(INSTANCE, INSTANCE)
            datagram, _ = await self.bbmd.next(DISTRIBUTE_BROADCAST)
            self.assertEqual(datagram[4:7], vmac(CLIENT_INSTANCE))


if __name__ == "__main__":
    unittest.main()
